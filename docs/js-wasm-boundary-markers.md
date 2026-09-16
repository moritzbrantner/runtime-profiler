# JS/WASM boundary markers

`runtime-profiler` reports JavaScript/WebAssembly boundary evidence only when the application emits explicit Chromium User Timing measures using one of these names:

- `runtime-profiler:js-to-wasm:<label>`
- `runtime-profiler:wasm-to-js:<label>`

`<label>` should identify the stable operation being instrumented, such as `update-map` or `result-copy`. Keep labels deterministic and independent of request IDs, user data, URLs, timestamps, or other per-run values.

## What the duration means

A boundary marker describes the duration of the instrumented section represented by that User Timing measure. It does not prove or isolate marshaling, copying, serialization, allocation, browser scheduling, or WebAssembly execution cost. Code inside the measured section can contribute to the duration, and nested Chromium event durations can overlap.

Use a reference and candidate with the same scenario, journey, runtime identity, trace configuration, and marker convention before interpreting a change. `runtime-profiler` records and normalizes the evidence; repository policy or Moonlight decides whether a measured change matters.

## Framework-neutral instrumentation

Applications do not need a runtime-profiler SDK or runtime dependency. Emit ordinary User Timing measures around an architecturally meaningful boundary section and use the naming convention above. The profiler recognizes those names in the captured Chromium trace and aggregates occurrences plus total and maximum instrumented-section duration.

Keep instrumentation at stable seams rather than every call. A small number of representative markers produces evidence that is easier to compare and does not turn profiling concerns into application architecture.

## Safety and ownership

Marker labels are evidence identifiers, not a place for payload data. Do not include secrets, personal data, full URLs, arbitrary input, or dynamic identifiers. Raw Chromium traces remain artifacts; agent-facing guidance uses bounded normalized summaries only.

The application owns where the boundary is instrumented. `runtime-profiler` owns capture, normalization, identity, and descriptive evidence. It does not infer semantic boundary cost when an explicit marker is absent and does not define a performance threshold or release verdict.
