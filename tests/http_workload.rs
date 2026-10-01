#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

const FIXTURE: &str = r#"
import http.server, os, pathlib, time, threading, json
class Handler(http.server.BaseHTTPRequestHandler):
    active, peak, requests = 0, 0, 0
    lock = threading.Lock()
    def enter(self):
        with Handler.lock:
            Handler.active += 1
            Handler.requests += 1
            Handler.peak = max(Handler.peak, Handler.active)
        time.sleep(0.02)
    def leave(self):
        with Handler.lock:
            Handler.active -= 1
            pathlib.Path('truth.json').write_text(json.dumps({'requests': Handler.requests, 'peak': Handler.peak}))
    def do_GET(self):
        self.enter()
        if self.path == '/slow': time.sleep(2)
        status = 503 if self.path == '/error' else 200
        if self.path == '/identity':
            status = 200 if self.headers.get('X-App-Id') == 'private-app' and self.headers.get('X-User-Id') == 'private-user' else 400
        self.send_response(status)
        self.end_headers()
        try: self.wfile.write(b'x' * 2097152 if self.path == '/large' else b'private-response')
        except BrokenPipeError: pass
        self.leave()
    def do_POST(self):
        self.enter()
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        self.send_response(201 if body == b'private-body' and self.headers.get('Content-Type') == 'application/json' else 400)
        self.end_headers()
        self.leave()
    def log_message(self, *args): pass
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
pathlib.Path('pid').write_text(str(os.getpid()))
pathlib.Path(os.environ['RUNTIME_PROFILER_PORT_FILE']).write_text(str(server.server_port))
server.serve_forever()
"#;

fn scenario(root: &Path, path: &str, expected_status: u16) -> Value {
    fs::write(root.join("fixture.py"), FIXTURE).expect("write fixture");
    json!({
        "schema_version": "runtime-profiler/scenario/v1", "id": "http-fixture",
        "target": {
            "type": "http-workload",
            "fixture": {
                "program": "python3", "args": ["fixture.py"], "working_directory": ".",
                "inherit_env": [], "health_path": "/health", "startup_timeout_seconds": 5,
                "teardown": {"program": "python3", "args": ["-c", "from pathlib import Path; Path('stopped').write_text('yes')"]}
            },
            "requests": [{"method": "GET", "path": path, "expected_status": expected_status}],
            "request_count": 4, "concurrency": 2, "request_timeout_seconds": 1
        },
        "run": {"warmup_iterations": 1, "measurement_iterations": 2, "timeout_seconds": 10},
        "collectors": ["http-curl"]
    })
}

fn capture(root: &Path, scenario: &Value) -> Output {
    fs::write(
        root.join("scenario.json"),
        serde_json::to_vec(scenario).expect("serialize scenario"),
    )
    .expect("write scenario");
    Command::new(env!("CARGO_BIN_EXE_runtime-profiler"))
        .current_dir(root)
        .args([
            "capture",
            "--scenario",
            "scenario.json",
            "--output",
            "bundle",
        ])
        .output()
        .expect("capture HTTP workload")
}

fn assert_stopped(root: &Path) {
    assert!(root.join("stopped").is_file(), "teardown must run");
    let pid = fs::read_to_string(root.join("pid")).expect("fixture PID");
    let alive = Command::new("kill")
        .args(["-0", pid.trim()])
        .output()
        .expect("probe fixture");
    assert!(!alive.status.success(), "fixture must be reaped");
}

