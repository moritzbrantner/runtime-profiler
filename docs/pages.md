# Agent-facing GitHub Pages

The Pages surface lets humans and browser-capable coding agents validate and inspect an already captured public runtime-profiler bundle without cloning this repository or running the workload.

## Discovery

```text
https://moritzbrantner.github.io/runtime-profiler/agent-tool.json
```

## Human inspection

```text
https://moritzbrantner.github.io/runtime-profiler/?manifest=<public-manifest-url>
```

## Machine-oriented JSON view

```text
https://moritzbrantner.github.io/runtime-profiler/validate.json/?manifest=<public-manifest-url>
```

GitHub Pages is static hosting, so the JSON view executes browser JavaScript. It is not a conventional server-side `application/json` endpoint.

The browser validator checks the v1 manifest schema, exact artifact set, safe relative paths, SHA-256 artifact integrity, artifact schema identifiers, scenario identity, environment fingerprint identity, and agent-guidance identity. It then exposes the validated metrics, environment, hotspots, and guidance in one result envelope.

The manifest host must allow browser CORS requests. Native `runtime-profiler validate --bundle ...` remains authoritative, and capture always stays local.
