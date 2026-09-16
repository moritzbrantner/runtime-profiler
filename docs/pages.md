# Agent-facing GitHub Pages

The Pages surface lets humans and browser-capable coding agents validate and inspect an already captured public runtime-profiler bundle without cloning this repository or running the workload.

## Discovery

```text
https://moritzbrantner.github.io/runtime-profiler/agent-tool.json
```

## Visual result explorer

```text
https://moritzbrantner.github.io/runtime-profiler/results/?manifest=<public-manifest-url>
```

The result explorer first runs the same browser validation and SHA-256 integrity checks as the validator. It then projects the validated evidence into a human-readable report with:

- run, source-revision, collector, and environment identity;
- process metric distributions and bounded iteration samples;
- native hotspot evidence when `native-perf` was collected;
- Chromium runtime attribution, long tasks, hot paths, and JS/WASM boundary markers for browser journeys;
- bounded agent observations and constraints;
- validation diagnostics and the artifact integrity trail.

The visual report is not an independent source of measurements and does not define regression or release policy. It renders only the normalized documents already carried by the immutable bundle; raw Chromium traces remain bundle artifacts.

## Human validation

```text
https://moritzbrantner.github.io/runtime-profiler/?manifest=<public-manifest-url>
```

## Machine-oriented JSON view

```text
https://moritzbrantner.github.io/runtime-profiler/validate.json/?manifest=<public-manifest-url>
```

GitHub Pages is static hosting, so the JSON view executes browser JavaScript. It is not a conventional server-side `application/json` endpoint.

The browser validator checks the v1 manifest schema, exact artifact set, safe relative paths, SHA-256 artifact integrity, artifact schema identifiers, scenario identity, environment fingerprint identity, and agent-guidance identity. It then exposes the validated metrics, environment, hotspots, browser summaries, and guidance in one result envelope.

The manifest host must allow browser CORS requests. Native `runtime-profiler validate --bundle ...` remains authoritative, and capture always stays local.
