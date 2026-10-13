import { validateBundleUrl } from "../validator.mjs";

const output = document.querySelector<HTMLElement>("#output");
if (!output) throw new Error("Missing required element: #output");

const manifestUrl = new URL(location.href).searchParams.get("manifest");

if (!manifestUrl) {
  output.textContent = `${JSON.stringify({
    schema_version: "runtime-profiler/pages-error/v1",
    status: "error",
    message: "Missing ?manifest=<public manifest.json URL>",
  }, null, 2)}\n`;
} else {
  try {
    const report = await validateBundleUrl(manifestUrl);
    output.textContent = `${JSON.stringify(report, null, 2)}\n`;
  } catch (error: unknown) {
    output.textContent = `${JSON.stringify({
      schema_version: "runtime-profiler/pages-error/v1",
      status: "error",
      message: error instanceof Error ? error.message : String(error),
    }, null, 2)}\n`;
  }
}
