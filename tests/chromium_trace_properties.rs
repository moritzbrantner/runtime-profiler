use std::panic::{AssertUnwindSafe, catch_unwind};

use runtime_profiler::chromium_trace::analyze_chromium_trace_bytes;
use serde_json::{Value, json};

fn outcome(bytes: &[u8]) -> String {
    match analyze_chromium_trace_bytes(bytes) {
        Ok(summary) => format!(
            "ok:{}",
            serde_json::to_string(&summary).expect("summary serialization")
        ),
        Err(error) => format!("error:{error:#}"),
    }
}

fn assert_deterministic_without_panic(bytes: &[u8]) {
    let first = catch_unwind(AssertUnwindSafe(|| outcome(bytes)))
        .expect("Chromium trace parser must not panic");
    let second = catch_unwind(AssertUnwindSafe(|| outcome(bytes)))
        .expect("Chromium trace parser must not panic on a repeated input");
    assert_eq!(first, second, "same trace bytes must have the same outcome");
}

fn renderer_metadata(pid: i64, tid: i64, name: &str) -> Value {
    json!({
        "ph": "M",
        "name": "thread_name",
        "pid": pid,
        "tid": tid,
        "args": { "name": name }
    })
}

#[test]
fn malformed_heterogeneous_corpus_is_deterministic_and_panic_free() {
    let mut corpus = vec![
        b"null".to_vec(),
        br#"{"traceEvents":"not-an-array"}"#.to_vec(),
        br#"{"traceEvents":[]}"#.to_vec(),
        br#"{"traceEvents":[null,true,42,"text",[],{}]}"#.to_vec(),
    ];

    for seed in 0_u64..64 {
        let mut events = vec![renderer_metadata(1, 2, "CrRendererMain")];
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1);
        for index in 0_u64..32 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let event = match state % 9 {
                0 => Value::Null,
                1 => json!("event"),
                2 => json!([index, state]),
                3 => json!({"ph":"I","name":"ignored","pid":1,"tid":2}),
                4 => json!({
                    "ph":"X",
                    "name":format!("Task-{index}"),
                    "cat":"toplevel",
                    "pid":1,
                    "tid":2,
                    "ts":index * 100,
                    "dur":state % 100_000
                }),
                5 => json!({
                    "ph":"X",
                    "name":"negative-duration",
                    "cat":"toplevel",
                    "pid":1,
                    "tid":2,
                    "ts":index,
                    "dur":-1
                }),
                6 => json!({
                    "ph":"X",
                    "name":"fractional",
                    "cat":"v8",
                    "pid":1,
                    "tid":2,
                    "ts":index as f64 + 0.4,
                    "dur":10.6
                }),
                7 => json!({
                    "ph":"X",
                    "name":"out-of-range-pid",
                    "cat":"toplevel",
                    "pid":u64::MAX,
                    "tid":2,
                    "ts":0,
                    "dur":1
                }),
                _ => renderer_metadata(
                    10 + i64::try_from(index).expect("small index"),
                    20 + i64::try_from(index).expect("small index"),
                    "RendererMain",
                ),
            };
            events.push(event);
        }
        corpus.push(
            serde_json::to_vec(&json!({"traceEvents": events})).expect("trace serialization"),
        );
    }

    for bytes in corpus {
        assert_deterministic_without_panic(&bytes);
    }
}

