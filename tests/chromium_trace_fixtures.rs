use runtime_profiler::chromium_trace::analyze_chromium_trace_bytes;
use serde_json::{Value, json};

const REPRESENTATIVE: &[u8] = include_bytes!("fixtures/chromium/renderer-nested.json");
const RAW_ARRAY: &[u8] = include_bytes!("fixtures/chromium/raw-array.json");
const MALFORMED: &[u8] = include_bytes!("fixtures/chromium/malformed-events.json");
const OUTPUT_BOUNDS_TEMPLATE: &[u8] =
    include_bytes!("fixtures/chromium/output-bounds-template.json");

#[test]
fn representative_fixture_covers_renderer_nested_boundary_and_worker_semantics() {
    let summary = analyze_chromium_trace_bytes(REPRESENTATIVE).expect("representative fixture");

    assert_eq!(summary.trace_event_count, 8);
    assert_eq!(summary.main_thread.process_id, 1);
    assert_eq!(summary.main_thread.thread_id, 10);
    assert_eq!(summary.main_thread.name, "CrRendererMain");
    assert_eq!(summary.long_task_count, 1);
    assert_eq!(summary.long_tasks[0].name, "RunTask");
    assert!(
        summary.hot_paths.iter().any(|path| {
            path.frames.iter().map(|frame| frame.name.as_str()).eq([
                "RunTask",
                "FunctionCall",
                "WebAssembly.execute",
            ])
        })
    );
    assert!(
        !summary
            .long_tasks
            .iter()
            .any(|task| task.name == "WorkerTaskMustBeExcluded")
    );
    assert_eq!(summary.boundary_marker_count, 2);
    assert!(summary.boundary_markers.iter().any(|marker| {
        marker.direction == "js-to-wasm"
            && marker.label == "update-map"
            && marker.total_duration_us == 2_000
    }));
    assert!(summary.boundary_markers.iter().any(|marker| {
        marker.direction == "wasm-to-js"
            && marker.label == "result-copy"
            && marker.total_duration_us == 1_000
    }));
}

#[test]
fn raw_array_fixture_remains_supported() {
    let summary = analyze_chromium_trace_bytes(RAW_ARRAY).expect("raw-array fixture");

    assert_eq!(summary.trace_event_count, 2);
    assert_eq!(summary.main_thread.process_id, 2);
    assert_eq!(summary.main_thread.thread_id, 20);
    assert_eq!(summary.top_level_task_count, 1);
    assert_eq!(summary.long_task_count, 1);
    assert_eq!(summary.long_tasks[0].name, "RawArrayTask");
}

#[test]
fn malformed_fixture_fails_closed_deterministically() {
    let first = analyze_chromium_trace_bytes(MALFORMED)
        .expect_err("negative duration must fail closed")
        .to_string();
    let second = analyze_chromium_trace_bytes(MALFORMED)
        .expect_err("negative duration must fail closed repeatedly")
        .to_string();

    assert_eq!(first, second);
    assert!(first.contains("must be non-negative"));
}

#[test]
fn output_bounds_template_expands_to_bounded_normalized_evidence() {
    let template: Value =
        serde_json::from_slice(OUTPUT_BOUNDS_TEMPLATE).expect("output-bounds fixture template");
    let metadata = template["metadata"].clone();
    let long_task_template = template["longTask"].clone();
    let boundary_template = template["boundaryMarker"].clone();

    let mut events = vec![metadata];
    for index in 0_u64..200 {
        let mut task = long_task_template.clone();
        let task = task.as_object_mut().expect("long-task template object");
        task.insert("name".to_owned(), json!(format!("Long-{index}")));
        task.insert("ts".to_owned(), json!(index * 100_000));
        events.push(Value::Object(task.clone()));
    }
    for index in 0_u64..100 {
        let mut marker = boundary_template.clone();
        let marker = marker
            .as_object_mut()
            .expect("boundary-marker template object");
        marker.insert(
            "name".to_owned(),
            json!(format!("runtime-profiler:js-to-wasm:boundary-{index}")),
        );
        marker.insert("ts".to_owned(), json!(30_000_000 + index * 10));
        events.push(Value::Object(marker.clone()));
    }

    let bytes = serde_json::to_vec(&json!({"traceEvents": events})).expect("expanded fixture");
    let summary = analyze_chromium_trace_bytes(&bytes).expect("bounded expanded fixture");

    assert!(summary.long_tasks_truncated);
    assert!(summary.long_tasks.len() <= 128);
    assert!(summary.hot_paths_truncated);
    assert!(summary.hot_paths.len() <= 64);
    assert_eq!(summary.boundary_marker_count, 100);
    assert!(summary.boundary_markers_truncated);
    assert!(summary.boundary_markers.len() <= 64);
}
