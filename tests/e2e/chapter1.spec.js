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

async function openReadyPage(page, initialControls = null) {
  await installContextAudit(page);
  if (initialControls !== null) {
    await page.route(
      "**/",
      async (route) => {
        const response = await route.fetch();
        const html = await response.text();
        const initialControlScript = `<script>
          document.querySelector("#cull-mode").value = ${JSON.stringify(String(initialControls.cullMode))};
          document.querySelector("#winding-debug").checked = ${JSON.stringify(initialControls.windingDebugMode === 1)};
          document.querySelector("#barycentric-debug").checked = ${JSON.stringify(initialControls.windingDebugMode === 2)};
          document.querySelector("#clip-debug").checked = ${JSON.stringify(initialControls.clipDebugEnabled ?? false)};
          document.querySelector("#coverage-debug").checked = ${JSON.stringify(initialControls.coverageDebugEnabled ?? false)};
          document.querySelector("#interpolation-debug").checked = ${JSON.stringify(initialControls.interpolationDebugEnabled ?? false)};
          document.querySelector("#depth-debug").checked = ${JSON.stringify(initialControls.depthDebugEnabled ?? false)};
          document.querySelector("#depth-order-reversed").checked = ${JSON.stringify(initialControls.depthOrderReversed ?? false)};
          document.querySelector("#depth-debug-mode").value = ${JSON.stringify(String(initialControls.depthDebugMode ?? 0))};
        </script>`;
        await route.fulfill({
          response,
          body: html.replace("</body>", `${initialControlScript}</body>`),
        });
      },
      { times: 1 },
    );
  }
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
    inputVertices: 24,
    inputTriangles: 12,
    transformedVertices: 24,
    submittedTriangles: 4,
    culledTriangles: 8,
    degenerateTriangles: 0,
    invalidTriangles: 0,
    fullyClippedTriangles: 0,
    clipInvalidTriangles: 0,
    generatedTriangles: 12,
    maxClipPolygonVertices: 3,
    rasterizedTriangles: 4,
    shadedSamples: 22958,
    invalidValues: 0,
  });
  expect(initial.stats.maxBarycentricSumError).toBeLessThanOrEqual(2 * Math.fround(2 ** -23));
  expect(initial.stats.depthPassedSamples).toBe(initial.stats.shadedSamples);
  expect(initial.stats.depthFailedSamples).toBe(0);
  expect(initial.stats.invalidDepthSamples).toBe(0);
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

