# runtime-profiler

`runtime-profiler` captures reproducible runtime evidence for coding agents. It
runs a declared scenario, records bounded measurements, and emits a versioned
evidence bundle that evaluators can consume across a baseline and a candidate.
It can also normalize two strictly comparable bundles into a descriptive 0–100
reference-retention score without owning release policy.

The project deliberately separates collection and descriptive normalization from policy evaluation:

- **runtime-profiler** captures and validates runtime facts and can summarize strictly comparable baseline/candidate evidence into a descriptive score.
- **agent-contracts** defines the neutral cross-repository evidence reference when profiler evidence crosses into another independently owned component.
- **Moonlight** owns project-specific thresholds, regression policy, and pass/fail evaluation; a runtime-profiler score is evidence, not a release verdict.
- **coding-tooling** can invoke profiler scenarios through repository-declared deterministic capabilities; first-class profiler discovery belongs there rather than in this repository.
- **coding-agent-conventions** defines how agents act on runtime evidence and performance policy.
- **agent-loop-orchestrator**, when orchestrated mode is selected, may schedule capture/change/recapture work and store evidence/evaluation references in durable run state. It is not required for direct profiler use.

The core boundary is therefore:

```text
scenario + source revision
        |
        v
runtime-profiler capture
        |
        +--> immutable profiler bundle
                 |
                 +--> direct CLI / CI / coding-agent consumer
                 |
                 +--> optional agent.evidence/v1 reference
                              |
                              +--> evaluator or orchestrator

compatible reference + candidate bundles
        |
        +--> runtime-profiler score / comparability (descriptive evidence)
        |
        v
Moonlight / evaluator (project policy and verdict)
```

The profiler-specific bundle remains the source artifact. `agent.evidence/v1`
is only the neutral reference envelope when evidence crosses an independently
owned component boundary; it does not duplicate measurements or pull evaluator
policy into this repository. Direct Moonlight bundle support is a separate
adapter step after both components expose their neutral landscape boundaries.

## Current release

The `0.1` foundation supports repeatable command scenarios and records:

- wall-clock duration distributions;
- success rate and timeout state;
- maximum observed resident memory on Linux when `/proc` is available;
- source revision and a privacy-preserving environment fingerprint;
- SHA-256 integrity for every evidence artifact;
- deterministic JSON guidance sized for an agent context window;
- optional `agent.evidence/v1` references for validated bundles crossing a component boundary;
- descriptive reference-relative scoring for strictly comparable validated bundles.

The browser foundation also supports repository-declared Playwright Chromium
journeys. A browser capture records an immutable raw trace plus bounded
normalized evidence for renderer-main long tasks, hot paths, runtime
attribution, and explicit `runtime-profiler:js-to-wasm:*` /
`runtime-profiler:wasm-to-js:*` User Timing measures. New captures fingerprint
the journey module, embedded Playwright driver, and Chromium trace normalizer,
and `compare-browser` verifies strict browser identity before reporting only
`comparable`, `incomparable`, or `insufficient-evidence`. Browser guidance is
derived from bounded normalized summaries; raw trace events are not copied into
agent prompts. Lighthouse, React render-budget ingestion, Expo/Tauri adapters,
and flagship consumer dogfooding remain roadmap work.

Linux resident memory is sampled from `VmRSS`; `process.max_observed_rss` is the
largest sampled value and is deliberately not presented as an exact OS peak.
The v1 `process.max_rss` identifier remains as a compatibility alias with
identical statistics and does not imply a stronger peak-memory guarantee.
Capture plans advertise both RSS identifiers only when a capability probe can
read resident memory from `/proc/self/status`; their absence from the process
collector's measurement list means RSS collection is unsupported in the current
environment.
On Unix, each workload starts in its own process group so timeout termination
also reaches ordinary descendants in that group. If group termination is not
available, the collector falls back to terminating the direct child and does
not claim stronger descendant-cleanup guarantees for that platform. The
short-lived CLI installs interruption handling for its process lifetime so it
can terminate and reap that group before exiting; library callers retain
ownership of their embedding process's signal policy.

The score command does not define regression budgets, block changes, or claim a
candidate should ship. Those policy decisions belong to Moonlight or another
evaluator.

## Quick start

```bash
cargo run -- detect
cargo run -- plan --scenario examples/command.yaml
cargo run -- capture \
  --scenario examples/command.yaml \
  --output .runtime-profiler/example
cargo run -- validate --bundle .runtime-profiler/example
cargo run -- summarize --bundle .runtime-profiler/example
cargo run -- render-agent-guidance --bundle .runtime-profiler/example
cargo run -- evidence-reference \
  --bundle .runtime-profiler/example \
  --uri .agent-loop/evidence/runtime-profiler/example
```

`evidence-reference` is optional and is needed only when a validated bundle must
cross an independently owned component boundary.

After capturing the same scenario on a comparable reference and candidate source revision:

```bash
cargo run -- score \
  --reference .runtime-profiler/reference \
  --candidate .runtime-profiler/candidate \
  --json
```

For browser-journey bundles, prove browser/runtime/workload identity before
interpreting the two captures together:

```bash
cargo run -- compare-browser \
  --reference .runtime-profiler/reference \
  --candidate .runtime-profiler/candidate \
  --json
```

The first capture creates an immutable directory. Choose a new output directory
for every run; the CLI refuses to overwrite an existing one.

## Scenario

```yaml
schema_version: runtime-profiler/scenario/v1
id: example-command
target:
  type: command
  program: sh
  args: ["-c", "printf 'profiled workload\\n'"]
run:
  warmup_iterations: 1
  measurement_iterations: 5
  timeout_seconds: 30
collectors:
  - process
```

Environment variables may be inherited by name with `target.inherit_env`. Their
values are never copied into the evidence bundle.

## Evidence bundle

```text
bundle/
├── manifest.json
├── environment.json
├── scenario.json
├── metrics.json
├── hotspots.json
└── agent-guidance.json
```

Browser-journey bundles additionally contain integrity-checked
`chromium-trace.json`, `chromium-trace-summary.json`, and
`browser-runtime.json` artifacts. The raw trace remains evidence storage; the
normalized summary is the bounded surface used for browser guidance and strict
comparison identity.

The native bundle is the authoritative profiler artifact. When another
component needs a neutral reference, `evidence-reference` first validates the
bundle and then emits `agent.evidence/v1` with:

- `kind: runtime-profile-bundle`;
- the caller-provided storage URI unchanged;
- `digest: sha256:<manifest hash>`, which commits to the validated manifest and
  therefore to every artifact digest listed by that manifest;
- `createdAt` deterministically derived from the bundle's creation timestamp.

The neutral envelope contains no profiler measurements and can be regenerated
for a relocated bundle by supplying a different URI; the content digest and
creation time remain unchanged.
Caller-provided URIs are preserved verbatim and limited to 2,048 UTF-8 bytes so
CLI and agent output remains bounded.

The pinned external contract revision is documented in [`contracts/README.md`](contracts/README.md).

See [Architecture](docs/architecture.md), [Runtime scoring](docs/scoring.md),
[Browser comparability](docs/browser-comparability.md),
[Browser hot-path identity](docs/browser-hot-path-identity.md),
[Browser agent guidance](docs/browser-agent-guidance.md),
[JS/WASM boundary markers](docs/js-wasm-boundary-markers.md),
[Moonlight contract](docs/moonlight.md), and the [Roadmap](ROADMAP.md) for the
planned collector adapters.

## Development

```bash
python3 -m pip install -r requirements-dev.txt
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
python3 scripts/check_contracts.py
```
