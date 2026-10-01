//! Bounded loopback HTTP evidence, using curl's transfer engine and owned fixtures.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::capture::{
    ensure_not_interrupted, execute_isolated_command, prepare_target_environment, statistics,
    terminate_process,
};
use crate::contract::{
    Collector, CollectorPlan, Detection, HttpFixture, HttpRequest, METRICS_SCHEMA_V1,
    MeasurementSample, MetricSummary, MetricsDocument, PreferredDirection, Scenario, Target,
};
use crate::digest::sha256_bytes;
use crate::scenario::LoadedScenario;

pub const ARTIFACT: &str = "http-workload.json";
pub const SCHEMA: &str = "runtime-profiler/http-workload/v1";
const PORT_FILE_ENV: &str = "RUNTIME_PROFILER_PORT_FILE";
const SCOPE_ENV: &str = "RUNTIME_PROFILER_FIXTURE_DIRECTORY";
const MAX_OUTPUT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HttpEvidence {
    pub schema_version: String,
    pub scenario_id: String,
    pub collector_version: String,
    pub adapter_digest: String,
    pub concurrency: u32,
    pub request_count_per_iteration: u32,
    pub fixture_setup_ms: f64,
    pub fixture_teardown_ms: f64,
    pub collector_wall_time_ms: f64,
    pub overhead_status: String,
    pub batches: Vec<HttpBatch>,
    pub samples: Vec<HttpSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HttpBatch {
    pub iteration: u32,
    pub wall_time_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HttpSample {
    pub iteration: u32,
    pub request_index: u32,
    pub endpoint_index: usize,
    pub status_code: u16,
    pub expected_status: u16,
    pub curl_exit_code: i32,
    pub duration_ms: f64,
    pub response_bytes: u64,
    pub succeeded: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurlSample {
    request_index: u32,
    status_code: String,
    exit_code: i32,
    time_total: f64,
    size_download: u64,
}

struct HttpBatchWorkload<'a> {
    requests: &'a [HttpRequest],
    count: u32,
    concurrency: u32,
    request_timeout_seconds: u64,
    batch_timeout: Duration,
}

pub fn validate_target(scenario: &Scenario) -> Result<()> {
    let Target::HttpWorkload {
        fixture,
        requests,
        request_count,
        concurrency,
        request_timeout_seconds,
    } = &scenario.target
    else {
        bail!("HTTP target required");
    };
    ensure!(
        scenario.collectors == [Collector::HttpCurl],
        "http-workload requires exactly the http-curl collector"
    );
    ensure!(
        (1..=1000).contains(request_count),
        "request_count must be between 1 and 1000"
    );
    ensure!(
        (1..=32).contains(concurrency) && concurrency <= request_count,
        "concurrency must be between 1 and min(32, request_count)"
    );
    ensure!(
        !requests.is_empty() && requests.len() <= 32 && requests.len() <= *request_count as usize,
        "HTTP workload requires 1 to min(32, request_count) endpoints"
    );
    ensure!(
        (1..=30).contains(request_timeout_seconds),
        "request_timeout_seconds must be between 1 and 30"
    );
    ensure!(
        u64::from(*request_count)
            * u64::from(scenario.run.warmup_iterations + scenario.run.measurement_iterations)
            <= 10_000,
        "HTTP capture must contain at most 10000 total requests"
    );
    ensure!(
        scenario.run.timeout_seconds <= 60,
        "HTTP batch timeout must be at most 60 seconds"
    );
    ensure!(
        scenario.run.warmup_iterations <= 3 && scenario.run.measurement_iterations <= 10,
        "HTTP capture requires at most 3 warmups and 10 measured batches"
    );
    ensure!(
        (1..=60).contains(&fixture.startup_timeout_seconds),
        "fixture startup timeout must be between 1 and 60 seconds"
    );
    validate_program(&fixture.program, &fixture.args)?;
    if let Some(teardown) = &fixture.teardown {
        validate_program(&teardown.program, &teardown.args)?;
    }
    ensure!(
        !fixture
            .inherit_env
            .iter()
            .any(|name| name == PORT_FILE_ENV || name == SCOPE_ENV),
        "fixture control environment names cannot be inherited"
    );
    validate_path(&fixture.health_path)?;
    for request in requests {
        ensure!(
            matches!(
                request.method.as_str(),
                "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
            ),
            "unsupported HTTP method"
        );
        validate_path(&request.path)?;
        ensure!(
            (100..=599).contains(&request.expected_status),
            "expected_status must be between 100 and 599"
        );
        ensure!(
            request
                .body
                .as_ref()
                .is_none_or(|body| body.len() <= 65_536),
            "HTTP body must be at most 65536 bytes"
        );
        ensure!(
            request.method != "HEAD" || request.body.is_none(),
            "HEAD cannot contain a body"
        );
        ensure!(
            request
                .content_type
                .as_ref()
                .is_none_or(|value| value.len() <= 128
                    && value.contains('/')
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic() || byte == b' ')),
            "invalid bounded HTTP content type"
        );
    }
    Ok(())
}

fn validate_program(program: &str, args: &[String]) -> Result<()> {
    ensure!(
        !program.trim().is_empty() && program.len() <= 4096 && !program.contains('\0'),
        "invalid fixture program"
    );
    ensure!(
        args.len() <= 128
            && args
                .iter()
                .all(|arg| arg.len() <= 65_536 && !arg.contains('\0')),
        "fixture arguments exceed limits or contain a null byte"
    );
    Ok(())
}

fn validate_path(path: &str) -> Result<()> {
    ensure!(
        path.starts_with('/')
            && !path.starts_with("//")
            && path.len() <= 2048
            && !path.contains('#')
            && path
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'\\'),
        "HTTP path must be a bounded ASCII origin path without fragment or backslash"
    );
    Ok(())
}