test("indexed_mesh: 24정점/36인덱스 큐브를 단색 coverage와 wireframe으로 표시한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "indexed_mesh" },
    { type: "steps", description: "17" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  const initial = await page.evaluate(() => {
    window.__softRasterizer.setCullMode(0);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(initial.stats.debugPixels).toBeGreaterThan(0);
  expect(initial.stats).toMatchObject({
    inputVertices: 24,
    inputTriangles: 12,
    transformedVertices: 24,
    submittedTriangles: 12,
    culledTriangles: 0,
    degenerateTriangles: 0,
    invalidTriangles: 0,
    rasterizedTriangles: 12,
    shadedSamples: 27680,
  });
  expect(initial.pixelHash).toBe("03b51a08");
  const projectionColorCounts = await page.locator("#framebuffer").evaluate((canvas) => {
    const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    const expected = [
      [238, 244, 255, 255],
      [255, 191, 64, 255],
      [255, 115, 191, 255],
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
  expect(projectionColorCounts.every((count) => count > 0)).toBe(true);
  await expect(page.locator("#coordinate-debug")).toContainText(
    "LH/+Z 카메라 · fov 60.0° · near 0.100 · far 100.0",
  );
  await expect(page.locator("#coordinate-debug")).toContainText("선택 정점 v6");
  await expect(page.locator("#coordinate-debug")).toContainText(
    "X-ray overlay · culling/depth 무관",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "indexed cube mesh · vertices 24 · indices 36 · triangles 12 · material 0",
  );
  await expect(page.locator("#coordinate-debug")).toContainText("normal (");
  await expect(page.locator("#coordinate-debug")).toContainText("UV (");
  await expect(page.locator("#coordinate-debug")).toContainText("w_clip");
  await expect(page.locator("#coordinate-debug")).toContainText("NDC (");
  await expect(page.locator("#coordinate-debug")).toContainText("Screen (");
  await expect(page.locator("#coordinate-debug")).toContainText("projection failures: 0");
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
  expect(invalid.stats.invalidValues).toBe(72);
  expect(invalid.stats.invalidTriangles).toBe(0);
  expect(invalid.stats.clipInvalidTriangles).toBe(12);
  expect(invalid.stats.submittedTriangles).toBe(0);
  await expect(page.locator("#coordinate-debug")).toContainText("NDC invalid · Screen invalid");
  await expect(page.locator("#coordinate-debug")).toContainText("projection failures: 24");
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
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter8-indexed-mesh.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter8-indexed-mesh", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, recovered, 0, browserLog, screenshotPath, { projectionColorCounts });
});

test("winding_culling: screen-space 면 방향과 culling/debug 모드를 전환한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "winding_culling" },
    { type: "steps", description: "18" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, { cullMode: 0, windingDebugMode: 1 });

  const restoredControls = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(restoredControls).toMatchObject({ cullMode: 0, windingDebugMode: 1 });
  expect(restoredControls.stats).toMatchObject({ submittedTriangles: 12, culledTriangles: 0 });
  await expect(page.locator("#coordinate-debug")).toContainText(
    "cull none · debug front green / back red",
  );

  await page.locator("#cull-mode").selectOption("1");
  await page.locator("#winding-debug").uncheck();

  const initial = await page.evaluate(() => {
    const canvas = document.querySelector("#framebuffer");
    const context = canvas.getContext("2d");
    window.__chapterNineBaseline = context.getImageData(0, 0, canvas.width, canvas.height).data.slice();
    return window.__softRasterizer.snapshot();
  });
  expect(initial.cullMode).toBe(1);
  expect(initial.windingDebugMode).toBe(0);
  expect(initial.stats).toMatchObject({
    inputTriangles: 12,
    submittedTriangles: 4,
    culledTriangles: 8,
    degenerateTriangles: 0,
    invalidTriangles: 0,
    rasterizedTriangles: 4,
    shadedSamples: 22958,
  });
  expect(initial.pixelHash).toBe("2f43ea13");
  await expect(page.locator("#coordinate-debug")).toContainText(
    "winding screen y-down orient2d > 0 front · cull back · debug vertex color",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "triangle stats input 12 · submitted 4 · culled 8 · degenerate 0 · invalid 0",
  );
  await expect(page.locator(".space-legend")).toContainText(
    "clip → fan → divide/viewport → edge/top-left → barycentric → affine z_ndc → strict depth < → color · 선택 정점 흰색(X-ray)",
  );

  await page.locator("#cull-mode").selectOption("0");
  const doubleSided = await page.evaluate(() => {
    const canvas = document.querySelector("#framebuffer");
    const current = canvas
      .getContext("2d")
      .getImageData(0, 0, canvas.width, canvas.height).data;
    const baseline = window.__chapterNineBaseline;
    let differingPixels = 0;
    let maxChannelDifference = 0;
    for (let index = 0; index < current.length; index += 4) {
      let differs = false;
      for (let channel = 0; channel < 4; channel += 1) {
        const difference = Math.abs(current[index + channel] - baseline[index + channel]);
        maxChannelDifference = Math.max(maxChannelDifference, difference);
        differs ||= difference !== 0;
      }
      differingPixels += Number(differs);
    }
    return {
      snapshot: window.__softRasterizer.snapshot(),
      differingPixels,
      maxChannelDifference,
    };
  });
  expect(doubleSided.snapshot.stats).toMatchObject({
    submittedTriangles: 12,
    culledTriangles: 0,
    degenerateTriangles: 0,
    invalidTriangles: 0,
    rasterizedTriangles: 12,
    shadedSamples: 27680,
  });
  expect(doubleSided.snapshot.pixelHash).toBe("03b51a08");
  expect(doubleSided.differingPixels).toBe(1283);
  expect(doubleSided.maxChannelDifference).toBe(215);

  await page.locator("#winding-debug").check();
  const facing = await page.evaluate(() => {
    const canvas = document.querySelector("#framebuffer");
    const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    const colors = [
      [72, 232, 112, 255],
      [255, 82, 92, 255],
    ];
    const counts = [0, 0];
    for (let index = 0; index < data.length; index += 4) {
      colors.forEach((color, colorIndex) => {
        if (color.every((channel, offset) => data[index + offset] === channel)) {
          counts[colorIndex] += 1;
        }
      });
    }
    return { snapshot: window.__softRasterizer.snapshot(), facingColorCounts: counts };
  });
  expect(facing.snapshot.stats).toMatchObject({
    submittedTriangles: 12,
    culledTriangles: 0,
    rasterizedTriangles: 12,
    shadedSamples: 27680,
  });
  expect(facing.facingColorCounts).toEqual([21920, 1280]);

  await page.locator("#cull-mode").selectOption("2");
  const frontCulled = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(frontCulled.stats).toMatchObject({ submittedTriangles: 8, culledTriangles: 4 });

  await page.locator("#cull-mode").selectOption("1");
  await page.locator("#winding-debug").uncheck();
  const restored = await page.evaluate(() => {
    delete window.__chapterNineBaseline;
    return window.__softRasterizer.snapshot();
  });
  expect(restored.pixelHash).toBe(initial.pixelHash);
  expect(restored.stats).toMatchObject({ submittedTriangles: 4, culledTriangles: 8 });

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter9-winding-culling.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter9-winding-culling", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, restored, 0, browserLog, screenshotPath, {
    doubleSidedDiff: {
      differingPixels: doubleSided.differingPixels,
      maxChannelDifference: doubleSided.maxChannelDifference,
    },
    facingColorCounts: facing.facingColorCounts,
  });
});

test("triangle_pipeline: homogeneous clipping을 divide 전에 적용한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "triangle_pipeline" },
    { type: "steps", description: "14" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, {
    cullMode: 0,
    windingDebugMode: 0,
    clipDebugEnabled: true,
  });

  const clipped = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(clipped).toMatchObject({
    cullMode: 0,
    windingDebugMode: 0,
    clipDebugEnabled: true,
  });
  expect(clipped.stats).toMatchObject({
    inputVertices: 3,
    inputTriangles: 1,
    transformedVertices: 3,
    submittedTriangles: 3,
    culledTriangles: 0,
    degenerateTriangles: 0,
    invalidTriangles: 0,
    fullyClippedTriangles: 0,
    clipInvalidTriangles: 0,
    generatedTriangles: 3,
    maxClipPolygonVertices: 5,
    rasterizedTriangles: 3,
    shadedSamples: 87042,
    invalidValues: 0,
  });
  expect(clipped.stats.debugPixels).toBeGreaterThan(0);
  expect(clipped.pixelHash).toBe("67de920e");
  await expect(page.locator("#coordinate-debug")).toContainText(
    "동차 clip fixture · identity M/V/P vertex stage · viewport aspect 1.778",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "clip debug mesh · vertices 3 · indices 3 · triangles 1 · material 0 · near/left/top 교차",
  );
  await expect(page.locator("#coordinate-debug")).toContainText("선택 정점 v2");
  await expect(page.locator("#coordinate-debug")).toContainText(
    "Object (-0.250, -0.250, 0.500, 1.000)",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "Clip   (-0.250, -0.250, 0.500, 1.000)",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "Screen (360.0, 337.5, z=0.500)",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "clip stats fully clipped 0 · clip invalid 0 · generated 3 · max polygon vertices 5",
  );

  await page.locator("#clip-debug").uncheck();
  const cube = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(cube.clipDebugEnabled).toBe(false);
  expect(cube.stats).toMatchObject({
    inputVertices: 24,
    inputTriangles: 12,
    generatedTriangles: 12,
    maxClipPolygonVertices: 3,
  });
  expect(cube.pixelHash).toBe("03b51a08");
  expect(cube.pixelHash).not.toBe(clipped.pixelHash);

  await page.locator("#clip-debug").check();
  const restored = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(restored.clipDebugEnabled).toBe(true);
  expect(restored.pixelHash).toBe(clipped.pixelHash);
  expect(restored.stats).toEqual({
    ...clipped.stats,
    frameIndex: clipped.stats.frameIndex + 2,
  });

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter10-homogeneous-clipping.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter10-homogeneous-clipping", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, restored, 0, browserLog, screenshotPath, {
    clippedPolygonVertices: restored.stats.maxClipPolygonVertices,
    generatedTriangles: restored.stats.generatedTriangles,
  });
});

