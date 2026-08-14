use std::fs;
use std::process::Command;

use serde_json::Value;

#[test]
fn captures_validates_and_summarizes_command_scenario() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let scenario = temp.path().join("scenario.json");
    let bundle = temp.path().join("bundle");
    fs::write(
        &scenario,
        r#"{
  "schema_version": "runtime-profiler/scenario/v1",
  "id": "integration-test",
  "target": {
    "type": "command",
    "program": "sh",
    "args": ["-c", "sleep 0.01"]
  },
  "run": {
    "warmup_iterations": 1,
    "measurement_iterations": 3,
    "timeout_seconds": 5
  },
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
    assert!(
        capture.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&capture.stderr)
    );

    let validate = Command::new(binary)
        .args([
            "validate",
            "--bundle",
            bundle.to_str().expect("UTF-8 bundle path"),
        ])
        .output()
        .expect("run validate");
    assert!(validate.status.success());
    let validation: Value = serde_json::from_slice(&validate.stdout).expect("validation JSON");
    assert_eq!(validation["valid"], true);
    assert_eq!(validation["verified_files"], 5);

    let summary = Command::new(binary)
        .args([
            "summarize",
            "--bundle",
            bundle.to_str().expect("UTF-8 bundle path"),
            "--json",
        ])
        .output()
        .expect("run summarize");
    assert!(summary.status.success());
    let metrics: Value = serde_json::from_slice(&summary.stdout).expect("metrics JSON");
    assert_eq!(metrics["scenario_id"], "integration-test");
    assert_eq!(metrics["samples"].as_array().map(Vec::len), Some(3));
}

#[test]
fn refuses_to_overwrite_bundle() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let scenario = temp.path().join("scenario.json");
    let bundle = temp.path().join("bundle");
    fs::write(
        &scenario,
        r#"{
  "schema_version": "runtime-profiler/scenario/v1",
  "id": "overwrite-test",
  "target": { "type": "command", "program": "true" },
  "run": { "warmup_iterations": 0, "measurement_iterations": 1, "timeout_seconds": 5 },
  "collectors": ["process"]
}"#,
    )
    .expect("write scenario");

    let binary = env!("CARGO_BIN_EXE_runtime-profiler");
    let arguments = [
        "capture",
        "--scenario",
        scenario.to_str().expect("UTF-8 scenario path"),
        "--output",
        bundle.to_str().expect("UTF-8 bundle path"),
    ];
    assert!(
        Command::new(binary)
            .args(arguments)
            .status()
            .expect("first capture")
            .success()
    );
    assert!(
        !Command::new(binary)
            .args(arguments)
            .status()
            .expect("second capture")
            .success()
    );
}
