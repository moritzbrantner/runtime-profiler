use std::collections::{BTreeMap, btree_map::Entry};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use crate::capture::execute_prepared_command;
use crate::contract::{
    CollectorPlan, Detection, HOTSPOTS_SCHEMA_V1, Hotspot, HotspotsDocument, Target,
};
use crate::digest::sha256_bytes;
use crate::scenario::LoadedScenario;

pub const COLLECTOR_ID: &str = "native-perf";
pub const METRIC_ID: &str = "native-perf.period";
pub const METRIC_UNIT: &str = "event-count";
pub const PERF_EVENT: &str = "cycles:u";
pub const PERF_SAMPLE_PERIOD: u64 = 100_000;
pub const SYMBOLIZATION_MODE: &str = "perf-report-srcline";
pub const TARGET_TOOLCHAIN_KIND: &str = "rustc";
pub const TARGET_TOOLCHAIN_FINGERPRINT_SCHEMA_V1: &str =
    "runtime-profiler/target-toolchain-fingerprint/rustc-v1";
pub const RAW_REPORT_ARTIFACT: &str = "native-perf-report.tsv";
pub const RAW_REPORT_MEDIA_TYPE: &str = "text/tab-separated-values; charset=utf-8";

const MAX_REPORT_BYTES: usize = 8 * 1024 * 1024;
const MAX_REPORT_LINES: usize = 100_000;
const MAX_FIELD_BYTES: usize = 2_048;
const MAX_HOTSPOTS: usize = 256;
const MAX_TOOLCHAIN_VERSION_BYTES: usize = 4 * 1024;

#[derive(Debug)]
pub struct NativePerfCapture {
    pub hotspots: HotspotsDocument,
    pub raw_report: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct HotspotKey {
    symbol: String,
    source_file: Option<String>,
    line: Option<u32>,
    dso: Option<String>,
}

#[derive(Debug, Serialize)]
struct TargetToolchainFingerprintInput<'a> {
    schema_version: &'static str,
    kind: &'static str,
    version: &'a str,
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
pub fn detect_native_perf() -> Detection {
    if env::consts::OS != "linux" {
        return detection_from_probe(env::consts::OS, None, None);
    }

    let version = perf_version();
    let probe = version.as_ref().map(|_| perf_recording_probe());
    detection_from_probe(env::consts::OS, version.as_deref(), probe)
}

#[must_use]
pub fn collector_plan() -> CollectorPlan {
    let detection = detect_native_perf();
    let mut configuration = BTreeMap::new();
    configuration.insert("event".to_owned(), PERF_EVENT.to_owned());
    configuration.insert("sample_period".to_owned(), PERF_SAMPLE_PERIOD.to_string());
    configuration.insert("symbolization".to_owned(), SYMBOLIZATION_MODE.to_owned());
    configuration.insert(
        "target_toolchain_fingerprint_schema".to_owned(),
        TARGET_TOOLCHAIN_FINGERPRINT_SCHEMA_V1.to_owned(),
    );

    CollectorPlan {
        id: COLLECTOR_ID.to_owned(),
        supported: detection.available,
        measurements: vec![METRIC_ID.to_owned()],
        reason: Some(detection.reason),
        tool_version: detection.tool_version,
        configuration,
    }
}

pub fn capture_native_perf(loaded: &LoadedScenario) -> Result<NativePerfCapture> {
    let detection = detect_native_perf();
    ensure!(
        detection.available,
        "native-perf collector unavailable: {}",
        detection.reason
    );
    let tool_version = detection
        .tool_version
        .context("native-perf collector did not report a perf version")?;
    let target_toolchain_fingerprint = detect_target_toolchain_fingerprint()?;

    let temp = create_temp_capture_directory()?;
    let perf_data = temp.path.join("perf.data");

    let Target::Command { program, args, .. } = &loaded.scenario.target;
    let mut record = Command::new("perf");
    record
        .args(["record", "--quiet", "--no-buildid-cache", "--period"])
        .arg("--event")
        .arg(PERF_EVENT)
        .arg("--count")
        .arg(PERF_SAMPLE_PERIOD.to_string())
        .arg("--output")
        .arg(&perf_data)
        .arg("--")
        .arg(program)
        .args(args);

    let sample = execute_prepared_command(loaded, record, 1, "perf record")
        .context("native-perf capture failed")?;
    ensure!(
        sample.succeeded,
        "native-perf target did not succeed (exit_code={:?}, timed_out={})",
        sample.exit_code,
        sample.timed_out
    );

    let report = Command::new("perf")
        .args(["report", "--stdio", "--quiet"])
        .arg("--input")
        .arg(&perf_data)
        .args([
            "--percent-limit=0",
            "--sort=srcline,symbol,dso",
            "--fields=sample,period,srcline,symbol,dso",
            "--field-separator=\t",
        ])
        .stdin(Stdio::null())
        .output()
        .context("failed to run perf report")?;
    ensure!(
        report.status.success(),
        "perf report failed with status {}",
        report.status
    );
    ensure!(
        report.stdout.len() <= MAX_REPORT_BYTES,
        "perf report exceeds the {} byte safety limit",
        MAX_REPORT_BYTES
    );
    let raw_report = String::from_utf8(report.stdout).context("perf report output is not UTF-8")?;
    let hotspots = parse_perf_report(
        &raw_report,
        loaded,
        tool_version,
        target_toolchain_fingerprint,
    )?;

    Ok(NativePerfCapture {
        hotspots,
        raw_report,
    })
}

fn perf_version() -> Option<String> {
    let output = Command::new("perf")
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        None
    } else {
        Some(value.to_owned())
    }
}

