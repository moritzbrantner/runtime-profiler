# Native `perf` hotspot collector

`native-perf` is an optional Linux collector for command scenarios. It adds sampled native CPU hotspot and bounded call-path evidence to the existing process measurements; it does not replace `process.wall_time`, RSS evidence, correctness tests, or evaluator policy.

## Scenario

Request it together with the process collector:

```yaml
schema_version: runtime-profiler/scenario/v1
id: native-hotspots
target:
  type: command
  program: ./target/release/example
run:
  warmup_iterations: 1
  measurement_iterations: 5
  timeout_seconds: 30
collectors:
  - process
  - native-perf
```

Use `cargo run -- detect` or `cargo run -- plan --scenario <path>` before capture. `native-perf` is available only when the host is Linux, `perf` returns a usable version, and the environment permits recording the configured user-space CPU event with the configured call-graph mode. A missing tool, denied profiling permission, or unavailable call-graph capture is reported as unavailable; process timing is never relabeled as hotspot evidence.

## Capture semantics

The collector profiles the same target argv, working directory, inherited environment names, timeout, and process-group lifecycle as the process collector. It records the `cycles:u` event with a fixed sample period and bounded DWARF call stacks (`dwarf,4096`). The same capture is normalized into:

- flat hotspot rows containing sample count, event period, source location, symbol, and DSO identity;
- folded caller-to-callee symbol paths, bounded to 32 frames, eight paths per hotspot, and 1,024 unique paths per report.

A successful native capture adds:

- normalized, deterministic entries to `hotspots.json`;
- optional bounded `call_paths` on matching hotspot symbols, including deterministic path ids, caller-to-callee symbol frames, and observed sample counts;
- the collector version, event, metric identity, sample period, symbolization/call-graph mode, totals, and truncation state;
- a privacy-preserving target Rust toolchain fingerprint when `rustc --version --verbose` is available;
- `native-perf-report.tsv` as a raw integrity-checked bundle artifact containing the flat report plus a comment-prefixed folded call-path section;
- at most a small top set of flat hotspot observations to agent guidance. The nested call paths remain available through the referenced hotspot evidence without expanding prompt output.

The target-toolchain fingerprint is a SHA-256 digest over a versioned normalized Rust compiler identity. Raw `rustc` output is not stored. If `rustc` is unavailable, native hotspot capture remains valid descriptive evidence, but strict hotspot comparison remains insufficient because the target toolchain cannot be proven compatible.

The raw report is evidence, not prompt text. Source locations are made repository-relative when safely resolvable; external absolute paths are not used as stable hotspot identities. Call paths deliberately contain symbols only. Their association with a flat hotspot is therefore symbol-level; the flat hotspot remains authoritative for source-file, line, and DSO identity.

## Comparability

Use the dedicated comparability report before interpreting baseline/candidate hotspot distributions:

```bash
cargo run -- compare-hotspots \
  --reference .runtime-profiler/reference \
  --candidate .runtime-profiler/candidate \
  --json
```

This check is separate from `score`. It never produces a performance score or a release verdict.

A report is `comparable` only when both bundles validate and the following identity matches:

- scenario/workload id and digest;
- environment-fingerprint schema and value;
- collector identity and `perf` version;
- event, metric, unit, and sample period;
- symbolization mode, which includes the call-graph capture/report mode for native call-path evidence;
- target toolchain kind, fingerprint schema, and fingerprint.

Source Git revisions may differ because baseline and candidate code are expected to differ. A real identity mismatch is `incomparable`. Missing required comparison identity is `insufficient-evidence`; it is never promoted to green by assumption. Bundles from the earlier flat-only native collector are intentionally not strictly comparable with call-path-enabled captures because their symbolization mode differs.

For the current native-Rust slice, target toolchain identity uses the ambient `rustc --version --verbose` contract. This is deliberately separate from the scenario digest: the scenario identifies the workload and target command shape, while the toolchain fingerprint identifies the compiler/runtime context expected to remain stable across a baseline/candidate code change.

## Interpretation

`native-perf.period` is event-count evidence derived from the sampled event period. It is not milliseconds. Flat hotspot weights from a different event, collector configuration, workload, tool/runtime fingerprint, or symbolization mode must not be treated as directly comparable.

Call-path `samples` are observed sampled stack counts, not event-count weights and not elapsed time. They answer which caller-to-callee paths reached an observed hot symbol during the capture; they do not assign the full flat-hotspot event weight to every frame in the path.

A hotspot or call path says where sampled execution cost was observed. Neither proves that a frame is the semantic root cause of a regression. Release thresholds and baseline/candidate verdicts remain evaluator or repository policy.

## Hosted CI

Parser, normalization, compatibility, and bundle-integrity tests are fixture-driven and do not require `perf` permission on an ordinary hosted runner. Environment-specific native capture is valid evidence only when capability detection actually succeeds; a skipped or unavailable native collector is not a green profiling result.
