# Chromium trace analysis

`runtime-profiler analyze-chromium-trace` normalizes an existing Chromium performance trace into bounded descriptive browser-runtime evidence. It is the parser/normalization foundation for the planned Playwright journey adapter; it does not launch a browser or create a runtime-profiler bundle yet.

```bash
cargo run -- analyze-chromium-trace --trace trace.json --json
```

The parser accepts Chromium's object form (`{"traceEvents": [...]}`) and a raw event array. It requires explicit renderer-main-thread metadata (`CrRendererMain`, `RendererMain`, or an equivalent renderer/main name) rather than guessing which thread is authoritative.

## Evidence

The v1 summary reports:

- top-level renderer-main-thread tasks and observed duration;
- tasks at or above the 50 ms long-task threshold;
- bounded nested event-name paths, ordered by descriptive inclusive event duration;
- descriptive runtime attribution for events explicitly identifiable as JavaScript/V8, WASM/WebAssembly, or other;
- optional explicit JS-to-WASM and WASM-to-JS boundary measures.

Nested Chromium events overlap by design, so hot-path and runtime-attribution duration is inclusive descriptive evidence, not exclusive CPU time. Worker, compositor, GPU, network, and other threads are not summarized in this slice.

## JS/WASM boundary measures

The parser does not infer boundary cost from adjacency or runtime names. Applications may expose an exact boundary section through browser User Timing measures named:

```text
runtime-profiler:js-to-wasm:<label>
runtime-profiler:wasm-to-js:<label>
```

For example, a caller can place ordinary `performance.mark`/`performance.measure` instrumentation around marshaling plus a WASM call. Chromium trace capture can then preserve that measure as a complete event, and runtime-profiler aggregates its count, total duration, and maximum duration by direction and label.

The measure describes the instrumented section only. It does not prove that all of the observed duration is ABI or copying overhead, and runtime-profiler does not apply a pass/fail threshold.

## Bounds and privacy

Input is bounded to 64 MiB and 500,000 events. Normalized output keeps at most 128 long tasks, 64 hot paths, 32 frames per path, and 64 boundary markers. Event text is bounded to 1,024 bytes.

Trace event argument objects are ignored except for the thread name used to identify the renderer main thread. URLs, request payloads, DOM data, and other trace args are therefore not copied into normalized evidence by this parser.

## Next integration step

The Playwright adapter will own browser execution and raw-trace capture, then invoke this parser and store both the raw trace and bounded normalized result in the immutable evidence bundle. Browser/runtime version and trace-configuration identity must be added before baseline/candidate browser evidence is treated as strictly comparable.