fn perf_recording_probe() -> bool {
    Command::new("perf")
        .args(["stat", "--event", PERF_EVENT, "--", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn detection_from_probe(
    operating_system: &str,
    version: Option<&str>,
    recording_probe: Option<bool>,
) -> Detection {
    if operating_system != "linux" {
        return Detection {
            available: false,
            reason: format!(
                "unsupported platform: native-perf requires Linux, found {operating_system}"
            ),
            tool_version: None,
        };
    }

    let Some(version) = version else {
        return Detection {
            available: false,
            reason: "`perf` is not installed or did not return a usable version".to_owned(),
            tool_version: None,
        };
    };

    if recording_probe != Some(true) {
        return Detection {
            available: false,
            reason: format!(
                "{version} is installed but cannot record the `{PERF_EVENT}` event in this environment"
            ),
            tool_version: Some(version.to_owned()),
        };
    }

    Detection {
        available: true,
        reason: format!(
            "implemented: {version} can record `{PERF_EVENT}` for bounded native hotspot evidence"
        ),
        tool_version: Some(version.to_owned()),
    }
}

fn detect_target_toolchain_fingerprint() -> Result<Option<String>> {
    let output = match Command::new("rustc")
        .args(["--version", "--verbose"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to inspect target Rust toolchain"),
    };
    if !output.status.success() {
        return Ok(None);
    }
    ensure!(
        output.stdout.len() <= MAX_TOOLCHAIN_VERSION_BYTES,
        "rustc version output exceeds the {} byte safety limit",
        MAX_TOOLCHAIN_VERSION_BYTES
    );
    let version = String::from_utf8(output.stdout).context("rustc version output is not UTF-8")?;
    toolchain_fingerprint_from_version(&version).map(Some)
}

fn toolchain_fingerprint_from_version(version: &str) -> Result<String> {
    let normalized = version
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    ensure!(!normalized.is_empty(), "rustc version output is empty");
    ensure!(
        normalized.len() <= MAX_TOOLCHAIN_VERSION_BYTES,
        "rustc version output exceeds the {} byte safety limit",
        MAX_TOOLCHAIN_VERSION_BYTES
    );
    let input = TargetToolchainFingerprintInput {
        schema_version: TARGET_TOOLCHAIN_FINGERPRINT_SCHEMA_V1,
        kind: TARGET_TOOLCHAIN_KIND,
        version: &normalized,
    };
    let bytes = serde_json::to_vec(&input).context("failed to fingerprint target Rust toolchain")?;
    Ok(sha256_bytes(&bytes))
}

fn parse_perf_report(
    report: &str,
    loaded: &LoadedScenario,
    tool_version: String,
    target_toolchain_fingerprint: Option<String>,
) -> Result<HotspotsDocument> {
    ensure!(
        report.len() <= MAX_REPORT_BYTES,
        "perf report exceeds the {} byte safety limit",
        MAX_REPORT_BYTES
    );

    let root = source_root(loaded);
    let target_program = match &loaded.scenario.target {
        Target::Command { program, .. } => Path::new(program)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(program),
    };
    let mut aggregated: BTreeMap<HotspotKey, (u64, u64)> = BTreeMap::new();
    let mut processed_lines = 0_usize;

    for line in report.lines() {
        processed_lines += 1;
        ensure!(
            processed_lines <= MAX_REPORT_LINES,
            "perf report exceeds the {} line safety limit",
            MAX_REPORT_LINES
        );
        ensure!(
            line.len() <= MAX_FIELD_BYTES * 5,
            "perf report line exceeds the safety limit"
        );
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        ensure!(
            fields.len() == 5,
            "malformed perf report row: expected 5 tab-separated fields, found {}",
            fields.len()
        );
        let samples = parse_u64_field(fields[0], "sample")?;
        let weight = parse_u64_field(fields[1], "period")?;
        let (source_file, source_line) = normalize_source_location(fields[2], root.as_deref())?;
        let symbol = bounded_field(fields[3], "symbol")?;
        let dso = optional_bounded_field(fields[4], "dso")?;

        let key = HotspotKey {
            symbol,
            source_file,
            line: source_line,
            dso,
        };
        match aggregated.entry(key) {
            Entry::Occupied(mut entry) => {
                let values = entry.get_mut();
                values.0 = values.0.saturating_add(weight);
                values.1 = values.1.saturating_add(samples);
            }
            Entry::Vacant(entry) => {
                entry.insert((weight, samples));
            }
        }
    }

    let total_weight = aggregated
        .values()
        .fold(0_u64, |sum, (weight, _)| sum.saturating_add(*weight));
    let total_samples = aggregated
        .values()
        .fold(0_u64, |sum, (_, samples)| sum.saturating_add(*samples));

    let mut hotspots: Vec<Hotspot> = aggregated
        .into_iter()
        .map(|(key, (weight, samples))| {
            let confidence = hotspot_confidence(&key, target_program);
            let identity = format!(
                "{}\0{}\0{}\0{}\0{}",
                METRIC_ID,
                key.symbol,
                key.source_file.as_deref().unwrap_or(""),
                key.line.map_or_else(String::new, |line| line.to_string()),
                key.dso.as_deref().unwrap_or("")
            );
            let id = format!("hotspot-{}", sha256_bytes(identity.as_bytes()));
            Hotspot {
                id: id.clone(),
                symbol: key.symbol,
                source_file: key.source_file,
                line: key.line,
                dso: key.dso,
                metric: METRIC_ID.to_owned(),
                unit: METRIC_UNIT.to_owned(),
                weight,
                samples,
                confidence,
                evidence_ref: format!("hotspots.json#{id}"),
            }
        })
        .collect();
    hotspots.sort_by(|left, right| {
        right
            .weight
            .cmp(&left.weight)
            .then_with(|| left.symbol.cmp(&right.symbol))
            .then_with(|| left.source_file.cmp(&right.source_file))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.dso.cmp(&right.dso))
    });
    let truncated = hotspots.len() > MAX_HOTSPOTS;
    hotspots.truncate(MAX_HOTSPOTS);

    let reason = if total_samples == 0 {
        "perf capture completed but the normalized report contained no samples".to_owned()
    } else {
        format!(
            "perf capture normalized {} hotspot entries from {total_samples} samples",
            hotspots.len()
        )
    };
    let target_toolchain_kind = target_toolchain_fingerprint
        .as_ref()
        .map(|_| TARGET_TOOLCHAIN_KIND.to_owned());
    let target_toolchain_fingerprint_schema_version = target_toolchain_fingerprint
        .as_ref()
        .map(|_| TARGET_TOOLCHAIN_FINGERPRINT_SCHEMA_V1.to_owned());

    Ok(HotspotsDocument {
        schema_version: HOTSPOTS_SCHEMA_V1.to_owned(),
        status: "collected".to_owned(),
        reason,
        collector: Some(COLLECTOR_ID.to_owned()),
        tool_version: Some(tool_version),
        event: Some(PERF_EVENT.to_owned()),
        metric: Some(METRIC_ID.to_owned()),
        unit: Some(METRIC_UNIT.to_owned()),
        sample_period: Some(PERF_SAMPLE_PERIOD),
        symbolization_mode: Some(SYMBOLIZATION_MODE.to_owned()),
        target_toolchain_kind,
        target_toolchain_fingerprint_schema_version,
        target_toolchain_fingerprint,
        total_weight,
        total_samples,
        truncated,
        hotspots,
    })
}

fn parse_u64_field(value: &str, field: &str) -> Result<u64> {
    let normalized: String = value
        .trim()
        .chars()
        .filter(|character| !matches!(character, ',' | '_'))
        .collect();
    normalized
        .parse::<u64>()
        .with_context(|| format!("invalid perf {field} field: {value:?}"))
}

fn bounded_field(value: &str, field: &str) -> Result<String> {
    let value = value.trim();
    ensure!(!value.is_empty(), "perf {field} field is empty");
    ensure!(
        value.len() <= MAX_FIELD_BYTES,
        "perf {field} field exceeds the {} byte safety limit",
        MAX_FIELD_BYTES
    );
    Ok(value.to_owned())
}

fn optional_bounded_field(value: &str, field: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    ensure!(
        value.len() <= MAX_FIELD_BYTES,
        "perf {field} field exceeds the {} byte safety limit",
        MAX_FIELD_BYTES
    );
    Ok(Some(value.to_owned()))
}

fn normalize_source_location(
    value: &str,
    root: Option<&Path>,
) -> Result<(Option<String>, Option<u32>)> {
    let value = value.trim();
    if value.is_empty() || matches!(value, "??" | "??:0" | "[unknown]" | "[unknown]:0") {
        return Ok((None, None));
    }
    ensure!(
        value.len() <= MAX_FIELD_BYTES,
        "perf source location exceeds the {} byte safety limit",
        MAX_FIELD_BYTES
    );

    let (path, line) = value
        .rsplit_once(':')
        .map_or((value, None), |(path, line)| {
            (path, line.parse::<u32>().ok().filter(|line| *line > 0))
        });
    if path.is_empty() || path == "??" || path == "[unknown]" {
        return Ok((None, line));
    }

    let path = Path::new(path);
    let normalized = normalize_source_path(path, root);
    Ok((normalized, line))
}

fn normalize_source_path(path: &Path, root: Option<&Path>) -> Option<String> {
    if path.is_absolute() {
        if let Some(root) = root {
            if let Ok(relative) = path.strip_prefix(root) {
                return safe_relative_path(relative);
            }
        }
        return path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
    }
    safe_relative_path(path)
}

fn safe_relative_path(path: &Path) -> Option<String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let value = normalized.to_str()?;
    if value.is_empty() || value.len() > MAX_FIELD_BYTES {
        None
    } else {
        Some(value.to_owned())
    }
}

fn source_root(loaded: &LoadedScenario) -> Option<PathBuf> {
    let scenario_directory = loaded
        .source_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    match &loaded.scenario.target {
        Target::Command {
            working_directory: Some(directory),
            ..
        } if directory.is_absolute() => Some(directory.clone()),
        Target::Command {
            working_directory: Some(directory),
            ..
        } => Some(scenario_directory.join(directory)),
        Target::Command { .. } => Some(scenario_directory.to_path_buf()),
    }
}

fn hotspot_confidence(key: &HotspotKey, target_program: &str) -> String {
    if key.symbol == "[unknown]" || key.symbol == "??" {
        return "unresolved".to_owned();
    }
    if let (Some(_), Some(_)) = (&key.source_file, key.line) {
        return "source-location".to_owned();
    }
    if key
        .dso
        .as_deref()
        .is_some_and(|dso| dso.contains("[kernel") || dso.contains("vmlinux"))
    {
        return "kernel".to_owned();
    }
    if key.dso.as_deref().is_some_and(|dso| {
        Path::new(dso)
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name != target_program)
    }) {
        return "runtime-or-library".to_owned();
    }
    "symbol-only".to_owned()
}

fn create_temp_capture_directory() -> Result<TempCaptureDirectory> {
    let base = env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_nanos();
    for attempt in 0..16_u8 {
        let path = base.join(format!(
            "runtime-profiler-native-perf-{}-{nanos}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(TempCaptureDirectory { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to create native-perf temporary directory: {}",
                        path.display()
                    )
                });
            }
        }
    }
    bail!("failed to allocate a unique native-perf temporary directory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Collector, RunConfig, Scenario};
    use crate::digest::sha256_bytes;

    fn loaded_scenario() -> LoadedScenario {
        let scenario = Scenario {
            schema_version: "runtime-profiler/scenario/v1".to_owned(),
            id: "native-perf-test".to_owned(),
            target: Target::Command {
                program: "target/debug/example".to_owned(),
                args: Vec::new(),
                working_directory: Some(PathBuf::from("..")),
                inherit_env: Vec::new(),
            },
            run: RunConfig::default(),
            collectors: vec![Collector::Process, Collector::NativePerf],
        };
        let normalized = serde_json::to_vec(&scenario).expect("scenario serialization");
        LoadedScenario {
            scenario,
            source_path: PathBuf::from("profiles/runtime-profiler/state.json"),
            digest: sha256_bytes(&normalized),
        }
    }

    #[test]
    fn detection_distinguishes_platform_tool_and_permission() {
        let unsupported = detection_from_probe("macos", None, None);
        assert!(!unsupported.available);
        assert!(unsupported.reason.contains("requires Linux"));

        let missing = detection_from_probe("linux", None, None);
        assert!(!missing.available);
        assert!(missing.reason.contains("not installed"));

        let denied = detection_from_probe("linux", Some("perf version 6.8"), Some(false));
        assert!(!denied.available);
        assert_eq!(denied.tool_version.as_deref(), Some("perf version 6.8"));

        let available = detection_from_probe("linux", Some("perf version 6.8"), Some(true));
        assert!(available.available);
        assert_eq!(available.tool_version.as_deref(), Some("perf version 6.8"));
    }

    #[test]
    fn parser_aggregates_and_orders_hotspots_deterministically() {
        let report = "# header\n1\t100\t/src/main.rs:10\twork\texample\n2\t200\t/src/main.rs:10\twork\texample\n1\t300\t??:0\t[unknown]\t[unknown]\n1\t300\tsrc/lib.rs:20\talpha\texample\n";
        let hotspots = parse_perf_report(
            report,
            &loaded_scenario(),
            "perf version test".to_owned(),
            Some("toolchain-fingerprint".to_owned()),
        )
        .expect("parse report");

        assert_eq!(hotspots.total_weight, 900);
        assert_eq!(hotspots.total_samples, 5);
        assert_eq!(hotspots.hotspots.len(), 3);
        assert_eq!(hotspots.sample_period, Some(PERF_SAMPLE_PERIOD));
        assert_eq!(hotspots.symbolization_mode.as_deref(), Some(SYMBOLIZATION_MODE));
        assert_eq!(
            hotspots.target_toolchain_fingerprint.as_deref(),
            Some("toolchain-fingerprint")
        );
        assert_eq!(hotspots.hotspots[0].symbol, "[unknown]");
        assert_eq!(hotspots.hotspots[1].symbol, "alpha");
        assert_eq!(hotspots.hotspots[2].symbol, "work");
        assert_eq!(hotspots.hotspots[2].weight, 300);
        assert_eq!(hotspots.hotspots[2].samples, 3);
    }

    #[test]
    fn parser_rejects_malformed_rows() {
        let error = parse_perf_report(
            "1\t2\ttoo-few\n",
            &loaded_scenario(),
            "perf test".to_owned(),
            Some("toolchain-fingerprint".to_owned()),
        )
        .expect_err("malformed report must fail");
        assert!(error.to_string().contains("expected 5"));
    }

    #[test]
    fn target_toolchain_fingerprint_is_deterministic_and_version_sensitive() {
        let first = toolchain_fingerprint_from_version(
            "rustc 1.98.0\nbinary: rustc\ncommit-hash: abc\nhost: x86_64-unknown-linux-gnu\n",
        )
        .expect("fingerprint");
        let same = toolchain_fingerprint_from_version(
            "rustc 1.98.0\nbinary: rustc\ncommit-hash: abc\nhost: x86_64-unknown-linux-gnu\n\n",
        )
        .expect("fingerprint");
        let changed = toolchain_fingerprint_from_version(
            "rustc 1.99.0\nbinary: rustc\ncommit-hash: def\nhost: x86_64-unknown-linux-gnu\n",
        )
        .expect("fingerprint");

        assert_eq!(first, same);
        assert_ne!(first, changed);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn source_paths_do_not_preserve_external_absolute_directories() {
        let normalized = normalize_source_path(
            Path::new("/home/user/secret/file.rs"),
            Some(Path::new("/repo")),
        );
        assert_eq!(normalized.as_deref(), Some("file.rs"));
        assert_eq!(
            normalize_source_path(Path::new("/repo/src/lib.rs"), Some(Path::new("/repo")))
                .as_deref(),
            Some("src/lib.rs")
        );
        assert_eq!(normalize_source_path(Path::new("../escape.rs"), None), None);
    }
}