pub fn detect_http_curl() -> Detection {
    match curl_version() {
        Ok(version) => Detection {
            available: true,
            reason: "bounded curl HTTP collector available".to_owned(),
            tool_version: Some(version),
        },
        Err(error) => Detection {
            available: false,
            reason: format!("HTTP collector unavailable: {error}"),
            tool_version: None,
        },
    }
}

#[must_use]
pub fn collector_plan() -> CollectorPlan {
    let detection = detect_http_curl();
    CollectorPlan {
        id: "http-curl".to_owned(),
        supported: detection.available,
        measurements: [
            "http.latency",
            "http.success_rate",
            "http.error_rate",
            "http.throughput",
        ]
        .map(str::to_owned)
        .to_vec(),
        reason: (!detection.available).then_some(detection.reason),
        tool_version: detection.tool_version,
        configuration: BTreeMap::from([
            (
                "endpoint_scope".to_owned(),
                "owned-loopback-fixture".to_owned(),
            ),
            ("overhead".to_owned(), "not-isolated".to_owned()),
        ]),
    }
}

fn curl_version() -> Result<String> {
    ensure!(
        cfg!(unix),
        "HTTP fixture descendant cleanup is unsupported on this platform"
    );
    let scope = tempfile::tempdir()?;
    let path = scope.path().join("version");
    let mut command = Command::new("curl");
    command
        .args(["-q", "--version"])
        .stdin(Stdio::null())
        .stdout(File::create(&path)?)
        .stderr(Stdio::null())
        .env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let result = execute_isolated_command(command, 0, "curl", Duration::from_secs(5), true)?;
    ensure!(result.succeeded, "curl version probe failed or timed out");
    let text = read_bounded(&path, 4096)?;
    let version = text.lines().next().context("curl version output missing")?;
    let number = version
        .split_whitespace()
        .nth(1)
        .context("invalid curl version")?;
    let mut parts = number.split('.');
    let major: u32 = parts
        .next()
        .context("curl major version missing")?
        .parse()?;
    let minor: u32 = parts
        .next()
        .context("curl minor version missing")?
        .parse()?;
    ensure!(
        major > 8 || (major == 8 && minor >= 4),
        "curl 8.4 or later is required for bounded streaming responses"
    );
    ensure!(
        text.lines().any(|line| line.starts_with("Protocols:")
            && line.split_whitespace().any(|protocol| protocol == "http")),
        "curl HTTP protocol unavailable"
    );
    Ok(version.to_owned())
}

struct OwnedFixture {
    child: Option<Child>,
    scope: TempDir,
}

