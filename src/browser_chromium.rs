use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::capture::execute_prepared_command;
use crate::chromium_trace::{ChromiumTraceSummary, analyze_chromium_trace_bytes};
use crate::contract::{CollectorPlan, Detection, Target};
use crate::digest::{sha256_bytes, sha256_file};
use crate::scenario::LoadedScenario;

pub const COLLECTOR_ID: &str = "browser-chromium";
pub const RAW_TRACE_ARTIFACT: &str = "chromium-trace.json";
pub const RAW_TRACE_MEDIA_TYPE: &str = "application/json";
pub const TRACE_SUMMARY_ARTIFACT: &str = "chromium-trace-summary.json";
pub const TRACE_SUMMARY_MEDIA_TYPE: &str = "application/json";
pub const BROWSER_RUNTIME_ARTIFACT: &str = "browser-runtime.json";
pub const BROWSER_RUNTIME_MEDIA_TYPE: &str = "application/json";
pub const BROWSER_RUNTIME_SCHEMA_V1: &str = "runtime-profiler/browser-runtime/v1";

const DRIVER_SOURCE: &str = include_str!("../scripts/playwright-driver.mjs");
const TRACE_NORMALIZER_SOURCE: &str = include_str!("chromium_trace.rs");
const DRIVER_FILE: &str = "playwright-driver.mjs";
const DRIVER_TRACE_FILE: &str = "trace.json";
const DRIVER_METADATA_FILE: &str = "browser-runtime.json";
const MAX_RUNTIME_METADATA_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub struct BrowserChromiumCapture {
    pub trace: Vec<u8>,
    pub summary: ChromiumTraceSummary,
    pub runtime: BrowserRuntimeDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRuntimeDocument {
    pub schema_version: String,
    pub adapter_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalizer_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journey_digest: Option<String>,
    pub node_version: String,
    pub playwright_version: String,
    pub browser_name: String,
    pub browser_version: String,
    pub viewport: BrowserViewport,
    pub trace_categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserViewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
struct TempCaptureDirectory {
    path: PathBuf,
}

impl Drop for TempCaptureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[must_use]
pub fn detect_browser_chromium() -> Detection {
    let output = Command::new("node")
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8(output.stdout).ok();
            let version = version
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            Detection {
                available: true,
                reason: "implemented: Node is available; Playwright and Chromium are verified in the consumer working directory at capture time".to_owned(),
                tool_version: version.map(ToOwned::to_owned),
            }
        }
        _ => Detection {
            available: false,
            reason:
                "browser-chromium requires Node and a consumer-provided Playwright installation"
                    .to_owned(),
            tool_version: None,
        },
    }
}

#[must_use]
pub fn collector_plan() -> CollectorPlan {
    let detection = detect_browser_chromium();
    let mut configuration = BTreeMap::new();
    configuration.insert("browser".to_owned(), "chromium".to_owned());
    configuration.insert("viewport".to_owned(), "1280x720".to_owned());
    configuration.insert(
        "trace_categories".to_owned(),
        "devtools.timeline,v8,v8.execute,blink.user_timing".to_owned(),
    );
    CollectorPlan {
        id: COLLECTOR_ID.to_owned(),
        supported: detection.available,
        measurements: vec![
            "browser.long_tasks".to_owned(),
            "browser.hot_paths".to_owned(),
            "browser.js_wasm_boundaries".to_owned(),
        ],
        reason: Some(detection.reason),
        tool_version: detection.tool_version,
        configuration,
    }
}

