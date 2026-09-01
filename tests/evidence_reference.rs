use std::fs;
use std::process::Command;

use serde_json::Value;

#[test]
fn emits_content_addressed_agent_evidence_reference() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let scenario = temp.path().join("scenario.json");
    let bundle = temp.path().join("bundle");
    fs::write(
        &scenario,
        r#"{
  "schema_version": "runtime-profiler/scenario/v1",
  "id": "evidence-reference-test",
  "target": { "type": "command", "program": "true" },
  "run": { "warmup_iterations": 0, "measurement_iterations": 1, "timeout_seconds": 5 },
  "collectors": ["process"]
}"#,
    )
    .expect("write scenario");

    let binary = env!("CARGO_BIN_EXE_runtime-profiler");
    let capture = Command::new(binary)
        .args([
            "capture",
            "--scenario",
            scenario.to_str().expect("UTF-8 scenario path"),
            "--output",
            bundle.to_str().expect("UTF-8 bundle path"),
        ])
        .output()
        .expect("run capture");
    assert!(capture.status.success());

    let uri = ".agent-loop/evidence/runtime-profiler/bundle-1";
    let reference = Command::new(binary)
        .args([
            "evidence-reference",
            "--bundle",
            bundle.to_str().expect("UTF-8 bundle path"),
            "--uri",
            uri,
        ])
        .output()
        .expect("emit evidence reference");
    assert!(
        reference.status.success(),
        "evidence reference failed: {}",
        String::from_utf8_lossy(&reference.stderr)
    );

    let reference: Value =
        serde_json::from_slice(&reference.stdout).expect("evidence reference JSON");
    let expected_digest = format!(
        "sha256:{}",
        runtime_profiler::digest::sha256_file(&bundle.join("manifest.json"))
            .expect("hash bundle manifest")
    );
    assert_eq!(reference["schemaVersion"], 1);
    assert_eq!(reference["kind"], "runtime-profile-bundle");
    assert_eq!(reference["uri"], uri);
    assert_eq!(reference["digest"], expected_digest);
    assert!(
        reference["createdAt"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z'))
    );
    assert_eq!(reference.as_object().map(serde_json::Map::len), Some(5));
}

#[test]
fn evidence_reference_rejects_corrupt_bundle() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let scenario = temp.path().join("scenario.json");
    let bundle = temp.path().join("bundle");
    fs::write(
        &scenario,
        r#"{
  "schema_version": "runtime-profiler/scenario/v1",
  "id": "invalid-evidence-reference-test",
  "target": { "type": "command", "program": "true" },
  "run": { "warmup_iterations": 0, "measurement_iterations": 1, "timeout_seconds": 5 },
  "collectors": ["process"]
}"#,
    )
    .expect("write scenario");

    let binary = env!("CARGO_BIN_EXE_runtime-profiler");
    assert!(
        Command::new(binary)
            .args([
                "capture",
                "--scenario",
                scenario.to_str().expect("UTF-8 scenario path"),
                "--output",
                bundle.to_str().expect("UTF-8 bundle path"),
            ])
            .status()
            .expect("capture bundle")
            .success()
    );
    fs::write(bundle.join("metrics.json"), "{}\n").expect("corrupt metrics artifact");

    let reference = Command::new(binary)
        .args([
            "evidence-reference",
            "--bundle",
            bundle.to_str().expect("UTF-8 bundle path"),
            "--uri",
            ".agent-loop/evidence/runtime-profiler/corrupt",
        ])
        .status()
        .expect("emit evidence reference");
    assert!(!reference.success());
}
