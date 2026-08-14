#!/usr/bin/env python3
"""Dependency-free structural checks for committed contract artifacts."""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCHEMAS = ROOT / "schemas"


def load_json(path: Path) -> object:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def check_schema(path: Path) -> None:
    document = load_json(path)
    assert isinstance(document, dict), f"{path} must contain an object"
    assert document.get("$schema") == "https://json-schema.org/draft/2020-12/schema"
    assert isinstance(document.get("$id"), str), f"{path} must declare $id"
    assert document.get("type") == "object", f"{path} must describe an object"
    assert document.get("additionalProperties") is False, (
        f"{path} must reject unknown top-level fields"
    )


def check_example() -> None:
    scenario = load_json(ROOT / "examples" / "command.json")
    assert isinstance(scenario, dict)
    assert scenario["schema_version"] == "runtime-profiler/scenario/v1"
    assert scenario["target"]["type"] == "command"
    assert scenario["collectors"] == ["process"]
    assert scenario["run"]["measurement_iterations"] > 0


def check_required_files() -> None:
    required = {
        "README.md",
        "ROADMAP.md",
        "AGENTS.md",
        "docs/architecture.md",
        "docs/moonlight.md",
        "docs/reproducibility.md",
        "docs/security.md",
        "schemas/scenario.schema.json",
        "schemas/scenario-evidence.schema.json",
        "schemas/bundle-manifest.schema.json",
        "schemas/environment.schema.json",
        "schemas/metrics.schema.json",
        "schemas/hotspots.schema.json",
        "schemas/agent-guidance.schema.json",
    }
    missing = sorted(path for path in required if not (ROOT / path).is_file())
    assert not missing, f"missing required files: {', '.join(missing)}"


def main() -> None:
    schema_paths = sorted(SCHEMAS.glob("*.schema.json"))
    assert schema_paths, "no schemas found"
    for path in schema_paths:
        check_schema(path)
    check_example()
    check_required_files()
    print(f"contract checks passed ({len(schema_paths)} schemas)")


if __name__ == "__main__":
    main()
