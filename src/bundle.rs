use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Component, Path};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use crate::capture::{capture_metrics, ensure_not_interrupted};
use crate::contract::{
    AgentGuidance, AgentObservation, ArtifactEntry, BundleManifest,
    ENVIRONMENT_FINGERPRINT_SCHEMA_LEGACY_V0, ENVIRONMENT_FINGERPRINT_SCHEMA_V1,
    ENVIRONMENT_SCHEMA_V1, EnvironmentDocument, GUIDANCE_SCHEMA_V1, HOTSPOTS_SCHEMA_V1,
    HotspotsDocument, MANIFEST_SCHEMA_V1, METRICS_SCHEMA_V1, MetricSummary, MetricsDocument,
    SCENARIO_EVIDENCE_SCHEMA_V1, ScenarioEvidence, SourceIdentity, ValidationReport,
};
use crate::digest::{sha256_bytes, sha256_file};
use crate::scenario::load_scenario;

const ARTIFACTS: [(&str, &str); 5] = [
    ("scenario.json", "application/json"),
    ("environment.json", "application/json"),
    ("metrics.json", "application/json"),
    ("hotspots.json", "application/json"),
    ("agent-guidance.json", "application/json"),
];
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
    let metrics = capture_metrics(&scenario)?;
    ensure_not_interrupted()?;
    let hotspots = HotspotsDocument {
        schema_version: HOTSPOTS_SCHEMA_V1.to_owned(),
        status: "not-collected".to_owned(),
        reason: "No source-level profiler adapter was requested by this v1 scenario".to_owned(),
        hotspots: Vec::new(),
    };
    let guidance = build_guidance(&metrics);
    ensure_not_interrupted()?;

    fs::create_dir_all(output)
        .with_context(|| format!("failed to create bundle directory: {}", output.display()))?;
    ensure_not_interrupted()?;
    write_json(&output.join("scenario.json"), &scenario.evidence())?;
    ensure_not_interrupted()?;
    write_json(&output.join("environment.json"), &environment)?;
    ensure_not_interrupted()?;
    write_json(&output.join("metrics.json"), &metrics)?;
    ensure_not_interrupted()?;
    write_json(&output.join("hotspots.json"), &hotspots)?;
    ensure_not_interrupted()?;
    write_json(&output.join("agent-guidance.json"), &guidance)?;
    ensure_not_interrupted()?;

    let mut files = Vec::with_capacity(ARTIFACTS.len());
    for (path, media_type) in ARTIFACTS {
        ensure_not_interrupted()?;
        files.push(ArtifactEntry {
            path: path.to_owned(),
            media_type: media_type.to_owned(),
            sha256: sha256_file(&output.join(path))?,
        });
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
    ensure_not_interrupted()?;
    write_json(&output.join("manifest.json"), &manifest)?;
    ensure_not_interrupted()?;

    Ok(manifest)
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
    let expected_paths: BTreeSet<&str> = ARTIFACTS.iter().map(|(path, _)| *path).collect();
    let actual_paths: BTreeSet<&str> = manifest
        .files
        .iter()
        .map(|artifact| artifact.path.as_str())
        .collect();
    if actual_paths != expected_paths {
        diagnostics.push("manifest artifact set does not match the v1 contract".to_owned());
    }
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

    let report = ValidationReport {
        schema_version: "runtime-profiler/validation/v1".to_owned(),
        bundle_id: manifest.bundle_id,
        valid: diagnostics.is_empty(),
        verified_files,
        diagnostics,
    };

    Ok(report)
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

fn build_guidance(metrics: &MetricsDocument) -> AgentGuidance {
    let observations = metrics
        .metrics
        .iter()
        .map(|metric| AgentObservation {
            id: metric.id.clone(),
            summary: summarize_metric(metric),
            evidence_ref: format!("metrics.json#{}", metric.id),
        })
        .collect();

    AgentGuidance {
        schema_version: GUIDANCE_SCHEMA_V1.to_owned(),
        scenario_id: metrics.scenario_id.clone(),
        observations,
        constraints: vec![
            "This bundle describes one version; it does not establish improvement or regression."
                .to_owned(),
            "Use Moonlight to compare a baseline and candidate with matching scenario digests."
                .to_owned(),
            "Treat cross-environment comparisons as inconclusive unless policy explicitly permits them."
                .to_owned(),
        ],
        evidence_refs: vec![
            "manifest.json".to_owned(),
            "environment.json".to_owned(),
            "metrics.json".to_owned(),
            "hotspots.json".to_owned(),
        ],
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
}
