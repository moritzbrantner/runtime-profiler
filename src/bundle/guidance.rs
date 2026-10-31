use std::path::Path;

use anyhow::{Result, ensure};

use super::read_json;
use super::validate::validate_bundle;
use crate::browser_chromium::{BROWSER_RUNTIME_ARTIFACT, TRACE_SUMMARY_ARTIFACT};
use crate::chromium_trace::ChromiumTraceSummary;
use crate::contract::{
    AgentGuidance, AgentObservation, GUIDANCE_SCHEMA_V1, HotspotsDocument, MetricSummary,
    MetricsDocument,
};

const MAX_BROWSER_GUIDANCE_PER_KIND: usize = 3;
const MAX_BROWSER_GUIDANCE_LABEL_CHARS: usize = 120;

pub fn render_agent_guidance(bundle: &Path) -> Result<String> {
    let report = validate_bundle(bundle)?;
    ensure!(
        report.valid,
        "bundle validation failed: {}",
        report.diagnostics.join("; ")
    );
    let guidance: AgentGuidance = read_json(&bundle.join("agent-guidance.json"))?;
    let mut output = format!("# Runtime evidence: {}\n\n", guidance.scenario_id);
    output.push_str("## Observations\n\n");
    for observation in guidance.observations {
        output.push_str(&format!(
            "- {} (`{}`)\n",
            observation.summary, observation.evidence_ref
        ));
    }
    output.push_str("\n## Constraints\n\n");
    for constraint in guidance.constraints {
        output.push_str(&format!("- {constraint}\n"));
    }
    Ok(output)
}

pub(super) fn build_guidance(
    metrics: &MetricsDocument,
    hotspots: &HotspotsDocument,
    browser_summary: Option<&ChromiumTraceSummary>,
) -> AgentGuidance {
    let mut observations: Vec<AgentObservation> = metrics
        .metrics
        .iter()
        .map(|metric| AgentObservation {
            id: metric.id.clone(),
            summary: summarize_metric(metric),
            evidence_ref: format!("metrics.json#{}", metric.id),
        })
        .collect();

    if hotspots.status == "collected" {
        observations.extend(hotspots.hotspots.iter().take(5).map(|hotspot| {
            let location = match (&hotspot.source_file, hotspot.line) {
                (Some(file), Some(line)) => format!(" at {file}:{line}"),
                (Some(file), None) => format!(" in {file}"),
                _ => String::new(),
            };
            AgentObservation {
                id: hotspot.id.clone(),
                summary: format!(
                    "Observed {} hotspot `{}`{} with weight {} {} across {} samples ({})",
                    hotspots.collector.as_deref().unwrap_or("profiler"),
                    hotspot.symbol,
                    location,
                    hotspot.weight,
                    hotspot.unit,
                    hotspot.samples,
                    hotspot.confidence
                ),
                evidence_ref: hotspot.evidence_ref.clone(),
            }
        }));
    }

    if let Some(summary) = browser_summary {
        observations.extend(
            summary
                .long_tasks
                .iter()
                .take(MAX_BROWSER_GUIDANCE_PER_KIND)
                .map(|task| AgentObservation {
                    id: task.id.clone(),
                    summary: format!(
                        "Observed Chromium long task `{}` lasting {} us on the renderer main thread ({})",
                        bounded_guidance_label(&task.name),
                        task.duration_us,
                        task.runtime_kind
                    ),
                    evidence_ref: task.evidence_ref.clone(),
                }),
        );
        observations.extend(
            summary
                .hot_paths
                .iter()
                .take(MAX_BROWSER_GUIDANCE_PER_KIND)
                .map(|path| {
                    let leaf = path
                        .frames
                        .last()
                        .map(|frame| bounded_guidance_label(&frame.name))
                        .unwrap_or_else(|| "unknown".to_owned());
                    AgentObservation {
                        id: path.id.clone(),
                        summary: format!(
                            "Observed Chromium hot path ending at `{leaf}` with {} occurrences, {} us total inclusive duration, and {} us maximum inclusive duration ({})",
                            path.occurrences,
                            path.total_duration_us,
                            path.max_duration_us,
                            path.leaf_runtime_kind
                        ),
                        evidence_ref: path.evidence_ref.clone(),
                    }
                }),
        );
        observations.extend(
            summary
                .boundary_markers
                .iter()
                .take(MAX_BROWSER_GUIDANCE_PER_KIND)
                .map(|marker| AgentObservation {
                    id: marker.id.clone(),
                    summary: format!(
                        "Observed Chromium {} boundary marker `{}` {} times with {} us total instrumented-section duration and {} us maximum instrumented-section duration",
                        marker.direction,
                        bounded_guidance_label(&marker.label),
                        marker.occurrences,
                        marker.total_duration_us,
                        marker.max_duration_us
                    ),
                    evidence_ref: marker.evidence_ref.clone(),
                }),
        );
    }

    let mut constraints = vec![
        "This bundle describes one version; it does not establish improvement or regression."
            .to_owned(),
        "Use Moonlight to compare a baseline and candidate with matching scenario digests."
            .to_owned(),
        "Treat cross-environment comparisons as inconclusive unless policy explicitly permits them."
            .to_owned(),
        "Profiler hotspots are sampled-cost correlations, not proof of semantic root cause."
            .to_owned(),
    ];
    let mut evidence_refs = vec![
        "manifest.json".to_owned(),
        "environment.json".to_owned(),
        "metrics.json".to_owned(),
        "hotspots.json".to_owned(),
    ];
    if browser_summary.is_some() {
        constraints.push(
            "Chromium trace summaries are descriptive inclusive timing evidence; explicit JS/WASM boundary markers measure instrumented sections rather than inferred marshaling cost."
                .to_owned(),
        );
        evidence_refs.push(TRACE_SUMMARY_ARTIFACT.to_owned());
        evidence_refs.push(BROWSER_RUNTIME_ARTIFACT.to_owned());
    }

    AgentGuidance {
        schema_version: GUIDANCE_SCHEMA_V1.to_owned(),
        scenario_id: metrics.scenario_id.clone(),
        observations,
        constraints,
        evidence_refs,
    }
}