impl OwnedFixture {
    fn new() -> Result<Self> {
        Ok(Self {
            child: None,
            scope: tempfile::tempdir()?,
        })
    }

    fn start(&mut self, loaded: &LoadedScenario, fixture: &HttpFixture) -> Result<()> {
        let mut command = Command::new(&fixture.program);
        command.args(&fixture.args);
        prepare_target_environment(loaded, &mut command);
        command
            .env(PORT_FILE_ENV, self.scope.path().join("port"))
            .env(SCOPE_ENV, self.scope.path());
        let child = command.spawn().context("failed to start HTTP fixture")?;
        self.child = Some(child);
        Ok(())
    }

    fn port(&mut self, timeout: Duration) -> Result<u16> {
        let start = Instant::now();
        loop {
            ensure_not_interrupted()?;
            let child = self
                .child
                .as_mut()
                .context("HTTP fixture already stopped")?;
            ensure!(
                child.try_wait()?.is_none(),
                "HTTP fixture exited before readiness"
            );
            let path = self.scope.path().join("port");
            if path.exists() {
                let text = read_bounded(&path, 16)?;
                if !text.trim().is_empty() {
                    let port: u16 = text
                        .trim()
                        .parse()
                        .context("fixture must publish a numeric loopback port")?;
                    ensure!(port != 0, "fixture port must be nonzero");
                    return Ok(port);
                }
            }
            ensure!(
                start.elapsed() < timeout,
                "HTTP fixture readiness timed out"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            // Even when the group leader exited, terminate ordinary descendants.
            if let Err(error) = terminate_process(&mut child) {
                if child.try_wait()?.is_none() {
                    return Err(error);
                }
            }
            child.wait().context("failed to reap HTTP fixture")?;
        }
        Ok(())
    }
}

impl Drop for OwnedFixture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub fn capture_http(loaded: &LoadedScenario) -> Result<(MetricsDocument, HttpEvidence)> {
    let Target::HttpWorkload {
        fixture,
        requests,
        request_count,
        concurrency,
        request_timeout_seconds,
    } = &loaded.scenario.target
    else {
        bail!("HTTP target required");
    };
    let collector_version = curl_version().context("HTTP collector unavailable")?;
    let start = Instant::now();
    let mut owned = OwnedFixture::new()?;
    let workload = HttpBatchWorkload {
        requests,
        count: *request_count,
        concurrency: *concurrency,
        request_timeout_seconds: *request_timeout_seconds,
        batch_timeout: Duration::from_secs(loaded.scenario.run.timeout_seconds),
    };
    let result = (|| {
        owned.start(loaded, fixture)?;
        let timeout = Duration::from_secs(fixture.startup_timeout_seconds);
        let port = owned.port(timeout)?;
        // A published port is insufficient: the declared health endpoint must succeed.
        let health = HttpRequest {
            method: "GET".to_owned(),
            path: fixture.health_path.clone(),
            body: None,
            content_type: None,
            expected_status: 200,
        };
        loop {
            let remaining = timeout
                .checked_sub(start.elapsed())
                .context("HTTP fixture health check timed out")?;
            let health_workload = HttpBatchWorkload {
                requests: std::slice::from_ref(&health),
                count: 1,
                concurrency: 1,
                request_timeout_seconds: 1,
                batch_timeout: remaining,
            };
            let (_, samples) = capture_batch(owned.scope.path(), port, &health_workload, 0)?;
            if samples.iter().all(|sample| sample.succeeded) {
                break;
            }
            ensure!(
                start.elapsed() < timeout,
                "HTTP fixture health check timed out"
            );
            ensure!(
                owned
                    .child
                    .as_mut()
                    .context("fixture stopped")?
                    .try_wait()?
                    .is_none(),
                "HTTP fixture exited during health check"
            );
            thread::sleep(Duration::from_millis(25));
        }
        let setup_ms = start.elapsed().as_secs_f64() * 1000.0;
        for iteration in 0..loaded.scenario.run.warmup_iterations {
            let (_, samples) = capture_batch(owned.scope.path(), port, &workload, iteration)?;
            ensure!(
                samples.iter().all(|sample| sample.succeeded),
                "HTTP warmup failed"
            );
        }
        let measured_start = Instant::now();
        let mut batches = Vec::new();
        let mut samples = Vec::new();
        for iteration in 0..loaded.scenario.run.measurement_iterations {
            ensure!(
                owned
                    .child
                    .as_mut()
                    .context("fixture stopped")?
                    .try_wait()?
                    .is_none(),
                "HTTP fixture exited during workload"
            );
            let (batch, measured) = capture_batch(owned.scope.path(), port, &workload, iteration)?;
            batches.push(batch);
            samples.extend(measured);
        }
        Ok(HttpEvidence {
            schema_version: SCHEMA.to_owned(),
            scenario_id: loaded.scenario.id.clone(),
            collector_version,
            adapter_digest: sha256_bytes(include_str!("http_workload.rs").as_bytes()),
            concurrency: *concurrency,
            request_count_per_iteration: *request_count,
            fixture_setup_ms: setup_ms,
            fixture_teardown_ms: 0.0,
            collector_wall_time_ms: measured_start.elapsed().as_secs_f64() * 1000.0,
            overhead_status: "not-isolated".to_owned(),
            batches,
            samples,
        })
    })();
    let teardown_start = Instant::now();
    let stopped = owned.stop();
    // Teardown still runs after startup, health, warmup, measurement, or interruption failures.
    let teardown = if let Some(teardown) = &fixture.teardown {
        let mut command = Command::new(&teardown.program);
        command.args(&teardown.args);
        prepare_target_environment(loaded, &mut command);
        command
            .env(PORT_FILE_ENV, owned.scope.path().join("port"))
            .env(SCOPE_ENV, owned.scope.path());
        execute_isolated_command(
            command,
            0,
            "HTTP fixture teardown",
            Duration::from_secs(30),
            false,
        )
        .and_then(|sample| {
            ensure!(
                sample.succeeded,
                "HTTP fixture teardown failed or timed out"
            );
            Ok(())
        })
    } else {
        Ok(())
    };
    stopped.context("HTTP fixture cleanup failed")?;
    teardown?;
    let mut evidence = result?;
    evidence.fixture_teardown_ms = teardown_start.elapsed().as_secs_f64() * 1000.0;
    Ok((metrics_from_evidence(&evidence), evidence))
}

fn capture_batch(
    scope: &Path,
    port: u16,
    workload: &HttpBatchWorkload<'_>,
    iteration: u32,
) -> Result<(HttpBatch, Vec<HttpSample>)> {
    ensure_not_interrupted()?;
    let HttpBatchWorkload {
        requests,
        count,
        concurrency,
        request_timeout_seconds: timeout_seconds,
        batch_timeout,
    } = workload;
    let mut config = String::new();
    for index in 0..*count {
        if index > 0 {
            config.push_str("next\n");
        }
        let endpoint = index as usize % requests.len();
        let request = &requests[endpoint];
        let url = format!("http://127.0.0.1:{port}{}", request.path);
        config.push_str(&format!("url = {}\nrequest = {}\nmax-time = {timeout_seconds}\nmax-filesize = 1048576\noutput = \"/dev/null\"\nproxy = \"\"\nnoproxy = \"*\"\nproto = \"=http\"\ngloboff\npath-as-is\n", quoted(&url), quoted(&request.method)));
        if request.method == "HEAD" {
            config.push_str("head\n");
        }
        if let Some(content_type) = &request.content_type {
            config.push_str(&format!(
                "header = {}\n",
                quoted(&format!("Content-Type: {content_type}"))
            ));
        }
        if let Some(body) = &request.body {
            let body_path = scope.join(format!("body-{endpoint}"));
            fs::write(&body_path, body)?;
            config.push_str(&format!(
                "data-binary = {}\n",
                quoted(&format!("@{}", body_path.display()))
            ));
        }
        let format = format!(
            "{{\"request_index\":{index},\"status_code\":\"%{{response_code}}\",\"exit_code\":%{{exitcode}},\"time_total\":%{{time_total}},\"size_download\":%{{size_download}}}}\n"
        );
        config.push_str(&format!("write-out = {}\n", quoted(&format)));
    }
    let config_path = scope.join("curl-config");
    fs::write(&config_path, config)?;
    let output_path = scope.join("curl-output");
    let mut command = Command::new("curl");
    // -q must be first: neither a user's curlrc nor proxy environment may alter the workload.
    command.args([
        "-q",
        "--silent",
        "--parallel",
        "--parallel-immediate",
        "--parallel-max",
        &concurrency.to_string(),
        "--config",
    ]);
    command
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(File::create(&output_path)?)
        .stderr(Stdio::null())
        .env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let batch = execute_isolated_command(
        command,
        iteration,
        "curl HTTP workload",
        *batch_timeout,
        true,
    )?;
    ensure!(
        !batch.timed_out,
        "HTTP batch timed out before complete measurements"
    );
    let text = read_bounded(&output_path, MAX_OUTPUT)?;
    let mut samples = Vec::new();
    for line in text.lines() {
        let raw: CurlSample =
            serde_json::from_str(line).context("invalid bounded curl measurement")?;
        let status_code: u16 = raw
            .status_code
            .parse()
            .context("invalid curl HTTP status")?;
        ensure!(
            raw.request_index < *count
                && raw.time_total.is_finite()
                && raw.time_total >= 0.0
                && raw.time_total <= *timeout_seconds as f64 + 1.0
                && status_code <= 599
                && (0..=99).contains(&raw.exit_code)
                && raw.size_download <= 1_048_576,
            "curl measurement exceeds declared limits"
        );
        let endpoint_index = raw.request_index as usize % requests.len();
        samples.push(HttpSample {
            iteration,
            request_index: raw.request_index,
            endpoint_index,
            status_code,
            expected_status: requests[endpoint_index].expected_status,
            curl_exit_code: raw.exit_code,
            duration_ms: raw.time_total * 1000.0,
            response_bytes: raw.size_download,
            succeeded: raw.exit_code == 0
                && status_code == requests[endpoint_index].expected_status,
        });
    }
    samples.sort_by_key(|sample| sample.request_index);
    ensure!(
        samples.len() == *count as usize
            && samples
                .iter()
                .enumerate()
                .all(|(index, sample)| sample.request_index as usize == index),
        "curl did not emit exactly the declared request measurements"
    );
    ensure!(
        batch.exit_code == Some(0) || samples.iter().any(|sample| sample.curl_exit_code != 0),
        "curl failed without a transfer error measurement"
    );
    Ok((
        HttpBatch {
            iteration,
            wall_time_ms: batch.duration_ms,
        },
        samples,
    ))
}

fn quoted(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    )
}

