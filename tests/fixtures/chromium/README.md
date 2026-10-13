# Chromium trace fixtures

These fixtures are synthetic regression inputs for the first-party Chromium trace normalizer.

- `renderer-nested.json` covers renderer-main metadata, nested complete events, a long task, explicit JS/WASM User Timing measures, and worker-thread exclusion.
- `raw-array.json` covers the raw event-array trace form.
- `malformed-events.json` mixes heterogeneous values with an invalid negative complete-event duration and must fail closed.
- `output-bounds-template.json` is expanded deterministically by the integration test to exercise long-task, hot-path, and boundary-marker output caps without committing a large repetitive trace.

All names, ids, timestamps, and labels are synthetic. Fixtures must not contain real browsing URLs, secrets, user data, or captured production payloads.
