# Runtime score history

`runtime-profiler/score-history/v1` records informational adjacent-revision runtime retention evidence for `main`. It is not a controlled benchmark series and does not establish regression budgets or release policy.

## Representative workload

Each comparison builds the current revision and its first parent on the same hosted runner. Both revisions then run the same `runtime-profiler-history-self-score-v1` capture scenario.

The measured target is each revision's own release binary executing:

```text
runtime-profiler score \
  --reference target/history-score-fixture \
  --candidate target/history-score-fixture \
  --json
```

Before measurement, the workflow copies the same checked-in immutable bundle from `tests/fixtures/history-score-bundle/` into both worktrees. The fixture's manifest SHA-256 is recorded as `workload_digest` in the history entry.

This exercises runtime-profiler startup, bundle integrity validation, JSON parsing, metric canonicalization, descriptive scoring, and result serialization. It replaces the old `sleep 0.02` target, which mostly measured scheduler noise rather than runtime-profiler work.

## Comparison model

The outer capture still compares parent and candidate bundles only when their scenario and environment fingerprints match. A numeric history point is therefore an adjacent-revision retention score:

- `100` means the candidate meets or beats its first parent on the scored evidence;
- values below `100` expose proportional retention loss;
- signed metric changes preserve direction and magnitude.

The history entry also retains `scenario_digest`, `environment_fingerprint`, and `workload_digest`. Pages only joins chart points from the latest matching scenario/workload cohort, so a workload revision cannot silently continue an older series.

## Hosted-runner boundary

Parent and candidate run back-to-back on the same GitHub-hosted runner for an individual comparison. This reduces some environment variance inside that pair, but different history points can still come from different hosted machines and background conditions.

Therefore the persisted history is informational evidence only. It must not be used as a blocking wall-clock regression gate or treated as a controlled long-term performance trend. Moonlight or another evaluator owns project policy and should use an appropriately controlled environment when its policy requires stronger conclusions.

If either revision cannot build, capture, validate, or compare, the workflow records an `unavailable` entry instead of inventing a score.

## Storage

Generated history is stored in `history.json` on the dedicated `score-history` branch. It never writes generated evidence back to `main`.

Each commit appears at most once. Re-running a commit replaces its existing entry, and the history retains the latest 1,000 adjacent comparisons.

## Pages surface

`/score/` shows the latest comparison, the current workload cohort, metric evidence, and recent entries. Older workload cohorts remain in the raw history for auditability but are not connected into the current chart.
