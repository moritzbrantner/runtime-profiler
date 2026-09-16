use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Component, Path};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use crate::browser_chromium::{
    BROWSER_RUNTIME_ARTIFACT, BROWSER_RUNTIME_MEDIA_TYPE, BROWSER_RUNTIME_SCHEMA_V1,
    BrowserRuntimeDocument, RAW_TRACE_ARTIFACT, RAW_TRACE_MEDIA_TYPE, TRACE_SUMMARY_ARTIFACT,
    TRACE_SUMMARY_MEDIA_TYPE, capture_browser_chromium, validate_runtime_metadata,
};
use crate::capture::{capture_metrics, ensure_not_interrupted};
use crate::chromium_trace::{CHROMIUM_TRACE_SUMMARY_SCHEMA_V1, ChromiumTraceSummary};
use crate::contract::{
    AgentGuidance, AgentObservation, ArtifactEntry, BundleManifest, Collector,
    ENVIRONMENT_FINGERPRINT_SCHEMA_LEGACY_V0, ENVIRONMENT_FINGERPRINT_SCHEMA_V1,
    ENVIRONMENT_SCHEMA_V1, EnvironmentDocument, GUIDANCE_SCHEMA_V1, HOTSPOTS_SCHEMA_V1,
    HotspotsDocument, MANIFEST_SCHEMA_V1, METRICS_SCHEMA_V1, MetricSummary, MetricsDocument,
    SCENARIO_EVIDENCE_SCHEMA_V1, ScenarioEvidence, SourceIdentity, Target, TargetEvidence,
    ValidationReport,
};
use crate::digest::{sha256_bytes, sha256_file};
use crate::native_perf::{
    COLLECTOR_ID as NATIVE_PERF_COLLECTOR_ID, RAW_REPORT_ARTIFACT, RAW_REPORT_MEDIA_TYPE,
    capture_native_perf,
};
use crate::scenario::load_scenario;

const REQUIRED_ARTIFACTS: [(&str, &str); 5] = [
    ("scenario.json", "application/json"),
    ("environment.json", "application/json"),
    ("metrics.json", "application/json"),
    ("hotspots.json", "application/json"),
    ("agent-guidance.json", "application/json"),
];
const OPTIONAL_ARTIFACTS: [(&str, &str); 4] = [
    (RAW_REPORT_ARTIFACT, RAW_REPORT_MEDIA_TYPE),
    (RAW_TRACE_ARTIFACT, RAW_TRACE_MEDIA_TYPE),
    (TRACE_SUMMARY_ARTIFACT, TRACE_SUMMARY_MEDIA_TYPE),
    (BROWSER_RUNTIME_ARTIFACT, BROWSER_RUNTIME_MEDIA_TYPE),
];
const MAX_BROWSER_GUIDANCE_PER_KIND: usize = 3;
const MAX_BROWSER_GUIDANCE_LABEL_CHARS: usize = 120;

#[derive(Serialize)]
struct EnvironmentFingerprintInput<'a> {
    schema_version: &'static str,
    operating_system: &'a str,
    architecture: &'a str,
    kernel_release: &'a Option<String>,
    logical_cpu_count: usize,
}