fn summarize_metric(metric: &MetricSummary) -> String {
    format!(
        "{}: median {:.3} {}, p95 {:.3} {}, mean {:.3} {} across {} samples",
        metric.id,
        metric.statistics.median,
        metric.unit,
        metric.statistics.p95,
        metric.unit,
        metric.statistics.mean,
        metric.unit,
        metric.statistics.sample_count
    )
}

fn bounded_guidance_label(value: &str) -> String {
    let mut chars = value.chars().map(|ch| {
        if ch.is_control() || ch == '`' {
            ' '
        } else {
            ch
        }
    });
    let mut result: String = chars
        .by_ref()
        .take(MAX_BROWSER_GUIDANCE_LABEL_CHARS)
        .collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_chromium::RAW_TRACE_ARTIFACT;
    use crate::chromium_trace::{
        CHROMIUM_TRACE_SUMMARY_SCHEMA_V1, ChromiumBoundaryMarker, ChromiumHotPath,
        ChromiumHotPathFrame, ChromiumLongTask, ChromiumMainThread,
    };
    use crate::contract::{HOTSPOTS_SCHEMA_V1, METRICS_SCHEMA_V1};

    fn empty_metrics() -> MetricsDocument {
        MetricsDocument {
            schema_version: METRICS_SCHEMA_V1.to_owned(),
            scenario_id: "browser-scenario".to_owned(),
            samples: Vec::new(),
            metrics: Vec::new(),
        }
    }

    fn empty_hotspots() -> HotspotsDocument {
        HotspotsDocument {
            schema_version: HOTSPOTS_SCHEMA_V1.to_owned(),
            status: "not-collected".to_owned(),
            reason: "not requested".to_owned(),
            collector: None,
            tool_version: None,
            event: None,
            metric: None,
            unit: None,
            sample_period: None,
            symbolization_mode: None,
            target_toolchain_kind: None,
            target_toolchain_fingerprint_schema_version: None,
            target_toolchain_fingerprint: None,
            total_weight: 0,
            total_samples: 0,
            truncated: false,
            hotspots: Vec::new(),
        }
    }

    fn browser_summary() -> ChromiumTraceSummary {
        ChromiumTraceSummary {
            schema_version: CHROMIUM_TRACE_SUMMARY_SCHEMA_V1.to_owned(),
            trace_event_count: 10,
            main_thread: ChromiumMainThread {
                process_id: 1,
                thread_id: 2,
                name: "CrRendererMain".to_owned(),
            },
            top_level_task_count: 4,
            top_level_duration_us: 400_000,
            long_task_count: 4,
            long_task_total_duration_us: 400_000,
            longest_task_us: Some(130_000),
            long_tasks_truncated: false,
            long_tasks: (0_u64..4)
                .map(|index| ChromiumLongTask {
                    id: format!("long-{index}"),
                    name: if index == 0 {
                        "Task\n`untrusted`".to_owned()
                    } else {
                        format!("Task-{index}")
                    },
                    category: "toplevel".to_owned(),
                    start_us: index * 100_000,
                    duration_us: 100_000 + index,
                    runtime_kind: "javascript".to_owned(),
                    evidence_ref: format!("chromium-trace-summary.json#long-{index}"),
                })
                .collect(),
            hot_path_count: 4,
            hot_paths_truncated: false,
            hot_path_depth_truncated: false,
            hot_paths: (0_u64..4)
                .map(|index| ChromiumHotPath {
                    id: format!("hot-{index}"),
                    frames: vec![ChromiumHotPathFrame {
                        name: format!("Leaf-{index}"),
                        category: "v8".to_owned(),
                    }],
                    leaf_runtime_kind: "javascript".to_owned(),
                    total_duration_us: 50_000 + index,
                    max_duration_us: 20_000 + index,
                    occurrences: 2 + index,
                    evidence_ref: format!("chromium-trace-summary.json#hot-{index}"),
                })
                .collect(),
            runtime_attribution: Vec::new(),
            boundary_marker_count: 4,
            boundary_markers_truncated: false,
            boundary_markers: (0_u64..4)
                .map(|index| ChromiumBoundaryMarker {
                    id: format!("boundary-{index}"),
                    direction: "js-to-wasm".to_owned(),
                    label: format!("boundary-{index}"),
                    total_duration_us: 1_000 + index,
                    max_duration_us: 500 + index,
                    occurrences: 1 + index,
                    evidence_ref: format!("chromium-trace-summary.json#boundary-{index}"),
                })
                .collect(),
            limitations: Vec::new(),
        }
    }

    #[test]
    fn browser_guidance_is_bounded_sanitized_and_normalized() {
        let guidance = build_guidance(
            &empty_metrics(),
            &empty_hotspots(),
            Some(&browser_summary()),
        );

        assert_eq!(guidance.observations.len(), 9);
        assert_eq!(
            guidance
                .observations
                .iter()
                .filter(|observation| observation.id.starts_with("long-"))
                .count(),
            MAX_BROWSER_GUIDANCE_PER_KIND
        );
        assert!(
            guidance
                .observations
                .iter()
                .all(|observation| observation.evidence_ref.starts_with(TRACE_SUMMARY_ARTIFACT))
        );
        assert!(
            guidance
                .observations
                .iter()
                .all(|observation| !observation.summary.contains('\n'))
        );
        assert!(
            guidance
                .observations
                .iter()
                .all(|observation| !observation.summary.contains("`untrusted`"))
        );
        assert!(guidance.observations.iter().any(|observation| {
            observation
                .summary
                .contains("instrumented-section duration")
        }));
        assert!(
            guidance
                .evidence_refs
                .contains(&TRACE_SUMMARY_ARTIFACT.to_owned())
        );
        assert!(
            guidance
                .evidence_refs
                .contains(&BROWSER_RUNTIME_ARTIFACT.to_owned())
        );
        assert!(
            !guidance
                .evidence_refs
                .contains(&RAW_TRACE_ARTIFACT.to_owned())
        );
    }
}
