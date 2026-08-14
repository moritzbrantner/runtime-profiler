# Agent instructions

## Purpose

This repository owns runtime evidence capture and normalization. Preserve the
boundary between measurement and evaluation.

## Hard boundaries

- Do not add baseline-versus-candidate comparison. Moonlight owns comparison.
- Do not turn collection results into pass/fail performance gates.
- Do not persist prompts, model output, raw application logs, environment
  values, secrets, private issue text, or arbitrary source contents.
- Do not silently overwrite evidence. A bundle is immutable once created.
- Do not let an LLM manufacture measurements or confidence claims.

## Change requirements

- Keep machine-readable contracts versioned and backward compatible within a
  schema version.
- Add fixtures and tests for contract changes.
- Prefer thin adapters around mature profilers over bespoke profiling engines.
- Report unsupported collectors explicitly rather than degrading silently.
- Keep command output bounded and JSON-capable for coding agents and CI.
- Run formatting, clippy, unit tests, integration tests, and contract checks.

## Architecture

- `contract`: serialized, versioned data structures.
- `scenario`: parsing and semantic validation.
- `capture`: scenario execution and raw measurement.
- `bundle`: immutable artifact creation and integrity validation.
- CLI: a thin adapter over the library.
