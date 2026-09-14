use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::digest::sha256_bytes;

pub const CHROMIUM_TRACE_SUMMARY_SCHEMA_V1: &str = "runtime-profiler/chromium-trace-summary/v1";
pub const LONG_TASK_THRESHOLD_US: u64 = 50_000;

const MAX_TRACE_BYTES: usize = 64 * 1024 * 1024;
const MAX_TRACE_EVENTS: usize = 500_000;
const MAX_TEXT_BYTES: usize = 1_024;
const MAX_HOT_PATH_DEPTH: usize = 32;
const MAX_UNIQUE_HOT_PATHS: usize = 16_384;
const MAX_HOT_PATHS: usize = 64;
const MAX_LONG_TASKS: usize = 128;
const MAX_BOUNDARY_MARKERS: usize = 64;
const MAX_UNIQUE_BOUNDARY_MARKERS: usize = 1_024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChromiumTraceSummary {
    pub schema_version: String,
    pub trace_event_count: usize,
    pub main_thread: ChromiumMainThread,
    pub top_level_task_count: usize,
    pub top_level_duration_us: u64,
    pub long_task_count: usize,
    pub long_task_total_duration_us: u64,
    pub longest_task_us: Option<u64>,
    pub long_tasks_truncated: bool,
    pub long_tasks: Vec<ChromiumLongTask>,
    pub hot_path_count: usize,
    pub hot_paths_truncated: bool,
    pub hot_path_depth_truncated: bool,
    pub hot_paths: Vec<ChromiumHotPath>,
    pub runtime_attribution: Vec<ChromiumRuntimeAttribution>,
    pub boundary_marker_count: usize,
    pub boundary_markers_truncated: bool,
    pub boundary_markers: Vec<ChromiumBoundaryMarker>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChromiumMainThread {
    pub process_id: i64,
    pub thread_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChromiumLongTask {
    pub id: String,
    pub name: String,
    pub category: String,
    pub start_us: u64,
    pub duration_us: u64,
    pub runtime_kind: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChromiumHotPath {
    pub id: String,
    pub frames: Vec<ChromiumHotPathFrame>,
    pub leaf_runtime_kind: String,
    pub total_duration_us: u64,
    pub max_duration_us: u64,
    pub occurrences: u64,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChromiumHotPathFrame {
    pub name: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChromiumRuntimeAttribution {
    pub runtime_kind: String,
    pub inclusive_duration_us: u64,
    pub event_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChromiumBoundaryMarker {
    pub id: String,
    pub direction: String,
    pub label: String,
    pub total_duration_us: u64,
    pub max_duration_us: u64,
    pub occurrences: u64,
    pub evidence_ref: String,
}

#[derive(Debug, Clone)]
struct TraceEvent {
    name: String,
    category: String,
    process_id: i64,
    thread_id: i64,
    start_us: u64,
    duration_us: u64,
}

impl TraceEvent {
    fn end_us(&self) -> u64 {
        self.start_us.saturating_add(self.duration_us)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Aggregate {
    total_duration_us: u64,
    max_duration_us: u64,
    occurrences: u64,
}

#[derive(Debug, Clone)]
struct BoundaryAggregate {
    direction: String,
    label: String,
    aggregate: Aggregate,
}

pub fn analyze_chromium_trace(path: &Path) -> Result<ChromiumTraceSummary> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect Chromium trace: {}", path.display()))?;
    ensure!(
        metadata.len() <= MAX_TRACE_BYTES as u64,
        "Chromium trace exceeds the {} byte safety limit",
        MAX_TRACE_BYTES
    );
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read Chromium trace: {}", path.display()))?;
    analyze_chromium_trace_bytes(&bytes)
}

pub fn analyze_chromium_trace_bytes(bytes: &[u8]) -> Result<ChromiumTraceSummary> {
    ensure!(
        bytes.len() <= MAX_TRACE_BYTES,
        "Chromium trace exceeds the {} byte safety limit",
        MAX_TRACE_BYTES
    );
    let document: Value = serde_json::from_slice(bytes).context("Chromium trace is not valid JSON")?;
    let events = extract_trace_events(document)?;
    ensure!(
        events.len() <= MAX_TRACE_EVENTS,
        "Chromium trace exceeds the {} event safety limit",
        MAX_TRACE_EVENTS
    );

    let main_thread = find_renderer_main_thread(&events)?;
    let mut complete_events = parse_main_thread_complete_events(&events, &main_thread)?;
    complete_events.sort_by(|left, right| {
        left.start_us
            .cmp(&right.start_us)
            .then_with(|| right.duration_us.cmp(&left.duration_us))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.category.cmp(&right.category))
    });

    Ok(summarize_complete_events(
        events.len(),
        main_thread,
        &complete_events,
    ))
}

fn extract_trace_events(document: Value) -> Result<Vec<Value>> {
    match document {
        Value::Array(events) => Ok(events),
        Value::Object(mut object) => match object.remove("traceEvents") {
            Some(Value::Array(events)) => Ok(events),
            Some(_) => bail!("Chromium trace `traceEvents` must be an array"),
            None => bail!("Chromium trace object is missing `traceEvents`"),
        },
        _ => bail!("Chromium trace must be an object or event array"),
    }
}

fn find_renderer_main_thread(events: &[Value]) -> Result<ChromiumMainThread> {
    let mut candidates = Vec::new();
    for event in events {
        if event.get("ph").and_then(Value::as_str) != Some("M")
            || event.get("name").and_then(Value::as_str) != Some("thread_name")
        {
            continue;
        }
        let Some(name) = event
            .get("args")
            .and_then(|args| args.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(rank) = renderer_main_thread_rank(name) else {
            continue;
        };
        candidates.push((
            rank,
            integer_field(event, "pid")?,
            integer_field(event, "tid")?,
            bounded_nonempty_text(name, "renderer main thread name")?,
        ));
    }

    candidates.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    let Some((_, process_id, thread_id, name)) = candidates.into_iter().next() else {
        bail!(
            "Chromium trace does not contain explicit renderer-main-thread metadata (`CrRendererMain`/`RendererMain`)"
        );
    };
    Ok(ChromiumMainThread {
        process_id,
        thread_id,
        name,
    })
}

fn renderer_main_thread_rank(name: &str) -> Option<u8> {
    match name {
        "CrRendererMain" => Some(0),
        "RendererMain" => Some(1),
        _ => {
            let normalized = name.to_ascii_lowercase();
            (normalized.contains("renderer") && normalized.contains("main")).then_some(2)
        }
    }
}

fn parse_main_thread_complete_events(
    events: &[Value],
    main_thread: &ChromiumMainThread,
) -> Result<Vec<TraceEvent>> {
    let mut result = Vec::new();
    for event in events {
        if event.get("ph").and_then(Value::as_str) != Some("X") {
            continue;
        }
        if integer_field_optional(event, "pid")? != Some(main_thread.process_id)
            || integer_field_optional(event, "tid")? != Some(main_thread.thread_id)
        {
            continue;
        }
        let Some(name) = event.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(start_us) = nonnegative_us_field_optional(event, "ts")? else {
            continue;
        };
        let Some(duration_us) = nonnegative_us_field_optional(event, "dur")? else {
            continue;
        };
        result.push(TraceEvent {
            name: bounded_nonempty_text(name, "trace event name")?,
            category: bounded_text(
                event.get("cat").and_then(Value::as_str).unwrap_or(""),
                "trace event category",
            )?,
            process_id: main_thread.process_id,
            thread_id: main_thread.thread_id,
            start_us,
            duration_us,
        });
    }
    Ok(result)
}

fn summarize_complete_events(
    trace_event_count: usize,
    main_thread: ChromiumMainThread,
    events: &[TraceEvent],
) -> ChromiumTraceSummary {
    let mut stack: Vec<usize> = Vec::new();
    let mut top_level_task_count = 0_usize;
    let mut top_level_duration_us = 0_u64;
    let mut long_task_count = 0_usize;
    let mut long_task_total_duration_us = 0_u64;
    let mut longest_task_us: Option<u64> = None;
    let mut long_tasks = Vec::new();
    let mut hot_path_aggregates: BTreeMap<Vec<ChromiumHotPathFrame>, Aggregate> = BTreeMap::new();
    let mut hot_paths_truncated = false;
    let mut hot_path_depth_truncated = false;
    let mut runtime_aggregates: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut boundary_aggregates: BTreeMap<(String, String), BoundaryAggregate> = BTreeMap::new();
    let mut boundary_markers_truncated = false;

    for (index, event) in events.iter().enumerate() {
        while let Some(parent_index) = stack.last().copied() {
            let parent = &events[parent_index];
            if event.start_us >= parent.end_us() {
                stack.pop();
                continue;
            }
            if event.end_us() <= parent.end_us() {
                break;
            }
            stack.pop();
        }

        if stack.is_empty() {
            top_level_task_count += 1;
            top_level_duration_us = top_level_duration_us.saturating_add(event.duration_us);
            if event.duration_us >= LONG_TASK_THRESHOLD_US {
                long_task_count += 1;
                long_task_total_duration_us =
                    long_task_total_duration_us.saturating_add(event.duration_us);
                longest_task_us = Some(longest_task_us.unwrap_or_default().max(event.duration_us));
                retain_long_task(&mut long_tasks, event);
            }
        }

        let runtime = runtime_kind(&event.name, &event.category);
        let entry = runtime_aggregates.entry(runtime.to_owned()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(event.duration_us);
        entry.1 = entry.1.saturating_add(1);

        if let Some((direction, label)) = parse_boundary_marker(&event.name) {
            let key = (direction.to_owned(), label.to_owned());
            if boundary_aggregates.len() < MAX_UNIQUE_BOUNDARY_MARKERS
                || boundary_aggregates.contains_key(&key)
            {
                let entry = boundary_aggregates.entry(key).or_insert_with(|| BoundaryAggregate {
                    direction: direction.to_owned(),
                    label: label.to_owned(),
                    aggregate: Aggregate::default(),
                });
                update_aggregate(&mut entry.aggregate, event.duration_us);
            } else {
                boundary_markers_truncated = true;
            }
        }

        let mut path = stack
            .iter()
            .map(|parent_index| frame(&events[*parent_index]))
            .chain(std::iter::once(frame(event)))
            .collect::<Vec<_>>();
        if path.len() > MAX_HOT_PATH_DEPTH {
            path = path.split_off(path.len() - MAX_HOT_PATH_DEPTH);
            hot_path_depth_truncated = true;
        }
        if hot_path_aggregates.len() < MAX_UNIQUE_HOT_PATHS
            || hot_path_aggregates.contains_key(&path)
        {
            update_aggregate(
                hot_path_aggregates.entry(path).or_default(),
                event.duration_us,
            );
        } else {
            hot_paths_truncated = true;
        }
        stack.push(index);
    }

    let hot_path_count = hot_path_aggregates.len();
    let mut hot_paths = hot_path_aggregates
        .into_iter()
        .map(|(frames, aggregate)| hot_path(frames, aggregate))
        .collect::<Vec<_>>();
    hot_paths.sort_by(|left, right| {
        right
            .total_duration_us
            .cmp(&left.total_duration_us)
            .then_with(|| right.max_duration_us.cmp(&left.max_duration_us))
            .then_with(|| left.frames.cmp(&right.frames))
    });
    if hot_paths.len() > MAX_HOT_PATHS {
        hot_paths_truncated = true;
        hot_paths.truncate(MAX_HOT_PATHS);
    }

    long_tasks.sort_by(|left, right| {
        right
            .duration_us
            .cmp(&left.duration_us)
            .then_with(|| left.start_us.cmp(&right.start_us))
            .then_with(|| left.name.cmp(&right.name))
    });
    let long_tasks_truncated = long_task_count > long_tasks.len();

    let runtime_attribution = runtime_aggregates
        .into_iter()
        .map(
            |(runtime_kind, (inclusive_duration_us, event_count))| ChromiumRuntimeAttribution {
                runtime_kind,
                inclusive_duration_us,
                event_count,
            },
        )
        .collect();

    let boundary_marker_count = boundary_aggregates.len();
    let mut boundary_markers = boundary_aggregates
        .into_values()
        .map(boundary_marker)
        .collect::<Vec<_>>();
    boundary_markers.sort_by(|left, right| {
        right
            .total_duration_us
            .cmp(&left.total_duration_us)
            .then_with(|| left.direction.cmp(&right.direction))
            .then_with(|| left.label.cmp(&right.label))
    });
    if boundary_markers.len() > MAX_BOUNDARY_MARKERS {
        boundary_markers_truncated = true;
        boundary_markers.truncate(MAX_BOUNDARY_MARKERS);
    }

    ChromiumTraceSummary {
        schema_version: CHROMIUM_TRACE_SUMMARY_SCHEMA_V1.to_owned(),
        trace_event_count,
        main_thread,
        top_level_task_count,
        top_level_duration_us,
        long_task_count,
        long_task_total_duration_us,
        longest_task_us,
        long_tasks_truncated,
        long_tasks,
        hot_path_count,
        hot_paths_truncated,
        hot_path_depth_truncated,
        hot_paths,
        runtime_attribution,
        boundary_marker_count,
        boundary_markers_truncated,
        boundary_markers,
        limitations: vec![
            "Trace durations are descriptive Chromium event durations, not exclusive CPU time; nested event durations can overlap in aggregate summaries.".to_owned(),
            "JS/WASM boundary evidence is reported only for explicit runtime-profiler User Timing measure names and is not inferred from adjacent runtime events.".to_owned(),
            "Only the explicitly identified renderer main thread is summarized; worker, compositor, GPU and other threads are outside this slice.".to_owned(),
            "Trace event args are not copied into normalized evidence except the renderer thread name used for main-thread identity.".to_owned(),
        ],
    }
}

fn retain_long_task(long_tasks: &mut Vec<ChromiumLongTask>, event: &TraceEvent) {
    long_tasks.push(long_task(event));
    if long_tasks.len() > MAX_LONG_TASKS {
        long_tasks.sort_by(|left, right| {
            right
                .duration_us
                .cmp(&left.duration_us)
                .then_with(|| left.start_us.cmp(&right.start_us))
                .then_with(|| left.name.cmp(&right.name))
        });
        long_tasks.truncate(MAX_LONG_TASKS);
    }
}

fn long_task(event: &TraceEvent) -> ChromiumLongTask {
    let identity = format!(
        "{}\0{}\0{}\0{}\0{}\0{}",
        event.process_id,
        event.thread_id,
        event.start_us,
        event.duration_us,
        event.category,
        event.name
    );
    let id = format!("chromium-long-task-{}", sha256_bytes(identity.as_bytes()));
    ChromiumLongTask {
        id: id.clone(),
        name: event.name.clone(),
        category: event.category.clone(),
        start_us: event.start_us,
        duration_us: event.duration_us,
        runtime_kind: runtime_kind(&event.name, &event.category).to_owned(),
        evidence_ref: format!("chromium-trace-summary.json#{id}"),
    }
}

fn hot_path(frames: Vec<ChromiumHotPathFrame>, aggregate: Aggregate) -> ChromiumHotPath {
    let identity = frames
        .iter()
        .map(|frame| format!("{}\0{}", frame.category, frame.name))
        .collect::<Vec<_>>()
        .join("\u{1f}");
    let id = format!("chromium-hot-path-{}", sha256_bytes(identity.as_bytes()));
    let leaf_runtime_kind = frames
        .last()
        .map(|frame| runtime_kind(&frame.name, &frame.category))
        .unwrap_or("other")
        .to_owned();
    ChromiumHotPath {
        id: id.clone(),
        frames,
        leaf_runtime_kind,
        total_duration_us: aggregate.total_duration_us,
        max_duration_us: aggregate.max_duration_us,
        occurrences: aggregate.occurrences,
        evidence_ref: format!("chromium-trace-summary.json#{id}"),
    }
}

fn boundary_marker(value: BoundaryAggregate) -> ChromiumBoundaryMarker {
    let identity = format!("{}\0{}", value.direction, value.label);
    let id = format!("chromium-boundary-{}", sha256_bytes(identity.as_bytes()));
    ChromiumBoundaryMarker {
        id: id.clone(),
        direction: value.direction,
        label: value.label,
        total_duration_us: value.aggregate.total_duration_us,
        max_duration_us: value.aggregate.max_duration_us,
        occurrences: value.aggregate.occurrences,
        evidence_ref: format!("chromium-trace-summary.json#{id}"),
    }
}

fn update_aggregate(aggregate: &mut Aggregate, duration_us: u64) {
    aggregate.total_duration_us = aggregate.total_duration_us.saturating_add(duration_us);
    aggregate.max_duration_us = aggregate.max_duration_us.max(duration_us);
    aggregate.occurrences = aggregate.occurrences.saturating_add(1);
}

fn frame(event: &TraceEvent) -> ChromiumHotPathFrame {
    ChromiumHotPathFrame {
        name: event.name.clone(),
        category: event.category.clone(),
    }
}

fn runtime_kind(name: &str, category: &str) -> &'static str {
    if parse_boundary_marker(name).is_some() {
        return "other";
    }
    let name = name.to_ascii_lowercase();
    let category = category.to_ascii_lowercase();
    if name.contains("webassembly")
        || name.contains("wasm")
        || category.contains("webassembly")
        || category.contains("wasm")
    {
        "wasm"
    } else if category.contains("v8")
        || name.contains("functioncall")
        || name.contains("evaluatescript")
        || name.contains("runmicrotasks")
        || name.contains("javascript")
        || name.starts_with("v8")
    {
        "javascript"
    } else {
        "other"
    }
}

fn parse_boundary_marker(name: &str) -> Option<(&'static str, &str)> {
    const JS_TO_WASM: &str = "runtime-profiler:js-to-wasm:";
    const WASM_TO_JS: &str = "runtime-profiler:wasm-to-js:";
    if let Some(label) = name.strip_prefix(JS_TO_WASM).filter(|value| !value.is_empty()) {
        return Some(("js-to-wasm", label));
    }
    name.strip_prefix(WASM_TO_JS)
        .filter(|value| !value.is_empty())
        .map(|label| ("wasm-to-js", label))
}

fn integer_field(value: &Value, field: &str) -> Result<i64> {
    integer_field_optional(value, field)?
        .with_context(|| format!("Chromium trace field `{field}` is missing or not an integer"))
}

fn integer_field_optional(value: &Value, field: &str) -> Result<Option<i64>> {
    let Some(number) = value.get(field) else {
        return Ok(None);
    };
    let Some(number) = number.as_number() else {
        return Ok(None);
    };
    if let Some(value) = number.as_i64() {
        return Ok(Some(value));
    }
    if let Some(value) = number.as_u64() {
        return i64::try_from(value)
            .map(Some)
            .with_context(|| format!("Chromium trace `{field}` exceeds i64 range"));
    }
    Ok(None)
}

fn nonnegative_us_field_optional(value: &Value, field: &str) -> Result<Option<u64>> {
    let Some(number) = value.get(field) else {
        return Ok(None);
    };
    let Some(number) = number.as_number() else {
        return Ok(None);
    };
    if let Some(value) = number.as_u64() {
        return Ok(Some(value));
    }
    if let Some(value) = number.as_i64() {
        ensure!(value >= 0, "Chromium trace `{field}` must be non-negative");
        return Ok(Some(value as u64));
    }
    if let Some(value) = number.as_f64() {
        ensure!(
            value.is_finite() && value >= 0.0 && value <= u64::MAX as f64,
            "Chromium trace `{field}` is outside the supported range"
        );
        return Ok(Some(value.round() as u64));
    }
    Ok(None)
}

fn bounded_nonempty_text(value: &str, field: &str) -> Result<String> {
    ensure!(!value.is_empty(), "Chromium trace {field} is empty");
    bounded_text(value, field)
}

fn bounded_text(value: &str, field: &str) -> Result<String> {
    ensure!(
        value.len() <= MAX_TEXT_BYTES,
        "Chromium trace {field} exceeds the {} byte safety limit",
        MAX_TEXT_BYTES
    );
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn representative_trace() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "traceEvents": [
                {"ph":"M","name":"thread_name","pid":100,"tid":7,"args":{"name":"CrRendererMain"}},
                {"ph":"M","name":"thread_name","pid":100,"tid":8,"args":{"name":"DedicatedWorker thread"}},
                {"ph":"X","name":"RunTask","cat":"toplevel","pid":100,"tid":7,"ts":0,"dur":100000},
                {"ph":"X","name":"FunctionCall","cat":"v8,devtools.timeline","pid":100,"tid":7,"ts":1000,"dur":70000},
                {"ph":"X","name":"WebAssembly.execute","cat":"v8.wasm","pid":100,"tid":7,"ts":10000,"dur":40000},
                {"ph":"X","name":"runtime-profiler:js-to-wasm:update-map","cat":"blink.user_timing","pid":100,"tid":7,"ts":15000,"dur":600},
                {"ph":"X","name":"WorkerTask","cat":"toplevel","pid":100,"tid":8,"ts":0,"dur":200000},
                {"ph":"X","name":"ShortTask","cat":"toplevel","pid":100,"tid":7,"ts":200000,"dur":10000}
            ]
        }))
        .expect("trace serialization")
    }

    #[test]
    fn summarizes_renderer_main_thread_long_tasks_and_hot_paths() {
        let summary = analyze_chromium_trace_bytes(&representative_trace()).expect("trace summary");
        assert_eq!(summary.trace_event_count, 8);
        assert_eq!(summary.main_thread.name, "CrRendererMain");
        assert_eq!(summary.main_thread.thread_id, 7);
        assert_eq!(summary.top_level_task_count, 2);
        assert_eq!(summary.top_level_duration_us, 110_000);
        assert_eq!(summary.long_task_count, 1);
        assert_eq!(summary.long_task_total_duration_us, 100_000);
        assert_eq!(summary.longest_task_us, Some(100_000));
        assert_eq!(summary.long_tasks[0].name, "RunTask");
        assert!(summary.hot_paths.iter().any(|path| {
            path.frames
                .iter()
                .map(|frame| frame.name.as_str())
                .eq(["RunTask", "FunctionCall", "WebAssembly.execute"])
        }));
        assert!(summary.runtime_attribution.iter().any(|entry| {
            entry.runtime_kind == "wasm" && entry.inclusive_duration_us == 40_000
        }));
        assert_eq!(summary.boundary_marker_count, 1);
        assert_eq!(summary.boundary_markers[0].direction, "js-to-wasm");
        assert_eq!(summary.boundary_markers[0].label, "update-map");
        assert_eq!(summary.boundary_markers[0].total_duration_us, 600);
    }

    #[test]
    fn ignores_non_renderer_threads() {
        let summary = analyze_chromium_trace_bytes(&representative_trace()).expect("trace summary");
        assert!(!summary.long_tasks.iter().any(|task| task.name == "WorkerTask"));
    }

    #[test]
    fn accepts_raw_event_array_form() {
        let trace = json!([
            {"ph":"M","name":"thread_name","pid":1,"tid":2,"args":{"name":"RendererMain"}},
            {"ph":"X","name":"Task","cat":"toplevel","pid":1,"tid":2,"ts":1,"dur":5}
        ]);
        let bytes = serde_json::to_vec(&trace).expect("trace serialization");
        let summary = analyze_chromium_trace_bytes(&bytes).expect("trace summary");
        assert_eq!(summary.trace_event_count, 2);
        assert_eq!(summary.top_level_task_count, 1);
    }

    #[test]
    fn fails_closed_without_renderer_main_metadata() {
        let trace = json!({
            "traceEvents": [
                {"ph":"X","name":"Task","cat":"toplevel","pid":1,"tid":2,"ts":1,"dur":5}
            ]
        });
        let bytes = serde_json::to_vec(&trace).expect("trace serialization");
        let error = analyze_chromium_trace_bytes(&bytes).expect_err("metadata must be required");
        assert!(error.to_string().contains("renderer-main-thread metadata"));
    }

    #[test]
    fn explicit_boundary_markers_are_directional_and_aggregated() {
        let trace = json!({
            "traceEvents": [
                {"ph":"M","name":"thread_name","pid":1,"tid":2,"args":{"name":"CrRendererMain"}},
                {"ph":"X","name":"Task","cat":"toplevel","pid":1,"tid":2,"ts":0,"dur":1000},
                {"ph":"X","name":"runtime-profiler:wasm-to-js:result-copy","cat":"blink.user_timing","pid":1,"tid":2,"ts":10,"dur":20},
                {"ph":"X","name":"runtime-profiler:wasm-to-js:result-copy","cat":"blink.user_timing","pid":1,"tid":2,"ts":40,"dur":30}
            ]
        });
        let bytes = serde_json::to_vec(&trace).expect("trace serialization");
        let summary = analyze_chromium_trace_bytes(&bytes).expect("trace summary");
        assert_eq!(summary.boundary_markers.len(), 1);
        assert_eq!(summary.boundary_markers[0].direction, "wasm-to-js");
        assert_eq!(summary.boundary_markers[0].occurrences, 2);
        assert_eq!(summary.boundary_markers[0].total_duration_us, 50);
        assert_eq!(summary.boundary_markers[0].max_duration_us, 30);
        assert!(summary.runtime_attribution.iter().any(|entry| {
            entry.runtime_kind == "other" && entry.inclusive_duration_us >= 1_050
        }));
    }
}
