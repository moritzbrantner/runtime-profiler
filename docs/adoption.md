# Adoption Guide

`runtime-profiler` is an evidence collector, not a universal repository gate. Adopt it where runtime cost is part of the product or a recurring engineering decision; leave tiny or performance-insensitive repositories alone.

## Rollout principle

Start with one repository-owned deterministic scenario that already represents meaningful behavior. Capture stable evidence first. Comparison, thresholds, and merge authority are later decisions owned by an evaluator/repository policy.

```text
repository workload
      |
      v
runtime-profiler capture
      |
      v
immutable evidence bundle
      |
      +--> human/agent diagnosis
      |
      `--> optional evaluator such as Moonlight
```

A profiler bundle by itself must not say that a candidate is good or bad.

## Ownership

| Component | Owns | Does not own |
|---|---|---|
| Consuming repository | Representative scenarios, deterministic fixtures, journey modules, benchmark harnesses, and any adopted policy | Profiler measurement semantics or bundle schemas |
| `coding-tooling` | Discovering and invoking repository-declared commands such as `benchmark:smoke` or `load:smoke` | Scenarios, metric definitions, thresholds |
| `runtime-profiler` | Capture, normalization, bundle integrity, comparability identity, bounded agent guidance, descriptive scoring | Thresholds, regression policy, release verdicts |
| Moonlight / evaluator | Noise-floor calibration, thresholds, baseline/candidate verdicts | Re-measuring or reinterpreting profiler evidence |

```text
.coding-tooling.json / package scripts
  -> declares when/how a scenario command is run

