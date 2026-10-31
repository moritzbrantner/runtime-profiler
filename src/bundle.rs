//! Immutable bundle artifacts: creation, integrity validation, and agent guidance.

mod create;
mod environment;
mod guidance;
mod validate;

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::browser_chromium::{
    BROWSER_RUNTIME_ARTIFACT, BROWSER_RUNTIME_MEDIA_TYPE, RAW_TRACE_ARTIFACT, RAW_TRACE_MEDIA_TYPE,
    TRACE_SUMMARY_ARTIFACT, TRACE_SUMMARY_MEDIA_TYPE,
};
use crate::native_perf::{RAW_REPORT_ARTIFACT, RAW_REPORT_MEDIA_TYPE};

pub use create::capture_bundle;
pub use guidance::render_agent_guidance;
pub use validate::{summarize_bundle, validate_bundle};

const REQUIRED_ARTIFACTS: [(&str, &str); 5] = [
    ("scenario.json", "application/json"),
    ("environment.json", "application/json"),
    ("metrics.json", "application/json"),
    ("hotspots.json", "application/json"),
    ("agent-guidance.json", "application/json"),
];
const OPTIONAL_ARTIFACTS: [(&str, &str); 5] = [
    (crate::http_workload::ARTIFACT, "application/json"),
    (RAW_REPORT_ARTIFACT, RAW_REPORT_MEDIA_TYPE),
    (RAW_TRACE_ARTIFACT, RAW_TRACE_MEDIA_TYPE),
    (TRACE_SUMMARY_ARTIFACT, TRACE_SUMMARY_MEDIA_TYPE),
    (BROWSER_RUNTIME_ARTIFACT, BROWSER_RUNTIME_MEDIA_TYPE),
];

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
