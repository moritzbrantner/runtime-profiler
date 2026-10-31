use std::collections::BTreeSet;
use std::path::{Component, Path};

use anyhow::{Result, ensure};

use super::{OPTIONAL_ARTIFACTS, REQUIRED_ARTIFACTS, read_json};
use crate::browser_chromium::{
    BROWSER_RUNTIME_ARTIFACT, BROWSER_RUNTIME_SCHEMA_V1, BrowserRuntimeDocument,
    RAW_TRACE_ARTIFACT, TRACE_SUMMARY_ARTIFACT, validate_runtime_metadata,
};
use crate::chromium_trace::{CHROMIUM_TRACE_SUMMARY_SCHEMA_V1, ChromiumTraceSummary};
use crate::contract::{
    AgentGuidance, BundleManifest, Collector, ENVIRONMENT_FINGERPRINT_SCHEMA_LEGACY_V0,
    ENVIRONMENT_FINGERPRINT_SCHEMA_V1, ENVIRONMENT_SCHEMA_V1, EnvironmentDocument,
    GUIDANCE_SCHEMA_V1, HOTSPOTS_SCHEMA_V1, HotspotsDocument, MANIFEST_SCHEMA_V1,
    METRICS_SCHEMA_V1, MetricsDocument, SCENARIO_EVIDENCE_SCHEMA_V1, ScenarioEvidence,
    TargetEvidence, ValidationReport,
};
use crate::digest::sha256_file;
use crate::native_perf::{COLLECTOR_ID as NATIVE_PERF_COLLECTOR_ID, RAW_REPORT_ARTIFACT};

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
    let http_present = manifest
        .files
        .iter()
        .any(|artifact| artifact.path == crate::http_workload::ARTIFACT);
    if scenario.collectors.contains(&Collector::HttpCurl) {
        if !http_present {
            diagnostics.push("HTTP workload is missing its measurement artifact".to_owned());
        } else {
            let evidence: crate::http_workload::HttpEvidence =
                read_json(&bundle.join(crate::http_workload::ARTIFACT))?;
            if let Err(error) =
                crate::http_workload::validate_evidence(&scenario, &evidence, &metrics)
            {
                diagnostics.push(format!("invalid HTTP workload evidence: {error}"));
            }
        }
    } else if http_present || matches!(scenario.target, TargetEvidence::HttpWorkload { .. }) {
        diagnostics.push("HTTP workload evidence requires the http-curl collector".to_owned());
    }

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

fn is_safe_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_artifact_paths() {
        assert!(is_safe_relative_path("metrics.json"));
        assert!(!is_safe_relative_path("../metrics.json"));
        assert!(!is_safe_relative_path("/tmp/metrics.json"));
        assert!(!is_safe_relative_path("nested/../metrics.json"));
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
}