#[test]
fn captures_bounded_http_metrics_without_private_payloads_and_stops_fixture() {
    let root = tempfile::tempdir().expect("temporary fixture");
    let mut scenario = scenario(root.path(), "/", 200);
    scenario["target"]["requests"] = json!([
        {"method": "GET", "path": "/", "expected_status": 200},
        {"method": "POST", "path": "/submit", "body": "private-body", "content_type": "application/json", "expected_status": 201}
    ]);
    let output = capture(root.path(), &scenario);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_stopped(root.path());
    let truth: Value =
        serde_json::from_slice(&fs::read(root.path().join("truth.json")).expect("fixture truth"))
            .expect("truth JSON");
    assert_eq!(
        truth["requests"], 13,
        "one health request, four warmups, eight measured requests"
    );
    assert_eq!(
        truth["peak"], 2,
        "declared concurrency must reach the service"
    );
    let bundle = root.path().join("bundle");
    assert_bundle_schemas(&bundle);
    assert!(
        runtime_profiler::validate_bundle(&bundle)
            .expect("validate")
            .valid
    );
    let metrics = runtime_profiler::summarize_bundle(&bundle).expect("summarize");
    assert_eq!(metrics.samples.len(), 8, "warmups must be excluded");
    assert!(metrics.samples.iter().all(|sample| sample.succeeded));
    for id in [
        "http.latency",
        "http.success_rate",
        "http.error_rate",
        "http.throughput",
    ] {
        assert!(
            metrics.metrics.iter().any(|metric| metric.id == id),
            "missing {id}"
        );
    }
    let http: Value = serde_json::from_slice(
        &fs::read(bundle.join("http-workload.json")).expect("HTTP evidence"),
    )
    .expect("parse evidence");
    assert_eq!(http["samples"].as_array().map(Vec::len), Some(8));
    assert_eq!(http["concurrency"], 2);
    for entry in fs::read_dir(bundle).expect("bundle files") {
        let contents = fs::read_to_string(entry.expect("artifact").path()).expect("read artifact");
        assert!(!contents.contains("private-body"));
        assert!(!contents.contains("private-response"));
        assert!(!contents.contains("127.0.0.1"));
    }
}

