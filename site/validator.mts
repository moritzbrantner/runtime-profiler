export type JsonRecord = Record<string, any>;

export interface ArtifactValidationStatus {
  path: string;
  verified: boolean;
  diagnostics: string[];
}

export interface BundleValidationReport {
  schema_version: "runtime-profiler/pages-validation/v1";
  operation: "validate-public-bundle";
  source: { manifest_url: string };
  bundle_id: string | null;
  scenario_id: string | null;
  valid: boolean;
  verified_files: number;
  diagnostics: string[];
  artifacts: ArtifactValidationStatus[];
  summary: {
    metric_count: number;
    sample_count: number;
    guidance_observation_count: number;
  };
  evidence: {
    manifest: JsonRecord;
    scenario: JsonRecord | null;
    environment: JsonRecord | null;
    metrics: JsonRecord | null;
    hotspots: JsonRecord | null;
    chromium_trace_summary: JsonRecord | null;
    browser_runtime: JsonRecord | null;
    agent_guidance: JsonRecord | null;
  };
  limitations: string[];
}

interface FetchResponse {
  ok: boolean;
  status: number;
  text(): Promise<string>;
  arrayBuffer(): Promise<ArrayBuffer>;
}

type FetchImpl = (input: string | URL) => Promise<FetchResponse>;
type CryptoImpl = Pick<Crypto, "subtle">;

interface ValidationOptions {
  fetchImpl?: FetchImpl;
  cryptoImpl?: CryptoImpl;
}

const REQUIRED_DOCUMENT_ARTIFACTS = new Map([
  ["scenario.json", "runtime-profiler/scenario-evidence/v1"],
  ["environment.json", "runtime-profiler/environment/v1"],
  ["metrics.json", "runtime-profiler/metrics/v1"],
  ["hotspots.json", "runtime-profiler/hotspots/v1"],
  ["agent-guidance.json", "runtime-profiler/agent-guidance/v1"],
]);
const OPTIONAL_DOCUMENT_ARTIFACTS = new Map([
  ["chromium-trace-summary.json", "runtime-profiler/chromium-trace-summary/v1"],
  ["browser-runtime.json", "runtime-profiler/browser-runtime/v1"],
]);
const DOCUMENT_ARTIFACTS = new Map([
  ...REQUIRED_DOCUMENT_ARTIFACTS,
  ...OPTIONAL_DOCUMENT_ARTIFACTS,
]);
const OPTIONAL_ARTIFACTS = new Set([
  "native-perf-report.tsv",
  "chromium-trace.json",
  ...OPTIONAL_DOCUMENT_ARTIFACTS.keys(),
]);

const MANIFEST_SCHEMA = "runtime-profiler/bundle-manifest/v1";
const FINGERPRINT_SCHEMAS = new Set([
  "runtime-profiler/environment-fingerprint/legacy-source-inclusive-v0",
  "runtime-profiler/environment-fingerprint/v1",
]);

function records(value: unknown): JsonRecord[] {
  return Array.isArray(value)
    ? value.filter((item): item is JsonRecord => Boolean(item) && typeof item === "object")
    : [];
}

function record(value: unknown): JsonRecord | null {
  return Boolean(value) && typeof value === "object" ? (value as JsonRecord) : null;
}

