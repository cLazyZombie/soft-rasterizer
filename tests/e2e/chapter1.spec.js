import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const EXECUTION_MODE = process.env.SOFT_RASTERIZER_E2E_MODE ?? "unspecified";

async function installContextAudit(page) {
  await page.addInitScript(() => {
    const requested = [];
    const originalGetContext = HTMLCanvasElement.prototype.getContext;
    Object.defineProperty(window, "__requestedCanvasContexts", {
      value: requested,
      configurable: false,
    });
    HTMLCanvasElement.prototype.getContext = function auditedGetContext(kind, ...args) {
      requested.push(String(kind));
      return originalGetContext.call(this, kind, ...args);
    };
  });
}

async function openReadyPage(page) {
  await installContextAudit(page);
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

function observeBrowserLog(page) {
  const entries = [];
  const errors = [];
  page.on("console", (message) => {
    entries.push({ type: message.type(), text: message.text() });
    if (message.type() === "error") {
      errors.push(`console: ${message.text()}`);
    }
  });
  page.on("pageerror", (error) => {
    entries.push({ type: "pageerror", text: error.message });
    errors.push(`pageerror: ${error.message}`);
  });
  return { entries, errors };
}

function recordEvidence(testInfo, snapshot, fixedDtSeconds, browserLog, screenshotPath, extra = {}) {
  testInfo.annotations.push({
    type: "evidence",
    description: JSON.stringify({
      executionMode: EXECUTION_MODE,
      seed: 0,
      fixedDtSeconds,
      internalSize: snapshot.internalSize,
      cssSize: snapshot.cssSize,
      deviceScaleFactor: snapshot.deviceScaleFactor,
      frameStats: snapshot.stats,
      pixelHash: snapshot.pixelHash,
      framebufferGeneration: snapshot.framebufferGeneration,
      typedArrayViewRebuilds: snapshot.typedArrayViewRebuilds,
      lastFrameMetrics: snapshot.lastFrameMetrics,
      screenshotPath,
      diffPath: null,
      consoleLog: browserLog.entries,
      ...extra,
    }),
  });
}

test("@smoke smoke_boot: Wasm RGBA8가 Canvas 2D에 표시된다", async ({ page }, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "smoke_boot" },
    { type: "steps", description: "7" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  const initial = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(initial.internalSize).toEqual([960, 540]);
  expect(initial.internalSize).toEqual(initial.cssSize);
  expect(initial.framebufferLength).toBe(960 * 540 * 4);
  expect(initial.framebufferGeneration).toBe(0);
  expect(initial.typedArrayViewRebuilds).toBe(1);
  expect(initial.updateAndRenderCalls).toBe(1);
  expect(initial.contextKind).toBe("2d");
  expect(initial.stats).toMatchObject({
    frameIndex: 1,
    inputBits: 0,
    inputVertices: 8,
    inputTriangles: 0,
    clippedTriangles: 0,
    rasterizedTriangles: 0,
    shadedSamples: 0,
    invalidValues: 0,
  });
  expect(initial.stats.debugPixels).toBeGreaterThan(0);

  const afterAdvance = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.1));
  expect(afterAdvance.stats.frameIndex).toBe(2);
  expect(afterAdvance.stats.dtSeconds).toBeCloseTo(0.1, 6);
  expect(afterAdvance.pixelHash).not.toBe(initial.pixelHash);
  expect(afterAdvance.typedArrayViewRebuilds).toBe(1);

  const pixelSummary = await page.locator("#framebuffer").evaluate((canvas) => {
    const context = canvas.getContext("2d");
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const first = Array.from(pixels.slice(0, 4));
    let differingPixels = 0;
    let nonOpaquePixels = 0;
    for (let index = 0; index < pixels.length; index += 4) {
      if (
        pixels[index] !== first[0] ||
        pixels[index + 1] !== first[1] ||
        pixels[index + 2] !== first[2]
      ) {
        differingPixels += 1;
      }
      if (pixels[index + 3] !== 255) {
        nonOpaquePixels += 1;
      }
    }
    return { first, differingPixels, nonOpaquePixels };
  });
  expect(pixelSummary.first).toEqual([0, 0, 220, 255]);
  expect(pixelSummary.differingPixels).toBeGreaterThan(0);
  expect(pixelSummary.nonOpaquePixels).toBe(0);

  const requestedContexts = await page.evaluate(() => window.__requestedCanvasContexts);
  expect(requestedContexts).toEqual(["2d", "2d"]);
  expect(requestedContexts).not.toContain("webgl");
  expect(requestedContexts).not.toContain("webgl2");
  expect(requestedContexts).not.toContain("webgpu");
  await expect(page.locator("#present-path")).toHaveText("Rust/Wasm RGBA8 → Canvas 2D");
  await expect(page.locator("#display-scale")).toContainText(`${initial.deviceScaleFactor}×`);
  await expect(page.locator(".title img")).toHaveJSProperty("complete", true);

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter1-canvas.png`,
  );
  await page.locator("#framebuffer").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter1-canvas", { path: screenshotPath, contentType: "image/png" });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, afterAdvance, 0.1, browserLog, screenshotPath, {
    requestedCanvasContexts: requestedContexts,
  });
});

test("framebuffer_pattern: RGBA gradient와 8x8 checker가 정확하다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "framebuffer_pattern" },
    { type: "steps", description: "8" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  const snapshot = await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(false);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(snapshot.stats.debugPixels).toBe(0);
  expect(snapshot.framebufferMiB).toBeCloseTo((960 * 540 * 4) / (1024 * 1024), 10);
  const pixels = await page.locator("#framebuffer").evaluate((canvas) => {
    const context = canvas.getContext("2d");
    const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const at = (x, y) => Array.from(data.slice(4 * (y * canvas.width + x), 4 * (y * canvas.width + x) + 4));
    let nonOpaquePixels = 0;
    for (let index = 3; index < data.length; index += 4) {
      nonOpaquePixels += Number(data[index] !== 255);
    }
    return {
      topLeft: at(0, 0),
      topRight: at(canvas.width - 1, 0),
      bottomLeft: at(0, canvas.height - 1),
      bottomRight: at(canvas.width - 1, canvas.height - 1),
      tile00: at(7, 7),
      tile10: at(8, 7),
      tile01: at(7, 8),
      tile11: at(8, 8),
      nonOpaquePixels,
    };
  });
  expect(pixels).toEqual({
    topLeft: [0, 0, 220, 255],
    topRight: [255, 0, 40, 255],
    bottomLeft: [0, 255, 40, 255],
    bottomRight: [255, 255, 220, 255],
    tile00: [2, 3, 220, 255],
    tile10: [2, 3, 40, 255],
    tile01: [2, 4, 40, 255],
    tile11: [2, 4, 220, 255],
    nonOpaquePixels: 0,
  });
  await expect(page.locator("#framebuffer-mib")).toHaveText("1.98 MiB");

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter3-pattern.png`,
  );
  await page.locator("#framebuffer").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter3-pattern", { path: screenshotPath, contentType: "image/png" });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, snapshot, 0, browserLog, screenshotPath);
});

