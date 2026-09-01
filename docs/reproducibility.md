# Reproducibility policy

A performance number is meaningful only together with its workload and
environment. Every capture therefore identifies:

- the normalized scenario digest;
- source revision and dirty state when Git is available;
- operating system, architecture, kernel, and logical CPU count;
- warm-up count, measured count, and per-iteration timeout;
- collector set and metric units.

## Environment handling

Command scenarios start with a cleared environment. `PATH` is retained so the
declared program can be resolved. Additional variables must be named in
`target.inherit_env`.

Environment values are inputs to the executed program but are never stored in
the bundle. Future releases may add keyed, one-way value fingerprints where
needed, but must not expose raw values.

`environment_fingerprint` is computed from normalized execution-environment
properties only. Source revision and dirty state remain recorded as provenance
but do not participate in the fingerprint, so changing code alone does not turn
a same-machine baseline/candidate pair into a cross-environment comparison.
The fingerprint input is deliberately limited to non-secret properties already
recorded in the environment document; usernames, hostnames, repository paths,
and environment-variable values are not added to it.

## Statistical interpretation

runtime-profiler reports samples and descriptive statistics. It does not infer
causality or significance. Moonlight must account for variance, measurement
overhead, multiple comparisons, and practical effect size before claiming an
improvement or regression.

## Dirty sources

A dirty working tree is valid evidence for local iteration but should not be
accepted as release evidence unless the consuming policy explicitly allows it.