export async function validateBundleUrl(
  manifestUrl: string | URL,
  options: ValidationOptions = {},
): Promise<BundleValidationReport> {
  const fetchImpl: FetchImpl = options.fetchImpl ?? ((input) => fetch(input));
  const cryptoImpl = options.cryptoImpl ?? crypto;
  const manifestResponse = await fetchImpl(manifestUrl);
  if (!manifestResponse.ok) {
    throw new Error(`Could not load manifest (${manifestResponse.status}).`);
  }

  const manifestText = await manifestResponse.text();
  let manifest: JsonRecord;
  try {
    manifest = JSON.parse(manifestText) as JsonRecord;
  } catch {
    throw new Error("Manifest is not valid JSON.");
  }

  const diagnostics: string[] = [];
  const artifacts: ArtifactValidationStatus[] = [];
  const documents: Record<string, JsonRecord> = {};
  let verifiedFiles = 0;

  if (manifest.schema_version !== MANIFEST_SCHEMA) {
    diagnostics.push(`unsupported bundle manifest schema: ${String(manifest.schema_version ?? "missing")}`);
  }

  const manifestFiles = records(manifest.files);
  const declaredPaths = new Set(manifestFiles.map((artifact) => String(artifact.path ?? "")));
  const requiredPaths = new Set(REQUIRED_DOCUMENT_ARTIFACTS.keys());
  const allowedPaths = new Set([...requiredPaths, ...OPTIONAL_ARTIFACTS]);
  if (![...requiredPaths].every((path) => declaredPaths.has(path))) {
    diagnostics.push("manifest is missing one or more required v1 artifacts");
  }
  if (![...declaredPaths].every((path) => allowedPaths.has(path))) {
    diagnostics.push("manifest contains an unsupported v1 artifact");
  }

  for (const artifact of manifestFiles) {
    const path = typeof artifact.path === "string" ? artifact.path : "";
    const status: ArtifactValidationStatus = { path, verified: false, diagnostics: [] };
    artifacts.push(status);
    if (!isSafeRelativePath(path)) {
      status.diagnostics.push("unsafe artifact path");
      diagnostics.push(`unsafe artifact path: ${path}`);
      continue;
    }

    let response: FetchResponse;
    try {
      response = await fetchImpl(new URL(path, manifestUrl));
    } catch {
      status.diagnostics.push("artifact request failed");
      diagnostics.push(`artifact request failed: ${path}`);
      continue;
    }
    if (!response.ok) {
      status.diagnostics.push(`HTTP ${response.status}`);
      diagnostics.push(`missing artifact: ${path}`);
      continue;
    }

    const bytes = new Uint8Array(await response.arrayBuffer());
    const digest = await sha256(bytes, cryptoImpl);
    if (digest !== artifact.sha256) {
      status.diagnostics.push("digest mismatch");
      diagnostics.push(`digest mismatch: ${path}`);
      continue;
    }

    verifiedFiles += 1;
    status.verified = true;
    if (!DOCUMENT_ARTIFACTS.has(path)) {
      continue;
    }
    try {
      const parsed = record(JSON.parse(new TextDecoder().decode(bytes)));
      if (parsed) {
        documents[path] = parsed;
      } else {
        throw new Error("document is not an object");
      }
    } catch {
      status.diagnostics.push("invalid JSON");
      diagnostics.push(`invalid JSON artifact: ${path}`);
    }
  }

  validateDocuments(manifest, documents, diagnostics);

  const metrics = documents["metrics.json"] ?? null;
  const guidance = documents["agent-guidance.json"] ?? null;
  return {
    schema_version: "runtime-profiler/pages-validation/v1",
    operation: "validate-public-bundle",
    source: { manifest_url: String(manifestUrl) },
    bundle_id: typeof manifest.bundle_id === "string" ? manifest.bundle_id : null,
    scenario_id: typeof manifest.scenario_id === "string" ? manifest.scenario_id : null,
    valid: diagnostics.length === 0,
    verified_files: verifiedFiles,
    diagnostics,
    artifacts,
    summary: {
      metric_count: records(metrics?.metrics).length,
      sample_count: records(metrics?.samples).length,
      guidance_observation_count: records(guidance?.observations).length,
    },
    evidence: {
      manifest,
      scenario: documents["scenario.json"] ?? null,
      environment: documents["environment.json"] ?? null,
      metrics,
      hotspots: documents["hotspots.json"] ?? null,
      chromium_trace_summary: documents["chromium-trace-summary.json"] ?? null,
      browser_runtime: documents["browser-runtime.json"] ?? null,
      agent_guidance: guidance,
    },
    limitations: [
      "This browser operation validates an already captured public bundle; it does not execute or profile a workload.",
      "The source server must permit browser CORS requests for the manifest and artifact files.",
      "The native runtime-profiler CLI remains authoritative for capture and local bundle validation.",
    ],
  };
}