test("coordinate_spaces: 회전 큐브의 Object/World/View/Clip 단계와 진단을 표시한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "coordinate_spaces" },
    { type: "steps", description: "12" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  const initial = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(initial.stats.debugPixels).toBeGreaterThan(0);
  expect(initial.stats.inputVertices).toBe(8);
  expect(initial.pixelHash).toBe("1a480e35");
  const stageColorCounts = await page.locator("#framebuffer").evaluate((canvas) => {
    const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    const expected = [
      [238, 244, 255, 255],
      [255, 210, 72, 255],
      [72, 224, 194, 255],
      [184, 132, 255, 255],
    ];
    const counts = Array(expected.length).fill(0);
    for (let index = 0; index < data.length; index += 4) {
      expected.forEach((color, colorIndex) => {
        if (color.every((channel, offset) => data[index + offset] === channel)) {
          counts[colorIndex] += 1;
        }
      });
    }
    return counts;
  });
  expect(stageColorCounts.every((count) => count > 0)).toBe(true);
  await expect(page.locator("#coordinate-debug")).toContainText("선택 정점 v6");
  await expect(page.locator("#coordinate-debug")).toContainText("invalid values: 0");
  const disabled = await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(false);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(disabled.stats.debugPixels).toBe(0);
  expect(disabled.pixelHash).not.toBe(initial.pixelHash);
  const restored = await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(true);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(restored.stats.debugPixels).toBe(initial.stats.debugPixels);
  expect(restored.pixelHash).toBe(initial.pixelHash);
  const rotated = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.1));
  expect(rotated.pixelHash).not.toBe(initial.pixelHash);
  await expect(page.locator("#coordinate-debug")).toContainText("model Y 0.075 rad");

  const invalid = await page.evaluate(() => {
    window.__softRasterizer.setModelRotationY(Number.NaN);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(invalid.stats.invalidValues).toBe(24);
  await expect(page.locator("#coordinate-debug")).toContainText("첫 공간: World");
  const recovered = await page.evaluate(() => {
    window.__softRasterizer.setModelRotationY(0);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(recovered.stats.invalidValues).toBe(0);
  expect(recovered.pixelHash).toBe(initial.pixelHash);
  await expect(page.locator("#line-algorithm")).toHaveText("All-octants Bresenham (Rust)");
  await expect(page.locator("#math-convention")).toHaveText("열벡터 · LH · +Z 전방");

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter6-coordinate-spaces.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter6-coordinate-spaces", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, recovered, 0, browserLog, screenshotPath, { stageColorCounts });
});

test("wasm_boundary: 프레임 호출과 단계 시간이 해상도에 비례하지 않는다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "wasm_boundary" },
    { type: "steps", description: "8" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  const before = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(before.lastFrameMetrics).toMatchObject({
    highLevelRenderCalls: 1,
    wasmBoundaryCalls: 6,
  });
  for (const value of [
    before.lastFrameMetrics.inputMs,
    before.lastFrameMetrics.updateMs,
    before.lastFrameMetrics.presentMs,
    before.lastFrameMetrics.totalMs,
  ]) {
    expect(Number.isFinite(value)).toBe(true);
    expect(value).toBeGreaterThanOrEqual(0);
  }

  await page.setViewportSize({ width: 700, height: 600 });
  await expect
    .poll(async () => page.evaluate(() => window.__softRasterizer.snapshot()))
    .toMatchObject({ internalSize: [668, 376], resizeEvents: 1 });
  const after = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.05));
  expect(after.lastFrameMetrics).toMatchObject({
    highLevelRenderCalls: 1,
    wasmBoundaryCalls: 6,
  });
  expect(after.stats.inputBits).toBe(0);
  await expect(page.locator("#high-level-calls")).toHaveText("1");
  await expect(page.locator("#wasm-boundary-calls")).toHaveText("6");
  await expect(page.locator("#frame-time")).toHaveText(/^\d+\.\d{3} ms$/);
  const constructorError = await page.evaluate(() =>
    window.__softRasterizer.invalidConstructorError(),
  );
  expect(constructorError).toContain("0보다");

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter2-boundary.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter2-boundary", { path: screenshotPath, contentType: "image/png" });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, after, 0.05, browserLog, screenshotPath, { constructorError });
});