pub fn capture_bundle(scenario_path: &Path, output: &Path) -> Result<BundleManifest> {
    ensure!(
        !output.exists(),
        "refusing to overwrite existing bundle: {}",
        output.display()
    );
    ensure_not_interrupted()?;

    let scenario = load_scenario(scenario_path)?;
    ensure_not_interrupted()?;
    let environment = detect_environment()?;
    ensure_not_interrupted()?;

    let metrics = match scenario.scenario.target {
        Target::Command { .. } => capture_metrics(&scenario)?,
        Target::BrowserJourney { .. } => MetricsDocument {
            schema_version: METRICS_SCHEMA_V1.to_owned(),
            scenario_id: scenario.scenario.id.clone(),
            samples: Vec::new(),
            metrics: Vec::new(),
        },
    };
    ensure_not_interrupted()?;

    let (hotspots, raw_native_perf_report) = if scenario
        .scenario
        .collectors
        .contains(&Collector::NativePerf)
    {
        let capture = capture_native_perf(&scenario)?;
        (capture.hotspots, Some(capture.raw_report))
    } else {
        (
            HotspotsDocument {
                schema_version: HOTSPOTS_SCHEMA_V1.to_owned(),
                status: "not-collected".to_owned(),
                reason: "No native source-level profiler adapter was requested by this scenario"
                    .to_owned(),
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
            },
            None,
        )
    };
    ensure_not_interrupted()?;

    let browser_capture = if scenario
        .scenario
        .collectors
        .contains(&Collector::BrowserChromium)
    {
        Some(capture_browser_chromium(&scenario)?)
    } else {
        None
    };
    ensure_not_interrupted()?;

    let guidance = build_guidance(
        &metrics,
        &hotspots,
        browser_capture.as_ref().map(|capture| &capture.summary),
    );
    fs::create_dir_all(output)
        .with_context(|| format!("failed to create bundle directory: {}", output.display()))?;
    ensure_not_interrupted()?;

    write_json(&output.join("scenario.json"), &scenario.evidence())?;
    write_json(&output.join("environment.json"), &environment)?;
    write_json(&output.join("metrics.json"), &metrics)?;
    write_json(&output.join("hotspots.json"), &hotspots)?;
    write_json(&output.join("agent-guidance.json"), &guidance)?;
    if let Some(report) = &raw_native_perf_report {
        fs::write(output.join(RAW_REPORT_ARTIFACT), report).with_context(|| {
            format!(
                "failed to write native-perf report: {}",
                output.join(RAW_REPORT_ARTIFACT).display()
            )
        })?;
    }
    if let Some(capture) = &browser_capture {
        fs::write(output.join(RAW_TRACE_ARTIFACT), &capture.trace).with_context(|| {
            format!(
                "failed to write Chromium trace: {}",
                output.join(RAW_TRACE_ARTIFACT).display()
            )
        })?;
        write_json(&output.join(TRACE_SUMMARY_ARTIFACT), &capture.summary)?;
        write_json(&output.join(BROWSER_RUNTIME_ARTIFACT), &capture.runtime)?;
    }
    ensure_not_interrupted()?;

    let optional_count = usize::from(raw_native_perf_report.is_some())
        + if browser_capture.is_some() { 3 } else { 0 };
    let mut files = Vec::with_capacity(REQUIRED_ARTIFACTS.len() + optional_count);
    for (path, media_type) in REQUIRED_ARTIFACTS {
        ensure_not_interrupted()?;
        files.push(ArtifactEntry {
            path: path.to_owned(),
            media_type: media_type.to_owned(),
            sha256: sha256_file(&output.join(path))?,
        });
    }
    if raw_native_perf_report.is_some() {
        files.push(artifact_entry(
            output,
            RAW_REPORT_ARTIFACT,
            RAW_REPORT_MEDIA_TYPE,
        )?);
    }
    if browser_capture.is_some() {
        files.push(artifact_entry(
            output,
            RAW_TRACE_ARTIFACT,
            RAW_TRACE_MEDIA_TYPE,
        )?);
        files.push(artifact_entry(
            output,
            TRACE_SUMMARY_ARTIFACT,
            TRACE_SUMMARY_MEDIA_TYPE,
        )?);
        files.push(artifact_entry(
            output,
            BROWSER_RUNTIME_ARTIFACT,
            BROWSER_RUNTIME_MEDIA_TYPE,
        )?);
    }
    ensure_not_interrupted()?;

    let created_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis();
    let bundle_identity = format!(
        "{}:{}:{created_unix_ms}",
        scenario.digest, environment.fingerprint
    );
    let manifest = BundleManifest {
        schema_version: MANIFEST_SCHEMA_V1.to_owned(),
        bundle_id: sha256_bytes(bundle_identity.as_bytes()),
        created_unix_ms,
        scenario_id: scenario.scenario.id,
        scenario_digest: scenario.digest,
        environment_fingerprint_schema_version: environment
            .environment_fingerprint_schema_version
            .clone(),
        environment_fingerprint: environment.fingerprint,
        source: environment.source,
        files,
    };
    write_json(&output.join("manifest.json"), &manifest)?;
    ensure_not_interrupted()?;
    Ok(manifest)
}

