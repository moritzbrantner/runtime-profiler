# Playwright Chromium journeys

`browser-chromium` captures one deterministic browser journey as Chromium runtime evidence. The consuming repository owns the journey actions; `runtime-profiler` owns browser launch, CDP tracing, normalization, bundle integrity, and runtime identity.

## Scenario

```yaml
schema_version: runtime-profiler/scenario/v1
id: map-camera-pan-v1
target:
  type: browser-journey
  module: profiles/camera-pan.mjs
  working_directory: ..
  inherit_env: []
run:
  warmup_iterations: 0
  measurement_iterations: 1
  timeout_seconds: 60
collectors:
  - browser-chromium
```

The module path is relative to the resolved working directory and must not escape it. In the example above, a scenario stored under `profiles/` uses `working_directory: ..` so Playwright is resolved from the repository package root and `profiles/camera-pan.mjs` is loaded from that same root.

A journey module exports one function:

```js
export async function run({ page }) {
  await page.goto("http://127.0.0.1:4173/");
  await page.getByRole("button", { name: "Load fixture" }).click();
}
```

The journey must arrange or connect to its own deterministic application fixture. This adapter does not start a development server, Docker Compose stack, or production service. Service lifecycle belongs to the later service/browser orchestration layer.

## Consumer prerequisites

The resolved working directory must provide:

- Node on `PATH`;
- the `playwright` package;
- a usable Playwright Chromium installation.

`runtime-profiler detect` can prove Node availability globally. Playwright package and Chromium availability are verified when the specific consumer journey is captured because they are properties of that consumer working directory rather than the profiler repository.

## Evidence

A successful browser capture keeps the ordinary immutable bundle contract and adds:

- `chromium-trace.json` — the raw Chromium trace;
- `chromium-trace-summary.json` — bounded renderer-main-thread long tasks, nested hot paths, runtime attribution, and explicit JS/WASM boundary measures;
- `browser-runtime.json` — adapter, Node, Playwright, Chromium, viewport, and trace-category identity.

`metrics.json` remains valid but intentionally has no process samples or metrics for a browser journey. The Node/Playwright driver is a measurement harness; its process wall time and RSS are not relabeled as application performance.

## JS/WASM boundary evidence

Applications may use the explicit User Timing names documented in [Chromium trace analysis](chromium-trace.md):

```text
runtime-profiler:js-to-wasm:<label>
runtime-profiler:wasm-to-js:<label>
```

The profiler aggregates only explicitly instrumented boundary sections. It does not infer TypeScript/WASM overhead from neighboring events.

## Reproducibility and comparison

A browser bundle records the runtime identity needed for future strict comparison, but this adapter alone does not declare two browser bundles comparable and does not produce a release verdict. Browser baseline/candidate comparison must additionally prove compatible scenario/journey identity, runtime versions, viewport, trace configuration, and normalization semantics.

Use one stable fixture and one stable journey identity for reference/candidate work. If the journey logic changes, treat that as a workload change rather than silently comparing the resulting evidence.

## Current scope

The first adapter is intentionally narrow:

- one Chromium journey per capture;
- fixed 1280×720 viewport;
- renderer-main-thread trace evidence;
- no Lighthouse or React-render policy;
- no Expo/Tauri capture;
- no implicit web-server lifecycle;
- no browser pass/fail thresholds in `runtime-profiler`.

Those capabilities can layer on the same scenario and evidence boundaries without turning application-specific performance policy into profiler behavior.
