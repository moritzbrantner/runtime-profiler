# Browser hot-path identity

Chromium hot-path frames in `runtime-profiler/chromium-trace-summary/v1` preserve the bounded Chromium trace event `category` and `name`. A hot-path id is deterministically derived from the ordered sequence of those raw frame pairs.

That identity is descriptive trace identity, not a normalized semantic call-stack vocabulary.

## Comparison contract

A raw Chromium hot path may be interpreted across a reference and candidate only after `compare-browser` has established strict browser comparability. In particular, the two bundles must record the same Chromium version, trace categories, Chromium trace summary schema, and trace-normalizer source digest.

Within that envelope, a matching hot-path id means that runtime-profiler observed the same ordered Chromium `(category, name)` path under the same recorded browser/normalizer contract. A changed id means the recorded path changed; it does not by itself establish a semantic application change or a regression.

Raw hot-path ids must not be compared across different Chromium versions. Chromium trace event names and categories are instrumentation-owned details and are not treated here as a stable cross-version ABI.

## Why there is no semantic frame vocabulary yet

Introducing labels such as `script`, `wasm-call`, `layout`, or `paint` as canonical frame identity would create a new profiler-owned interpretation layer. That could hide browser instrumentation changes and would require its own versioned mapping, tests, provenance, and comparability digest.

Until a concrete consumer requires cross-Chromium hot-path comparison, runtime-profiler keeps the lossless bounded raw frame identity and fails closed at the existing browser-version boundary instead of guessing semantic equivalence.

If a semantic frame vocabulary is introduced later, it must be additive or use a new schema/versioned identity contract. Existing v1 raw paths remain descriptive evidence and are not silently reinterpreted.

## Ownership boundary

`runtime-profiler` owns deterministic extraction, bounded aggregation, and evidence identity. Chromium owns the underlying trace instrumentation. Repository policy or Moonlight may evaluate strictly comparable evidence, but neither should treat a raw Chromium hot-path id as a cross-version semantic function identity.