pub fn capture_browser_chromium(loaded: &LoadedScenario) -> Result<BrowserChromiumCapture> {
    let Target::BrowserJourney {
        module,
        working_directory,
        ..
    } = &loaded.scenario.target
    else {
        bail!("browser-chromium collector requires a browser-journey target");
    };

    let detection = detect_browser_chromium();
    ensure!(
        detection.available,
        "browser-chromium collector unavailable: {}",
        detection.reason
    );

    let working_directory = resolve_working_directory(loaded, working_directory.as_deref());
    let journey = working_directory.join(module);
    ensure!(
        journey.is_file(),
        "browser journey module does not exist: {}",
        journey.display()
    );
    let journey_digest_before = sha256_file(&journey)?;

    let temp = create_temp_capture_directory()?;
    let driver = temp.path.join(DRIVER_FILE);
    let trace = temp.path.join(DRIVER_TRACE_FILE);
    let metadata = temp.path.join(DRIVER_METADATA_FILE);
    fs::write(&driver, DRIVER_SOURCE).context("failed to materialize Playwright driver")?;

    let mut command = Command::new("node");
    command
        .arg(&driver)
        .arg("--journey")
        .arg(&journey)
        .arg("--trace")
        .arg(&trace)
        .arg("--metadata")
        .arg(&metadata);
    let sample = execute_prepared_command(loaded, command, 1, "Playwright browser journey")
        .context("browser-chromium capture failed")?;
    ensure!(
        sample.succeeded,
        "browser journey did not succeed (exit_code={:?}, timed_out={})",
        sample.exit_code,
        sample.timed_out
    );

    let trace = fs::read(&trace).context("Playwright driver did not produce a Chromium trace")?;
    let summary = analyze_chromium_trace_bytes(&trace)
        .context("captured Chromium trace could not be normalized")?;
    let metadata =
        fs::read(&metadata).context("Playwright driver did not produce runtime metadata")?;
    ensure!(
        metadata.len() <= MAX_RUNTIME_METADATA_BYTES,
        "browser runtime metadata exceeds the {} byte safety limit",
        MAX_RUNTIME_METADATA_BYTES
    );
    let mut runtime: BrowserRuntimeDocument =
        serde_json::from_slice(&metadata).context("browser runtime metadata is invalid")?;

    let journey_digest_after = sha256_file(&journey)?;
    ensure!(
        journey_digest_before == journey_digest_after,
        "browser journey module changed during capture: {}",
        journey.display()
    );
    runtime.adapter_digest = Some(prefixed_sha256(DRIVER_SOURCE.as_bytes()));
    runtime.normalizer_digest = Some(prefixed_sha256(TRACE_NORMALIZER_SOURCE.as_bytes()));
    runtime.journey_digest = Some(format!("sha256:{journey_digest_before}"));
    validate_runtime_metadata(&runtime)?;

    Ok(BrowserChromiumCapture {
        trace,
        summary,
        runtime,
    })
}

pub(crate) fn validate_runtime_metadata(runtime: &BrowserRuntimeDocument) -> Result<()> {
    ensure!(
        runtime.schema_version == BROWSER_RUNTIME_SCHEMA_V1,
        "unsupported browser runtime metadata schema: {}",
        runtime.schema_version
    );
    ensure!(
        runtime.browser_name == "chromium",
        "browser adapter returned a non-Chromium browser"
    );
    ensure!(
        runtime.viewport.width > 0 && runtime.viewport.height > 0,
        "browser viewport must be positive"
    );
    ensure!(
        !runtime.trace_categories.is_empty(),
        "browser trace categories are missing"
    );
    for value in [
        runtime.adapter_version.as_str(),
        runtime.node_version.as_str(),
        runtime.playwright_version.as_str(),
        runtime.browser_version.as_str(),
    ] {
        ensure!(
            !value.is_empty() && value.len() <= 512,
            "browser runtime identity is missing or too large"
        );
    }
    for (label, digest) in [
        ("browser adapter digest", runtime.adapter_digest.as_deref()),
        (
            "Chromium trace normalizer digest",
            runtime.normalizer_digest.as_deref(),
        ),
        ("browser journey digest", runtime.journey_digest.as_deref()),
    ] {
        ensure!(
            digest.is_some_and(is_prefixed_sha256),
            "{label} is missing or invalid"
        );
    }
    Ok(())
}

fn prefixed_sha256(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_bytes(bytes))
}

fn is_prefixed_sha256(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn resolve_working_directory(loaded: &LoadedScenario, configured: Option<&Path>) -> PathBuf {
    let scenario_directory = loaded
        .source_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    match configured {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => scenario_directory.join(path),
        None => scenario_directory.to_path_buf(),
    }
}

fn create_temp_capture_directory() -> Result<TempCaptureDirectory> {
    let base = env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_nanos();
    for attempt in 0..16_u8 {
        let path = base.join(format!(
            "runtime-profiler-browser-chromium-{}-{nanos}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(TempCaptureDirectory { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to create browser capture directory: {}",
                        path.display()
                    )
                });
            }
        }
    }
    bail!("failed to allocate a unique browser capture directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> BrowserRuntimeDocument {
        BrowserRuntimeDocument {
            schema_version: BROWSER_RUNTIME_SCHEMA_V1.to_owned(),
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
            trace_categories: vec!["devtools.timeline".to_owned()],
        }
    }

    #[test]
    fn validates_complete_runtime_metadata() {
        assert!(validate_runtime_metadata(&runtime()).is_ok());
    }

    #[test]
    fn rejects_missing_runtime_identity() {
        let mut runtime = runtime();
        runtime.adapter_version = String::new();
        assert!(validate_runtime_metadata(&runtime).is_err());
    }

    #[test]
    fn rejects_missing_comparison_digest() {
        let mut runtime = runtime();
        runtime.journey_digest = None;
        assert!(validate_runtime_metadata(&runtime).is_err());
    }
}