fn artifact_entry(output: &Path, path: &str, media_type: &str) -> Result<ArtifactEntry> {
    Ok(ArtifactEntry {
        path: path.to_owned(),
        media_type: media_type.to_owned(),
        sha256: sha256_file(&output.join(path))?,
    })
}

pub fn validate_bundle(bundle: &Path) -> Result<ValidationReport> {
    let manifest_path = bundle.join("manifest.json");
    let manifest: BundleManifest = read_json(&manifest_path)?;
    ensure!(
        manifest.schema_version == MANIFEST_SCHEMA_V1,
        "unsupported bundle manifest schema: {}",
        manifest.schema_version
    );

    let mut diagnostics = Vec::new();
    let mut verified_files = 0;
    validate_artifact_set(&manifest, &mut diagnostics);
    for artifact in &manifest.files {
        if !is_safe_relative_path(&artifact.path) {
            diagnostics.push(format!("unsafe artifact path: {}", artifact.path));
            continue;
        }
        let path = bundle.join(&artifact.path);
        if !path.is_file() {
            diagnostics.push(format!("missing artifact: {}", artifact.path));
            continue;
        }
        let actual = sha256_file(&path)?;
        if actual != artifact.sha256 {
            diagnostics.push(format!("digest mismatch: {}", artifact.path));
            continue;
        }
        verified_files += 1;
    }

    let metrics: MetricsDocument = read_json(&bundle.join("metrics.json"))?;
    if metrics.schema_version != METRICS_SCHEMA_V1 {
        diagnostics.push(format!(
            "unsupported metrics schema: {}",
            metrics.schema_version
        ));
    }
    if metrics.scenario_id != manifest.scenario_id {
        diagnostics.push("metrics scenario id does not match manifest".to_owned());
    }

    let scenario: ScenarioEvidence = read_json(&bundle.join("scenario.json"))?;
    if scenario.schema_version != SCENARIO_EVIDENCE_SCHEMA_V1 {
        diagnostics.push(format!(
            "unsupported scenario evidence schema: {}",
            scenario.schema_version
        ));
    }
    if scenario.id != manifest.scenario_id || scenario.digest != manifest.scenario_digest {
        diagnostics.push("scenario evidence identity does not match manifest".to_owned());
    }

    let environment: EnvironmentDocument = read_json(&bundle.join("environment.json"))?;
    if environment.schema_version != ENVIRONMENT_SCHEMA_V1 {
        diagnostics.push(format!(
            "unsupported environment schema: {}",
            environment.schema_version
        ));
    }
    if environment.fingerprint != manifest.environment_fingerprint {
        diagnostics.push("environment fingerprint does not match manifest".to_owned());
    }
    if environment.environment_fingerprint_schema_version
        != manifest.environment_fingerprint_schema_version
    {
        diagnostics.push("environment fingerprint schema does not match manifest".to_owned());
    }
    if !matches!(
        manifest.environment_fingerprint_schema_version.as_str(),
        ENVIRONMENT_FINGERPRINT_SCHEMA_LEGACY_V0 | ENVIRONMENT_FINGERPRINT_SCHEMA_V1
    ) {
        diagnostics.push(format!(
            "unsupported environment fingerprint schema: {}",
            manifest.environment_fingerprint_schema_version
        ));
    }

    let guidance: AgentGuidance = read_json(&bundle.join("agent-guidance.json"))?;
    if guidance.schema_version != GUIDANCE_SCHEMA_V1 || guidance.scenario_id != manifest.scenario_id
    {
        diagnostics.push("agent guidance identity is incompatible with manifest".to_owned());
    }

    let hotspots: HotspotsDocument = read_json(&bundle.join("hotspots.json"))?;
    if hotspots.schema_version != HOTSPOTS_SCHEMA_V1 {
        diagnostics.push(format!(
            "unsupported hotspots schema: {}",
            hotspots.schema_version
        ));
    }
    validate_hotspot_artifacts(&scenario, &manifest, &hotspots, &mut diagnostics);
    validate_browser_artifacts(bundle, &scenario, &manifest, &metrics, &mut diagnostics)?;

    Ok(ValidationReport {
        schema_version: "runtime-profiler/validation/v1".to_owned(),
        bundle_id: manifest.bundle_id,
        valid: diagnostics.is_empty(),
        verified_files,
        diagnostics,
    })
}

