# Roadmap

The roadmap is ordered by evidence value, reproducibility, and integration
risk. A later phase must not weaken contracts established by an earlier phase.

## Phase 1 — Deterministic process evidence

- [x] Versioned scenario contract.
- [x] `detect`, `plan`, `capture`, `summarize`, `validate`, and
  `render-agent-guidance` commands.
- [x] Warm-up and repeated measurement runs.
- [x] Duration, success, timeout, and Linux maximum-observed-RSS measurements.
- [x] Immutable, integrity-checked evidence bundle.
- [x] Agent-sized structured guidance.
- [x] Moonlight compatibility rules.

Exit criterion: the same command scenario can be captured repeatedly and every
bundle can be validated without access to the original repository.

## Phase 2 — Workload and service adapters

- [ ] HTTP workload adapter with bounded request bodies and status summaries.
- [ ] Docker Compose target lifecycle and health checks.
- [ ] Fixture setup and teardown commands with explicit failure semantics.
- [ ] OpenTelemetry trace and metric ingestion.
- [ ] Collector-overhead metadata.

Exit criterion: a local API service can be started, exercised, stopped, and
represented as one self-contained evidence bundle.

## Phase 3 — Runtime-specific profiling

- [ ] .NET EventPipe adapter (`dotnet-counters`, `dotnet-trace`, GC dumps).
- [ ] Rust/native pprof or `perf` adapter with symbolization metadata.
- [ ] Bun/JavaScript CPU and heap profile adapter.
- [ ] Standard hotspot contract with symbol, file, line, and confidence.

Exit criterion: slow or resource-heavy service behavior can be traced to a
bounded set of source-level hotspots.

## Phase 4 — Browser and application journeys

- [ ] Playwright journey adapter.
- [ ] Chromium performance trace ingestion.
- [ ] Lighthouse navigation, timespan, and snapshot evidence.
- [ ] React render and long-task summaries.
- [ ] Tauri and Expo adapter discovery.

Exit criterion: frontend and backend evidence share the same scenario identity
and can be correlated without embedding raw traces in an agent prompt.

## Phase 5 — Ecosystem integration and dogfooding

- [ ] `coding-tooling` detection and invocation adapter.
- [ ] Moonlight baseline/candidate bundle importer.
- [ ] Agent-loop-orchestrator run contract.
- [ ] Convention profiles for performance work.
- [ ] Dogfood against runtime-profiler and Moonlight.
- [ ] Noise-floor calibration and cross-machine comparison policy.

Exit criterion: an orchestrated agent can identify a measured problem, change
code, rerun the identical scenario, and receive a defensible Moonlight verdict.
