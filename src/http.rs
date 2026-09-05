use std::collections::{BTreeMap, BTreeSet};
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use crate::capture::{ensure_not_interrupted, statistics};
use crate::contract::{
    CollectorPlan, METRICS_SCHEMA_V1, MeasurementSample, MetricSummary, MetricsDocument,
    PreferredDirection, Target, TargetEvidence,
};
use crate::scenario::LoadedScenario;

pub const COLLECTOR_ID: &str = "http";
pub const RESPONSE_TIME_METRIC_ID: &str = "http.response_time";
pub const SUCCESS_RATE_METRIC_ID: &str = "http.success_rate";

const MAX_REQUEST_TARGET_BYTES: usize = 2_048;
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_HEADERS: usize = 32;
const MAX_HEADER_NAME_BYTES: usize = 64;
const MAX_HEADER_VALUE_BYTES: usize = 1024;
const MAX_EXPECTED_STATUSES: usize = 16;
const IO_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedHttpUrl {
    host: String,
    port: u16,
    request_target: String,
}

impl ParsedHttpUrl {
    fn origin(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

#[must_use]
pub fn collector_plan() -> CollectorPlan {
    let mut configuration = BTreeMap::new();
    configuration.insert("transport".to_owned(), "http/1.1".to_owned());
    configuration.insert("scope".to_owned(), "loopback-only".to_owned());
    configuration.insert(
        "max_response_bytes".to_owned(),
        MAX_RESPONSE_BYTES.to_string(),
    );

    CollectorPlan {
        id: COLLECTOR_ID.to_owned(),
        supported: true,
        measurements: vec![
            RESPONSE_TIME_METRIC_ID.to_owned(),
            SUCCESS_RATE_METRIC_ID.to_owned(),
        ],
        reason: Some(
            "implemented: bounded loopback HTTP/1.1 request latency and expected-status evidence"
                .to_owned(),
        ),
        tool_version: None,
        configuration,
    }
}

pub fn validate_target(target: &Target) -> Result<()> {
    let Target::Http {
        url,
        method,
        headers,
        body,
        expected_statuses,
    } = target
    else {
        bail!("HTTP target validation requires an HTTP target");
    };

    parse_loopback_http_url(url)?;
    ensure!(
        !method.is_empty()
            && method.len() <= 32
            && method
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte == b'-'),
        "HTTP method must be an uppercase ASCII token"
    );
    ensure!(
        headers.len() <= MAX_HEADERS,
        "HTTP target may declare at most {MAX_HEADERS} headers"
    );
    for (name, value) in headers {
        validate_header(name, value)?;
    }
    let body_bytes = body.as_deref().unwrap_or_default().as_bytes();
    ensure!(
        body_bytes.len() <= MAX_REQUEST_BODY_BYTES,
        "HTTP request body exceeds the {MAX_REQUEST_BODY_BYTES} byte safety limit"
    );
    ensure!(
        !expected_statuses.is_empty() && expected_statuses.len() <= MAX_EXPECTED_STATUSES,
        "HTTP expected_statuses must contain between 1 and {MAX_EXPECTED_STATUSES} values"
    );
    ensure!(
        expected_statuses
            .iter()
            .all(|status| (100..=599).contains(status)),
        "HTTP expected statuses must be between 100 and 599"
    );
    let unique: BTreeSet<u16> = expected_statuses.iter().copied().collect();
    ensure!(
        unique.len() == expected_statuses.len(),
        "HTTP expected_statuses must not contain duplicates"
    );

    Ok(())
}

pub fn target_evidence(target: &Target) -> Result<TargetEvidence> {
    validate_target(target)?;
    let Target::Http {
        url,
        method,
        headers,
        body,
        expected_statuses,
    } = target
    else {
        unreachable!("validated HTTP target must remain HTTP");
    };
    let parsed = parse_loopback_http_url(url)?;

    Ok(TargetEvidence {
        target_type: "http".to_owned(),
        program: None,
        argument_count: 0,
        working_directory_set: false,
        inherited_environment_names: Vec::new(),
        http_origin: Some(parsed.origin()),
        http_method: Some(method.clone()),
        http_request_target_bytes: parsed.request_target.len(),
        http_request_body_bytes: body.as_deref().unwrap_or_default().len(),
        http_header_names: headers.keys().cloned().collect(),
        http_expected_statuses: expected_statuses.clone(),
    })
}

pub fn capture_metrics(loaded: &LoadedScenario) -> Result<MetricsDocument> {
    validate_target(&loaded.scenario.target)?;

    for warmup in 0..loaded.scenario.run.warmup_iterations {
        let sample = execute_once(loaded, warmup + 1)?;
        if !sample.succeeded {
            bail!(
                "HTTP warm-up iteration {} did not succeed (status={:?}, timed_out={}, failure={:?})",
                warmup + 1,
                sample.http_status,
                sample.timed_out,
                sample.http_failure_kind
            );
        }
    }

    let mut samples = Vec::with_capacity(loaded.scenario.run.measurement_iterations as usize);
    for iteration in 1..=loaded.scenario.run.measurement_iterations {
        samples.push(execute_once(loaded, iteration)?);
    }

    let successes: Vec<f64> = samples
        .iter()
        .map(|sample| if sample.succeeded { 1.0 } else { 0.0 })
        .collect();
    let response_times: Vec<f64> = samples
        .iter()
        .filter(|sample| sample.http_status.is_some())
        .map(|sample| sample.duration_ms)
        .collect();

    let mut metrics = vec![MetricSummary {
        id: SUCCESS_RATE_METRIC_ID.to_owned(),
        unit: "ratio".to_owned(),
        preferred_direction: PreferredDirection::Higher,
        statistics: statistics(&successes),
    }];
    if !response_times.is_empty() {
        metrics.insert(
            0,
            MetricSummary {
                id: RESPONSE_TIME_METRIC_ID.to_owned(),
                unit: "ms".to_owned(),
                preferred_direction: PreferredDirection::Lower,
                statistics: statistics(&response_times),
            },
        );
    }

    Ok(MetricsDocument {
        schema_version: METRICS_SCHEMA_V1.to_owned(),
        scenario_id: loaded.scenario.id.clone(),
        samples,
        metrics,
    })
}

fn execute_once(loaded: &LoadedScenario, iteration: u32) -> Result<MeasurementSample> {
    ensure_not_interrupted()?;
    let Target::Http {
        url,
        method,
        headers,
        body,
        expected_statuses,
    } = &loaded.scenario.target
    else {
        bail!("HTTP capture requires an HTTP target");
    };
    let parsed = parse_loopback_http_url(url)?;
    let timeout = Duration::from_secs(loaded.scenario.run.timeout_seconds);
    let started = Instant::now();

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, parsed.port));
    let mut stream = match TcpStream::connect_timeout(&address, timeout) {
        Ok(stream) => stream,
        Err(error) => {
            return Ok(failure_sample(
                iteration,
                started.elapsed(),
                is_timeout(&error),
                if is_timeout(&error) {
                    "timeout"
                } else {
                    "connect-error"
                },
            ));
        }
    };
    stream
        .set_nodelay(true)
        .context("failed to configure loopback HTTP socket")?;
    stream
        .set_write_timeout(Some(timeout))
        .context("failed to configure HTTP write timeout")?;
    stream
        .set_read_timeout(Some(IO_POLL_INTERVAL))
        .context("failed to configure HTTP read polling")?;

    let request = build_request(&parsed, method, headers, body.as_deref())?;
    if let Err(error) = stream.write_all(&request) {
        return Ok(failure_sample(
            iteration,
            started.elapsed(),
            is_timeout(&error),
            if is_timeout(&error) {
                "timeout"
            } else {
                "write-error"
            },
        ));
    }

    let deadline = started + timeout;
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        ensure_not_interrupted()?;
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if response.len().saturating_add(read) > MAX_RESPONSE_BYTES {
                    return Ok(failure_sample(
                        iteration,
                        started.elapsed(),
                        false,
                        "response-too-large",
                    ));
                }
                response.extend_from_slice(&buffer[..read]);
            }
            Err(error) if is_timeout(&error) && Instant::now() < deadline => continue,
            Err(error) if is_timeout(&error) => {
                return Ok(failure_sample(
                    iteration,
                    started.elapsed(),
                    true,
                    "timeout",
                ));
            }
            Err(_) => {
                return Ok(failure_sample(
                    iteration,
                    started.elapsed(),
                    false,
                    "read-error",
                ));
            }
        }
    }

    let status = match parse_status(&response) {
        Ok(status) => status,
        Err(_) => {
            return Ok(MeasurementSample {
                iteration,
                duration_ms: started.elapsed().as_secs_f64() * 1_000.0,
                max_rss_kib: None,
                exit_code: None,
                timed_out: false,
                succeeded: false,
                http_status: None,
                response_bytes: Some(response.len() as u64),
                http_failure_kind: Some("protocol-error".to_owned()),
            });
        }
    };
    let succeeded = expected_statuses.contains(&status);

    Ok(MeasurementSample {
        iteration,
        duration_ms: started.elapsed().as_secs_f64() * 1_000.0,
        max_rss_kib: None,
        exit_code: None,
        timed_out: false,
        succeeded,
        http_status: Some(status),
        response_bytes: Some(response.len() as u64),
        http_failure_kind: (!succeeded).then(|| "unexpected-status".to_owned()),
    })
}