profiles/runtime-profiler/*.yaml
  -> declares what runtime-profiler captures

runtime-profiler
  -> emits immutable evidence

Moonlight / repository policy
  -> optionally evaluates compatible evidence
```

Do not add a generic profiler capability to every repository merely because the executable is available.

## Choosing evidence

Pick the tool that measures the question being asked. These kinds of evidence are not substitutes for each other.

| Question | Evidence | Owner |
|---|---|---|
| How fast is this function/kernel in isolation? | Micro-benchmark (Criterion, Iai-Callgrind, Vitest bench, BenchmarkDotNet) | Repository benchmark tooling |
| How long and how much memory does a representative command or journey take, and where is time spent? | Profiler evidence: `process`, `native-perf`, `browser-chromium` | `runtime-profiler` |
| How does a service behave under a bounded request workload? | Load evidence: `http-workload` with the `http-curl` collector | `runtime-profiler` (bounded smoke load only; not capacity testing) |
| How large is the built artifact? | Size evidence (bundle analyzers, `cargo bloat`, size budgets) | Repository / `coding-tooling` size budgets; no runtime-profiler adapter exists yet |

A benchmark harness can still be the *workload* of a profiler scenario: profile the command that runs it when process-level time, memory, or hotspots matter.

## Choosing representative scenarios

- Prefer a workload that already exists in tests or benchmarks and represents a real user or product path.
- Use fixed inputs, local fixtures, and bounded iterations. Avoid network downloads and mutable external corpora.
- Give each scenario a stable `id`. Changing the workload changes the scenario digest, which is a new comparison boundary.
- Keep the scenario file in the consuming repository so workload changes go through normal review.
- Start with one scenario per workload shape; add more only after repeated captures of the first are understood.

First canary candidates spanning different workload shapes: `rect` (JavaScript/TypeScript state propagation), `dirbase` (CLI/server parity), `rust-kernels` (native kernels), `collision-lab` (compute-heavy simulation), `audio-analysis` and `native-whisperx` (native/media), `nlp-stack` (pipeline throughput), and `maps` (browser journeys). Do not add all of them at once.

## Minimal examples

Run `cargo run -- detect` and `cargo run -- plan --scenario <path>` before capturing. `detect` reports which collectors are implemented and usable on the current host.

### Rust / native command

```yaml
schema_version: runtime-profiler/scenario/v1
id: kernel-smoke
target:
  type: command
  program: ./target/release/kernel-bench
  args: ["--fixture", "fixtures/small.bin"]
run:
  warmup_iterations: 1
  measurement_iterations: 5
  timeout_seconds: 30
collectors:
  - process
  - native-perf
```

`process` records wall time, exit state, timeout, and Linux maximum observed RSS. `native-perf` adds sampled CPU hotspots and bounded call paths when Linux `perf` is usable; see [native-perf.md](native-perf.md). Build the target before capture; the profiler does not build it.

### React / web journey

```yaml
schema_version: runtime-profiler/scenario/v1
id: dashboard-filter-v1
target:
  type: browser-journey
  module: profiles/dashboard-filter.mjs
  working_directory: ..
  inherit_env: []
run:
  warmup_iterations: 0
  measurement_iterations: 1
  timeout_seconds: 60
collectors:
  - browser-chromium
```

The journey module exports `run({ page })` and connects to its own deterministic application fixture; runtime-profiler owns Chromium tracing and normalization. See [playwright.md](playwright.md). Lighthouse and React render-budget evidence are not implemented; keep using the repository's existing Lighthouse configuration for those and do not relabel them as profiler metrics.

### .NET service

```json
{
  "schema_version": "runtime-profiler/scenario/v1",
  "id": "orders-api-smoke",
  "target": {
    "type": "http-workload",
    "fixture": {
      "program": "dotnet",
      "args": ["run", "--no-build", "--configuration", "Release", "--project", "src/Orders.Api"],
      "working_directory": "..",
      "health_path": "/health",
      "startup_timeout_seconds": 30
    },
    "requests": [{ "method": "GET", "path": "/orders?page=1", "expected_status": 200 }],
    "request_count": 100,
    "concurrency": 4,
    "request_timeout_seconds": 5
  },
  "run": { "warmup_iterations": 1, "measurement_iterations": 5, "timeout_seconds": 60 },
  "collectors": ["http-curl"]
}
```

The service must bind `127.0.0.1` on an OS-assigned port and write that port to `RUNTIME_PROFILER_PORT_FILE`; see [http-workloads.md](http-workloads.md). This yields request latency, status, and throughput evidence only. The .NET EventPipe adapter (`dotnet-counters`, `dotnet-trace`, GC evidence) is not implemented and `detect` reports `dotnet-eventpipe` as unavailable.

### Expo / mobile

No mobile or Expo runtime adapter exists. Device startup, frame stalls, and on-device CPU/memory cannot be captured by runtime-profiler today and must be reported as unavailable, not inferred from other measurements. A deterministic pure-JavaScript workload in the app (for example a data-transform module driven by a Node or Bun harness) can be profiled as an ordinary `command` scenario with the `process` collector; that evidence describes the harness process, not the mobile app runtime.

## Reference and candidate capture

1. Capture the reference revision into a new output directory.
2. Change the code without changing the scenario file, fixtures, toolchain, or host.
3. Capture the candidate into another new output directory.
4. Validate both bundles, then check comparability before interpreting differences.

```bash
cargo run -- capture --scenario profiles/runtime-profiler/kernel.yaml --output .runtime-profiler/reference
# change code, rebuild
cargo run -- capture --scenario profiles/runtime-profiler/kernel.yaml --output .runtime-profiler/candidate
cargo run -- validate --bundle .runtime-profiler/reference
cargo run -- validate --bundle .runtime-profiler/candidate
cargo run -- score --reference .runtime-profiler/reference --candidate .runtime-profiler/candidate --json
cargo run -- compare-hotspots --reference .runtime-profiler/reference --candidate .runtime-profiler/candidate --json  # native-perf
cargo run -- compare-browser --reference .runtime-profiler/reference --candidate .runtime-profiler/candidate --json   # browser-chromium
```

`score` is descriptive (see [scoring.md](scoring.md)). The comparison commands report only `comparable`, `incomparable`, or `insufficient-evidence`; none of them is a verdict.

## Environment comparability

Every bundle records the source revision separately from a privacy-preserving environment fingerprint. Source revisions are expected to differ between reference and candidate; scenario digests and environment identity are not. Collector-specific identity (perf version and sampling configuration, Rust toolchain, Chromium version and trace categories, curl build) must also match. Captures from different machines or runner classes are not comparable by default. Ordinary shared CI runner wall-clock results are informational. See [reproducibility.md](reproducibility.md), [native-perf.md](native-perf.md#comparability), and [browser-comparability.md](browser-comparability.md).

## Unsupported, unavailable, and missing evidence

- **Unsupported / not implemented**: no adapter exists for the runtime (for example .NET EventPipe, Bun CPU profiles, Lighthouse, Expo, Tauri). Say so; do not substitute a weaker measurement under the same name.
- **Unavailable**: the adapter exists but the host cannot run it (for example `perf` missing or recording denied). `detect` and `plan` report this explicitly.
- **Missing**: evidence that should exist for an applicable scenario was not captured or does not validate.

None of these is a pass. Absent evidence means "not measured" or "not comparable", never green.

## Baseline and retention rules

1. Bind every bundle to the source revision and environment fingerprint captured by the profiler.
2. Never overwrite an existing bundle; a new capture gets a new output directory.
3. Treat a scenario change as a new comparison boundary unless compatibility is explicit.
4. Refresh a performance baseline only after the candidate is accepted for semantic reasons; do not move the baseline merely to make a regression disappear.
5. Retain enough recent accepted/candidate evidence to diagnose a regression, but do not commit routine generated bundles to source control. CI/local artifact storage is preferable.

## Promotion to evaluation

Only add Moonlight or another evaluator after the capture scenario is stable enough that repeated unchanged-source runs have understood variance. Start evaluation as advisory.

A repository may later make a specific evaluation blocking when:

- the workload represents a real product/public performance contract;
- baseline and candidate inputs are comparable;
- environment incompatibility is distinguishable from regression;
- variance and normalization are understood;
- repeated canary changes show the classification is trustworthy;
- the repository explicitly adopts the threshold/policy.

The collector itself remains non-blocking: it captures facts and validates bundle integrity.