test("triangle_pipeline: fixed-point top-left quad가 각 sample을 한 번만 소유한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "triangle_pipeline" },
    { type: "steps", description: "16" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, {
    cullMode: 1,
    windingDebugMode: 0,
    clipDebugEnabled: false,
    coverageDebugEnabled: true,
  });
  const covered = await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(false);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(covered).toMatchObject({
    clipDebugEnabled: false,
    coverageDebugEnabled: true,
  });
  expect(covered.stats).toMatchObject({
    inputVertices: 6,
    inputTriangles: 2,
    transformedVertices: 6,
    submittedTriangles: 2,
    culledTriangles: 0,
    degenerateTriangles: 0,
    invalidTriangles: 0,
    fullyClippedTriangles: 0,
    clipInvalidTriangles: 0,
    generatedTriangles: 2,
    maxClipPolygonVertices: 3,
    rasterizedTriangles: 2,
    debugPixels: 0,
    invalidValues: 0,
  });
  const expectedSamples = (covered.internalSize[0] / 2) * (covered.internalSize[1] / 2);
  expect(covered.stats.shadedSamples).toBe(expectedSamples);

  const coveragePixels = await page.locator("#framebuffer").evaluate((canvas) => {
    const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    const colors = [
      [255, 89, 38, 255],
      [38, 191, 255, 255],
    ];
    const counts = [0, 0];
    let coloredOutsideQuad = 0;
    const minX = canvas.width / 4;
    const maxX = (canvas.width * 3) / 4;
    const minY = canvas.height / 4;
    const maxY = (canvas.height * 3) / 4;
    for (let y = 0; y < canvas.height; y += 1) {
      for (let x = 0; x < canvas.width; x += 1) {
        const index = 4 * (y * canvas.width + x);
        const colorIndex = colors.findIndex((color) =>
          color.every((channel, offset) => data[index + offset] === channel),
        );
        if (colorIndex >= 0) {
          counts[colorIndex] += 1;
          coloredOutsideQuad += Number(x < minX || x >= maxX || y < minY || y >= maxY);
        }
      }
    }
    return { counts, coloredOutsideQuad };
  });
  expect(coveragePixels.counts.every((count) => count > 0)).toBe(true);
  expect(coveragePixels.counts[0] + coveragePixels.counts[1]).toBe(expectedSamples);
  expect(coveragePixels.coloredOutsideQuad).toBe(0);
  expect(covered.pixelHash).toBe("5e465ac5");
  await expect(page.locator("#coordinate-debug")).toContainText(
    "top-left coverage fixture · identity M/V/P vertex stage · viewport aspect 1.778",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "coverage quad mesh · vertices 6 · indices 6 · triangles 2 · material 0 · 두 삼각형/공유 대각선",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    `coverage stats rasterized 2 · shaded samples ${expectedSamples} · S=256 pixel center/top-left`,
  );
  await expect(page.locator("#coverage-algorithm")).toHaveText(
    "S=256 incremental edge · pixel center · top-left (Rust)",
  );

  await page.locator("#coverage-debug").uncheck();
  const cube = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(cube).toMatchObject({ clipDebugEnabled: false, coverageDebugEnabled: false });
  expect(cube.stats.inputTriangles).toBe(12);
  await page.locator("#clip-debug").check();
  const clipping = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(clipping).toMatchObject({ clipDebugEnabled: true, coverageDebugEnabled: false });

  await page.locator("#clip-debug").uncheck();
  await page.locator("#coverage-debug").check();
  const restored = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(restored.coverageDebugEnabled).toBe(true);
  expect(restored.pixelHash).toBe(covered.pixelHash);

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter11-top-left-coverage.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter11-top-left-coverage", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, restored, 0, browserLog, screenshotPath, {
    coveragePixels,
    expectedSamples,
  });
});

