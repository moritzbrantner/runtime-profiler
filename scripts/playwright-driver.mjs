import { createRequire } from "node:module";
import { writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const TRACE_CATEGORIES = [
  "devtools.timeline",
  "v8",
  "v8.execute",
  "blink.user_timing",
].join(",");
const ADAPTER_VERSION = "runtime-profiler/playwright-driver/v1";

function parseArgs(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      throw new Error("expected --name value arguments");
    }
    values.set(name, value);
  }
  for (const required of ["--journey", "--trace", "--metadata"]) {
    if (!values.has(required)) {
      throw new Error(`missing required argument ${required}`);
    }
  }
  return values;
}

function loadPlaywright() {
  const requireFromConsumer = createRequire(path.join(process.cwd(), "package.json"));
  try {
    return {
      api: requireFromConsumer("playwright"),
      version: requireFromConsumer("playwright/package.json").version,
    };
  } catch (error) {
    throw new Error(
      "browser-chromium requires the consumer working directory to provide the `playwright` package",
      { cause: error },
    );
  }
}

async function readTraceStream(session, stream) {
  const chunks = [];
  for (;;) {
    const result = await session.send("IO.read", { handle: stream });
    if (result.data) {
      chunks.push(
        result.base64Encoded ? Buffer.from(result.data, "base64") : Buffer.from(result.data),
      );
    }
    if (result.eof) break;
  }
  await session.send("IO.close", { handle: stream });
  return Buffer.concat(chunks);
}

async function stopTrace(session) {
  const completed = new Promise((resolve) => session.once("Tracing.tracingComplete", resolve));
  await session.send("Tracing.end");
  const { stream } = await completed;
  if (!stream) throw new Error("Chromium tracing completed without a stream handle");
  return readTraceStream(session, stream);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const playwright = loadPlaywright();
  const { chromium } = playwright.api;
  const journeyUrl = pathToFileURL(path.resolve(args.get("--journey"))).href;
  const journey = await import(journeyUrl);
  if (typeof journey.run !== "function") {
    throw new Error("Playwright journey module must export async function run({ page })");
  }

  const browser = await chromium.launch({ headless: true });
  let session;
  let traceStarted = false;
  try {
    const context = await browser.newContext({ viewport: { width: 1280, height: 720 } });
    const page = await context.newPage();
    session = await context.newCDPSession(page);
    await session.send("Tracing.start", {
      categories: TRACE_CATEGORIES,
      transferMode: "ReturnAsStream",
    });
    traceStarted = true;

    await journey.run({ page });

    const trace = await stopTrace(session);
    traceStarted = false;
    await writeFile(args.get("--trace"), trace);
    await writeFile(
      args.get("--metadata"),
      JSON.stringify(
        {
          schema_version: "runtime-profiler/browser-runtime/v1",
          adapter_version: ADAPTER_VERSION,
          node_version: process.version,
          playwright_version: playwright.version,
          browser_name: "chromium",
          browser_version: browser.version(),
          viewport: { width: 1280, height: 720 },
          trace_categories: TRACE_CATEGORIES.split(","),
        },
        null,
        2,
      ),
    );
  } finally {
    if (traceStarted && session) {
      try {
        await stopTrace(session);
      } catch {
        // Capture is already failing; do not replace the original error.
      }
    }
    await browser.close();
  }
}

await main();
