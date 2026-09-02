const EXPECTED_ARTIFACTS = new Map([
  ["scenario.json", "runtime-profiler/scenario-evidence/v1"],
  ["environment.json", "runtime-profiler/environment/v1"],
  ["metrics.json", "runtime-profiler/metrics/v1"],
  ["hotspots.json", "runtime-profiler/hotspots/v1"],
  ["agent-guidance.json", "runtime-profiler/agent-guidance/v1"],
]);

const MANIFEST_SCHEMA = "runtime-profiler/bundle-manifest/v1";
const FINGERPRINT_SCHEMAS = new Set([
  "runtime-profiler/environment-fingerprint/legacy-source-inclusive-v0",
  "runtime-profiler/environment-fingerprint/v1",
]);

export async function validateBundleUrl(manifestUrl, options = {}) {
  const fetchImpl = options.fetchImpl ?? fetch;
  const cryptoImpl = options.cryptoImpl ?? crypto;
  const manifestResponse = await fetchImpl(manifestUrl);
  if (!manifestResponse.ok) {
    throw new Error(`Could not load manifest (${manifestResponse.status}).`);
  }

  const manifestText = await manifestResponse.text();
  let manifest;
  try {
    manifest = JSON.parse(manifestText);
  } catch {
    throw new Error("Manifest is not valid JSON.");
  }

  const diagnostics = [];
  const artifacts = [];
  const documents = {};
  let verifiedFiles = 0;

  if (manifest.schema_version !== MANIFEST_SCHEMA) {
    diagnostics.push(`unsupported bundle manifest schema: ${manifest.schema_version ?? "missing"}`);
  }

  const declaredPaths = new Set((manifest.files ?? []).map((artifact) => artifact.path));
  const expectedPaths = new Set(EXPECTED_ARTIFACTS.keys());
  if (!sameSet(declaredPaths, expectedPaths)) {
    diagnostics.push("manifest artifact set does not match the v1 contract");
  }

  for (const artifact of manifest.files ?? []) {
    const status = { path: artifact.path, verified: false, diagnostics: [] };
    artifacts.push(status);
    if (!isSafeRelativePath(artifact.path)) {
      status.diagnostics.push("unsafe artifact path");
      diagnostics.push(`unsafe artifact path: ${artifact.path}`);
      continue;
    }

    let response;
    try {
      response = await fetchImpl(new URL(artifact.path, manifestUrl));
    } catch {
      status.diagnostics.push("artifact request failed");
      diagnostics.push(`artifact request failed: ${artifact.path}`);
      continue;
    }
    if (!response.ok) {
      status.diagnostics.push(`HTTP ${response.status}`);
      diagnostics.push(`missing artifact: ${artifact.path}`);
      continue;
    }

    const bytes = new Uint8Array(await response.arrayBuffer());
    const digest = await sha256(bytes, cryptoImpl);
    if (digest !== artifact.sha256) {
      status.diagnostics.push("digest mismatch");
      diagnostics.push(`digest mismatch: ${artifact.path}`);
      continue;
    }

    verifiedFiles += 1;
    status.verified = true;
    try {
      documents[artifact.path] = JSON.parse(new TextDecoder().decode(bytes));
    } catch {
      status.diagnostics.push("invalid JSON");
      diagnostics.push(`invalid JSON artifact: ${artifact.path}`);
    }
  }

  validateDocuments(manifest, documents, diagnostics);

  const metrics = documents["metrics.json"] ?? null;
  const guidance = documents["agent-guidance.json"] ?? null;
  return {
    schema_version: "runtime-profiler/pages-validation/v1",
    operation: "validate-public-bundle",
    source: { manifest_url: String(manifestUrl) },
    bundle_id: manifest.bundle_id ?? null,
    scenario_id: manifest.scenario_id ?? null,
    valid: diagnostics.length === 0,
    verified_files: verifiedFiles,
    diagnostics,
    artifacts,
    summary: {
      metric_count: Array.isArray(metrics?.metrics) ? metrics.metrics.length : 0,
      sample_count: Array.isArray(metrics?.samples) ? metrics.samples.length : 0,
      guidance_observation_count: Array.isArray(guidance?.observations) ? guidance.observations.length : 0,
    },
    evidence: {
      manifest,
      scenario: documents["scenario.json"] ?? null,
      environment: documents["environment.json"] ?? null,
      metrics,
      hotspots: documents["hotspots.json"] ?? null,
      agent_guidance: guidance,
    },
    limitations: [
      "This browser operation validates an already captured public bundle; it does not execute or profile a workload.",
      "The source server must permit browser CORS requests for the manifest and artifact files.",
      "The native runtime-profiler CLI remains authoritative for capture and local bundle validation.",
    ],
  };
}

export function validateDocuments(manifest, documents, diagnostics = []) {
  for (const [path, expectedSchema] of EXPECTED_ARTIFACTS) {
    const document = documents[path];
    if (!document) continue;
    if (document.schema_version !== expectedSchema) {
      diagnostics.push(`unsupported ${path} schema: ${document.schema_version ?? "missing"}`);
    }
  }

  const metrics = documents["metrics.json"];
  if (metrics && metrics.scenario_id !== manifest.scenario_id) {
    diagnostics.push("metrics scenario id does not match manifest");
  }

  const scenario = documents["scenario.json"];
  if (
    scenario &&
    (scenario.id !== manifest.scenario_id || scenario.digest !== manifest.scenario_digest)
  ) {
    diagnostics.push("scenario evidence identity does not match manifest");
  }

  const environment = documents["environment.json"];
  if (environment) {
    if (environment.fingerprint !== manifest.environment_fingerprint) {
      diagnostics.push("environment fingerprint does not match manifest");
    }
    if (
      environment.environment_fingerprint_schema_version !==
      manifest.environment_fingerprint_schema_version
    ) {
      diagnostics.push("environment fingerprint schema does not match manifest");
    }
  }
  if (!FINGERPRINT_SCHEMAS.has(manifest.environment_fingerprint_schema_version)) {
    diagnostics.push(
      `unsupported environment fingerprint schema: ${manifest.environment_fingerprint_schema_version ?? "missing"}`,
    );
  }

  const guidance = documents["agent-guidance.json"];
  if (guidance && guidance.scenario_id !== manifest.scenario_id) {
    diagnostics.push("agent guidance identity is incompatible with manifest");
  }

  return diagnostics;
}

export async function sha256(bytes, cryptoImpl = crypto) {
  const digest = await cryptoImpl.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

export function isSafeRelativePath(path) {
  if (typeof path !== "string" || !path || path.startsWith("/") || path.includes("\\")) {
    return false;
  }
  const segments = path.split("/");
  return !segments.some((segment) => !segment || segment === "." || segment === "..");
}

function sameSet(left, right) {
  return left.size === right.size && [...left].every((value) => right.has(value));
}