test("triangle_pipeline: barycentric 좌표로 R/G/B 정점 색을 affine 보간한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "triangle_pipeline" },
    { type: "steps", description: "17" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, {
    cullMode: 1,
    windingDebugMode: 0,
    clipDebugEnabled: false,
    coverageDebugEnabled: false,
    interpolationDebugEnabled: true,
  });
  const affine = await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(false);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(affine).toMatchObject({
    windingDebugMode: 0,
    clipDebugEnabled: false,
    coverageDebugEnabled: false,
    interpolationDebugEnabled: true,
  });
  expect(affine.stats).toMatchObject({
    inputVertices: 3,
    inputTriangles: 1,
    transformedVertices: 3,
    submittedTriangles: 1,
    culledTriangles: 0,
    degenerateTriangles: 0,
    invalidTriangles: 0,
    fullyClippedTriangles: 0,
    clipInvalidTriangles: 0,
    generatedTriangles: 1,
    maxClipPolygonVertices: 3,
    rasterizedTriangles: 1,
    debugPixels: 0,
    invalidValues: 0,
  });
  expect(affine.stats.shadedSamples).toBe(109824);
  expect(affine.stats.maxBarycentricSumError).toBeLessThanOrEqual(2 * Math.fround(2 ** -23));

  const sampleColors = await page.locator("#framebuffer").evaluate((canvas) => {
    const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    const pixel = (x, y) => Array.from(data.slice(4 * (y * canvas.width + x), 4 * (y * canvas.width + x) + 4));
    return {
      nearRed: pixel(180, 105),
      nearGreen: pixel(780, 105),
      nearBlue: pixel(480, 430),
      centroid: pixel(480, 212),
    };
  });
  expect({ sampleColors, pixelHash: affine.pixelHash }).toEqual({
    sampleColors: {
      nearRed: [246, 1, 8, 255],
      nearGreen: [1, 246, 8, 255],
      nearBlue: [5, 6, 244, 255],
      centroid: [84, 85, 86, 255],
    },
    pixelHash: "aabc25f9",
  });
  await expect(page.locator("#coordinate-debug")).toContainText(
    "affine RGB fixture · identity M/V/P vertex stage · viewport aspect 1.778",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "barycentric RGB triangle mesh · vertices 3 · indices 3 · triangles 1 · material 0 · vertex colors R/G/B",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "interpolation stats max |lambda sum - 1|",
  );
  await expect(page.locator("#interpolation-algorithm")).toHaveText(
    "edge / area barycentric · affine color (Rust)",
  );

  await page.locator("#barycentric-debug").check();
  const barycentric = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(barycentric.windingDebugMode).toBe(2);
  expect(barycentric.stats).toMatchObject({
    submittedTriangles: affine.stats.submittedTriangles,
    rasterizedTriangles: affine.stats.rasterizedTriangles,
    shadedSamples: affine.stats.shadedSamples,
  });
  expect(barycentric.pixelHash).toBe(affine.pixelHash);
  await expect(page.locator("#coordinate-debug")).toContainText("debug barycentric RGB");

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter12-barycentric-affine.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter12-barycentric-affine", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, barycentric, 0, browserLog, screenshotPath, { sampleColors });
});

