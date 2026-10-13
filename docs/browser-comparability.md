# Browser bundle comparability

Browser evidence must be proven comparable before reference/candidate runtime results are interpreted together. `runtime-profiler` performs this identity check without defining a regression threshold or release verdict.

```bash
cargo run -- compare-browser \
  --reference .runtime-profiler/reference \
  --candidate .runtime-profiler/candidate \
  --json
```

The result is one of:

- `comparable` — all required recorded identity matches;
- `incomparable` — at least one recorded identity value differs;
- `insufficient-evidence` — required identity is missing from one or both otherwise matching bundles.

## Required identity

Strict comparison requires matching:

- scenario id and scenario digest;
- execution-environment fingerprint schema and value;
- browser runtime schema;
- Playwright adapter version and adapter-source digest;
- Chromium trace normalizer-source digest;
- consumer journey-module digest;
- Node version;
- Playwright version;
- browser name and Chromium version;
- viewport;
- ordered Chromium trace categories;
- Chromium trace summary schema.

Source Git revisions may differ because a reference and candidate are expected to represent different code revisions. A changed journey module is not treated as a candidate performance change: its digest changes and the bundles are `incomparable` because the workload changed.

## Hot-path identity

Normalized Chromium hot paths preserve ordered raw trace-event `(category, name)` frame pairs. Their deterministic ids are descriptive trace identity, not a profiler-defined semantic call-stack vocabulary.

A hot-path id may be interpreted across a reference and candidate only after the strict comparison above has established the same Chromium version, trace categories, summary schema, and normalizer digest. Raw hot-path ids must not be compared across different Chromium versions, and an id change by itself is not a performance or semantic verdict.

See [Browser hot-path identity](browser-hot-path-identity.md) for the full boundary and the conditions that would be required before introducing a versioned semantic frame vocabulary.

## Legacy browser bundles

Browser bundles captured before journey, adapter, and normalizer digests were recorded remain valid immutable evidence. They are not silently upgraded to strict comparison evidence. `compare-browser` reports `insufficient-evidence` when those identity fields are absent.

Capture a new reference bundle with the current profiler before using strict browser comparison.

## Ownership boundary

`runtime-profiler` owns identity recording, validation, and this descriptive comparability report. It does not define acceptable long-task counts, boundary costs, hot-path budgets, or a release decision. Repository policy or Moonlight owns those decisions after evidence has first been proven comparable.
