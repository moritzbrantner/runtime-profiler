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

## First canary set

Use a deliberately small set spanning different workload shapes:

- `rect` — deterministic JavaScript/TypeScript state-propagation benchmark;
- `dirbase` — CLI/server parity workload against JSON Server;
- `rust-kernels` — native algorithm/kernel workloads;
- `collision-lab` — compute-heavy simulation workloads;
- `audio-analysis` — native/media analysis command workloads;
- `nlp-stack` — pipeline workloads where throughput/latency matters;
- `native-whisperx` — native transcription/alignment workloads where runtime and memory are important.

Do not add all of these at once. Promote the scenario pattern after at least one canary of that shape is reproducible across repeated captures.

## Scenario archetypes

### CLI or batch command

Use one command with fixed inputs and bounded iterations. Prefer a fixture already used by repository tests or benchmarks.

```yaml
schema_version: runtime-profiler/scenario/v1
id: parse-fixture

target:
  type: command
  program: bun
  args: ["run", "benchmark:smoke"]

run:
  warmup_iterations: 1
  measurement_iterations: 5
  timeout_seconds: 30

collectors:
  - process
```

### Library/kernel workload

Do not profile an arbitrary test runner when the actual performance contract is a public function/kernel. Add a small repository-owned executable/benchmark harness that calls the stable public seam with deterministic fixtures, then profile that command.

The harness owns domain input generation and correctness assertions. `runtime-profiler` owns repeatable capture.

### Web/browser workload

Keep browser orchestration in the repository's existing deterministic browser/performance command. Profile that command only when process-level timing/memory is meaningful. Browser rendering metrics such as LCP/CLS or style/render timings belong to the browser-specific tool that measures them; do not reinterpret them as process metrics.

### Native/desktop/media workload

Use a deterministic local fixture, bounded output location, and explicit timeout. Avoid network downloads and mutable external corpora in the normal scenario. For GPU/audio/video work, record relevant environment identity and keep unsupported collectors explicit rather than fabricating a metric.

## coding-tooling integration

A consumer may expose the capture command through a repository-owned `benchmark`, `benchmark:smoke`, or another explicit `capabilityCommands` entry. `coding-tooling` may discover/invoke that command; it does not own profiler scenarios, metric definitions, thresholds, or regression policy.

Prefer this split:

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

## Baseline and retention rules

1. Bind every bundle to the source revision and environment fingerprint captured by the profiler.
2. Never overwrite an existing bundle; a new capture gets a new output directory.
3. Keep the scenario file in the consumer repository so the workload definition changes through normal review.
4. Treat a scenario change as a new comparison boundary unless compatibility is explicit.
5. Refresh a performance baseline only after the candidate is accepted for semantic reasons; do not move the baseline merely to make a regression disappear.
6. Retain enough recent accepted/candidate evidence to diagnose a regression, but do not commit routine generated bundles to source control. CI/local artifact storage is preferable.
7. Missing or incompatible environment evidence means "not comparable", not pass/fail.

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
