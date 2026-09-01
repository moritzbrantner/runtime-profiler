#!/usr/bin/env python3
"""Structural checks for native contracts and pinned interchange schemas."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker


ROOT = Path(__file__).resolve().parent.parent
SCHEMAS = ROOT / "schemas"
AGENT_EVIDENCE_SCHEMA = ROOT / "contracts" / "agent-evidence-v1.schema.json"
AGENT_EVIDENCE_SCHEMA_SHA256 = (
    "bd5ebac98d98e03656d07a28f079e1544e3f37ba7bf8756935ad997e73485e92"
)
FINGERPRINT_SCHEMA_VERSIONS = [
    "runtime-profiler/environment-fingerprint/legacy-source-inclusive-v0",
    "runtime-profiler/environment-fingerprint/v1",
]


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


def agent_evidence_validator() -> Draft202012Validator:
    schema_bytes = AGENT_EVIDENCE_SCHEMA.read_bytes()
    actual_sha256 = hashlib.sha256(schema_bytes).hexdigest()
    assert actual_sha256 == AGENT_EVIDENCE_SCHEMA_SHA256, (
        "pinned agent.evidence/v1 schema changed without updating its contract pin"
    )

    schema = json.loads(schema_bytes)
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema, format_checker=FormatChecker())


def check_agent_evidence_contract(emitted_reference: Path | None) -> None:
    validator = agent_evidence_validator()
    example = load_json(ROOT / "examples" / "agent-evidence-reference.json")
    validator.validate(example)
    assert isinstance(example, dict)
    invalid_date = {**example, "createdAt": "not-a-date"}
    assert not validator.is_valid(invalid_date), (
        "agent.evidence/v1 date-time formats must be enforced"
    )
    if emitted_reference is not None:
        validator.validate(load_json(emitted_reference))


def check_fingerprint_schema_version(path: Path) -> None:
    schema = load_json(path)
    assert isinstance(schema, dict)
    required = schema.get("required")
    properties = schema.get("properties")
    assert isinstance(required, list)
    assert isinstance(properties, dict)
    assert "environment_fingerprint_schema_version" not in required, (
        f"{path} must accept legacy documents without a fingerprint schema field"
    )
    assert properties.get("environment_fingerprint_schema_version") == {
        "enum": FINGERPRINT_SCHEMA_VERSIONS
    }, f"{path} must constrain supported fingerprint schema versions"


def check_required_files() -> None:
    required = {
        "README.md",
        "ROADMAP.md",
        "AGENTS.md",
        "contracts/README.md",
        "contracts/agent-evidence-v1.schema.json",
        "docs/architecture.md",
        "docs/moonlight.md",
        "docs/reproducibility.md",
        "docs/security.md",
        "examples/agent-evidence-reference.json",
        "requirements-dev.txt",
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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--evidence-reference",
        type=Path,
        help="optional CLI-emitted agent.evidence/v1 JSON to validate",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    schema_paths = sorted(SCHEMAS.glob("*.schema.json"))
    assert schema_paths, "no schemas found"
    for path in schema_paths:
        check_schema(path)
    for name in ["environment.schema.json", "bundle-manifest.schema.json"]:
        check_fingerprint_schema_version(SCHEMAS / name)
    check_example()
    check_agent_evidence_contract(args.evidence_reference)
    check_required_files()
    print(f"contract checks passed ({len(schema_paths)} native schemas + agent.evidence/v1)")


if __name__ == "__main__":
    main()
