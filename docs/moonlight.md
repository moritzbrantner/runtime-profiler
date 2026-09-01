# Moonlight integration contract

This document specifies the **planned producer-specific adapter boundary** between
runtime-profiler bundles and Moonlight. It is intentionally downstream of the
neutral landscape contracts:

1. runtime-profiler exposes a complete immutable bundle as an `agent.evidence/v1` reference;
2. Moonlight exposes its evaluation as `agent.evaluation-result/v1`;
3. only then does a Moonlight adapter consume the profiler-native bundle described here.

The adapter may understand runtime-profiler bundle internals. The orchestrator,
`coding-tooling`, and `coding-agent-conventions` must not need to.

Moonlight consumes complete bundles; it must not scrape human CLI output.

## Compatibility checks

Before comparing baseline and candidate, a Moonlight runtime-profiler adapter must check:

1. Both `manifest.json` files and every listed artifact validate.
2. Both bundle manifest schema versions are supported.
3. `scenario_digest` values are equal.
4. Metric identifiers, units, and preferred directions match.
5. Sample counts satisfy the selected Moonlight policy.
6. Environment fingerprints match, or an explicit cross-environment policy is
   active.

Source revisions are expected to differ. Scenario digests are not.

`environment_fingerprint` identifies the execution environment, not the source
revision. Git SHA and dirty state are provenance fields and are deliberately
excluded from the fingerprint input. A code-only baseline/candidate change on
the same execution environment should therefore retain the same environment
fingerprint; a relevant execution-environment change should not.
Evaluators must also require matching `environment_fingerprint_schema_version`
values. A missing value in an older bundle identifies the legacy
source-inclusive algorithm rather than the current source-independent v1 input.

## Comparison strength

| Condition | Maximum conclusion |
|---|---|
| Same scenario and environment, adequate samples | Comparable |
| Same scenario, different environment | Inconclusive by default |
| Different scenario digest | Invalid comparison |
| Missing or corrupt artifact | Invalid evidence |
| Candidate correctness failure | Regression/blocking |

runtime-profiler records descriptive statistics. Moonlight owns noise-floor
calibration, confidence intervals, practical thresholds, and verdict language.
`coding-agent-conventions` owns the policy that decides when such a comparison
is required; the orchestrator owns when it is scheduled.

## Shared-contract handoff

The profiler bundle remains referenced by its original `agent.evidence/v1`
object. Moonlight's `agent.evaluation-result/v1` should retain that evidence
reference alongside its differential result rather than copying the profiler
bundle into evaluator or orchestrator state.

This preserves two independent compatibility axes:

- profiler bundle compatibility between runtime-profiler and the Moonlight adapter;
- neutral agent-contract compatibility between each component and the wider landscape.

## Agent handoff

`agent-guidance.json` is intentionally single-run evidence. A Moonlight result
may combine two such documents with its differential result, but must retain
the original evidence references. An agent should receive normalized summaries
and targeted artifact references, not complete traces or application logs.