fn validate_artifact_set(manifest: &BundleManifest, diagnostics: &mut Vec<String>) {
    let required: BTreeSet<&str> = REQUIRED_ARTIFACTS.iter().map(|(path, _)| *path).collect();
    let allowed: BTreeSet<&str> = REQUIRED_ARTIFACTS
        .iter()
        .chain(OPTIONAL_ARTIFACTS.iter())
        .map(|(path, _)| *path)
        .collect();
    let actual: BTreeSet<&str> = manifest
        .files
        .iter()
        .map(|artifact| artifact.path.as_str())
        .collect();

    if manifest.files.len() != actual.len() {
        diagnostics.push("manifest contains duplicate artifact paths".to_owned());
    }
    if !required.is_subset(&actual) {
        diagnostics.push("manifest is missing one or more required v1 artifacts".to_owned());
    }
    if !actual.is_subset(&allowed) {
        diagnostics.push("manifest contains an unsupported v1 artifact".to_owned());
    }
}

fn validate_hotspot_artifacts(
    scenario: &ScenarioEvidence,
    manifest: &BundleManifest,
    hotspots: &HotspotsDocument,
    diagnostics: &mut Vec<String>,
) {
    let native_requested = scenario.collectors.contains(&Collector::NativePerf);
    let raw_report_present = manifest
        .files
        .iter()
        .any(|artifact| artifact.path == RAW_REPORT_ARTIFACT);

    if native_requested {
        if hotspots.status != "collected"
            || hotspots.collector.as_deref() != Some(NATIVE_PERF_COLLECTOR_ID)
        {
            diagnostics.push(
                "native-perf scenario does not contain collected native-perf hotspot evidence"
                    .to_owned(),
            );
        }
        if !raw_report_present {
            diagnostics
                .push("native-perf scenario is missing its raw perf report artifact".to_owned());
        }
        if hotspots.tool_version.is_none()
            || hotspots.event.is_none()
            || hotspots.metric.is_none()
            || hotspots.unit.is_none()
            || hotspots.sample_period.is_none()
            || hotspots.symbolization_mode.is_none()
        {
            diagnostics.push(
                "native-perf hotspot evidence is missing collector comparability metadata"
                    .to_owned(),
            );
        }
        let toolchain_fields_present = [
            hotspots.target_toolchain_kind.is_some(),
            hotspots
                .target_toolchain_fingerprint_schema_version
                .is_some(),
            hotspots.target_toolchain_fingerprint.is_some(),
        ];
        if toolchain_fields_present.iter().any(|present| *present)
            && !toolchain_fields_present.iter().all(|present| *present)
        {
            diagnostics.push(
                "native-perf target toolchain identity is only partially recorded".to_owned(),
            );
        }
        if hotspots.sample_period == Some(0) {
            diagnostics.push("native-perf sample period must be positive".to_owned());
        }
    } else {
        if raw_report_present {
            diagnostics.push(
                "non-native scenario unexpectedly contains native-perf raw evidence".to_owned(),
            );
        }
        if hotspots.status == "collected" || hotspots.collector.is_some() {
            diagnostics.push(
                "non-native scenario unexpectedly claims collected native hotspot evidence"
                    .to_owned(),
            );
        }
    }
}