fn assert_bundle_schemas(bundle: &Path) {
    let result = Command::new("python3")
        .args([
            "-c",
            r#"
import json, pathlib, sys
from jsonschema import Draft202012Validator
from referencing import Registry, Resource
schemas = pathlib.Path(sys.argv[1]) / 'schemas'
documents = [json.loads(p.read_text()) for p in schemas.glob('*.schema.json')]
registry = Registry().with_resources((s['$id'], Resource.from_contents(s)) for s in documents)
bundle = pathlib.Path(sys.argv[2])
scenario_schema = json.loads((schemas / 'scenario.schema.json').read_text())
Draft202012Validator(scenario_schema, registry=registry).validate(json.loads((bundle.parent / 'scenario.json').read_text()))
names = {'manifest': 'bundle-manifest', 'scenario': 'scenario-evidence'}
for path in bundle.glob('*.json'):
    schema = json.loads((schemas / (names.get(path.stem, path.stem) + '.schema.json')).read_text())
    Draft202012Validator(schema, registry=registry).validate(json.loads(path.read_text()))
"#,
            env!("CARGO_MANIFEST_DIR"),
        ])
        .arg(bundle)
        .output()
        .expect("independent schema validation");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn retains_status_errors_and_timeouts_as_measurements() {
    for (path, status, timed_out) in [
        ("/error", 503, false),
        ("/slow", 0, true),
        ("/large", 200, false),
    ] {
        let root = tempfile::tempdir().expect("temporary fixture");
        let mut scenario = scenario(root.path(), path, 200);
        scenario["run"]["warmup_iterations"] = json!(0);
        let output = capture(root.path(), &scenario);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_stopped(root.path());
        let metrics =
            runtime_profiler::summarize_bundle(&root.path().join("bundle")).expect("summarize");
        assert!(
            metrics
                .samples
                .iter()
                .all(|sample| !sample.succeeded && sample.timed_out == timed_out)
        );
        let evidence: Value = serde_json::from_slice(
            &fs::read(root.path().join("bundle/http-workload.json")).expect("HTTP evidence"),
        )
        .expect("parse");
        assert!(
            evidence["samples"]
                .as_array()
                .expect("samples")
                .iter()
                .all(|sample| sample["status_code"] == status)
        );
    }
}

#[test]
fn teardown_failure_and_failed_warmup_do_not_create_bundle() {
    for failed_teardown in [false, true] {
        let root = tempfile::tempdir().expect("temporary fixture");
        let mut scenario = scenario(
            root.path(),
            if failed_teardown { "/" } else { "/error" },
            200,
        );
        if failed_teardown {
            scenario["target"]["fixture"]["teardown"]["args"] = json!([
                "-c",
                "from pathlib import Path; Path('stopped').write_text('yes'); raise SystemExit(1)"
            ]);
        }
        let output = capture(root.path(), &scenario);
        assert!(!output.status.success());
        assert_stopped(root.path());
        assert!(!root.path().join("bundle").exists());
    }
}

#[test]
fn startup_and_health_failures_still_run_teardown() {
    for spawn_failure in [true, false] {
        let root = tempfile::tempdir().expect("temporary fixture");
        let mut scenario = scenario(root.path(), "/", 200);
        if spawn_failure {
            scenario["target"]["fixture"]["program"] = json!("runtime-profiler-missing-fixture");
        } else {
            scenario["target"]["fixture"]["health_path"] = json!("/error");
            scenario["target"]["fixture"]["startup_timeout_seconds"] = json!(1);
        }
        let output = capture(root.path(), &scenario);
        assert!(!output.status.success());
        assert!(root.path().join("stopped").is_file());
        if !spawn_failure {
            assert_stopped(root.path());
        }
        assert!(!root.path().join("bundle").exists());
    }
}

#[test]
fn rejects_unbounded_workloads_and_arbitrary_endpoint_authorities() {
    let root = tempfile::tempdir().expect("temporary fixture");
    let valid = scenario(root.path(), "/", 200);
    for (field, value) in [
        ("concurrency", json!(33)),
        ("request_count", json!(1001)),
        ("request_timeout_seconds", json!(31)),
    ] {
        let mut invalid = valid.clone();
        invalid["target"][field] = value;
        assert!(!capture(root.path(), &invalid).status.success());
        assert!(!root.path().join("pid").exists());
    }
    for path in [
        "https://production.invalid",
        "//production.invalid",
        "/path#fragment",
        "/bad\\path",
    ] {
        let mut invalid = valid.clone();
        invalid["target"]["requests"][0]["path"] = json!(path);
        assert!(!capture(root.path(), &invalid).status.success());
        assert!(!root.path().join("pid").exists());
    }
}

#[test]
fn missing_curl_is_unavailable_before_fixture_start() {
    let root = tempfile::tempdir().expect("temporary fixture");
    let scenario = scenario(root.path(), "/", 200);
    fs::write(
        root.path().join("scenario.json"),
        serde_json::to_vec(&scenario).expect("scenario JSON"),
    )
    .expect("write");
    let binary = env!("CARGO_BIN_EXE_runtime-profiler");
    let plan = Command::new(binary)
        .current_dir(root.path())
        .env("PATH", root.path())
        .args(["plan", "--scenario", "scenario.json"])
        .output()
        .expect("plan");
    assert!(plan.status.success());
    let plan: Value = serde_json::from_slice(&plan.stdout).expect("plan JSON");
    assert_eq!(plan["collectors"][0]["supported"], false);
    let capture = Command::new(binary)
        .current_dir(root.path())
        .env("PATH", root.path())
        .args([
            "capture",
            "--scenario",
            "scenario.json",
            "--output",
            "bundle",
        ])
        .output()
        .expect("capture");
    assert!(!capture.status.success());
    assert!(String::from_utf8_lossy(&capture.stderr).contains("unavailable"));
    assert!(!root.path().join("pid").exists());
    assert!(!root.path().join("bundle").exists());
}

#[test]
fn interruption_reaps_fixture_and_runs_teardown() {
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().expect("temporary fixture");
    let mut scenario = scenario(root.path(), "/slow", 200);
    scenario["target"]["request_timeout_seconds"] = json!(5);
    fs::write(
        root.path().join("scenario.json"),
        serde_json::to_vec(&scenario).expect("scenario JSON"),
    )
    .expect("write");
    let mut child = Command::new(env!("CARGO_BIN_EXE_runtime-profiler"))
        .current_dir(root.path())
        .args([
            "capture",
            "--scenario",
            "scenario.json",
            "--output",
            "bundle",
        ])
        .spawn()
        .expect("capture");
    let start = Instant::now();
    while !root.path().join("pid").is_file() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "fixture readiness"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .expect("signal")
            .success()
    );
    assert!(!child.wait().expect("reap capture").success());
    assert_stopped(root.path());
    assert!(!root.path().join("bundle").exists());
}

