# Runtime score history

`runtime-profiler/score-history/v1` records how each `main` revision retains runtime behavior relative to its first parent.

## Comparison model

The `Publish Score History` workflow checks out full repository history and resolves the current commit's first parent. It then builds both runtime-profiler revisions on the same GitHub runner and captures the same `examples/history-score.yaml` process workload.

The parent bundle is the reference and the current commit bundle is the candidate. The current profiler scores the pair with `runtime-profiler score` only when their runtime evidence remains comparable under the normal score contract.

This makes every numeric history point an adjacent-revision retention score:

- `100` means the current commit meets or beats its first parent on all scored evidence;
- values below `100` expose proportional regressions;
- the signed metric changes are retained so improvements remain visible even though retention scores cap at `100`.

The history is not an absolute performance index and does not establish regression budgets or release policy.

## Measurement boundary

Both revisions run on the same hosted runner and use the same scenario file, warmup count, measurement count, timeout, and process collector. Source revision is intentionally not part of the runtime environment fingerprint, so adjacent revisions can be compared when the execution environment is otherwise stable.

If either revision cannot build, capture, validate, or compare, the workflow does not invent a number. It retains an `unavailable` entry with diagnostic context instead.

## Storage

Generated history is stored in `history.json` on a dedicated `score-history` branch. It never writes generated evidence back to `main`.

Each commit appears at most once. Re-running a commit replaces its existing entry, and the history retains the latest 1,000 commit comparisons.

A scored entry includes:

- candidate and first-parent commit SHAs;
- commit timestamp;
- parent-relative score and rating;
- average signed change across retained metric summaries;
- per-metric scores and signed statistic changes;
- scenario and environment-fingerprint contract metadata;
- excluded metric evidence.

## Pages surface

`/score/` reads the raw history document from the persistence branch and renders the latest parent-relative score, signed changes, a 60-commit retention chart, the latest metric breakdown, and recent comparisons.

Pages only displays the captured history. Native runtime-profiler execution remains authoritative for producing and validating each comparison.
