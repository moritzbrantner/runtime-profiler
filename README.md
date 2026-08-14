# runtime-profiler

`runtime-profiler` captures reproducible runtime evidence for coding agents. It
runs a declared scenario, records bounded measurements, and emits a versioned
evidence bundle that Moonlight can compare across a baseline and a candidate.

The project deliberately separates collection from evaluation:

- **runtime-profiler** captures and validates facts.
- **Moonlight** compares two compatible bundles and evaluates change.
- **coding-tooling** detects and invokes the profiler through stable commands.
- **coding-agent-conventions** defines how agents act on the evidence.
- **agent-loop-orchestrator** schedules the capture/change/recapture loop.

## Current release

The `0.1` foundation supports repeatable command scenarios and records:

- wall-clock duration distributions;
- success rate and timeout state;
- peak resident memory on Linux when `/proc` is available;
- source revision and a privacy-preserving environment fingerprint;
- SHA-256 integrity for every evidence artifact;
- deterministic JSON guidance sized for an agent context window.

It does **not** compare runs or claim that a candidate is better. That belongs
to Moonlight.

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