fn rewrite_http_artifact(bundle: &Path, evidence: &Value) {
    let path = bundle.join("http-workload.json");
    fs::write(
        &path,
        serde_json::to_vec(evidence).expect("serialize evidence"),
    )
    .expect("write artifact");
    let manifest_path = bundle.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).expect("manifest"))
        .expect("parse manifest");
    for artifact in manifest["files"].as_array_mut().expect("files") {
        if artifact["path"] == "http-workload.json" {
            artifact["sha256"] =
                json!(runtime_profiler::digest::sha256_file(&path).expect("hash artifact"));
        }
    }
    fs::write(
        manifest_path,
        serde_json::to_vec(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
}

#[test]
fn fixture_descendants_are_terminated_without_external_kill() {
    use std::os::unix::fs::symlink;
    use std::time::Duration;
    let root = tempfile::tempdir().expect("temporary fixture");
    let bin = root.path().join("bin");
    fs::create_dir(&bin).expect("tool directory");
    // Keep only curl in PATH. Fixture and descendant use explicit absolute programs.
    let curl = Command::new("sh")
        .args(["-c", "command -v curl"])
        .output()
        .expect("locate curl");
    let curl = String::from_utf8(curl.stdout).expect("curl path");
    symlink(curl.trim(), bin.join("curl")).expect("expose curl");
    let python = Command::new("sh")
        .args(["-c", "command -v python3"])
        .output()
        .expect("locate Python");
    let python = String::from_utf8(python.stdout).expect("Python path");
    let mut scenario = scenario(root.path(), "/", 200);
    let fixture = FIXTURE.replace("server.serve_forever()", "import subprocess\nsubprocess.Popen(['/bin/sh', '-c', 'sleep 2; echo survived > descendant-survived'], env={'PATH': '/usr/bin:/bin'})\nserver.serve_forever()");
    fs::write(root.path().join("fixture.py"), fixture).expect("descendant fixture");
    scenario["target"]["fixture"]["program"] = json!(python.trim());
    scenario["target"]["fixture"]["teardown"]["program"] = json!(python.trim());
    fs::write(
        root.path().join("scenario.json"),
        serde_json::to_vec(&scenario).expect("scenario JSON"),
    )
    .expect("write scenario");
    let output = Command::new(env!("CARGO_BIN_EXE_runtime-profiler"))
        .current_dir(root.path())
        .env("PATH", &bin)
        .args([
            "capture",
            "--scenario",
            "scenario.json",
            "--output",
            "bundle",
        ])
        .output()
        .expect("capture without kill");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::thread::sleep(Duration::from_millis(2200));
    assert!(
        !root.path().join("descendant-survived").exists(),
        "ordinary fixture descendants must be terminated"
    );
    assert_stopped(root.path());
}

#[test]
fn dynamic_ports_compare_but_collector_changes_and_forged_metrics_do_not() {
    let root = tempfile::tempdir().expect("temporary fixture");
    let scenario = scenario(root.path(), "/", 200);
    assert!(capture(root.path(), &scenario).status.success());
    let reference = root.path().join("reference");
    fs::rename(root.path().join("bundle"), &reference).expect("retain reference");
    assert!(capture(root.path(), &scenario).status.success());
    let candidate = root.path().join("bundle");
    let score = runtime_profiler::score_bundles(&reference, &candidate)
        .expect("same workload and tool can compare");
    for id in ["http.success_rate", "http.error_rate"] {
        let metric = score
            .metrics
            .iter()
            .find(|metric| metric.id == id)
            .expect("HTTP rate score");
        assert_eq!(metric.statistics[0].statistic, "mean");
    }
    let original: Value = serde_json::from_slice(
        &fs::read(candidate.join("http-workload.json")).expect("HTTP artifact"),
    )
    .expect("parse evidence");
    let mut changed = original.clone();
    changed["collector_version"] = json!("curl 99.0.0 incompatible build");
    rewrite_http_artifact(&candidate, &changed);
    assert!(
        runtime_profiler::validate_bundle(&candidate)
            .expect("valid artifact")
            .valid
    );
    assert!(
        runtime_profiler::score_bundles(&reference, &candidate)
            .expect_err("changed tool cannot compare")
            .to_string()
            .contains("identities")
    );
    let mut forged = original.clone();
    forged["samples"][0]["duration_ms"] = json!(9.0);
    rewrite_http_artifact(&candidate, &forged);
    assert!(
        !runtime_profiler::validate_bundle(&candidate)
            .expect("validate inconsistent evidence")
            .valid
    );
    forged = original;
    forged["batches"] = json!([]);
    rewrite_http_artifact(&candidate, &forged);
    assert!(
        !runtime_profiler::validate_bundle(&candidate)
            .expect("validate empty evidence without panic")
            .valid
    );
}

#[test]
fn evidence_validation_counts_warmups_toward_total_request_limit() {
    use runtime_profiler::contract::TargetEvidence;
    use runtime_profiler::http_workload::{
        HttpBatch, HttpEvidence, HttpSample, metrics_from_evidence, validate_evidence,
    };
    let root = tempfile::tempdir().expect("temporary fixture");
    let scenario = scenario(root.path(), "/", 200);
    let path = root.path().join("scenario.json");
    fs::write(
        &path,
        serde_json::to_vec(&scenario).expect("serialize scenario"),
    )
    .expect("write scenario");
    let mut scenario = runtime_profiler::load_scenario(&path)
        .expect("valid scenario")
        .evidence();
    scenario.run.warmup_iterations = 3;
    scenario.run.measurement_iterations = 10;
    let TargetEvidence::HttpWorkload {
        request_count,
        endpoint_count,
        concurrency,
        ..
    } = &mut scenario.target
    else {
        panic!("HTTP target expected");
    };
    *request_count = 1000;
    *endpoint_count = 1;
    *concurrency = 1;
    let evidence = HttpEvidence {
        schema_version: "runtime-profiler/http-workload/v1".to_owned(),
        scenario_id: scenario.id.clone(),
        collector_version: "curl 8.4.0 synthetic contract fixture".to_owned(),
        adapter_digest: "a".repeat(64),
        concurrency: 1,
        request_count_per_iteration: 1000,
        fixture_setup_ms: 1.0,
        fixture_teardown_ms: 1.0,
        collector_wall_time_ms: 1000.0,
        overhead_status: "not-isolated".to_owned(),
        batches: (0..10)
            .map(|iteration| HttpBatch {
                iteration,
                wall_time_ms: 100.0,
            })
            .collect(),
        samples: (0..10000)
            .map(|index| HttpSample {
                iteration: index / 1000,
                request_index: index % 1000,
                endpoint_index: 0,
                status_code: 200,
                expected_status: 200,
                curl_exit_code: 0,
                duration_ms: 1.0,
                response_bytes: 1,
                succeeded: true,
            })
            .collect(),
    };
    assert!(
        validate_evidence(&scenario, &evidence, &metrics_from_evidence(&evidence)).is_err(),
        "13000 total requests must be rejected even when only 10000 are measured"
    );
    scenario.run.warmup_iterations = 0;
    assert!(
        validate_evidence(&scenario, &evidence, &metrics_from_evidence(&evidence)).is_ok(),
        "the exact 10000 total-request limit remains supported"
    );
}

#[test]
fn declared_identity_headers_reach_the_fixture_without_entering_evidence() {
    let root = tempfile::tempdir().expect("temporary fixture");
    let mut scenario = scenario(root.path(), "/identity", 200);
    scenario["target"]["requests"][0]["headers"] = json!({
        "x-app-id": "private-app", "x-user-id": "private-user"
    });
    let output = capture(root.path(), &scenario);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_stopped(root.path());
    let bundle = root.path().join("bundle");
    assert_bundle_schemas(&bundle);
    assert!(
        runtime_profiler::summarize_bundle(&bundle)
            .expect("metrics")
            .samples
            .iter()
            .all(|sample| sample.succeeded)
    );
    for entry in fs::read_dir(&bundle).expect("bundle") {
        let contents = fs::read_to_string(entry.expect("artifact").path()).expect("contents");
        assert!(!contents.contains("private-app"));
        assert!(!contents.contains("private-user"));
        assert!(!contents.contains("x-app-id"));
    }
    let first =
        runtime_profiler::load_scenario(&root.path().join("scenario.json")).expect("scenario");
    scenario["target"]["requests"][0]["headers"]["x-app-id"] = json!("another-app");
    fs::write(
        root.path().join("scenario.json"),
        serde_json::to_vec(&scenario).expect("JSON"),
    )
    .expect("write");
    let changed = runtime_profiler::load_scenario(&root.path().join("scenario.json"))
        .expect("changed scenario");
    assert_ne!(
        first.digest, changed.digest,
        "headers must participate in workload identity"
    );
}

#[test]
fn rejects_unbounded_injected_duplicate_and_transport_control_headers_before_startup() {
    let root = tempfile::tempdir().expect("temporary fixture");
    let valid = scenario(root.path(), "/", 200);
    let too_many: serde_json::Map<String, Value> = (0..17)
        .map(|index| (format!("x-{index}"), json!("value")))
        .collect();
    for headers in [
        json!({"bad name": "value"}),
        json!({"x-name\r\nInjected": "value"}),
        json!({"x-name": "value\r\nInjected: true"}),
        json!({"x-name": "é"}),
        json!({"x-name": "x".repeat(2049)}),
        json!({"x".repeat(65): "value"}),
        json!({"X-Name": "one", "x-name": "two"}),
        json!(too_many),
        json!({"Host": "production.invalid"}),
        json!({"Content-Length": "999"}),
        json!({"Transfer-Encoding": "chunked"}),
        json!({"Content-Type": "text/plain"}),
    ] {
        let mut invalid = valid.clone();
        invalid["target"]["requests"][0]["headers"] = headers;
        assert!(!capture(root.path(), &invalid).status.success());
        assert!(!root.path().join("pid").exists());
        assert!(!root.path().join("bundle").exists());
    }
}

#[test]
fn omitted_and_empty_headers_preserve_existing_workload_identity() {
    let root = tempfile::tempdir().expect("temporary fixture");
    let mut scenario = scenario(root.path(), "/", 200);
    let path = root.path().join("scenario.json");
    fs::write(&path, serde_json::to_vec(&scenario).expect("JSON")).expect("write");
    let original = runtime_profiler::load_scenario(&path).expect("existing scenario");
    scenario["target"]["requests"][0]["headers"] = json!({});
    fs::write(&path, serde_json::to_vec(&scenario).expect("JSON")).expect("write");
    let explicit_empty = runtime_profiler::load_scenario(&path).expect("empty headers");
    assert_eq!(original.digest, explicit_empty.digest);
    let normalized = serde_json::to_value(original.scenario).expect("normalize");
    assert!(normalized["target"]["requests"][0].get("headers").is_none());
}
