# Browser agent guidance

Browser journey bundles keep raw Chromium traces as immutable artifacts, but agent-facing guidance is intentionally smaller.

For each normalized Chromium trace summary, runtime-profiler may surface at most three observations from each of these categories:

- renderer-main-thread long tasks;
- normalized hot paths;
- explicit JS/WASM boundary markers.

The summary text is derived only from normalized trace evidence. Event-derived labels are bounded and sanitized before they enter agent guidance. Raw trace events and raw trace arguments are not copied into prompts.

The guidance remains descriptive. Long-task and hot-path durations are Chromium trace durations, not exclusive CPU time. Explicit JS/WASM marker durations describe instrumented sections rather than inferred marshaling cost. Baseline/candidate comparability and repository policy remain separate concerns.