fn validate_browser_artifacts(
    bundle: &Path,
    scenario: &ScenarioEvidence,
    manifest: &BundleManifest,
    metrics: &MetricsDocument,
    diagnostics: &mut Vec<String>,
) -> Result<()> {
    let browser_requested = scenario.collectors.contains(&Collector::BrowserChromium);
    let browser_paths = [
        RAW_TRACE_ARTIFACT,
        TRACE_SUMMARY_ARTIFACT,
        BROWSER_RUNTIME_ARTIFACT,
    ];
    let browser_presence =
        browser_paths.map(|path| manifest.files.iter().any(|artifact| artifact.path == path));

    if browser_requested {
        if !matches!(scenario.target, TargetEvidence::BrowserJourney { .. }) {
            diagnostics
                .push("browser-chromium collector requires browser-journey evidence".to_owned());
        }
        if !browser_presence.iter().all(|present| *present) {
            diagnostics
                .push("browser journey is missing Chromium trace evidence artifacts".to_owned());
            return Ok(());
        }
        if !metrics.metrics.is_empty() || !metrics.samples.is_empty() {
            diagnostics.push(
                "browser journey must not relabel Playwright driver process measurements as application metrics"
                    .to_owned(),
            );
        }
        let summary: ChromiumTraceSummary = read_json(&bundle.join(TRACE_SUMMARY_ARTIFACT))?;
        if summary.schema_version != CHROMIUM_TRACE_SUMMARY_SCHEMA_V1 {
            diagnostics.push(format!(
                "unsupported Chromium trace summary schema: {}",
                summary.schema_version
            ));
        }
        let runtime: BrowserRuntimeDocument = read_json(&bundle.join(BROWSER_RUNTIME_ARTIFACT))?;
        if runtime.schema_version != BROWSER_RUNTIME_SCHEMA_V1 {
            diagnostics.push(format!(
                "unsupported browser runtime schema: {}",
                runtime.schema_version
            ));
        }
        if let Err(error) = validate_runtime_metadata(&runtime) {
            diagnostics.push(format!("invalid browser runtime metadata: {error}"));
        }
    } else if browser_presence.iter().any(|present| *present) {
        diagnostics
            .push("non-browser scenario unexpectedly contains browser trace evidence".to_owned());
    }
    Ok(())
}

pub fn summarize_bundle(bundle: &Path) -> Result<MetricsDocument> {
    let report = validate_bundle(bundle)?;
    ensure!(
        report.valid,
        "bundle validation failed: {}",
        report.diagnostics.join("; ")
    );
    read_json(&bundle.join("metrics.json"))
}

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