test("triangle_pipeline: strict depth가 제출 순서와 무관한 가려짐과 debug view를 만든다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "triangle_pipeline" },
    { type: "steps", description: "24" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, {
    cullMode: 1,
    windingDebugMode: 0,
    depthDebugEnabled: true,
    depthOrderReversed: false,
    depthDebugMode: 0,
  });
  const nearFirst = await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(false);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(nearFirst).toMatchObject({
    depthDebugEnabled: true,
    depthOrderReversed: false,
    depthDebugMode: 0,
  });
  expect(nearFirst.stats).toMatchObject({
    inputVertices: 6,
    inputTriangles: 2,
    transformedVertices: 6,
    submittedTriangles: 2,
    culledTriangles: 0,
    degenerateTriangles: 0,
    invalidTriangles: 0,
    fullyClippedTriangles: 0,
    clipInvalidTriangles: 0,
    generatedTriangles: 2,
    maxClipPolygonVertices: 3,
    rasterizedTriangles: 2,
    invalidDepthSamples: 0,
    debugPixels: 0,
    invalidValues: 0,
  });

  const farFirst = await page.evaluate(() => {
    window.__softRasterizer.setDepthOrderReversed(true);
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(farFirst.depthOrderReversed).toBe(true);
  expect(farFirst.pixelHash).toBe(nearFirst.pixelHash);
  expect(farFirst.stats.depthFailedSamples).toBe(0);
  expect(farFirst.stats.depthPassedSamples).toBeGreaterThan(
    nearFirst.stats.depthPassedSamples,
  );

  const canvasSamples = async () =>
    page.locator("#framebuffer").evaluate((canvas) => {
      const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
      const pixel = (x, y) =>
        Array.from(data.slice(4 * (y * canvas.width + x), 4 * (y * canvas.width + x) + 4));
      return {
        overlapNear: pixel(450, 211),
        farOnly: pixel(720, 211),
        nearOnly: pixel(225, 169),
        infinityBackground: pixel(0, 0),
      };
    });
  const baseSamples = await canvasSamples();

  const grayscale = await page.evaluate(() => {
    window.__softRasterizer.setDepthDebugMode(1);
    return window.__softRasterizer.advanceFrame(0);
  });
  const grayscaleSamples = await canvasSamples();
  const heatmap = await page.evaluate(() => {
    window.__softRasterizer.setDepthDebugMode(2);
    return window.__softRasterizer.advanceFrame(0);
  });
  const heatmapSamples = await canvasSamples();
  for (const debugSnapshot of [grayscale, heatmap]) {
    expect(debugSnapshot.stats).toMatchObject({
      submittedTriangles: farFirst.stats.submittedTriangles,
      rasterizedTriangles: farFirst.stats.rasterizedTriangles,
      shadedSamples: farFirst.stats.shadedSamples,
      depthPassedSamples: farFirst.stats.depthPassedSamples,
      depthFailedSamples: farFirst.stats.depthFailedSamples,
      invalidDepthSamples: 0,
    });
  }

  expect({
    nearFirstDepth: [
      nearFirst.stats.depthPassedSamples,
      nearFirst.stats.depthFailedSamples,
      nearFirst.stats.shadedSamples,
    ],
    farFirstDepth: [
      farFirst.stats.depthPassedSamples,
      farFirst.stats.depthFailedSamples,
      farFirst.stats.shadedSamples,
    ],
    hashes: [nearFirst.pixelHash, grayscale.pixelHash, heatmap.pixelHash],
    baseSamples,
    grayscaleSamples,
    heatmapSamples,
  }).toEqual({
    nearFirstDepth: [151992, 26736, 151992],
    farFirstDepth: [178728, 0, 178728],
    hashes: ["4dea536c", "b687761d", "0d9a6422"],
    baseSamples: {
      overlapNear: [255, 51, 38, 255],
      farOnly: [38, 89, 255, 255],
      nearOnly: [255, 51, 38, 255],
      infinityBackground: [0, 0, 220, 255],
    },
    grayscaleSamples: {
      overlapNear: [64, 64, 64, 255],
      farOnly: [191, 191, 191, 255],
      nearOnly: [64, 64, 64, 255],
      infinityBackground: [12, 18, 28, 255],
    },
    heatmapSamples: {
      overlapNear: [0, 128, 128, 255],
      farOnly: [128, 128, 0, 255],
      nearOnly: [0, 128, 128, 255],
      infinityBackground: [12, 18, 28, 255],
    },
  });

  await page.locator("#depth-debug-mode").selectOption("1");
  const restoredGrayscale = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(restoredGrayscale.depthDebugMode).toBe(1);
  await expect(page.locator("#coordinate-debug")).toContainText(
    "depth overlap fixture · identity M/V/P vertex stage · viewport aspect 1.778",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "near/far overlap triangle mesh · vertices 6 · indices 6 · triangles 2 · material 0 · far-first submission",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "strict < · clear +infinity · debug grayscale",
  );
  await expect(page.locator("#depth-algorithm")).toHaveText(
    "affine z_ndc · strict < · +infinity clear (Rust)",
  );

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter13-depth-grayscale.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter13-depth-grayscale", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, restoredGrayscale, 0, browserLog, screenshotPath, {
    orderIndependentColorHashes: [nearFirst.pixelHash, farFirst.pixelHash],
    baseSamples,
    grayscaleSamples,
    heatmapSamples,
  });
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
  await expect(page.locator("#coordinate-debug")).toContainText("aspect 1.777");

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
