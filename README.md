# runtime-profiler

`runtime-profiler` captures reproducible runtime evidence for coding agents. It
runs a declared scenario, records bounded measurements, and emits a versioned
evidence bundle that evaluators can consume across a baseline and a candidate.

The project deliberately separates collection from evaluation:

- **runtime-profiler** captures and validates runtime facts.
- **agent-contracts** defines the neutral cross-repository evidence reference used by the wider agent landscape.
- **Moonlight** compares compatible baseline/candidate evidence and evaluates change; runtime-profiler does not own verdicts.
- **coding-tooling** can invoke profiler scenarios through repository-declared deterministic capabilities; first-class profiler discovery belongs there rather than in this repository.
- **coding-agent-conventions** defines how agents act on runtime evidence and performance policy.
- **agent-loop-orchestrator** schedules capture/change/recapture work and stores evidence/evaluation references in durable run state.

The intended boundary is therefore:

```text
scenario + source revision
        |
        v
runtime-profiler
        |
        +--> immutable profiler bundle
        |
        +--> agent.evidence/v1 reference
                    |
                    v
          orchestrator / evaluator
```

The profiler-specific bundle remains the source artifact. `agent.evidence/v1`
is only the neutral reference envelope; it must not duplicate measurements or
pull evaluator policy into this repository. Direct Moonlight bundle support is a
separate adapter step after both components expose their neutral landscape
boundaries.

## Current release

The `0.1` foundation supports repeatable command scenarios and records:

- wall-clock duration distributions;
- success rate and timeout state;
- peak resident memory on Linux when `/proc` is available;
- source revision and a privacy-preserving environment fingerprint;
- SHA-256 integrity for every evidence artifact;
- deterministic JSON guidance sized for an agent context window.

It does **not** compare runs or claim that a candidate is better. That belongs
to Moonlight or another evaluator.

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

See [Architecture](docs/architecture.md), [Moonlight contract](docs/moonlight.md),
and the [Roadmap](ROADMAP.md) for the planned collector adapters.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
python3 scripts/check_contracts.py
```