fn read_bounded(path: &Path, maximum: u64) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= maximum,
        "collector control or output file exceeds limit or is not regular"
    );
    let file = File::open(path)?;
    let mut contents = String::new();
    file.take(maximum + 1).read_to_string(&mut contents)?;
    ensure!(
        contents.len() as u64 <= maximum,
        "collector output exceeds limit"
    );
    Ok(contents)
}

#[must_use]
pub fn metrics_from_evidence(evidence: &HttpEvidence) -> MetricsDocument {
    let latencies: Vec<f64> = evidence
        .samples
        .iter()
        .map(|sample| sample.duration_ms)
        .collect();
    let success: Vec<f64> = evidence
        .samples
        .iter()
        .map(|sample| f64::from(sample.succeeded))
        .collect();
    let errors: Vec<f64> = success.iter().map(|value| 1.0 - value).collect();
    let throughput: Vec<f64> = evidence
        .batches
        .iter()
        .map(|batch| f64::from(evidence.request_count_per_iteration) * 1000.0 / batch.wall_time_ms)
        .collect();
    let metrics = [
        ("http.latency", "ms", PreferredDirection::Lower, latencies),
        (
            "http.success_rate",
            "ratio",
            PreferredDirection::Higher,
            success,
        ),
        (
            "http.error_rate",
            "ratio",
            PreferredDirection::Lower,
            errors,
        ),
        (
            "http.throughput",
            "requests/s",
            PreferredDirection::Higher,
            throughput,
        ),
    ]
    .into_iter()
    .map(|(id, unit, preferred_direction, values)| MetricSummary {
        id: id.to_owned(),
        unit: unit.to_owned(),
        preferred_direction,
        statistics: statistics(&values),
    })
    .collect();
    let samples = evidence
        .samples
        .iter()
        .enumerate()
        .map(|(index, sample)| MeasurementSample {
            iteration: index as u32 + 1,
            duration_ms: sample.duration_ms,
            max_rss_kib: None,
            exit_code: Some(sample.curl_exit_code),
            timed_out: sample.curl_exit_code == 28,
            succeeded: sample.succeeded,
        })
        .collect();
    MetricsDocument {
        schema_version: METRICS_SCHEMA_V1.to_owned(),
        scenario_id: evidence.scenario_id.clone(),
        samples,
        metrics,
    }
}

