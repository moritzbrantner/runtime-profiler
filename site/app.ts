import { validateBundleUrl } from "./validator.mjs";

function mustElement<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`Missing required element: ${selector}`);
  return element;
}

const form = mustElement<HTMLFormElement>("form");
const input = mustElement<HTMLInputElement>("#manifest");
const status = mustElement<HTMLElement>("#status");
const result = mustElement<HTMLElement>("#result");
const output = mustElement<HTMLElement>("#output");
const summary = mustElement<HTMLElement>("#summary");
const machineLink = mustElement<HTMLAnchorElement>("#machine-link");
let latestRun = 0;

const initial = new URL(location.href).searchParams.get("manifest");
if (initial) {
  input.value = initial;
  void run(initial);
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  void run(input.value);
});

async function run(manifestUrl: string): Promise<void> {
  const runId = ++latestRun;
  status.textContent = "Loading and verifying public evidence…";
  status.dataset.state = "normal";
  result.hidden = true;
  try {
    const report = await validateBundleUrl(manifestUrl);
    if (runId !== latestRun) return;
    const json = `${JSON.stringify(report, null, 2)}\n`;
    output.textContent = json;
    summary.replaceChildren(
      metric("Valid", report.valid ? "yes" : "no"),
      metric("Verified files", report.verified_files),
      metric("Metrics", report.summary.metric_count),
      metric("Samples", report.summary.sample_count),
    );
    machineLink.href = `./validate.json/?manifest=${encodeURIComponent(manifestUrl)}`;
    result.hidden = false;
    status.textContent = report.valid
      ? "Bundle satisfies the browser-verifiable v1 contract."
      : `Bundle has ${report.diagnostics.length} validation diagnostic(s).`;
    history.replaceState(null, "", `?manifest=${encodeURIComponent(manifestUrl)}`);
  } catch (error: unknown) {
    if (runId !== latestRun) return;
    status.dataset.state = "error";
    status.textContent = error instanceof Error ? error.message : String(error);
  }
}

function metric(label: string, value: unknown): HTMLDivElement {
  const element = document.createElement("div");
  const strong = document.createElement("strong");
  strong.textContent = String(value);
  const span = document.createElement("span");
  span.textContent = label;
  element.append(strong, span);
  return element;
}
