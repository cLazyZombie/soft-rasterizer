import { expect, test } from "@playwright/test";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

function observeErrors(page) {
  const errors = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("requestfailed", (request) => {
    errors.push(`requestfailed: ${request.url()} · ${request.failure()?.errorText ?? "unknown"}`);
  });
  return errors;
}

test("chapter_production_boot: production launcher와 standalone Canvas가 표시된다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "chapter_production_boot" },
    { type: "steps", description: "8" },
  );
  const errors = observeErrors(page);
  await page.addInitScript(() => {
    const requested = [];
    const originalGetContext = HTMLCanvasElement.prototype.getContext;
    Object.defineProperty(window, "__chapterRequestedContexts", { value: requested });
    HTMLCanvasElement.prototype.getContext = function auditedGetContext(kind, ...args) {
      requested.push(String(kind));
      return originalGetContext.call(this, kind, ...args);
    };
  });

  await page.goto("./?chapter=26");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  const chapter = page.frameLocator("#chapter-frame");
  await expect(chapter.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(chapter.locator("html")).toHaveAttribute("data-chapter-ui-scope", "26");
  await expect(chapter.locator("#framebuffer")).toBeVisible();
  await expect(chapter.locator('label[for="animation-clip"]')).toBeVisible();
  await expect(chapter.locator('label[for="cull-mode"]')).toBeHidden();
  await chapter.locator("html").evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );

  const canvasEvidence = await chapter.locator("#framebuffer").evaluate((canvas) => {
    const context = canvas.getContext("2d");
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const firstRgb = `${pixels[0]},${pixels[1]},${pixels[2]}`;
    let hasDifferentRgb = false;
    let opaquePixels = 0;
    for (let index = 0; index < pixels.length; index += 4) {
      if (pixels[index + 3] === 255) opaquePixels += 1;
      if (`${pixels[index]},${pixels[index + 1]},${pixels[index + 2]}` !== firstRgb) {
        hasDifferentRgb = true;
      }
    }
    return {
      width: canvas.width,
      height: canvas.height,
      hasDifferentRgb,
      opaquePixels,
      requestedContexts: window.__chapterRequestedContexts,
    };
  });

  expect(canvasEvidence.width).toBeGreaterThan(0);
  expect(canvasEvidence.height).toBeGreaterThan(0);
  expect(canvasEvidence.opaquePixels).toBe(canvasEvidence.width * canvasEvidence.height);
  expect(canvasEvidence.hasDifferentRgb).toBe(true);
  expect(canvasEvidence.requestedContexts).toContain("2d");
  expect(canvasEvidence.requestedContexts).not.toContain("webgl");
  expect(canvasEvidence.requestedContexts).not.toContain("webgl2");
  expect(canvasEvidence.requestedContexts).not.toContain("webgpu");
  expect(errors).toEqual([]);

  testInfo.annotations.push({
    type: "evidence",
    description: JSON.stringify({
      chapter: "26",
      ...canvasEvidence,
      consoleErrors: errors,
    }),
  });
});

test("chapter_documentation: 교재 원본, 장 전환과 project subpath를 검증한다", async ({
  page,
  request,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "chapter_documentation" },
    { type: "steps", description: "39" },
  );
  const errors = observeErrors(page);
  const response = await request.get("./chapter-docs.json");
  expect(response.ok()).toBe(true);
  const docs = await response.json();
  expect(docs.chapters).toHaveLength(26);
  for (const chapter of docs.chapters) {
    const source = readFileSync(chapter.source, "utf8");
    expect(chapter.sourceSha256).toBe(createHash("sha256").update(source).digest("hex"));
    const html = await request.get(chapter.href);
    expect(html.ok()).toBe(true);
    expect(await html.text()).toContain("<article>");
  }

  await page.goto("./?chapter=14&view=reading");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#reading-view")).toHaveAttribute("aria-pressed", "true");
  const reader = page.frameLocator("#document-frame");
  await expect(reader.locator("h1")).toHaveText(docs.chapters.find((entry) => entry.number === "14").title);
  await expect(reader.locator("pre").first()).toBeVisible();
  const screenshotPath = testInfo.outputPath("chapter-14-reading.png");
  await page.screenshot({ path: screenshotPath });
  const perspectiveDiagram = reader.locator("article img");
  await perspectiveDiagram.evaluate((image) => image.decode());
  const diagramScreenshotPath = testInfo.outputPath("perspective-midpoints.png");
  await perspectiveDiagram.screenshot({ path: diagramScreenshotPath });
  await page.locator("#result-view").click();
  await expect(page.frameLocator("#chapter-frame").locator("#framebuffer")).toBeVisible();
  await page.locator("#reading-view").click();
  await page.locator("#chapter-select").selectOption("06");
  await expect(reader.locator("h1")).toHaveText(docs.chapters.find((entry) => entry.number === "06").title);
  const diagram = reader.locator("article img");
  await expect(diagram).toBeVisible();
  await diagram.evaluate((image) => image.decode());
  expect(await diagram.evaluate((image) => image.naturalWidth)).toBeGreaterThan(0);

  await page.locator("#chapter-select").selectOption("24");
  await expect(reader.locator("h1")).toHaveText(docs.chapters.find((entry) => entry.number === "24").title);
  await page.goBack();
  await expect(page.locator("#chapter-select")).toHaveValue("06");
  await expect(reader.locator("h1")).toHaveText(docs.chapters.find((entry) => entry.number === "06").title);
  await expect(page.locator("#chapter-frame")).toHaveAttribute("src", "./chapters/06/");
  await page.goForward();
  await expect(page.locator("#chapter-select")).toHaveValue("24");
  await expect(reader.locator("h1")).toHaveText(docs.chapters.find((entry) => entry.number === "24").title);
  await expect(page.locator("#chapter-frame")).toHaveAttribute("src", "./chapters/24/");
  await reader.getByRole("link", { name: "진단과 성능 측정 기준선" }).click();
  await expect(reader.locator("h1")).toHaveText("진단과 성능 측정 기준선");
  await expect(reader.locator('link[rel="stylesheet"]')).toHaveAttribute("href", "../../docs.css");
  // iframe 내부 history 이동은 main-frame load를 기다리지 않고 문서 ready를 확인한다.
  await page.evaluate(() => window.history.back());
  await expect(reader.locator("h1")).toHaveText(docs.chapters.find((entry) => entry.number === "24").title);

  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  const mobileScreenshotPath = testInfo.outputPath("chapter-24-reading-mobile.png");
  await page.screenshot({ path: mobileScreenshotPath });
  const chapter14 = docs.chapters.find((entry) => entry.number === "14");
  await page.goto(chapter14.href);
  await expect(page.locator("h1")).toHaveText(chapter14.title);
  const icon = await page.locator('link[rel="icon"]').getAttribute("href");
  expect((await request.get(new URL(icon, page.url()).href)).ok()).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  expect(errors).toEqual([]);
  testInfo.annotations.push({
    type: "evidence",
    description: JSON.stringify({ chapters: docs.chapters.length, screenshotPath, diagramScreenshotPath, mobileScreenshotPath, consoleErrors: errors }),
  });
});
