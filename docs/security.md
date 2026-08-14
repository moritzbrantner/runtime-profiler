# Security and privacy

Profiling artifacts can contain source locations, customer data, request
payloads, credentials, and memory contents. The default bundle is intentionally
minimal.

The v1 process collector does not persist:

- stdout or stderr;
- environment values;
- command arguments;
- request or response bodies;
- raw logs;
- prompts or model output;
- source contents.

`scenario.json` stores the program name and argument count, not the argument
values. The manifest stores artifact digests and source identity, not Git diffs.

Future adapters must provide explicit redaction before their normalized output
can be included. Raw traces, dumps, and heap profiles should be opt-in sidecars,
classified as sensitive, and excluded from agent context by default.