fn failure_sample(
    iteration: u32,
    elapsed: Duration,
    timed_out: bool,
    failure_kind: &str,
) -> MeasurementSample {
    MeasurementSample {
        iteration,
        duration_ms: elapsed.as_secs_f64() * 1_000.0,
        max_rss_kib: None,
        exit_code: None,
        timed_out,
        succeeded: false,
        http_status: None,
        response_bytes: None,
        http_failure_kind: Some(failure_kind.to_owned()),
    }
}

fn build_request(
    parsed: &ParsedHttpUrl,
    method: &str,
    headers: &BTreeMap<String, String>,
    body: Option<&str>,
) -> Result<Vec<u8>> {
    let body = body.unwrap_or_default().as_bytes();
    let mut request = Vec::with_capacity(512 + body.len());
    write!(
        request,
        "{method} {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nContent-Length: {}\r\n",
        parsed.request_target,
        parsed.host,
        parsed.port,
        body.len()
    )
    .context("failed to build HTTP request line")?;
    for (name, value) in headers {
        write!(request, "{name}: {value}\r\n").context("failed to build HTTP request header")?;
    }
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(body);
    Ok(request)
}

fn parse_status(response: &[u8]) -> Result<u16> {
    let line_end = response
        .windows(2)
        .position(|window| window == b"\r\n")
        .context("HTTP response is missing a status-line terminator")?;
    let status_line = std::str::from_utf8(&response[..line_end])
        .context("HTTP response status line is not UTF-8")?;
    let mut fields = status_line.split_whitespace();
    let version = fields.next().context("HTTP response is missing a version")?;
    ensure!(
        matches!(version, "HTTP/1.0" | "HTTP/1.1"),
        "unsupported HTTP response version: {version}"
    );
    let status = fields
        .next()
        .context("HTTP response is missing a status code")?
        .parse::<u16>()
        .context("HTTP response status code is invalid")?;
    ensure!(
        (100..=599).contains(&status),
        "HTTP response status code is out of range"
    );
    Ok(status)
}

