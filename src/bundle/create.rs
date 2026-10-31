use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};

use super::environment::detect_environment;
use super::guidance::build_guidance;
use super::{REQUIRED_ARTIFACTS, write_json};
use crate::browser_chromium::{
    BROWSER_RUNTIME_ARTIFACT, BROWSER_RUNTIME_MEDIA_TYPE, RAW_TRACE_ARTIFACT, RAW_TRACE_MEDIA_TYPE,
    TRACE_SUMMARY_ARTIFACT, TRACE_SUMMARY_MEDIA_TYPE, capture_browser_chromium,
};
use crate::capture::{capture_metrics, ensure_not_interrupted};
use crate::contract::{
    ArtifactEntry, BundleManifest, Collector, HOTSPOTS_SCHEMA_V1, HotspotsDocument,
    MANIFEST_SCHEMA_V1, METRICS_SCHEMA_V1, MetricsDocument, Target,
};
use crate::digest::{sha256_bytes, sha256_file};
use crate::native_perf::{RAW_REPORT_ARTIFACT, RAW_REPORT_MEDIA_TYPE, capture_native_perf};
use crate::scenario::load_scenario;

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

    let (metrics, http_capture) = match scenario.scenario.target {
        Target::Command { .. } => (capture_metrics(&scenario)?, None),
        Target::HttpWorkload { .. } => {
            let (metrics, evidence) = crate::http_workload::capture_http(&scenario)?;
            (metrics, Some(evidence))
        }
        Target::BrowserJourney { .. } => (
            MetricsDocument {
                schema_version: METRICS_SCHEMA_V1.to_owned(),
                scenario_id: scenario.scenario.id.clone(),
                samples: Vec::new(),
                metrics: Vec::new(),
            },
            None,
        ),
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
    if let Some(evidence) = &http_capture {
        write_json(&output.join(crate::http_workload::ARTIFACT), evidence)?;
    }
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

    let optional_count = usize::from(http_capture.is_some())
        + usize::from(raw_native_perf_report.is_some())
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
    if http_capture.is_some() {
        files.push(artifact_entry(
            output,
            crate::http_workload::ARTIFACT,
            "application/json",
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
