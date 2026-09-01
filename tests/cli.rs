use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;

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
    "args": ["-c", "sleep 0.1"]
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
    if cfg!(target_os = "linux") {
        let metrics = metrics["metrics"].as_array().expect("metrics array");
        let legacy = metrics
            .iter()
            .find(|metric| metric["id"] == "process.max_rss");
        let observed = metrics
            .iter()
            .find(|metric| metric["id"] == "process.max_observed_rss");
        assert_eq!(legacy.is_some(), observed.is_some());
        if let (Some(legacy), Some(observed)) = (legacy, observed) {
            assert_eq!(legacy["statistics"], observed["statistics"]);
        }
    }
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

#[cfg(unix)]
#[test]
fn timeout_terminates_descendant_process_group() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let scenario = temp.path().join("scenario.json");
    let bundle = temp.path().join("bundle");
    let survivor = temp.path().join("descendant-survived");
    fs::write(
        &scenario,
        r#"{
  "schema_version": "runtime-profiler/scenario/v1",
  "id": "process-group-timeout-test",
  "target": {
    "type": "command",
    "program": "sh",
    "args": ["-c", "(sleep 2; touch descendant-survived) & wait"],
    "working_directory": "."
  },
  "run": {
    "warmup_iterations": 0,
    "measurement_iterations": 1,
    "timeout_seconds": 1
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

    thread::sleep(Duration::from_millis(1_500));
    assert!(
        !survivor.exists(),
        "descendant process survived the timed-out capture"
    );

    let metrics: Value = serde_json::from_slice(
        &fs::read(bundle.join("metrics.json")).expect("read metrics document"),
    )
    .expect("metrics JSON");
    assert_eq!(metrics["samples"][0]["timed_out"], true);
    assert_eq!(metrics["samples"][0]["succeeded"], false);
}

#[cfg(unix)]
#[test]
fn interruption_terminates_descendant_process_group() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let scenario = temp.path().join("scenario.json");
    let bundle = temp.path().join("bundle");
    let started = temp.path().join("workload-started");
    let survivor = temp.path().join("descendant-survived");
    fs::write(
        &scenario,
        r#"{
  "schema_version": "runtime-profiler/scenario/v1",
  "id": "process-group-interruption-test",
  "target": {
    "type": "command",
    "program": "sh",
    "args": ["-c", "touch workload-started; (sleep 2; touch descendant-survived) & wait"],
    "working_directory": "."
  },
  "run": {
    "warmup_iterations": 0,
    "measurement_iterations": 1,
    "timeout_seconds": 10
  },
  "collectors": ["process"]
}"#,
    )
    .expect("write scenario");

    let binary = env!("CARGO_BIN_EXE_runtime-profiler");
    let mut capture = Command::new(binary)
        .args([
            "capture",
            "--scenario",
            scenario.to_str().expect("UTF-8 scenario path"),
            "--output",
            bundle.to_str().expect("UTF-8 bundle path"),
        ])
        .spawn()
        .expect("start capture");

    for _ in 0..200 {
        if started.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(started.exists(), "profiled workload did not start");

    let interrupt = Command::new("kill")
        .args(["-INT", &capture.id().to_string()])
        .status()
        .expect("interrupt capture");
    assert!(interrupt.success());
    assert!(
        !capture
            .wait()
            .expect("wait for interrupted capture")
            .success()
    );

    thread::sleep(Duration::from_millis(2_200));
    assert!(
        !survivor.exists(),
        "descendant process survived the interrupted capture"
    );
}