fn build_guidance(
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
    let mut chars = value
        .chars()
        .map(|ch| if ch.is_control() || ch == '`' { ' ' } else { ch });
    let mut result: String = chars
        .by_ref()
        .take(MAX_BROWSER_GUIDANCE_LABEL_CHARS)
        .collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

fn detect_environment() -> Result<EnvironmentDocument> {
    let source = SourceIdentity {
        git_sha: command_output("git", &["rev-parse", "HEAD"]),
        dirty: command_output("git", &["status", "--porcelain"]).map(|output| !output.is_empty()),
    };
    let mut environment = EnvironmentDocument {
        schema_version: ENVIRONMENT_SCHEMA_V1.to_owned(),
        environment_fingerprint_schema_version: ENVIRONMENT_FINGERPRINT_SCHEMA_V1.to_owned(),
        fingerprint: String::new(),
        operating_system: env::consts::OS.to_owned(),
        architecture: env::consts::ARCH.to_owned(),
        kernel_release: command_output("uname", &["-r"]),
        logical_cpu_count: thread_count(),
        source,
    };
    environment.fingerprint = environment_fingerprint(&environment)?;
    Ok(environment)
}

fn environment_fingerprint(environment: &EnvironmentDocument) -> Result<String> {
    let input = EnvironmentFingerprintInput {
        schema_version: ENVIRONMENT_FINGERPRINT_SCHEMA_V1,
        operating_system: &environment.operating_system,
        architecture: &environment.architecture,
        kernel_release: &environment.kernel_release,
        logical_cpu_count: environment.logical_cpu_count,
    };
    let normalized = serde_json::to_vec(&input).context("failed to fingerprint environment")?;
    Ok(sha256_bytes(&normalized))
}

fn thread_count() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn is_safe_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("failed to serialize artifact: {}", path.display()))?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("failed to write artifact: {}", path.display()))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read JSON artifact: {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid JSON artifact: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chromium_trace::{
        ChromiumBoundaryMarker, ChromiumHotPath, ChromiumHotPathFrame, ChromiumLongTask,
        ChromiumMainThread,
    };

    fn test_environment(source: SourceIdentity) -> EnvironmentDocument {
        EnvironmentDocument {
            schema_version: ENVIRONMENT_SCHEMA_V1.to_owned(),
            environment_fingerprint_schema_version: ENVIRONMENT_FINGERPRINT_SCHEMA_V1.to_owned(),
            fingerprint: String::new(),
            operating_system: "linux".to_owned(),
            architecture: "x86_64".to_owned(),
            kernel_release: Some("6.12.0".to_owned()),
            logical_cpu_count: 8,
            source,
        }
    }

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
    fn rejects_unsafe_artifact_paths() {
        assert!(is_safe_relative_path("metrics.json"));
        assert!(!is_safe_relative_path("../metrics.json"));
        assert!(!is_safe_relative_path("/tmp/metrics.json"));
        assert!(!is_safe_relative_path("nested/../metrics.json"));
    }

    #[test]
    fn environment_fingerprint_ignores_source_identity() {
        let baseline = test_environment(SourceIdentity {
            git_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned()),
            dirty: Some(false),
        });
        let candidate = test_environment(SourceIdentity {
            git_sha: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned()),
            dirty: Some(true),
        });

        assert_eq!(
            environment_fingerprint(&baseline).expect("baseline fingerprint"),
            environment_fingerprint(&candidate).expect("candidate fingerprint")
        );
    }

    #[test]
    fn environment_fingerprint_changes_with_execution_environment() {
        let baseline = test_environment(SourceIdentity {
            git_sha: None,
            dirty: None,
        });
        let mut candidate = baseline.clone();
        candidate.logical_cpu_count = 16;

        assert_ne!(
            environment_fingerprint(&baseline).expect("baseline fingerprint"),
            environment_fingerprint(&candidate).expect("candidate fingerprint")
        );
    }

    #[test]
    fn legacy_bundle_documents_get_an_explicit_fingerprint_schema() {
        let legacy: EnvironmentDocument = serde_json::from_str(
            r#"{
  "schema_version": "runtime-profiler/environment/v1",
  "fingerprint": "legacy-digest",
  "operating_system": "linux",
  "architecture": "x86_64",
  "kernel_release": "6.12.0",
  "logical_cpu_count": 8,
  "source": { "git_sha": null, "dirty": null }
}"#,
        )
        .expect("legacy environment document");

        assert_eq!(
            legacy.environment_fingerprint_schema_version,
            ENVIRONMENT_FINGERPRINT_SCHEMA_LEGACY_V0
        );

        let legacy_manifest: BundleManifest = serde_json::from_str(
            r#"{
  "schema_version": "runtime-profiler/bundle-manifest/v1",
  "bundle_id": "legacy-bundle",
  "created_unix_ms": 1,
  "scenario_id": "legacy-scenario",
  "scenario_digest": "legacy-scenario-digest",
  "environment_fingerprint": "legacy-digest",
  "source": { "git_sha": null, "dirty": null },
  "files": []
}"#,
        )
        .expect("legacy manifest");

        assert_eq!(
            legacy_manifest.environment_fingerprint_schema_version,
            ENVIRONMENT_FINGERPRINT_SCHEMA_LEGACY_V0
        );
    }

    #[test]
    fn legacy_hotspots_document_remains_readable() {
        let hotspots: HotspotsDocument = serde_json::from_str(
            r#"{
  "schema_version": "runtime-profiler/hotspots/v1",
  "status": "not-collected",
  "reason": "legacy",
  "hotspots": []
}"#,
        )
        .expect("legacy hotspots");

        assert_eq!(hotspots.total_samples, 0);
        assert!(hotspots.collector.is_none());
        assert!(hotspots.sample_period.is_none());
        assert!(hotspots.target_toolchain_fingerprint.is_none());
    }

    #[test]
    fn browser_guidance_is_bounded_sanitized_and_normalized() {
        let guidance = build_guidance(&empty_metrics(), &empty_hotspots(), Some(&browser_summary()));

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
        assert!(
            guidance
                .observations
                .iter()
                .any(|observation| observation.summary.contains("instrumented-section duration"))
        );
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
