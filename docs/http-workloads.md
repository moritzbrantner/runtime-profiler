# Bounded HTTP workloads

`http-workload` captures HTTP latency, success/error rates, status codes, response
byte counts, request timeouts, and completed-request throughput. The `http-curl`
collector delegates transfers and concurrency to curl 8.4 or newer. Capture
requires Unix process-group cleanup and curl's HTTP support; detection and plans
report other environments as unsupported. It never chooses a remote API URL.

```bash
cargo run --locked -- plan --scenario examples/http-workload.json
cargo run --locked -- capture --scenario examples/http-workload.json --output .runtime-profiler/http-1
cargo run --locked -- validate --bundle .runtime-profiler/http-1
cargo run --locked -- summarize --bundle .runtime-profiler/http-1 --json
```

## Fixture ownership and cleanup

The foreground fixture command must create deterministic test data, bind an
OS-assigned port on `127.0.0.1`, and write the numeric port to the path supplied
in `RUNTIME_PROFILER_PORT_FILE`. The profiler supplies a private temporary
directory through `RUNTIME_PROFILER_FIXTURE_DIRECTORY`. Both paths are transient
control bindings; neither their values nor the port enter workload identity or
evidence. Working directories resolve relative to the scenario file. Only PATH
and explicitly named `inherit_env` variables reach the fixture; their values
are never persisted. Fixture output is discarded.

Publishing a port is followed by a bounded GET to `health_path`, requiring HTTP
200. The fixture remains owned throughout warmup and measurement. The profiler
terminates its process group through rustix's safe native Unix signal API and reaps it on success, startup or health failure,
measurement failure, batch timeout, or CLI interruption.

An optional `fixture.teardown` command with `program` and `args` runs after group
termination, including when fixture startup fails. It receives the same working
directory, inherited environment, and temporary control paths. It can release
resources outside the fixture process group, such as a disposable database or
container. Teardown has a 30-second timeout and ignores the first CLI cancellation
so cleanup can finish. Cleanup failure prevents bundle creation. Repository-owned
commands can wrap Compose; a built-in Compose or OpenTelemetry adapter remains
separate roadmap work.

## Workload and measurement limits

Each batch cycles through 1–32 declared requests in order. Each request names a
GET, HEAD, POST, PUT, PATCH, or DELETE method, an origin path of at most 2,048 ASCII
bytes, an expected status from 100–599, and an optional body of at most 65,536
UTF-8 bytes, plus an optional bounded `content_type`. Optional `headers` supply
at most 16 unique case-insensitive names (1–64 HTTP token bytes), each with a
nonblank printable ASCII value of at most 2,048 bytes. Routing/framing headers
(`Host`, `Content-Length`, `Transfer-Encoding`, `Connection`, `TE`, `Trailer`,
`Upgrade`, `Expect`) remain collector-controlled; use `content_type` for
`Content-Type`. Header names and values enter only the workload digest and
private curl configuration, never captured evidence. Store only disposable
fixture identities in scenario files, never production credentials. Bodies without an explicit
content type use curl's default media type. HEAD bodies, URL authorities, fragments, and backslashes are rejected.
Bodies use curl's binary transfer support. No redirects, retries, user curlrc,
or environment proxies participate. Response bodies are discarded and each
transfer permits at most 1 MiB of response data. curl 8.4 is required because
[earlier versions do not enforce this limit on unknown-length streams](https://curl.se/docs/manpage.html#--max-filesize).

Bounds are 1–1,000 requests per batch, concurrency 1–32 no greater than the request
count, request timeout 1–30 seconds, batch timeout 1–60 seconds, at most three
warmup batches and ten measured batches, and at most 10,000 total requests.
Startup is bounded to 1–60 seconds. A batch timeout or incomplete collector output
fails capture. Completed transfers with unexpected statuses, transport errors,
or individual request timeouts remain failed request measurements. A failed
warmup prevents measurement. No load-test performance threshold is applied.

Warmup and health requests are excluded from `metrics.json` and
`http-workload.json`. Latency statistics describe curl's per-transfer total time.
Success and error rates use all measured transfers. Throughput is completed
requests divided by collector batch wall time, including curl startup and polling;
it is not a service capacity claim. `http-workload.json` retains bounded numeric
samples and batch wall times without URLs, request headers, request bodies, response bodies, or
application logs. Normalized metrics are reconstructed and checked against this
artifact during bundle validation, using exact floating-point JSON round trips.

Fixture setup/teardown and collector wall times are recorded independently.
`overhead_status: not-isolated` means instrumentation overhead has not been
measured in a separate control run; the profiler does not subtract a fabricated
overhead estimate. These runs measure both fixture behavior and curl's observer
cost under the declared host conditions.

## Comparability and evaluation

Existing command and browser scenarios remain valid. HTTP scenarios add a tagged
target, collector, and versioned sidecar to the existing immutable bundle format.
The normalized scenario digest includes declared endpoints, payloads, methods,
counts, concurrency, fixture commands, and timeouts, but excludes dynamic ports.
Source revision remains separate from environment identity. Descriptive scoring
also requires identical curl build identity and collector adapter digest; changed
workloads, environments, tools, or adapters cannot silently compare. HTTP success
and error rates score their means so intermittent failures remain visible.

`coding-tooling` may invoke this native capture through `load:smoke`; it must not
provide another HTTP engine. A repository or evaluator owns thresholds, fixture
semantics, rollout decisions, and any performance verdict.