fn validate_header(name: &str, value: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= MAX_HEADER_NAME_BYTES
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "HTTP header names must be bounded ASCII tokens"
    );
    ensure!(
        !matches!(
            name.to_ascii_lowercase().as_str(),
            "host" | "content-length" | "connection"
        ),
        "HTTP header {name} is owned by runtime-profiler"
    );
    ensure!(
        value.len() <= MAX_HEADER_VALUE_BYTES
            && !value.contains('\r')
            && !value.contains('\n'),
        "HTTP header value for {name} is invalid or too large"
    );
    Ok(())
}

fn parse_loopback_http_url(url: &str) -> Result<ParsedHttpUrl> {
    ensure!(url.len() <= 4_096, "HTTP URL exceeds the safety limit");
    ensure!(
        !url.contains('#'),
        "HTTP workload URLs must not contain fragments"
    );
    let rest = url
        .strip_prefix("http://")
        .context("HTTP workload URL must use http://")?;
    let split = rest
        .find(|character| matches!(character, '/' | '?'))
        .unwrap_or(rest.len());
    let authority = &rest[..split];
    let suffix = &rest[split..];
    ensure!(!authority.is_empty(), "HTTP URL is missing an authority");
    ensure!(
        !authority.contains('@'),
        "HTTP workload URL must not contain user information"
    );
    ensure!(
        !authority.contains('[') && !authority.contains(']'),
        "HTTP workload URL IPv6 literals are not supported in this first loopback adapter"
    );

    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => {
            ensure!(!host.is_empty(), "HTTP URL host is empty");
            let port = port.parse::<u16>().context("HTTP URL port is invalid")?;
            ensure!(port > 0, "HTTP URL port must be non-zero");
            (host, port)
        }
        None => (authority, 80),
    };
    ensure!(
        matches!(host, "127.0.0.1" | "localhost"),
        "HTTP workload target must be loopback (127.0.0.1 or localhost)"
    );

    let request_target = if suffix.is_empty() {
        "/".to_owned()
    } else if suffix.starts_with('?') {
        format!("/{suffix}")
    } else {
        suffix.to_owned()
    };
    ensure!(
        request_target.len() <= MAX_REQUEST_TARGET_BYTES,
        "HTTP request target exceeds the {MAX_REQUEST_TARGET_BYTES} byte safety limit"
    );
    ensure!(
        request_target.is_ascii()
            && request_target
                .bytes()
                .all(|byte| !byte.is_ascii_control() && byte != b' '),
        "HTTP request target must contain only visible ASCII without spaces"
    );

    Ok(ParsedHttpUrl {
        host: host.to_owned(),
        port,
        request_target,
    })
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;

    use super::*;
    use crate::contract::{Collector, RunConfig, Scenario};
    use crate::digest::sha256_bytes;

    #[test]
    fn loopback_url_parser_rejects_remote_targets_and_redacts_query() {
        let parsed = parse_loopback_http_url("http://127.0.0.1:8080/health?token=secret")
            .expect("valid loopback URL");
        assert_eq!(parsed.port, 8080);
        assert_eq!(parsed.request_target, "/health?token=secret");
        assert_eq!(parsed.origin(), "http://127.0.0.1:8080");

        assert!(parse_loopback_http_url("https://127.0.0.1/health").is_err());
        assert!(parse_loopback_http_url("http://example.com/health").is_err());
        assert!(parse_loopback_http_url("http://user@127.0.0.1/health").is_err());
    }

    #[test]
    fn evidence_keeps_header_values_and_request_target_out() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_owned(), "Bearer secret".to_owned());
        let target = Target::Http {
            url: "http://127.0.0.1:8080/private?token=secret".to_owned(),
            method: "POST".to_owned(),
            headers,
            body: Some("private-body".to_owned()),
            expected_statuses: vec![201],
        };

        let evidence = target_evidence(&target).expect("target evidence");
        let serialized = serde_json::to_string(&evidence).expect("serialize evidence");
        assert!(!serialized.contains("Bearer secret"));
        assert!(!serialized.contains("private?token=secret"));
        assert!(!serialized.contains("private-body"));
        assert!(serialized.contains("Authorization"));
        assert!(serialized.contains("POST"));
    }

    #[test]
    fn local_http_capture_records_expected_status_and_latency() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind test server");
        let port = listener.local_addr().expect("test server address").port();
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept test request");
                let mut request = [0_u8; 2048];
                let read = stream.read(&mut request).expect("read test request");
                let request = std::str::from_utf8(&request[..read]).expect("request UTF-8");
                assert!(request.starts_with("GET /health HTTP/1.1\r\n"));
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                    .expect("write test response");
            }
        });

        let scenario = Scenario {
            schema_version: "runtime-profiler/scenario/v1".to_owned(),
            id: "http-test".to_owned(),
            target: Target::Http {
                url: format!("http://127.0.0.1:{port}/health"),
                method: "GET".to_owned(),
                headers: BTreeMap::new(),
                body: None,
                expected_statuses: vec![200],
            },
            run: RunConfig {
                warmup_iterations: 1,
                measurement_iterations: 2,
                timeout_seconds: 2,
            },
            collectors: vec![Collector::Http],
        };
        let normalized = serde_json::to_vec(&scenario).expect("scenario serialization");
        let loaded = LoadedScenario {
            scenario,
            source_path: PathBuf::from("http-test.json"),
            digest: sha256_bytes(&normalized),
        };

        let metrics = capture_metrics(&loaded).expect("capture HTTP metrics");
        server.join().expect("test server thread");

        assert_eq!(metrics.samples.len(), 2);
        assert!(metrics.samples.iter().all(|sample| sample.succeeded));
        assert!(
            metrics
                .samples
                .iter()
                .all(|sample| sample.http_status == Some(200))
        );
        assert_eq!(metrics.metrics[0].id, RESPONSE_TIME_METRIC_ID);
        assert_eq!(metrics.metrics[1].id, SUCCESS_RATE_METRIC_ID);
    }

    #[test]
    fn unexpected_status_is_evidence_not_transport_success() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind test server");
        let port = listener.local_addr().expect("test server address").port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).expect("read request");
            stream
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .expect("write response");
        });

        let scenario = Scenario {
            schema_version: "runtime-profiler/scenario/v1".to_owned(),
            id: "http-status-test".to_owned(),
            target: Target::Http {
                url: format!("http://127.0.0.1:{port}/health"),
                method: "GET".to_owned(),
                headers: BTreeMap::new(),
                body: None,
                expected_statuses: vec![200],
            },
            run: RunConfig {
                warmup_iterations: 0,
                measurement_iterations: 1,
                timeout_seconds: 2,
            },
            collectors: vec![Collector::Http],
        };
        let normalized = serde_json::to_vec(&scenario).expect("scenario serialization");
        let loaded = LoadedScenario {
            scenario,
            source_path: PathBuf::from("http-status-test.json"),
            digest: sha256_bytes(&normalized),
        };

        let metrics = capture_metrics(&loaded).expect("capture HTTP metrics");
        server.join().expect("test server thread");

        let sample = &metrics.samples[0];
        assert!(!sample.succeeded);
        assert_eq!(sample.http_status, Some(503));
        assert_eq!(sample.http_failure_kind.as_deref(), Some("unexpected-status"));
    }
}