test("resize_memory_view: CSS 논리 해상도로 resize하고 Wasm view를 재생성한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "resize_memory_view" },
    { type: "steps", description: "8" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  const before = await page.evaluate(() => window.__softRasterizer.snapshot());
  const memoryGrowth = await page.evaluate(() => window.__softRasterizer.growMemory(1));
  expect(memoryGrowth.bufferChanged).toBe(true);
  expect(memoryGrowth.currentPages).toBe(memoryGrowth.previousPages + 1);
  const afterMemoryGrowth = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.05));
  expect(afterMemoryGrowth.internalSize).toEqual(before.internalSize);
  expect(afterMemoryGrowth.framebufferGeneration).toBe(0);
  expect(afterMemoryGrowth.typedArrayViewRebuilds).toBe(2);

  await page.setViewportSize({ width: 700, height: 600 });
  await expect
    .poll(async () => page.evaluate(() => window.__softRasterizer.snapshot()))
    .toMatchObject({
      internalSize: [668, 376],
      cssSize: [668, 376],
      framebufferGeneration: 1,
      resizeEvents: 1,
    });

  const after = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.05));
  expect(after.deviceScaleFactor).toBe(before.deviceScaleFactor);
  expect(after.internalSize).toEqual(after.cssSize);
  expect(after.framebufferLength).toBe(668 * 376 * 4);
  expect(after.typedArrayViewRebuilds).toBe(3);
  expect(after.pixelHash).not.toBe(before.pixelHash);
  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter1-resize.png`,
  );
  await page.locator("#framebuffer").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter1-resize", { path: screenshotPath, contentType: "image/png" });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, after, 0.05, browserLog, screenshotPath, { memoryGrowth });
});
