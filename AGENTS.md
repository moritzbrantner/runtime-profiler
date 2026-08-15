# Agent instructions

## Purpose

This repository owns runtime evidence capture and normalization. Preserve the
boundary between measurement and evaluation.

## Hard boundaries

- Do not add baseline-versus-candidate comparison. Moonlight owns comparison.
- Do not turn collection results into pass/fail performance gates.
- Do not make Moonlight's private data model a runtime-profiler contract.
- Cross-repository evidence references belong to `agent-contracts`; profiler-specific measurements and bundle schemas stay owned here.
- Do not persist prompts, model output, raw application logs, environment
  values, secrets, private issue text, or arbitrary source contents.
- Do not silently overwrite evidence. A bundle is immutable once created.
- Do not let an LLM manufacture measurements or confidence claims.

## Change requirements

- Keep machine-readable profiler contracts versioned and backward compatible within a
  schema version.
- Keep neutral interchange adapters compatible with the pinned `agent-contracts` contract version they claim to emit.
- Add fixtures and tests for contract changes.
- Prefer thin adapters around mature profilers over bespoke profiling engines.
- Report unsupported collectors explicitly rather than degrading silently.
- Keep command output bounded and JSON-capable for coding agents and CI.
- Run formatting, clippy, unit tests, integration tests, and contract checks.

## Landscape handoff

- `coding-tooling` may invoke declared profiler scenarios but does not own profiler semantics.
- `agent-loop-orchestrator` owns when captures run and where evidence references are attached to a durable run.
- `coding-agent-conventions` owns policy about when performance evidence is required and how agents react to it.
- Moonlight or another evaluator owns baseline/candidate comparison and verdict language.
- Direct evaluator adapters must preserve the neutral `agent.evidence/v1` boundary instead of replacing it.

## Architecture

- `contract`: serialized, versioned profiler data structures.
- `scenario`: parsing and semantic validation.
- `capture`: scenario execution and raw measurement.
- `bundle`: immutable artifact creation and integrity validation.
- CLI: a thin adapter over the library.
