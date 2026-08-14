# Moonlight integration contract

Moonlight consumes complete bundles; it must not scrape human CLI output.

## Compatibility checks

Before comparing baseline and candidate, Moonlight must check:

1. Both `manifest.json` files and every listed artifact validate.
2. Both bundle manifest schema versions are supported.
3. `scenario_digest` values are equal.
4. Metric identifiers, units, and preferred directions match.
5. Sample counts satisfy the selected Moonlight policy.
6. Environment fingerprints match, or an explicit cross-environment policy is
   active.

Source revisions are expected to differ. Scenario digests are not.

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

## Agent handoff

`agent-guidance.json` is intentionally single-run evidence. A Moonlight result
may combine two such documents with its differential result, but must retain
the original evidence references. An agent should receive normalized summaries
and targeted artifact references, not complete traces or application logs.
