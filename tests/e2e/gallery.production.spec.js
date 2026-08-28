import { expect, test } from "@playwright/test";

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

  await page.goto("/?chapter=26");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  const chapter = page.frameLocator("#chapter-frame");
  await expect(chapter.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(chapter.locator("#framebuffer")).toBeVisible();
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
