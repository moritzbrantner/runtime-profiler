# Runtime score

`runtime-profiler score` provides a compact 0–100 descriptive comparison between two already captured, compatible evidence bundles. It is intended to make runtime movement easy to scan without discarding the underlying measurements.

This does not turn `runtime-profiler` into the policy evaluator. The command does not define acceptable regression budgets, block a change, or declare that a candidate should ship. Moonlight or another evaluator can apply project-specific policy to the same immutable evidence.

## Usage

```bash
runtime-profiler score \
  --reference .runtime-profiler/baseline \
  --candidate .runtime-profiler/candidate \
  --json
```

Both bundles are validated before comparison. Scoring is refused unless they have:

- the same scenario digest;
- the same environment fingerprint schema version;
- the same environment fingerprint;
- the same canonical metric set, units, and preferred directions.

These checks make the score a comparison of like-for-like runtime evidence rather than a number assembled across incompatible workloads or machines.

## Formula

The v1 score is reference-relative. A statistic that matches the reference scores `100`. A regression retains a proportional share of that score, while an improvement remains capped at `100` and is shown explicitly through `change_percent`.

For lower-is-better metrics:

```text
statistic score = 100 × min(1, reference / candidate)
```

For higher-is-better metrics:

```text
statistic score = 100 × min(1, candidate / reference)
```

`change_percent` always uses a positive sign for improvement and a negative sign for regression. When the reference is zero and a percentage change is mathematically undefined, the field is `null` rather than inventing a percentage.

Wall-time, memory, and future distribution-style metrics score both median and p95, then average those statistic scores. `process.success_rate` scores the mean because a binary median can hide intermittent failures. Metric scores are averaged equally into the overall score.

The v1 `process.max_rss` compatibility alias is canonicalized to `process.max_observed_rss` and excluded when both are present, so identical memory evidence is never double-weighted.

## Interpretation

The result uses the same scan-friendly bands as the repository score:

- `good`: 90–100;
- `needs-improvement`: 50–89;
- `poor`: below 50.

These labels describe retention against the selected reference; they are not release policy. For example, a score of 95 does not mean that a 5% regression is acceptable for a latency-sensitive project. An evaluator can set a much tighter threshold on wall-time p95 while still using this score as a compact dashboard signal.

The JSON contract is `runtime-profiler/score/v1` and retains the per-metric reference value, candidate value, signed change percentage, component score, bundle identities, source identities, and environment identity alongside the overall number.