#[test]
fn event_and_duplicate_metadata_order_do_not_change_normalized_evidence() {
    let events = vec![
        renderer_metadata(2, 9, "CrRendererMain"),
        renderer_metadata(1, 3, "CrRendererMain"),
        renderer_metadata(0, 1, "RendererMain"),
        json!({"ph":"X","name":"Outer","cat":"toplevel","pid":1,"tid":3,"ts":0,"dur":100_000}),
        json!({"ph":"X","name":"FunctionCall","cat":"v8","pid":1,"tid":3,"ts":1_000,"dur":30_000}),
        json!({"ph":"X","name":"WebAssembly.execute","cat":"v8.wasm","pid":1,"tid":3,"ts":2_000,"dur":10_000}),
        json!({"ph":"X","name":"Short","cat":"toplevel","pid":1,"tid":3,"ts":200_000,"dur":1_000}),
    ];
    let mut reversed = events.clone();
    reversed.reverse();

    let forward = serde_json::to_vec(&json!({"traceEvents": events})).expect("forward trace");
    let reversed =
        serde_json::to_vec(&json!({"traceEvents": reversed})).expect("reversed trace");

    let forward = analyze_chromium_trace_bytes(&forward).expect("forward summary");
    let reversed = analyze_chromium_trace_bytes(&reversed).expect("reversed summary");

    assert_eq!(forward, reversed);
    assert_eq!(forward.main_thread.process_id, 1);
    assert_eq!(forward.main_thread.thread_id, 3);
}

#[test]
fn nested_and_high_cardinality_traces_stay_within_output_bounds() {
    let mut events = vec![renderer_metadata(1, 2, "CrRendererMain")];

    for depth in 0_u64..40 {
        events.push(json!({
            "ph":"X",
            "name":format!("Nested-{depth}"),
            "cat":"v8",
            "pid":1,
            "tid":2,
            "ts":depth * 10,
            "dur":1_000_000 - depth * 20
        }));
    }

    for index in 0_u64..200 {
        events.push(json!({
            "ph":"X",
            "name":format!("Long-{index}"),
            "cat":"toplevel",
            "pid":1,
            "tid":2,
            "ts":2_000_000 + index * 100_000,
            "dur":60_000
        }));
    }

    for index in 0_u64..100 {
        events.push(json!({
            "ph":"X",
            "name":format!("runtime-profiler:js-to-wasm:boundary-{index}"),
            "cat":"blink.user_timing",
            "pid":1,
            "tid":2,
            "ts":30_000_000 + index * 10,
            "dur":1
        }));
    }

    let bytes = serde_json::to_vec(&json!({"traceEvents": events})).expect("trace serialization");
    let summary = analyze_chromium_trace_bytes(&bytes).expect("bounded summary");

    assert!(summary.hot_path_depth_truncated);
    assert!(summary.hot_paths.iter().all(|path| path.frames.len() <= 32));

    assert!(summary.long_tasks_truncated);
    assert!(summary.long_tasks.len() <= 128);

    assert!(summary.hot_paths_truncated);
    assert!(summary.hot_paths.len() <= 64);

    assert_eq!(summary.boundary_marker_count, 100);
    assert!(summary.boundary_markers_truncated);
    assert!(summary.boundary_markers.len() <= 64);
}

#[test]
fn extreme_numeric_values_are_bounded_and_deterministic() {
    let valid_extreme = json!({
        "traceEvents": [
            renderer_metadata(1, 2, "CrRendererMain"),
            {"ph":"X","name":"Extreme","cat":"toplevel","pid":1,"tid":2,"ts":u64::MAX,"dur":u64::MAX}
        ]
    });
    let valid_bytes = serde_json::to_vec(&valid_extreme).expect("extreme trace serialization");
    assert_deterministic_without_panic(&valid_bytes);
    let summary = analyze_chromium_trace_bytes(&valid_bytes).expect("u64 values are supported");
    assert_eq!(summary.top_level_duration_us, u64::MAX);
    assert_eq!(summary.longest_task_us, Some(u64::MAX));

    let outside_supported_range = json!({
        "traceEvents": [
            renderer_metadata(1, 2, "CrRendererMain"),
            {"ph":"X","name":"TooLarge","cat":"toplevel","pid":1,"tid":2,"ts":1e300,"dur":1}
        ]
    });
    let outside_bytes =
        serde_json::to_vec(&outside_supported_range).expect("out-of-range trace serialization");
    assert_deterministic_without_panic(&outside_bytes);
    let error = analyze_chromium_trace_bytes(&outside_bytes).expect_err("range must fail closed");
    assert!(error.to_string().contains("outside the supported range"));
}
