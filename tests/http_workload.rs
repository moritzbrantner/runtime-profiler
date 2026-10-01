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
