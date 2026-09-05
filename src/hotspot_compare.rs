use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::bundle::validate_bundle;
use crate::contract::{BundleManifest, HotspotsDocument, SourceIdentity};

pub const HOTSPOT_COMPARABILITY_SCHEMA_V1: &str =
    "runtime-profiler/hotspot-comparability/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HotspotComparabilityStatus {
    Comparable,
    Incomparable,
    InsufficientEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HotspotComparabilityReport {
    pub schema_version: String,
    pub reference_bundle_id: String,
    pub candidate_bundle_id: String,
    pub reference_source: SourceIdentity,
    pub candidate_source: SourceIdentity,
    pub status: HotspotComparabilityStatus,
    pub reasons: Vec<String>,
    pub notes: Vec<String>,
}

pub fn compare_hotspot_bundles(
    reference: &Path,
    candidate: &Path,
) -> Result<HotspotComparabilityReport> {
    validate(reference, "reference")?;
    validate(candidate, "candidate")?;

    let reference_manifest: BundleManifest = read_json(&reference.join("manifest.json"))?;
    let candidate_manifest: BundleManifest = read_json(&candidate.join("manifest.json"))?;
    let reference_hotspots: HotspotsDocument = read_json(&reference.join("hotspots.json"))?;
    let candidate_hotspots: HotspotsDocument = read_json(&candidate.join("hotspots.json"))?;

    let assessment = assess_recorded_identity(
        &reference_manifest,
        &candidate_manifest,
        &reference_hotspots,
        &candidate_hotspots,
    );

    Ok(HotspotComparabilityReport {
        schema_version: HOTSPOT_COMPARABILITY_SCHEMA_V1.to_owned(),
        reference_bundle_id: reference_manifest.bundle_id,
        candidate_bundle_id: candidate_manifest.bundle_id,
        reference_source: reference_manifest.source,
        candidate_source: candidate_manifest.source,
        status: assessment.status,
        reasons: assessment.reasons,
        notes: vec![
            "Hotspot comparability is independent from runtime score; this command never produces a performance verdict.".to_owned(),
            "Source revisions may differ. Scenario/workload and execution-environment identity must remain compatible.".to_owned(),
            "The current hotspots/v1 artifact records collector, perf version, event, metric, unit, scenario digest, and environment fingerprint, but not yet sample-period, symbolization-mode, or independent target/toolchain fingerprints. Matching current bundles therefore remain insufficient evidence rather than being guessed comparable.".to_owned(),
        ],
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Assessment {
    status: HotspotComparabilityStatus,
    reasons: Vec<String>,
}

fn assess_recorded_identity(
    reference_manifest: &BundleManifest,
    candidate_manifest: &BundleManifest,
    reference_hotspots: &HotspotsDocument,
    candidate_hotspots: &HotspotsDocument,
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

    if reference_hotspots.status != "collected" || candidate_hotspots.status != "collected" {
        missing.push(
            "both bundles must contain successfully collected hotspot evidence".to_owned(),
        );
    }

    compare_required(
        "collector",
        reference_hotspots.collector.as_deref(),
        candidate_hotspots.collector.as_deref(),
        &mut mismatches,
        &mut missing,
    );
    compare_required(
        "perf/tool version",
        reference_hotspots.tool_version.as_deref(),
        candidate_hotspots.tool_version.as_deref(),
        &mut mismatches,
        &mut missing,
    );
    compare_required(
        "event",
        reference_hotspots.event.as_deref(),
        candidate_hotspots.event.as_deref(),
        &mut mismatches,
        &mut missing,
    );
    compare_required(
        "metric",
        reference_hotspots.metric.as_deref(),
        candidate_hotspots.metric.as_deref(),
        &mut mismatches,
        &mut missing,
    );
    compare_required(
        "unit",
        reference_hotspots.unit.as_deref(),
        candidate_hotspots.unit.as_deref(),
        &mut mismatches,
        &mut missing,
    );

    // These are deliberate fail-closed gaps in hotspots/v1. Do not infer them
    // from the current runtime-profiler implementation, because an old bundle
    // may have been produced by different sampling or symbolization behavior.
    missing.push("sample-period identity is not recorded in hotspots/v1".to_owned());
    missing.push("symbolization-mode identity is not recorded in hotspots/v1".to_owned());
    missing.push(
        "independent target/toolchain fingerprint is not recorded in hotspots/v1".to_owned(),
    );

    if !mismatches.is_empty() {
        mismatches.extend(missing);
        return Assessment {
            status: HotspotComparabilityStatus::Incomparable,
            reasons: mismatches,
        };
    }
    if !missing.is_empty() {
        return Assessment {
            status: HotspotComparabilityStatus::InsufficientEvidence,
            reasons: missing,
        };
    }

    Assessment {
        status: HotspotComparabilityStatus::Comparable,
        reasons: Vec::new(),
    }
}

fn compare_required(
    label: &str,
    reference: Option<&str>,
    candidate: Option<&str>,
    mismatches: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    match (reference.filter(|value| !value.is_empty()), candidate.filter(|value| !value.is_empty())) {
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

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> BundleManifest {
        BundleManifest {
            schema_version: "runtime-profiler/bundle-manifest/v1".to_owned(),
            bundle_id: "bundle".to_owned(),
            created_unix_ms: 1,
            scenario_id: "scenario".to_owned(),
            scenario_digest: "scenario-digest".to_owned(),
            environment_fingerprint_schema_version:
                "runtime-profiler/environment-fingerprint/v1".to_owned(),
            environment_fingerprint: "environment".to_owned(),
            source: SourceIdentity {
                git_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned()),
                dirty: Some(false),
            },
            files: Vec::new(),
        }
    }

    fn hotspots() -> HotspotsDocument {
        HotspotsDocument {
            schema_version: "runtime-profiler/hotspots/v1".to_owned(),
            status: "collected".to_owned(),
            reason: "fixture".to_owned(),
            collector: Some("native-perf".to_owned()),
            tool_version: Some("perf version 6.8".to_owned()),
            event: Some("cycles:u".to_owned()),
            metric: Some("native-perf.period".to_owned()),
            unit: Some("event-count".to_owned()),
            total_weight: 100,
            total_samples: 1,
            truncated: false,
            hotspots: Vec::new(),
        }
    }

    #[test]
    fn matching_recorded_identity_is_insufficient_instead_of_guessed_comparable() {
        let assessment = assess_recorded_identity(&manifest(), &manifest(), &hotspots(), &hotspots());
        assert_eq!(
            assessment.status,
            HotspotComparabilityStatus::InsufficientEvidence
        );
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.contains("sample-period"))
        );
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.contains("target/toolchain"))
        );
    }

    #[test]
    fn different_recorded_identity_is_incomparable() {
        let reference_manifest = manifest();
        let mut candidate_manifest = manifest();
        candidate_manifest.environment_fingerprint = "other-environment".to_owned();
        candidate_manifest.source.git_sha = Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned());

        let reference_hotspots = hotspots();
        let mut candidate_hotspots = hotspots();
        candidate_hotspots.event = Some("instructions:u".to_owned());

        let assessment = assess_recorded_identity(
            &reference_manifest,
            &candidate_manifest,
            &reference_hotspots,
            &candidate_hotspots,
        );
        assert_eq!(assessment.status, HotspotComparabilityStatus::Incomparable);
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.contains("environment fingerprint differs"))
        );
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.contains("event differs"))
        );
    }

    #[test]
    fn source_revision_difference_is_not_itself_an_incompatibility() {
        let reference_manifest = manifest();
        let mut candidate_manifest = manifest();
        candidate_manifest.source.git_sha = Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned());

        let assessment = assess_recorded_identity(
            &reference_manifest,
            &candidate_manifest,
            &hotspots(),
            &hotspots(),
        );
        assert_eq!(
            assessment.status,
            HotspotComparabilityStatus::InsufficientEvidence
        );
        assert!(
            assessment
                .reasons
                .iter()
                .all(|reason| !reason.contains("source"))
        );
    }
}