pub fn validate_evidence(
    scenario: &crate::contract::ScenarioEvidence,
    evidence: &HttpEvidence,
    metrics: &MetricsDocument,
) -> Result<()> {
    let crate::contract::TargetEvidence::HttpWorkload {
        request_count,
        endpoint_count,
        concurrency,
        request_timeout_seconds,
        ..
    } = &scenario.target
    else {
        bail!("http-curl requires an HTTP workload target");
    };
    ensure!(
        scenario.collectors == [Collector::HttpCurl]
            && evidence.schema_version == SCHEMA
            && evidence.scenario_id == scenario.id,
        "HTTP evidence identity mismatch"
    );
    ensure!(
        (1..=1000).contains(request_count)
            && (1..=32).contains(concurrency)
            && concurrency <= request_count
            && (1..=32).contains(endpoint_count)
            && *endpoint_count <= *request_count as usize
            && (1..=30).contains(request_timeout_seconds),
        "invalid HTTP workload bounds"
    );
    ensure!(
        evidence.concurrency == *concurrency
            && evidence.request_count_per_iteration == *request_count,
        "HTTP evidence workload bounds differ"
    );
    ensure!(
        evidence.adapter_digest.len() == 64
            && evidence
                .adapter_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            && !evidence.collector_version.is_empty()
            && evidence.collector_version.len() <= 4096,
        "HTTP collector identity missing or invalid"
    );
    ensure!(
        evidence.overhead_status == "not-isolated"
            && [
                evidence.fixture_setup_ms,
                evidence.fixture_teardown_ms,
                evidence.collector_wall_time_ms
            ]
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0),
        "invalid HTTP overhead metadata"
    );
    let iterations = scenario.run.measurement_iterations as usize;
    ensure!(
        (1..=10).contains(&iterations)
            && scenario.run.warmup_iterations <= 3
            && (1..=60).contains(&scenario.run.timeout_seconds)
            && iterations * *request_count as usize <= 10_000
            && evidence.batches.len() == iterations
            && evidence.samples.len() == iterations * *request_count as usize,
        "HTTP evidence request count mismatch"
    );
    for (index, batch) in evidence.batches.iter().enumerate() {
        ensure!(
            batch.iteration as usize == index
                && batch.wall_time_ms.is_finite()
                && batch.wall_time_ms > 0.0
                && batch.wall_time_ms <= scenario.run.timeout_seconds as f64 * 1000.0 + 1000.0,
            "invalid HTTP batch measurement"
        );
    }
    for (index, sample) in evidence.samples.iter().enumerate() {
        ensure!(
            sample.iteration as usize == index / *request_count as usize
                && sample.request_index as usize == index % *request_count as usize
                && sample.endpoint_index == sample.request_index as usize % endpoint_count,
            "HTTP request identity mismatch"
        );
        ensure!(
            sample.status_code <= 599
                && (100..=599).contains(&sample.expected_status)
                && (0..=99).contains(&sample.curl_exit_code)
                && sample.response_bytes <= 1_048_576
                && sample.duration_ms.is_finite()
                && sample.duration_ms >= 0.0
                && sample.duration_ms <= *request_timeout_seconds as f64 * 1000.0 + 1000.0,
            "HTTP request measurement exceeds bounds"
        );
        ensure!(
            sample.succeeded
                == (sample.curl_exit_code == 0 && sample.status_code == sample.expected_status),
            "HTTP success state contradicts request measurement"
        );
    }
    ensure!(
        &metrics_from_evidence(evidence) == metrics,
        "normalized HTTP metrics differ from recorded request evidence"
    );
    Ok(())
}
