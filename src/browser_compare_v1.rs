use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::browser_chromium::{BROWSER_RUNTIME_ARTIFACT, BrowserRuntimeDocument};
use crate::bundle::validate_bundle;
use crate::chromium_trace::ChromiumTraceSummary;
use crate::contract::{BundleManifest, SourceIdentity};

pub const BROWSER_COMPARABILITY_SCHEMA_V1: &str = "runtime-profiler/browser-comparability/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserComparabilityStatus {
    Comparable,
    Incomparable,
    InsufficientEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserComparabilityReport {
    pub schema_version: String,
    pub reference_bundle_id: String,
    pub candidate_bundle_id: String,
    pub reference_source: SourceIdentity,
    pub candidate_source: SourceIdentity,
    pub status: BrowserComparabilityStatus,
    pub reasons: Vec<String>,
    pub notes: Vec<String>,
}

pub fn compare_browser_bundles(
    reference: &Path,
    candidate: &Path,
) -> Result<BrowserComparabilityReport> {
    validate(reference, "reference")?;
    validate(candidate, "candidate")?;

    let reference_manifest: BundleManifest = read_json(&reference.join("manifest.json"))?;
    let candidate_manifest: BundleManifest = read_json(&candidate.join("manifest.json"))?;
    let reference_runtime: Option<BrowserRuntimeDocument> =
        read_optional_json(&reference.join(BROWSER_RUNTIME_ARTIFACT))?;
    let candidate_runtime: Option<BrowserRuntimeDocument> =
        read_optional_json(&candidate.join(BROWSER_RUNTIME_ARTIFACT))?;
    let reference_summary: Option<ChromiumTraceSummary> =
        read_optional_json(&reference.join("chromium-trace-summary.json"))?;
    let candidate_summary: Option<ChromiumTraceSummary> =
        read_optional_json(&candidate.join("chromium-trace-summary.json"))?;

    let assessment = assess_recorded_identity(
        &reference_manifest,
        &candidate_manifest,
        reference_runtime.as_ref(),
        candidate_runtime.as_ref(),
        reference_summary.as_ref(),
        candidate_summary.as_ref(),
    );

    Ok(BrowserComparabilityReport {
        schema_version: BROWSER_COMPARABILITY_SCHEMA_V1.to_owned(),
        reference_bundle_id: reference_manifest.bundle_id,
        candidate_bundle_id: candidate_manifest.bundle_id,
        reference_source: reference_manifest.source,
        candidate_source: candidate_manifest.source,
        status: assessment.status,
        reasons: assessment.reasons,
        notes: vec![
            "Browser comparability is descriptive evidence only; it does not produce a performance or release verdict.".to_owned(),
            "Source revisions may differ. Scenario, journey, execution environment, browser/runtime, tracing, and profiler-normalization identity must remain compatible.".to_owned(),
            "Browser bundles captured before journey/adapter/normalizer digests were recorded remain valid evidence but are insufficient for strict comparison.".to_owned(),
            "A changed journey digest is a workload change, not a candidate performance result.".to_owned(),
        ],
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Assessment {
    status: BrowserComparabilityStatus,
    reasons: Vec<String>,
}

fn assess_recorded_identity(
    reference_manifest: &BundleManifest,
    candidate_manifest: &BundleManifest,
    reference_runtime: Option<&BrowserRuntimeDocument>,
    candidate_runtime: Option<&BrowserRuntimeDocument>,
    reference_summary: Option<&ChromiumTraceSummary>,
    candidate_summary: Option<&ChromiumTraceSummary>,
) -> Assessment {
    let mut mismatches = Vec::new();
    let mut missing = Vec::new();

    compare_required(
        "scenario id",
        Some(reference_manifest.scenario_id.as_str()),
        Some(candidate_manifest.scenario_id.as_str()),
        &mut mismatches,
        &mut missing,
    );
    compare_required(
        "scenario digest",
        Some(reference_manifest.scenario_digest.as_str()),
        Some(candidate_manifest.scenario_digest.as_str()),
        &mut mismatches,
        &mut missing,
    );
    compare_required(
        "environment fingerprint schema",
        Some(
            reference_manifest
                .environment_fingerprint_schema_version
                .as_str(),
        ),
        Some(
            candidate_manifest
                .environment_fingerprint_schema_version
                .as_str(),
        ),
        &mut mismatches,
        &mut missing,
    );
    compare_required(
        "environment fingerprint",
        Some(reference_manifest.environment_fingerprint.as_str()),
        Some(candidate_manifest.environment_fingerprint.as_str()),
        &mut mismatches,
        &mut missing,
    );

    match (reference_runtime, candidate_runtime) {
        (Some(reference), Some(candidate)) => {
            compare_runtime_identity(reference, candidate, &mut mismatches, &mut missing)
        }
        _ => {
            missing.push("browser runtime evidence is missing from one or both bundles".to_owned())
        }
    }

    match (reference_summary, candidate_summary) {
        (Some(reference), Some(candidate)) => compare_required(
            "Chromium trace summary schema",
            Some(reference.schema_version.as_str()),
            Some(candidate.schema_version.as_str()),
            &mut mismatches,
            &mut missing,
        ),
        _ => missing.push("Chromium trace summary is missing from one or both bundles".to_owned()),
    }

    if !mismatches.is_empty() {
        mismatches.extend(missing);
        return Assessment {
            status: BrowserComparabilityStatus::Incomparable,
            reasons: mismatches,
        };
    }
    if !missing.is_empty() {
        return Assessment {
            status: BrowserComparabilityStatus::InsufficientEvidence,
            reasons: missing,
        };
    }

    Assessment {
        status: BrowserComparabilityStatus::Comparable,
        reasons: Vec::new(),
    }
}

fn compare_runtime_identity(
    reference: &BrowserRuntimeDocument,
    candidate: &BrowserRuntimeDocument,
    mismatches: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    for (label, reference, candidate) in [
        (
            "browser runtime schema",
            Some(reference.schema_version.as_str()),
            Some(candidate.schema_version.as_str()),
        ),
        (
            "browser adapter version",
            Some(reference.adapter_version.as_str()),
            Some(candidate.adapter_version.as_str()),
        ),
        (
            "browser adapter digest",
            reference.adapter_digest.as_deref(),
            candidate.adapter_digest.as_deref(),
        ),
        (
            "Chromium trace normalizer digest",
            reference.normalizer_digest.as_deref(),
            candidate.normalizer_digest.as_deref(),
        ),
        (
            "browser journey digest",
            reference.journey_digest.as_deref(),
            candidate.journey_digest.as_deref(),
        ),
        (
            "Node version",
            Some(reference.node_version.as_str()),
            Some(candidate.node_version.as_str()),
        ),
        (
            "Playwright version",
            Some(reference.playwright_version.as_str()),
            Some(candidate.playwright_version.as_str()),
        ),
        (
            "browser name",
            Some(reference.browser_name.as_str()),
            Some(candidate.browser_name.as_str()),
        ),
        (
            "browser version",
            Some(reference.browser_version.as_str()),
            Some(candidate.browser_version.as_str()),
        ),
    ] {
        compare_required(label, reference, candidate, mismatches, missing);
    }

    let reference_viewport = format!("{}x{}", reference.viewport.width, reference.viewport.height);
    let candidate_viewport = format!("{}x{}", candidate.viewport.width, candidate.viewport.height);
    compare_required(
        "browser viewport",
        Some(reference_viewport.as_str()),
        Some(candidate_viewport.as_str()),
        mismatches,
        missing,
    );

    let reference_categories = reference.trace_categories.join("\u{1f}");
    let candidate_categories = candidate.trace_categories.join("\u{1f}");
    compare_required(
        "Chromium trace categories",
        Some(reference_categories.as_str()),
        Some(candidate_categories.as_str()),
        mismatches,
        missing,
    );
}

fn compare_required(
    label: &str,
    reference: Option<&str>,
    candidate: Option<&str>,
    mismatches: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    match (
        reference.filter(|value| !value.is_empty()),
        candidate.filter(|value| !value.is_empty()),
    ) {
        (Some(reference), Some(candidate)) if reference == candidate => {}
        (Some(reference), Some(candidate)) => mismatches.push(format!(
            "{label} differs (reference={reference:?}, candidate={candidate:?})"
        )),
        _ => missing.push(format!("{label} is missing from one or both bundles")),
    }
}

fn validate(bundle: &Path, label: &str) -> Result<()> {
    let report = validate_bundle(bundle)
        .with_context(|| format!("failed to validate {label} bundle: {}", bundle.display()))?;
    ensure!(
        report.valid,
        "{label} bundle is invalid: {}",
        report.diagnostics.join("; ")
    );
    Ok(())
}

fn read_optional_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.is_file() {
        return Ok(None);
    }
    read_json(path).map(Some)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_chromium::{BrowserRuntimeDocument, BrowserViewport};
    use crate::chromium_trace::{
        CHROMIUM_TRACE_SUMMARY_SCHEMA_V1, ChromiumMainThread, ChromiumTraceSummary,
    };

    fn manifest() -> BundleManifest {
        BundleManifest {
            schema_version: "runtime-profiler/bundle-manifest/v1".to_owned(),
            bundle_id: "bundle".to_owned(),
            created_unix_ms: 1,
            scenario_id: "scenario".to_owned(),
            scenario_digest: "scenario-digest".to_owned(),
            environment_fingerprint_schema_version: "runtime-profiler/environment-fingerprint/v1"
                .to_owned(),
            environment_fingerprint: "environment".to_owned(),
            source: SourceIdentity {
                git_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned()),
                dirty: Some(false),
            },
            files: Vec::new(),
        }
    }

    fn runtime() -> BrowserRuntimeDocument {
        BrowserRuntimeDocument {
            schema_version: "runtime-profiler/browser-runtime/v1".to_owned(),
            adapter_version: "runtime-profiler/playwright-driver/v1".to_owned(),
            adapter_digest: Some(format!("sha256:{}", "a".repeat(64))),
            normalizer_digest: Some(format!("sha256:{}", "b".repeat(64))),
            journey_digest: Some(format!("sha256:{}", "c".repeat(64))),
            node_version: "v24.0.0".to_owned(),
            playwright_version: "1.58.0".to_owned(),
            browser_name: "chromium".to_owned(),
            browser_version: "140.0.0".to_owned(),
            viewport: BrowserViewport {
                width: 1280,
                height: 720,
            },
            trace_categories: vec![
                "devtools.timeline".to_owned(),
                "v8".to_owned(),
                "v8.execute".to_owned(),
                "blink.user_timing".to_owned(),
            ],
        }
    }

    fn summary() -> ChromiumTraceSummary {
        ChromiumTraceSummary {
            schema_version: CHROMIUM_TRACE_SUMMARY_SCHEMA_V1.to_owned(),
            trace_event_count: 0,
            main_thread: ChromiumMainThread {
                process_id: 1,
                thread_id: 2,
                name: "CrRendererMain".to_owned(),
            },
            top_level_task_count: 0,
            top_level_duration_us: 0,
            long_task_count: 0,
            long_task_total_duration_us: 0,
            longest_task_us: None,
            long_tasks_truncated: false,
            long_tasks: Vec::new(),
            hot_path_count: 0,
            hot_paths_truncated: false,
            hot_path_depth_truncated: false,
            hot_paths: Vec::new(),
            runtime_attribution: Vec::new(),
            boundary_marker_count: 0,
            boundary_markers_truncated: false,
            boundary_markers: Vec::new(),
            limitations: Vec::new(),
        }
    }

    #[test]
    fn matching_complete_identity_is_comparable() {
        let assessment = assess_recorded_identity(
            &manifest(),
            &manifest(),
            Some(&runtime()),
            Some(&runtime()),
            Some(&summary()),
            Some(&summary()),
        );
        assert_eq!(assessment.status, BrowserComparabilityStatus::Comparable);
        assert!(assessment.reasons.is_empty());
    }

    #[test]
    fn missing_new_digest_is_insufficient_instead_of_guessed_comparable() {
        let reference = runtime();
        let mut candidate = runtime();
        candidate.journey_digest = None;
        let assessment = assess_recorded_identity(
            &manifest(),
            &manifest(),
            Some(&reference),
            Some(&candidate),
            Some(&summary()),
            Some(&summary()),
        );
        assert_eq!(
            assessment.status,
            BrowserComparabilityStatus::InsufficientEvidence
        );
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.contains("browser journey digest"))
        );
    }

    #[test]
    fn changed_journey_or_runtime_is_incomparable() {
        let reference = runtime();
        let mut candidate = runtime();
        candidate.journey_digest = Some(format!("sha256:{}", "d".repeat(64)));
        candidate.browser_version = "141.0.0".to_owned();
        let assessment = assess_recorded_identity(
            &manifest(),
            &manifest(),
            Some(&reference),
            Some(&candidate),
            Some(&summary()),
            Some(&summary()),
        );
        assert_eq!(assessment.status, BrowserComparabilityStatus::Incomparable);
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.contains("browser journey digest differs"))
        );
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.contains("browser version differs"))
        );
    }

    #[test]
    fn source_revision_difference_is_not_itself_an_incompatibility() {
        let reference_manifest = manifest();
        let mut candidate_manifest = manifest();
        candidate_manifest.source.git_sha =
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned());
        let assessment = assess_recorded_identity(
            &reference_manifest,
            &candidate_manifest,
            Some(&runtime()),
            Some(&runtime()),
            Some(&summary()),
            Some(&summary()),
        );
        assert_eq!(assessment.status, BrowserComparabilityStatus::Comparable);
        assert!(assessment.reasons.is_empty());
    }
}