export function validateDocuments(
  manifest: JsonRecord,
  documents: Record<string, JsonRecord>,
  diagnostics: string[] = [],
): string[] {
  for (const [path, expectedSchema] of DOCUMENT_ARTIFACTS) {
    const document = documents[path];
    if (!document) continue;
    if (document.schema_version !== expectedSchema) {
      diagnostics.push(`unsupported ${path} schema: ${String(document.schema_version ?? "missing")}`);
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
  if (!FINGERPRINT_SCHEMAS.has(String(manifest.environment_fingerprint_schema_version ?? ""))) {
    diagnostics.push(
      `unsupported environment fingerprint schema: ${String(manifest.environment_fingerprint_schema_version ?? "missing")}`,
    );
  }

  const guidance = documents["agent-guidance.json"];
  if (guidance && guidance.scenario_id !== manifest.scenario_id) {
    diagnostics.push("agent guidance identity is incompatible with manifest");
  }

  validateNativePerf(manifest, scenario ?? null, documents["hotspots.json"] ?? null, diagnostics);
  validateBrowserJourney(manifest, scenario ?? null, metrics ?? null, documents, diagnostics);
  return diagnostics;
}

function validateNativePerf(
  manifest: JsonRecord,
  scenario: JsonRecord | null,
  hotspots: JsonRecord | null,
  diagnostics: string[],
): void {
  const collectors = records(scenario?.collectors);
  const collectorNames = Array.isArray(scenario?.collectors) ? scenario.collectors : [];
  void collectors;
  const nativeRequested = collectorNames.includes("native-perf");
  const rawPerfPresent = hasArtifact(manifest, "native-perf-report.tsv");
  if (nativeRequested) {
    if (
      hotspots?.status !== "collected" ||
      hotspots?.collector !== "native-perf" ||
      !hotspots?.tool_version ||
      !hotspots?.event ||
      !hotspots?.metric ||
      !hotspots?.unit ||
      !Number.isInteger(hotspots?.sample_period) ||
      Number(hotspots?.sample_period) <= 0 ||
      !hotspots?.symbolization_mode
    ) {
      diagnostics.push("native-perf scenario does not contain complete native-perf hotspot evidence");
    }
    const toolchainFields = [
      hotspots?.target_toolchain_kind,
      hotspots?.target_toolchain_fingerprint_schema_version,
      hotspots?.target_toolchain_fingerprint,
    ];
    const toolchainFieldsPresent = toolchainFields.map((value) => Boolean(value));
    if (
      toolchainFieldsPresent.some((present) => present) &&
      !toolchainFieldsPresent.every((present) => present)
    ) {
      diagnostics.push("native-perf target toolchain identity is only partially recorded");
    }
    if (
      typeof hotspots?.target_toolchain_fingerprint === "string" &&
      !/^[a-f0-9]{64}$/.test(hotspots.target_toolchain_fingerprint)
    ) {
      diagnostics.push("native-perf target toolchain fingerprint is malformed");
    }
    if (!rawPerfPresent) {
      diagnostics.push("native-perf scenario is missing its raw perf report artifact");
    }
  } else {
    if (rawPerfPresent) {
      diagnostics.push("non-native scenario unexpectedly contains native-perf raw evidence");
    }
    if (hotspots?.status === "collected" || hotspots?.collector) {
      diagnostics.push("non-native scenario unexpectedly claims collected native hotspot evidence");
    }
  }
}

function validateBrowserJourney(
  manifest: JsonRecord,
  scenario: JsonRecord | null,
  metrics: JsonRecord | null,
  documents: Record<string, JsonRecord>,
  diagnostics: string[],
): void {
  const collectorNames = Array.isArray(scenario?.collectors) ? scenario.collectors : [];
  const browserRequested = collectorNames.includes("browser-chromium");
  const browserPaths = ["chromium-trace.json", "chromium-trace-summary.json", "browser-runtime.json"];
  const browserPresence = browserPaths.map((path) => hasArtifact(manifest, path));

  if (browserRequested) {
    if (record(scenario?.target)?.target_type !== "browser-journey") {
      diagnostics.push("browser-chromium collector requires browser-journey evidence");
    }
    if (!browserPresence.every(Boolean)) {
      diagnostics.push("browser journey is missing Chromium trace evidence artifacts");
      return;
    }
    if (records(metrics?.metrics).length !== 0 || records(metrics?.samples).length !== 0) {
      diagnostics.push(
        "browser journey must not relabel Playwright driver process measurements as application metrics",
      );
    }
    const runtime = documents["browser-runtime.json"];
    const viewport = record(runtime?.viewport);
    if (
      !runtime?.adapter_version ||
      !runtime?.node_version ||
      !runtime?.playwright_version ||
      runtime?.browser_name !== "chromium" ||
      !runtime?.browser_version ||
      !Number.isInteger(viewport?.width) ||
      Number(viewport?.width) <= 0 ||
      !Number.isInteger(viewport?.height) ||
      Number(viewport?.height) <= 0 ||
      !Array.isArray(runtime?.trace_categories) ||
      runtime.trace_categories.length === 0
    ) {
      diagnostics.push("browser runtime metadata is incomplete");
    }
  } else if (browserPresence.some(Boolean)) {
    diagnostics.push("non-browser scenario unexpectedly contains browser trace evidence");
  }
}

function hasArtifact(manifest: JsonRecord, path: string): boolean {
  return records(manifest.files).some((artifact) => artifact.path === path);
}

export async function sha256(
  bytes: Uint8Array,
  cryptoImpl: CryptoImpl = crypto,
): Promise<string> {
  const digest = await cryptoImpl.subtle.digest("SHA-256", bytes as BufferSource);
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

export function isSafeRelativePath(path: unknown): path is string {
  if (typeof path !== "string" || !path || path.startsWith("/") || path.includes("\\")) {
    return false;
  }
  const segments = path.split("/");
  return !segments.some((segment) => !segment || segment === "." || segment === "..");
}
