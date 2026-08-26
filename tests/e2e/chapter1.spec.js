import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { FrameTimingRing, summarizeFrameTimings } from "../../web/frame-timing.js";
import { resolveRasterPath } from "../../web/raster-path.js";

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
    const pipelineDebugMode =
      initialControls.pipelineDebugMode ??
      (initialControls.depthDebugMode === 1
        ? 4
        : initialControls.depthDebugMode === 2
          ? 5
          : initialControls.windingDebugMode === 1
            ? 6
            : initialControls.windingDebugMode === 2
              ? 3
              : 0);
    await page.route(
      "**/",
      async (route) => {
        const response = await route.fetch();
        const html = await response.text();
        const initialControlScript = `<script>
          document.querySelector("#cull-mode").value = ${JSON.stringify(String(initialControls.cullMode))};
          document.querySelector("#pipeline-debug-mode").value = ${JSON.stringify(String(pipelineDebugMode))};
          document.querySelector("#winding-debug").checked = ${JSON.stringify(initialControls.windingDebugMode === 1)};
          document.querySelector("#barycentric-debug").checked = ${JSON.stringify(initialControls.windingDebugMode === 2)};
          document.querySelector("#clip-debug").checked = ${JSON.stringify(initialControls.clipDebugEnabled ?? false)};
          document.querySelector("#coverage-debug").checked = ${JSON.stringify(initialControls.coverageDebugEnabled ?? false)};
          document.querySelector("#interpolation-debug").checked = ${JSON.stringify(initialControls.interpolationDebugEnabled ?? false)};
          document.querySelector("#perspective-debug").checked = ${JSON.stringify(initialControls.perspectiveDebugEnabled ?? false)};
          document.querySelector("#attribute-interpolation-mode").value = ${JSON.stringify(String(initialControls.attributeInterpolationMode ?? 1))};
          document.querySelector("#depth-debug").checked = ${JSON.stringify(initialControls.depthDebugEnabled ?? false)};
          document.querySelector("#depth-order-reversed").checked = ${JSON.stringify(initialControls.depthOrderReversed ?? false)};
          document.querySelector("#depth-debug-mode").value = ${JSON.stringify(String(initialControls.depthDebugMode ?? 0))};
          document.querySelector("#light-x").value = ${JSON.stringify(String(initialControls.lightX ?? -0.4))};
          document.querySelector("#light-y").value = ${JSON.stringify(String(initialControls.lightY ?? 0.8))};
          document.querySelector("#light-z").value = ${JSON.stringify(String(initialControls.lightZ ?? -0.45))};
          document.querySelector("#light-intensity").value = ${JSON.stringify(String(initialControls.lightIntensity ?? 0.9))};
          document.querySelector("#shader-mode").value = ${JSON.stringify(String(initialControls.shaderMode ?? 1))};
          document.querySelector("#specular-color").value = ${JSON.stringify(initialControls.specularColor ?? "#ffffff")};
          document.querySelector("#shininess").value = ${JSON.stringify(String(initialControls.shininess ?? 32))};
          document.querySelector("#alpha-mode").value = ${JSON.stringify(String(initialControls.alphaMode ?? 0))};
          document.querySelector("#alpha-cutoff").value = ${JSON.stringify(String(initialControls.alphaCutoff ?? 0.5))};
          document.querySelector("#transparency-debug").checked = ${JSON.stringify(initialControls.transparencyDebugEnabled ?? false)};
          document.querySelector("#transparent-sort").checked = ${JSON.stringify(initialControls.transparentSortEnabled ?? true)};
          document.querySelector("#blend-color-space").value = ${JSON.stringify(String(initialControls.blendColorSpace ?? 0))};
          document.querySelector("#quality-mode").value = ${JSON.stringify(String(initialControls.qualityMode ?? 0))};
          document.querySelector("#raster-path").value = ${JSON.stringify(String(initialControls.rasterPath ?? 0))};
          document.querySelector("#mipmap-enabled").checked = ${JSON.stringify(initialControls.mipmapEnabled ?? false)};
          document.querySelector("#mip-debug").checked = ${JSON.stringify(initialControls.mipDebugEnabled ?? false)};
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
    { type: "steps", description: "8" },
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
    shadedSamples: 62786,
    sampleCounterOverflow: false,
    invalidValues: 0,
  });
  expect(initial.stats.maxBarycentricSumError).toBeLessThanOrEqual(2 * Math.fround(2 ** -23));
  expect(initial.stats.depthPassedSamples).toBe(initial.stats.shadedSamples);
  expect(initial.stats.depthFailedSamples).toBe(0);
  expect(initial.stats.invalidDepthSamples).toBe(0);
  expect(initial.stats.debugPixels).toBe(0);
  await expect(page.locator("#coordinate-debug")).toContainText(
    "X-ray overlay off · culling/depth 무관",
  );

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
    { type: "steps", description: "18" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  const initial = await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(true);
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
    shadedSamples: 75292,
  });
  expect(initial.pixelHash).toBe("abfcc5b8");
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
    "X-ray overlay on · culling/depth 무관",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "indexed mesh · vertices 24 · indices 36 · triangles 12 · material 0",
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
  await expect(page.locator("#coordinate-debug")).toContainText(
    "X-ray overlay off · culling/depth 무관",
  );
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

  await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(true);
    window.__softRasterizer.advanceFrame(0);
  });

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
    shadedSamples: 62786,
  });
  expect(initial.pixelHash).toBe("3a900c98");
  await expect(page.locator("#coordinate-debug")).toContainText(
    "winding screen y-down orient2d > 0 front · cull back · debug vertex color",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "triangle stats input 12 · submitted 4 · culled 8 · degenerate 0 · invalid 0",
  );
  await expect(page.locator(".space-legend")).toContainText(
    "transform → clip → fan → divide/viewport + 1/w → cull/setup → scalar 또는 disjoint 16×16 tile coverage → affine z_ndc → strict depth < → perspective attributes → linear shade → sRGB write",
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
    shadedSamples: 75292,
  });
  expect(doubleSided.snapshot.pixelHash).toBe("abfcc5b8");
  expect(doubleSided.differingPixels).toBe(1783);
  expect(doubleSided.maxChannelDifference).toBe(191);

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
    shadedSamples: 75292,
  });
  expect(facing.facingColorCounts).toEqual([61488, 1781]);

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

  const clipped = await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(true);
    return window.__softRasterizer.advanceFrame(0);
  });
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
  expect(clipped.pixelHash).toBe("504dddc4");
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
  expect(cube.pixelHash).toBe("abfcc5b8");
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
      [255, 160, 108, 255],
      [108, 225, 255, 255],
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
  expect(covered.pixelHash).toBe("2d9aae5d");
  await expect(page.locator("#coordinate-debug")).toContainText(
    "top-left coverage fixture · identity M/V/P vertex stage · viewport aspect 1.778",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "coverage quad mesh · vertices 6 · indices 6 · triangles 2 · material 0 · 두 삼각형/공유 대각선",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    `coverage stats rasterized 2 · covered ${expectedSamples} · shaded samples ${expectedSamples} · counter overflow false · S=256 pixel center/top-left`,
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
      nearRed: [251, 14, 50, 255],
      nearGreen: [9, 251, 50, 255],
      nearBlue: [39, 41, 250, 255],
      centroid: [156, 156, 157, 255],
    },
    pixelHash: "8316d55d",
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
    "Σ(λ · attribute/w) ÷ Σ(λ/w) · normal 재정규화 (Rust)",
  );

  await page.locator("#barycentric-debug").check();
  const barycentric = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(barycentric.windingDebugMode).toBe(2);
  expect(barycentric.stats).toMatchObject({
    submittedTriangles: affine.stats.submittedTriangles,
    rasterizedTriangles: affine.stats.rasterizedTriangles,
    shadedSamples: affine.stats.shadedSamples,
  });
  expect(barycentric.pixelHash).toBe("aabc25f9");
  expect(barycentric.pixelHash).not.toBe(affine.pixelHash);
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
    hashes: ["7d07efac", "b687761d", "0d9a6422"],
    baseSamples: {
      overlapNear: [255, 124, 108, 255],
      farOnly: [108, 160, 255, 255],
      nearOnly: [255, 124, 108, 255],
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

test("triangle_pipeline: 기울어진 UV quad를 perspective-correct로 복원한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "triangle_pipeline" },
    { type: "steps", description: "22" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, {
    cullMode: 1,
    windingDebugMode: 0,
    perspectiveDebugEnabled: true,
    attributeInterpolationMode: 0,
  });

  const affine = await page.evaluate(() => {
    window.__softRasterizer.setDebugLinesEnabled(false);
    const snapshot = window.__softRasterizer.advanceFrame(0);
    const canvas = document.querySelector("#framebuffer");
    window.__chapterFourteenAffine = canvas
      .getContext("2d")
      .getImageData(0, 0, canvas.width, canvas.height).data.slice();
    return snapshot;
  });
  expect(affine).toMatchObject({
    perspectiveDebugEnabled: true,
    attributeInterpolationMode: 0,
  });
  expect(affine.stats).toMatchObject({
    inputVertices: 4,
    inputTriangles: 2,
    transformedVertices: 4,
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
    invalidInterpolationSamples: 0,
    debugPixels: 0,
    invalidValues: 0,
  });
  expect(affine.stats.interpolatedInvWSamples).toBe(affine.stats.shadedSamples);
  expect(affine.stats.minInterpolatedInvW).toBeGreaterThan(0.2);
  expect(affine.stats.maxInterpolatedInvW).toBeLessThan(0.5);

  await page.locator("#attribute-interpolation-mode").selectOption("1");
  const perspective = await page.locator("#framebuffer").evaluate((canvas) => {
    const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    const affineData = window.__chapterFourteenAffine;
    let differingPixels = 0;
    let maxChannelDifference = 0;
    for (let index = 0; index < data.length; index += 4) {
      let differs = false;
      for (let channel = 0; channel < 4; channel += 1) {
        const difference = Math.abs(data[index + channel] - affineData[index + channel]);
        maxChannelDifference = Math.max(maxChannelDifference, difference);
        differs ||= difference !== 0;
      }
      differingPixels += Number(differs);
    }
    const pixel = (x, y) =>
      Array.from(data.slice(4 * (y * canvas.width + x), 4 * (y * canvas.width + x) + 4));
    return {
      snapshot: window.__softRasterizer.snapshot(),
      differingPixels,
      maxChannelDifference,
      samples: {
        near: pixel(300, 270),
        middle: pixel(450, 270),
        far: pixel(540, 270),
      },
    };
  });
  expect(perspective.snapshot).toMatchObject({
    perspectiveDebugEnabled: true,
    attributeInterpolationMode: 1,
  });
  expect(perspective.snapshot.stats).toEqual({
    ...affine.stats,
    frameIndex: affine.stats.frameIndex + 1,
  });
  expect(perspective.differingPixels).toBeGreaterThan(0);
  expect(perspective.maxChannelDifference).toBeGreaterThan(0);
  expect(perspective.snapshot.pixelHash).not.toBe(affine.pixelHash);
  expect({
    shadedSamples: perspective.snapshot.stats.shadedSamples,
    qSamples: perspective.snapshot.stats.interpolatedInvWSamples,
    qRange: [
      perspective.snapshot.stats.minInterpolatedInvW,
      perspective.snapshot.stats.maxInterpolatedInvW,
    ],
    hashes: [affine.pixelHash, perspective.snapshot.pixelHash],
    differingPixels: perspective.differingPixels,
    maxChannelDifference: perspective.maxChannelDifference,
    samples: perspective.samples,
  }).toEqual({
    shadedSamples: 107350,
    qSamples: 107350,
    qRange: [0.20002862811088562, 0.49969935417175293],
    hashes: ["cc59bf30", "ddafc684"],
    differingPixels: 47687,
    maxChannelDifference: 208,
    samples: {
      near: [242, 246, 255, 255],
      middle: [34, 75, 132, 255],
      far: [242, 246, 255, 255],
    },
  });
  await expect(page.locator("#coordinate-debug")).toContainText(
    "perspective UV fixture · identity M/V · LH zero-to-one P · viewport aspect 1.778",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "tilted procedural checker quad mesh · vertices 4 · indices 6 · triangles 2 · material 0 · perspective-correct UV",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "mode perspective-correct",
  );
  await expect(page.locator("#coordinate-debug")).toContainText("inv_w stats samples");
  await expect(page.locator("#interpolation-algorithm")).toHaveText(
    "Σ(λ · attribute/w) ÷ Σ(λ/w) · normal 재정규화 (Rust)",
  );
  await expect(page.locator(".space-legend")).toContainText(
    "perspective attributes",
  );

  await page.locator("#clip-debug").check();
  const clipping = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(clipping).toMatchObject({
    clipDebugEnabled: true,
    perspectiveDebugEnabled: false,
    stats: {
      inputVertices: 3,
      inputTriangles: 1,
      generatedTriangles: 3,
      submittedTriangles: 3,
      invalidTriangles: 0,
    },
  });
  expect(clipping.pixelHash).not.toBe(perspective.snapshot.pixelHash);
  await expect(page.locator("#coordinate-debug")).toContainText("동차 clip fixture");
  await expect(page.locator("#coordinate-debug")).toContainText("clip debug mesh");
  await page.locator("#perspective-debug").check();
  const restored = await page.evaluate(() => {
    delete window.__chapterFourteenAffine;
    return window.__softRasterizer.snapshot();
  });
  expect(restored).toMatchObject({
    clipDebugEnabled: false,
    perspectiveDebugEnabled: true,
    attributeInterpolationMode: 1,
    pixelHash: perspective.snapshot.pixelHash,
  });
  const deterministic = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(deterministic.pixelHash).toBe(restored.pixelHash);
  expect(deterministic.stats).toEqual({
    ...restored.stats,
    frameIndex: restored.stats.frameIndex + 1,
  });

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter14-perspective-correct.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter14-perspective-correct", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, deterministic, 0, browserLog, screenshotPath, {
    affinePixelHash: affine.pixelHash,
    perspectiveDiff: {
      differingPixels: perspective.differingPixels,
      maxChannelDifference: perspective.maxChannelDifference,
    },
    perspectiveSamples: perspective.samples,
  });
});

test("triangle_pipeline: 15장 scalar 컬러 큐브가 통합 debug view를 공유한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "triangle_pipeline" },
    { type: "steps", description: "36" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, {
    cullMode: 1,
    windingDebugMode: 0,
    pipelineDebugMode: 0,
  });

  await page.evaluate(() => window.__softRasterizer.setModelRotationY(0.65));
  await page.locator("#depth-debug-mode").selectOption("1");
  await page.locator("#winding-debug").check();
  const windingAlias = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(windingAlias).toMatchObject({
    pipelineDebugMode: 6,
    windingDebugMode: 1,
    depthDebugMode: 0,
    pixelHash: "bd779bc6",
  });
  await expect(page.locator("#pipeline-debug-mode")).toHaveValue("6");
  await expect(page.locator("#depth-debug-mode")).toHaveValue("0");
  await expect(page.locator("#coordinate-debug")).toContainText(
    "pipeline state debug front green / back red",
  );

  await page.locator("#depth-debug-mode").selectOption("2");
  await page.locator("#barycentric-debug").check();
  const barycentricAlias = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(barycentricAlias).toMatchObject({
    pipelineDebugMode: 3,
    windingDebugMode: 2,
    depthDebugMode: 0,
    pixelHash: "5725935f",
  });
  await expect(page.locator("#pipeline-debug-mode")).toHaveValue("3");
  await expect(page.locator("#depth-debug-mode")).toHaveValue("0");
  await expect(page.locator("#coordinate-debug")).toContainText(
    "pipeline state debug barycentric RGB",
  );
  await page.locator("#pipeline-debug-mode").selectOption("0");

  const modes = await page.evaluate(() => {
    window.__softRasterizer.setModelRotationY(0.65);
    const canvas = document.querySelector("#framebuffer");
    const context = canvas.getContext("2d");
    const pixel = (data, x, y) =>
      Array.from(
        data.slice(4 * (y * canvas.width + x), 4 * (y * canvas.width + x) + 4),
      );
    const results = [];
    for (let mode = 0; mode <= 6; mode += 1) {
      window.__softRasterizer.setPipelineDebugMode(mode);
      const snapshot = window.__softRasterizer.advanceFrame(0);
      const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
      results.push({
        mode,
        snapshot,
        samples: {
          center: pixel(data, 480, 270),
          upper: pixel(data, 480, 205),
          left: pixel(data, 390, 270),
        },
      });
    }
    return results;
  });

  expect(modes.map(({ mode, snapshot }) => [mode, snapshot.pipelineDebugMode])).toEqual(
    modes.map(({ mode }) => [mode, mode]),
  );
  const reference = modes[0].snapshot.stats;
  expect(reference).toMatchObject({
    inputVertices: 24,
    inputTriangles: 12,
    transformedVertices: 24,
    generatedTriangles: 12,
    submittedTriangles: 6,
    culledTriangles: 6,
    degenerateTriangles: 0,
    invalidTriangles: 0,
    fullyClippedTriangles: 0,
    clipInvalidTriangles: 0,
    rasterizedTriangles: 6,
    invalidDepthSamples: 0,
    invalidInterpolationSamples: 0,
    sampleCounterOverflow: false,
    debugPixels: 0,
    invalidValues: 0,
  });
  for (const { snapshot } of modes) {
    expect(snapshot.stats).toMatchObject({
      submittedTriangles: reference.submittedTriangles,
      culledTriangles: reference.culledTriangles,
      rasterizedTriangles: reference.rasterizedTriangles,
      coveredSamples: reference.coveredSamples,
      depthPassedSamples: reference.depthPassedSamples,
      depthFailedSamples: reference.depthFailedSamples,
      shadedSamples: reference.shadedSamples,
      interpolatedInvWSamples: reference.interpolatedInvWSamples,
    });
    expect(snapshot.stats.coveredSamples).toBe(
      snapshot.stats.depthPassedSamples +
        snapshot.stats.depthFailedSamples +
        snapshot.stats.invalidDepthSamples +
        snapshot.stats.invalidInterpolationSamples,
    );
    expect(snapshot.stats.shadedSamples).toBe(snapshot.stats.depthPassedSamples);
    expect(snapshot.stats.interpolatedInvWSamples).toBe(snapshot.stats.shadedSamples);
  }
  expect(new Set(modes.map(({ snapshot }) => snapshot.pixelHash)).size).toBe(7);
  expect({
    coveredSamples: reference.coveredSamples,
    depthPassedSamples: reference.depthPassedSamples,
    depthFailedSamples: reference.depthFailedSamples,
    qRange: [reference.minInterpolatedInvW, reference.maxInterpolatedInvW],
    modes: modes.map(({ mode, snapshot, samples }) => ({
      mode,
      pixelHash: snapshot.pixelHash,
      samples,
    })),
  }).toEqual({
    coveredSamples: 64809,
    depthPassedSamples: 64809,
    depthFailedSamples: 0,
    qRange: [0.2781980335712433, 0.5104668736457825],
    modes: [
      {
        mode: 0,
        pixelHash: "96778118",
        samples: {
          center: [255, 160, 137, 255],
          upper: [255, 160, 137, 255],
          left: [255, 160, 137, 255],
        },
      },
      {
        mode: 1,
        pixelHash: "80f79608",
        samples: {
          center: [12, 18, 28, 255],
          upper: [12, 18, 28, 255],
          left: [12, 18, 28, 255],
        },
      },
      {
        mode: 2,
        pixelHash: "f6f56528",
        samples: {
          center: [255, 167, 38, 255],
          upper: [255, 167, 38, 255],
          left: [255, 167, 38, 255],
        },
      },
      {
        mode: 3,
        pixelHash: "5725935f",
        samples: {
          center: [24, 74, 157, 255],
          upper: [51, 143, 62, 255],
          left: [155, 63, 38, 255],
        },
      },
      {
        mode: 4,
        pixelHash: "bd1bd1b8",
        samples: {
          center: [243, 243, 243, 255],
          upper: [244, 244, 244, 255],
          left: [245, 245, 245, 255],
        },
      },
      {
        mode: 5,
        pixelHash: "13552604",
        samples: {
          center: [231, 24, 0, 255],
          upper: [234, 21, 0, 255],
          left: [235, 20, 0, 255],
        },
      },
      {
        mode: 6,
        pixelHash: "bd779bc6",
        samples: {
          center: [72, 232, 112, 255],
          upper: [72, 232, 112, 255],
          left: [72, 232, 112, 255],
        },
      },
    ],
  });

  const restored = await page.evaluate(() => {
    window.__softRasterizer.setPipelineDebugMode(0);
    return window.__softRasterizer.advanceFrame(0);
  });
  const deterministic = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(deterministic.pixelHash).toBe(restored.pixelHash);
  expect(deterministic.stats).toEqual({
    ...restored.stats,
    frameIndex: restored.stats.frameIndex + 1,
  });
  expect(restored).toMatchObject({
    pipelineDebugMode: 0,
    clipDebugEnabled: false,
    coverageDebugEnabled: false,
    interpolationDebugEnabled: false,
    perspectiveDebugEnabled: false,
    depthDebugEnabled: false,
  });
  await expect(page.locator("#coordinate-debug")).toContainText(
    "indexed mesh · vertices 24 · indices 36 · triangles 12 · material 0",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "pipeline state debug solid vertex color · strict depth test/write · material 0",
  );
  await expect(page.locator("#coordinate-debug")).toContainText(
    "coverage stats rasterized 6 · covered",
  );
  await expect(page.locator("#pipeline-algorithm")).toHaveText(
    "Solid vertex color · 같은 Rust coverage/depth 경로",
  );

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter15-scalar-color-cube.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter15-scalar-color-cube", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, deterministic, 0, browserLog, screenshotPath, {
    pipelineModes: modes.map(({ mode, snapshot, samples }) => ({
      mode,
      pixelHash: snapshot.pixelHash,
      samples,
    })),
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

test("texture_upload: 브라우저 RGBA8 디코드와 Rust 소유 texture debug 경로", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "texture_upload" },
    { type: "steps", description: "15" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  await page.evaluate(async () => {
    const canvas = document.createElement("canvas");
    canvas.width = 2;
    canvas.height = 2;
    const context = canvas.getContext("2d");
    const image = context.createImageData(2, 2);
    image.data.set([
      255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ]);
    context.putImageData(image, 0, 0);
    const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
    const transfer = new DataTransfer();
    transfer.items.add(new File([blob], "corners.png", { type: "image/png" }));
    const input = document.querySelector("#texture-file");
    input.files = transfer.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await expect
    .poll(async () => page.evaluate(() => window.__softRasterizer.snapshot().textureStatus))
    .toMatchObject({ activeId: 1, width: 2, height: 2, successes: 1, failures: 0 });
  await expect(page.locator("#texture-status")).toContainText("Rust 소유 복사");

  const corners = await page.locator("#framebuffer").evaluate((canvas) => {
    const context = canvas.getContext("2d");
    const points = [
      [0, 0],
      [canvas.width - 1, 0],
      [0, canvas.height - 1],
      [canvas.width - 1, canvas.height - 1],
    ];
    return points.map(([x, y]) => Array.from(context.getImageData(x, y, 1, 1).data));
  });
  expect(corners).toEqual([
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [255, 255, 255, 255],
  ]);

  const beforeMalformed = await page.evaluate(() => window.__softRasterizer.snapshot());
  await page.evaluate(() => {
    const transfer = new DataTransfer();
    transfer.items.add(
      new File([new Uint8Array([0, 1, 2, 3])], "broken.png", { type: "image/png" }),
    );
    const input = document.querySelector("#texture-file");
    input.files = transfer.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await expect(page.locator("#error")).toContainText("RGBA8로 디코딩하지 못했습니다");
  const afterMalformed = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(afterMalformed.pixelHash).toBe(beforeMalformed.pixelHash);
  expect(afterMalformed.textureStatus).toMatchObject({
    activeId: 1,
    successes: 1,
    failures: 0,
  });
  await expect(page.locator("#texture-status")).toContainText("디코딩 실패");

  const oversized = await page.evaluate(() =>
    window.__softRasterizer.testOversizedDecodeGuard(),
  );
  expect(oversized).toMatchObject({ canvasCreated: false, bitmapClosed: true });
  expect(oversized.error).toContain("16777216");

  const failed = await page.evaluate(() =>
    window.__softRasterizer.uploadTextureRgba(2, 2, new Array(15).fill(0)),
  );
  expect(failed.id).toBeNull();
  expect(failed.error).toContain("16이어야");
  expect(failed.snapshot.textureStatus).toMatchObject({ activeId: 1, successes: 1, failures: 1 });

  const owned = await page.evaluate(() =>
    window.__softRasterizer.uploadTextureRgba(1, 1, [12, 34, 56, 78]),
  );
  expect(owned).toMatchObject({ id: 2, error: null });
  expect(owned.snapshot.stats).toMatchObject({
    inputTriangles: 0,
    textureDebugPixels: 960 * 540,
    textureUploadSuccesses: 2,
    textureUploadFailures: 1,
    activeTextureId: 2,
  });
  const ownedPixel = await page
    .locator("#framebuffer")
    .evaluate((canvas) => Array.from(canvas.getContext("2d").getImageData(0, 0, 1, 1).data));
  expect(ownedPixel).toEqual([12, 34, 56, 255]);

  const stable = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.05));
  expect(stable.pixelHash).toBe(owned.snapshot.pixelHash);
  expect(stable.textureStatus).toMatchObject({ activeId: 2, successes: 2, failures: 1 });

  const latestSelection = await page.evaluate(() =>
    window.__softRasterizer.testLatestTextureSelectionWins(),
  );
  expect(latestSelection.afterSecond.textureStatus).toMatchObject({
    activeId: 3,
    width: 1,
    height: 1,
    successes: 3,
    failures: 1,
  });
  expect(latestSelection.afterFirst.textureStatus).toMatchObject({
    activeId: 3,
    successes: 3,
    failures: 1,
  });
  expect(latestSelection.afterFirst.pixelHash).toBe(latestSelection.afterSecond.pixelHash);
  expect(latestSelection.afterFirst.textureStatus.text).toContain("texture 3");

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter16-texture-upload.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter16-texture-upload", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, latestSelection.afterFirst, 0.05, browserLog, screenshotPath, {
    textureCorners: corners,
    failedUpload: failed.error,
    malformedDecode: afterMalformed.textureStatus.text,
    oversizedDecode: oversized,
  });
});

test("texture_sampling: perspective UV로 nearest/bilinear와 repeat/clamp를 비교한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "texture_sampling" },
    { type: "steps", description: "16" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  await page.evaluate(() => {
    window.__softRasterizer.uploadTextureRgba(2, 2, [
      255, 32, 16, 255, 16, 255, 32, 255, 32, 16, 255, 255, 240, 240, 240, 255,
    ]);
    window.__softRasterizer.setModelRotationY(0.35);
  });
  await page.locator("#texture-sampling").check();
  const nearest = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(nearest).toMatchObject({
    textureDebugEnabled: false,
    textureSamplingEnabled: true,
    samplerState: { filter: 0, addressU: 0, addressV: 0 },
  });
  expect(nearest.stats.textureSamples).toBe(nearest.stats.shadedSamples);
  expect(nearest.stats.textureSamples).toBeGreaterThan(0);
  expect(nearest.pixelHash).toBe("a7612563");

  await page.locator("#texture-filter").selectOption("1");
  const bilinearRepeat = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(bilinearRepeat.samplerState).toEqual({ filter: 1, addressU: 0, addressV: 0 });
  expect(bilinearRepeat.pixelHash).toBe("08fe46a3");
  expect(bilinearRepeat.stats.textureSamples).toBe(bilinearRepeat.stats.shadedSamples);

  await page.locator("#texture-address-u").selectOption("1");
  await page.locator("#texture-address-v").selectOption("1");
  const bilinearClamp = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(bilinearClamp.samplerState).toEqual({ filter: 1, addressU: 1, addressV: 1 });
  expect(bilinearClamp.pixelHash).toBe("96665871");
  await expect(page.locator("#texture-sampler")).toContainText(
    "Bilinear · U Clamp · V Clamp",
  );

  await page.locator("#texture-debug").check();
  const disabled = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(disabled.textureDebugEnabled).toBe(true);
  expect(disabled.textureSamplingEnabled).toBe(false);
  expect(disabled.stats.textureSamples).toBe(0);
  expect(disabled.pixelHash).not.toBe(bilinearClamp.pixelHash);
  await page.locator("#texture-sampling").check();
  const restored = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(restored.textureDebugEnabled).toBe(false);
  expect(restored.textureSamplingEnabled).toBe(true);
  expect(restored.pixelHash).toBe(bilinearClamp.pixelHash);

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter17-texture-sampling.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter17-texture-sampling", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, restored, 0, browserLog, screenshotPath, {
    nearestHash: nearest.pixelHash,
    bilinearRepeatHash: bilinearRepeat.pixelHash,
    bilinearClampHash: bilinearClamp.pixelHash,
  });
});

test("lambert_lighting: normal matrix와 world-space 방향광 debug를 검증한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "lambert_lighting" },
    { type: "steps", description: "36" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, {
    cullMode: 1,
    lightX: 0,
    lightY: 0,
    lightZ: -1,
    lightIntensity: -1,
  });
  const recoveredBoot = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(recoveredBoot.directionalLight.intensity).toBeCloseTo(0.9, 6);
  expect(recoveredBoot.directionalLight.surfaceToLight[2]).toBeLessThan(-0.4);
  await expect(page.locator("#light-intensity")).toHaveValue("0.9");

  await page.evaluate(() =>
    window.__softRasterizer.setDirectionalLight(-0.4, 0.8, -0.45, 0.9),
  );

  await page.evaluate(() => {
    window.__softRasterizer.uploadTextureRgba(2, 2, [
      255, 32, 16, 255, 16, 255, 32, 255, 32, 16, 255, 255, 240, 240, 240, 255,
    ]);
    window.__softRasterizer.setModelRotationY(0.35);
  });
  await page.locator("#texture-sampling").check();
  await page.locator("#lighting-enabled").check();
  const lit = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(lit).toMatchObject({
    textureSamplingEnabled: true,
    lightingEnabled: true,
    normalMode: 0,
  });
  expect(lit.stats.textureSamples).toBe(lit.stats.shadedSamples);
  expect(lit.stats.lightingSamples).toBe(lit.stats.shadedSamples);
  expect(lit.pixelHash).toBe("ad175728");

  await page.locator("#pipeline-debug-mode").selectOption("7");
  const normal = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(normal.pipelineDebugMode).toBe(7);
  expect(normal.stats.lightingSamples).toBe(0);
  expect(normal.pixelHash).toBe("9f75d849");

  await page.locator("#pipeline-debug-mode").selectOption("8");
  const ndotl = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(ndotl.pipelineDebugMode).toBe(8);
  expect(ndotl.stats.lightingSamples).toBe(ndotl.stats.shadedSamples);
  expect(ndotl.pixelHash).toBe("e1f88a20");

  await page.locator("#interpolation-debug").check();
  const smoothFixture = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(smoothFixture.interpolationDebugEnabled).toBe(true);
  expect(smoothFixture.normalMode).toBe(0);
  expect(smoothFixture.pixelHash).toBe("2dd6e34b");
  await page.locator("#normal-mode").selectOption("1");
  const flatFixture = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(flatFixture.normalMode).toBe(1);
  expect(flatFixture.pixelHash).toBe("52c8b439");
  await page.locator("#interpolation-debug").uncheck();
  const flat = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(flat.normalMode).toBe(1);
  expect(flat.stats.lightingSamples).toBe(flat.stats.shadedSamples);
  expect(flat.pixelHash).toBe("e1f88a20");

  await page.locator("#light-x").fill("0.7");
  await page.locator("#light-x").press("Enter");
  const movedLight = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(movedLight.directionalLight.surfaceToLight[0]).toBeGreaterThan(0.5);
  expect(movedLight.pixelHash).toBe("1ef1d825");

  await page.locator("#pipeline-debug-mode").selectOption("0");
  const movedLightLit = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(movedLightLit.stats.lightingSamples).toBe(movedLightLit.stats.shadedSamples);
  expect(movedLightLit.pixelHash).toBe("4a0f8fdf");
  await page.locator("#light-intensity").fill("0.25");
  await page.locator("#light-intensity").press("Enter");
  const lowIntensity = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(lowIntensity.directionalLight.intensity).toBe(0.25);
  expect(lowIntensity.pixelHash).toBe("8d93438c");
  await page.locator("#light-intensity").fill("-1");
  await page.locator("#light-intensity").press("Enter");
  await expect(page.locator("#error")).toContainText("intensity");
  const afterInvalidIntensity = await page.evaluate(() =>
    window.__softRasterizer.advanceFrame(0),
  );
  expect(afterInvalidIntensity.directionalLight.intensity).toBe(0.25);
  expect(afterInvalidIntensity.pixelHash).toBe(lowIntensity.pixelHash);

  await page.locator("#light-intensity").fill("0.0000004");
  await page.locator("#light-intensity").press("Enter");
  const tinyIntensity = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(tinyIntensity.directionalLight.intensity).toBeCloseTo(0.0000004, 12);

  await page.locator("#light-x").fill("0");
  await page.locator("#light-x").press("Enter");
  await page.locator("#light-y").fill("0");
  await page.locator("#light-y").press("Enter");
  await page.locator("#light-z").fill("0");
  await page.locator("#light-z").press("Enter");
  await expect(page.locator("#error")).toContainText("surface_to_light");
  await expect(page.locator("#light-intensity")).toHaveValue("4e-7");
  await page.locator("#light-x").fill("0.1");
  await page.locator("#light-x").press("Enter");
  const afterRecoveredEdit = await page.evaluate(() =>
    window.__softRasterizer.advanceFrame(0),
  );
  expect(afterRecoveredEdit.directionalLight.intensity).toBeCloseTo(0.0000004, 12);

  await page.locator("#normal-mode").selectOption("0");
  await page.locator("#light-x").fill("-0.4");
  await page.locator("#light-y").fill("0.8");
  await page.locator("#light-z").fill("-0.45");
  await page.locator("#light-z").press("Enter");
  await page.locator("#light-intensity").fill("0.9");
  await page.locator("#light-intensity").press("Enter");
  const restored = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(restored.pixelHash).toBe(lit.pixelHash);
  await expect(page.locator("#lighting-status")).toContainText("Lambert 켬");

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter18-lambert-lighting.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter18-lambert-lighting", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, restored, 0, browserLog, screenshotPath, {
    litHash: lit.pixelHash,
    normalHash: normal.pixelHash,
    ndotlHash: ndotl.pixelHash,
    flatHash: flat.pixelHash,
    smoothFixtureHash: smoothFixture.pixelHash,
    flatFixtureHash: flatFixture.pixelHash,
    movedLightHash: movedLight.pixelHash,
    movedLightLitHash: movedLightLit.pixelHash,
    lowIntensityHash: lowIntensity.pixelHash,
  });
});

test("blinn_phong_color_space: linear shading과 sRGB wrong-way를 비교한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "blinn_phong_color_space" },
    { type: "steps", description: "34" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, { cullMode: 1, shaderMode: 2, shininess: -1 });
  const recoveredBoot = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(recoveredBoot.materialSpecular.shininess).toBe(32);
  await expect(page.locator("#shininess")).toHaveValue("32");

  await page.evaluate(() => {
    window.__softRasterizer.setMaterialSpecular(1, 1, 1, 32);
    window.__softRasterizer.uploadTextureRgba(2, 2, [
      0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255,
    ]);
    window.__softRasterizer.setModelRotationY(0.35);
  });
  await page.locator("#texture-sampling").check();
  await page.locator("#texture-filter").selectOption("1");
  await page.locator("#texture-address-u").selectOption("1");
  await page.locator("#texture-address-v").selectOption("1");
  await page.locator("#lighting-enabled").check();
  const blinn = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(blinn).toMatchObject({
    shaderMode: 2,
    lightingEnabled: true,
    textureSamplingEnabled: true,
  });
  expect(blinn.stats.textureSamples).toBe(blinn.stats.shadedSamples);
  expect(blinn.stats.lightingSamples).toBe(blinn.stats.shadedSamples);
  expect(blinn.pixelHash).toBe("bce6586a");

  await page.locator("#shader-mode").selectOption("1");
  const lambert = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(lambert.shaderMode).toBe(1);
  expect(lambert.pixelHash).toBe("c66de251");
  expect(lambert.stats.coveredSamples).toBe(blinn.stats.coveredSamples);

  await page.locator("#shader-mode").selectOption("2");
  await page.locator("#pipeline-debug-mode").selectOption("9");
  const diffuse = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(diffuse.pipelineDebugMode).toBe(9);
  expect(diffuse.pixelHash).toBe("6aba03d3");
  expect(diffuse.stats.depthPassedSamples).toBe(blinn.stats.depthPassedSamples);

  await page.locator("#pipeline-debug-mode").selectOption("10");
  const specular = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(specular.pipelineDebugMode).toBe(10);
  expect(specular.pixelHash).toBe("89c0fae2");
  expect(specular.stats.lightingSamples).toBe(specular.stats.shadedSamples);

  await page.locator("#pipeline-debug-mode").selectOption("0");
  await page.locator("#lighting-enabled").uncheck();
  const correctUnlit = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(correctUnlit.shaderMode).toBe(0);
  expect(correctUnlit.pixelHash).toBe("0e2dbc20");
  await page.locator("#pipeline-debug-mode").selectOption("11");
  const comparison = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(comparison.pipelineDebugMode).toBe(11);
  expect(comparison.pixelHash).toBe("3fa65fa9");
  expect(comparison.stats.textureSamples).toBe(comparison.stats.shadedSamples);
  expect(comparison.stats.depthPassedSamples).toBe(blinn.stats.depthPassedSamples);
  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const comparisonScreenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter19-srgb-comparison.png`,
  );
  await page.locator("main").screenshot({ path: comparisonScreenshotPath });
  await testInfo.attach("chapter19-srgb-comparison", {
    path: comparisonScreenshotPath,
    contentType: "image/png",
  });

  await page.locator("#pipeline-debug-mode").selectOption("10");
  await page.locator("#lighting-enabled").check();
  await page.locator("#shininess").fill("4");
  await page.locator("#shininess").press("Enter");
  const broadHighlight = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(broadHighlight.materialSpecular.shininess).toBe(4);
  expect(broadHighlight.pixelHash).toBe("3828192b");
  await page.locator("#shininess").fill("128");
  await page.locator("#shininess").press("Enter");
  const tightHighlight = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(tightHighlight.materialSpecular.shininess).toBe(128);
  expect(tightHighlight.pixelHash).toBe("8f7be926");

  await page.locator("#specular-color").fill("#ff2020");
  await page.locator("#specular-color").press("Enter");
  const redHighlight = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(redHighlight.materialSpecular.color[0]).toBe(1);
  expect(redHighlight.materialSpecular.color[1]).toBeCloseTo(32 / 255, 6);
  expect(redHighlight.pixelHash).toBe("23f3ada1");
  await page.locator("#shininess").evaluate((input) => {
    input.value = "-1";
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await expect(page.locator("#error")).toContainText("shininess");
  const afterInvalid = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(afterInvalid.materialSpecular.shininess).toBe(128);
  expect(afterInvalid.pixelHash).toBe(redHighlight.pixelHash);

  await page.locator("#specular-color").fill("#ffffff");
  await page.locator("#shininess").evaluate((input) => {
    input.value = "32";
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await page.locator("#pipeline-debug-mode").selectOption("0");
  const restored = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  await expect(page.locator("#lighting-status")).toContainText("Blinn-Phong 켬");
  expect(restored.shaderMode).toBe(2);
  expect(restored.pixelHash).toBe(blinn.pixelHash);

  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter19-blinn-phong-color-space.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter19-blinn-phong-color-space", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, restored, 0, browserLog, screenshotPath, {
    blinnHash: blinn.pixelHash,
    lambertHash: lambert.pixelHash,
    diffuseHash: diffuse.pixelHash,
    specularHash: specular.pixelHash,
    correctUnlitHash: correctUnlit.pixelHash,
    comparisonHash: comparison.pixelHash,
    broadHighlightHash: broadHighlight.pixelHash,
    tightHighlightHash: tightHighlight.pixelHash,
    redHighlightHash: redHighlight.pixelHash,
    comparisonScreenshotPath,
  });
});

test("input_camera_fast_release: rAF 사이에 끝난 drag delta를 한 번 적용한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "input_camera_fast_release" },
    { type: "steps", description: "10" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);
  const canvas = page.locator("#framebuffer");
  const initial = await page.evaluate(() => window.__softRasterizer.snapshot());
  await canvas.scrollIntoViewIfNeeded();
  const bounds = await canvas.boundingBox();
  expect(bounds).not.toBeNull();
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 + 96, bounds.y + bounds.height / 2 + 24);
  await page.mouse.up();
  expect(await page.evaluate(() => window.__softRasterizer.inputState())).toMatchObject({
    dragging: false,
    pointerButtons: 0,
  });
  const applied = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(applied.inputSnapshot).toMatchObject({
    pointerDx: 96,
    pointerDy: 24,
    pointerButtons: 0,
  });
  expect(applied.inputSnapshot.flags & 1).toBe(1);
  expect(applied.camera.yaw).toBeGreaterThan(0);
  expect(applied.camera.pitch).toBeGreaterThan(0);
  expect(applied.pixelHash).not.toBe(initial.pixelHash);
  const consumed = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(consumed.inputSnapshot).toMatchObject({ pointerDx: 0, pointerDy: 0, flags: 0 });
  expect(consumed.camera).toEqual(applied.camera);
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, consumed, 0, browserLog, null, {
    initialCamera: initial.camera,
    appliedCamera: applied.camera,
  });
});

test("input_camera: DOM collector를 거쳐 Orbit/Fly와 focus 해제를 검증한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "input_camera" },
    { type: "steps", description: "78" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);
  const canvas = page.locator("#framebuffer");
  const initial = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(initial.camera).toMatchObject({
    mode: 0,
    eye: [0, 0, -3],
    forward: [0, 0, 1],
    yaw: 0,
    pitch: 0,
    orbitRadius: 3,
    input: { heldBits: 0, dragging: false, pointerButtons: 0 },
  });

  const invalidInput = await page.evaluate(() =>
    window.__softRasterizer.testInputSnapshot([0, 0, 0, 0, 0, 0, 0]),
  );
  expect(invalidInput.error).toContain("input snapshot 길이는 8");
  expect(invalidInput.snapshot.camera).toEqual(initial.camera);
  expect(invalidInput.snapshot.pixelHash).toBe(initial.pixelHash);

  await canvas.scrollIntoViewIfNeeded();
  const bounds = await canvas.boundingBox();
  expect(bounds).not.toBeNull();
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  expect(await page.evaluate(() => window.__softRasterizer.inputState())).toMatchObject({
    dragging: true,
    pointerButtons: 1,
  });
  await page.mouse.move(bounds.x + bounds.width + 40, bounds.y + bounds.height / 2 + 30, {
    steps: 4,
  });
  const orbitDragged = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(orbitDragged.camera.forward[0]).toBeGreaterThan(0.5);
  expect(orbitDragged.camera.pitch).toBeGreaterThan(0);
  expect(orbitDragged.stats.inputBits).toBe(0);
  expect(orbitDragged.inputSnapshot.pointerDx).toBeGreaterThan(500);
  expect(orbitDragged.inputSnapshot.pointerDy).toBe(30);
  expect(orbitDragged.inputSnapshot.pointerButtons).toBe(1);
  expect(orbitDragged.inputSnapshot.flags & 1).toBe(1);
  expect(orbitDragged.pixelHash).not.toBe(initial.pixelHash);
  await page.mouse.up();
  expect(await page.evaluate(() => window.__softRasterizer.inputState())).toMatchObject({
    dragging: false,
    pointerButtons: 0,
  });

  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  const radiusBeforeWheel = orbitDragged.camera.orbitRadius;
  await canvas.dispatchEvent("wheel", { deltaY: 200 });
  const orbitZoomed = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(orbitZoomed.camera.orbitRadius).toBeGreaterThan(radiusBeforeWheel);
  expect(orbitZoomed.camera.eye).not.toEqual(orbitDragged.camera.eye);
  expect(orbitZoomed.inputSnapshot.wheelDelta).toBe(200);

  await page.mouse.down();
  expect((await page.evaluate(() => window.__softRasterizer.inputState())).dragging).toBe(true);
  await canvas.dispatchEvent("pointercancel", {
    pointerId: 1,
    pointerType: "mouse",
    buttons: 0,
  });
  expect(await page.evaluate(() => window.__softRasterizer.inputState())).toMatchObject({
    dragging: false,
    pointerButtons: 0,
  });
  await page.mouse.up();

  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 + 20, bounds.y + bounds.height / 2 + 10);
  await canvas.dispatchEvent("lostpointercapture", {
    pointerId: 1,
    pointerType: "mouse",
    buttons: 0,
  });
  expect(await page.evaluate(() => window.__softRasterizer.inputState())).toMatchObject({
    dragging: false,
    pointerButtons: 0,
  });
  const afterLostCapture = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(afterLostCapture.inputSnapshot).toMatchObject({
    pointerDx: 0,
    pointerDy: 0,
    pointerButtons: 0,
    flags: 0,
  });
  await page.mouse.up();

  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 + 15, bounds.y + bounds.height / 2 + 12);
  await canvas.dispatchEvent("wheel", { deltaY: 75 });
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  const afterQueuedBlur = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(afterQueuedBlur.inputSnapshot).toMatchObject({
    pointerDx: 0,
    pointerDy: 0,
    wheelDelta: 0,
    pointerButtons: 0,
    flags: 0,
  });
  expect(afterQueuedBlur.camera.eye).toEqual(orbitZoomed.camera.eye);
  await page.mouse.up();

  const flyStart = await page.evaluate(() => window.__softRasterizer.setCameraMode(1));
  expect(flyStart.camera.mode).toBe(1);
  await canvas.focus();

  await page.keyboard.down("Control");
  const controlHeld = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(controlHeld.inputSnapshot.flags & 4).toBe(4);
  await page.keyboard.up("Control");
  const controlReleased = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(controlReleased.inputSnapshot.flags & 4).toBe(0);

  await page.keyboard.down("w");
  const aliasFirst = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(aliasFirst.inputSnapshot).toMatchObject({ heldBits: 1, pressedBits: 1 });
  await page.evaluate(() =>
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        code: "KeyW",
        key: "w",
        repeat: true,
        bubbles: true,
      }),
    ),
  );
  const repeatedAlias = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(repeatedAlias.inputSnapshot).toMatchObject({ heldBits: 1, pressedBits: 0 });
  await page.keyboard.down("ArrowUp");
  const aliasSecond = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(aliasSecond.inputSnapshot).toMatchObject({ heldBits: 1, pressedBits: 0 });
  await page.keyboard.up("w");
  const aliasOneReleased = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(aliasOneReleased.inputSnapshot).toMatchObject({ heldBits: 1, releasedBits: 0 });
  await page.keyboard.down("w");
  const aliasRepressed = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(aliasRepressed.inputSnapshot).toMatchObject({ heldBits: 1, pressedBits: 0 });
  await page.keyboard.up("ArrowUp");
  const aliasOtherReleased = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(aliasOtherReleased.inputSnapshot).toMatchObject({ heldBits: 1, releasedBits: 0 });
  await page.keyboard.up("w");
  const aliasFullyReleased = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(aliasFullyReleased.inputSnapshot).toMatchObject({ heldBits: 0, releasedBits: 1 });

  await page.keyboard.down("w");
  const pressedForward = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(pressedForward.inputSnapshot).toMatchObject({
    heldBits: 1,
    pressedBits: 1,
    releasedBits: 0,
  });
  const heldForward = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(heldForward.inputSnapshot).toMatchObject({
    heldBits: 1,
    pressedBits: 0,
    releasedBits: 0,
  });
  const sixtyFrames = await page.evaluate(() => {
    let snapshot;
    for (let frame = 0; frame < 60; frame += 1) {
      snapshot = window.__softRasterizer.advanceFrame(1 / 120);
    }
    return snapshot;
  });
  expect(sixtyFrames.stats.inputBits).toBe(1);
  await page.keyboard.up("w");
  const releasedForward = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(releasedForward.inputSnapshot).toMatchObject({
    heldBits: 0,
    pressedBits: 0,
    releasedBits: 1,
  });
  const displacement60 = Math.hypot(
    ...sixtyFrames.camera.eye.map((value, index) => value - flyStart.camera.eye[index]),
  );
  expect(displacement60).toBeCloseTo(1.5, 4);

  await page.keyboard.down("s");
  const returned = await page.evaluate(() => {
    let snapshot;
    for (let frame = 0; frame < 60; frame += 1) {
      snapshot = window.__softRasterizer.advanceFrame(1 / 120);
    }
    return snapshot;
  });
  await page.keyboard.up("s");
  await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(returned.camera.eye[0]).toBeCloseTo(flyStart.camera.eye[0], 4);
  expect(returned.camera.eye[1]).toBeCloseTo(flyStart.camera.eye[1], 4);
  expect(returned.camera.eye[2]).toBeCloseTo(flyStart.camera.eye[2], 4);

  await page.keyboard.down("w");
  const thirtyFrames = await page.evaluate(() => {
    let snapshot;
    for (let frame = 0; frame < 30; frame += 1) {
      snapshot = window.__softRasterizer.advanceFrame(1 / 60);
    }
    return snapshot;
  });
  await page.keyboard.up("w");
  await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  const displacement30 = Math.hypot(
    ...thirtyFrames.camera.eye.map((value, index) => value - returned.camera.eye[index]),
  );
  expect(displacement30).toBeCloseTo(displacement60, 4);

  await page.keyboard.down("w");
  expect((await page.evaluate(() => window.__softRasterizer.inputState())).heldBits).toBe(1);
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  expect((await page.evaluate(() => window.__softRasterizer.inputState())).heldBits).toBe(0);
  const afterBlur = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.1));
  expect(afterBlur.stats.inputBits).toBe(0);
  expect(afterBlur.inputSnapshot.releasedBits).toBe(1);
  expect(afterBlur.camera.eye).toEqual(thirtyFrames.camera.eye);
  await page.keyboard.up("w");

  await canvas.focus();
  await page.keyboard.down("d");
  expect((await page.evaluate(() => window.__softRasterizer.inputState())).heldBits).toBe(8);
  await canvas.dispatchEvent("wheel", { deltaY: 90 });
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, value: true });
    document.dispatchEvent(new Event("visibilitychange"));
    Object.defineProperty(document, "hidden", { configurable: true, value: false });
  });
  expect((await page.evaluate(() => window.__softRasterizer.inputState())).heldBits).toBe(0);
  await page.keyboard.up("d");
  const afterVisibility = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(afterVisibility.inputSnapshot.releasedBits).toBe(8);
  expect(afterVisibility.inputSnapshot.wheelDelta).toBe(0);

  await canvas.focus();
  await page.keyboard.down("s");
  const returnedAgain = await page.evaluate(() => {
    let snapshot;
    for (let frame = 0; frame < 30; frame += 1) {
      snapshot = window.__softRasterizer.advanceFrame(1 / 60);
    }
    return snapshot;
  });
  await page.keyboard.up("s");
  await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(returnedAgain.camera.eye[0]).toBeCloseTo(flyStart.camera.eye[0], 4);
  expect(returnedAgain.camera.eye[1]).toBeCloseTo(flyStart.camera.eye[1], 4);
  expect(returnedAgain.camera.eye[2]).toBeCloseTo(flyStart.camera.eye[2], 4);

  await page.keyboard.down("w");
  await page.keyboard.down("d");
  const diagonal = await page.evaluate(() => {
    let snapshot;
    for (let frame = 0; frame < 30; frame += 1) {
      snapshot = window.__softRasterizer.advanceFrame(1 / 60);
    }
    return snapshot;
  });
  await page.keyboard.up("w");
  await page.keyboard.up("d");
  await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  const diagonalDisplacement = Math.hypot(
    ...diagonal.camera.eye.map((value, index) => value - returnedAgain.camera.eye[index]),
  );
  expect(diagonalDisplacement).toBeCloseTo(displacement30, 4);

  await page.keyboard.down("s");
  await page.keyboard.down("a");
  const finalCamera = await page.evaluate(() => {
    let snapshot;
    for (let frame = 0; frame < 30; frame += 1) {
      snapshot = window.__softRasterizer.advanceFrame(1 / 60);
    }
    return snapshot;
  });
  await page.keyboard.up("s");
  await page.keyboard.up("a");
  const finalSnapshot = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(finalCamera.camera.eye[0]).toBeCloseTo(flyStart.camera.eye[0], 4);
  expect(finalCamera.camera.eye[1]).toBeCloseTo(flyStart.camera.eye[1], 4);
  expect(finalCamera.camera.eye[2]).toBeCloseTo(flyStart.camera.eye[2], 4);

  await canvas.evaluate((element) => {
    element.style.width = "800px";
  });
  const resized = await page.evaluate(() => {
    window.__softRasterizer.applyDisplayResize();
    return window.__softRasterizer.snapshot();
  });
  expect(resized.internalSize).toEqual([800, 450]);
  expect(resized.camera.eye).toEqual(finalSnapshot.camera.eye);
  await canvas.evaluate((element) => {
    element.style.width = "";
  });
  const finalRestored = await page.evaluate(() => {
    window.__softRasterizer.applyDisplayResize();
    return window.__softRasterizer.advanceFrame(0);
  });
  expect(finalRestored.internalSize).toEqual([960, 540]);
  expect(finalRestored.camera.eye).toEqual(finalSnapshot.camera.eye);
  expect(finalRestored.pixelHash).toBe(finalSnapshot.pixelHash);
  expect(finalRestored.pixelHash).toBe("5600f051");
  expect(finalRestored.stats).toMatchObject({
    inputBits: 0,
    inputTriangles: 12,
    submittedTriangles: 6,
    culledTriangles: 6,
    coveredSamples: 41798,
    invalidValues: 0,
  });

  await expect(page.locator("#camera-status")).toContainText("Fly · drag/WASD");
  await expect(page.locator("#camera-status")).toContainText("forward (");
  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter20-input-camera.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter20-input-camera", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, finalRestored, 0, browserLog, screenshotPath, {
    initialCamera: initial.camera,
    orbitDraggedCamera: orbitDragged.camera,
    orbitZoomedCamera: orbitZoomed.camera,
    flyStartCamera: flyStart.camera,
    sixtyFrameCamera: sixtyFrames.camera,
    thirtyFrameCamera: thirtyFrames.camera,
    displacement60,
    displacement30,
    diagonalDisplacement,
    invalidInputError: invalidInput.error,
  });
});

test("asset_failure: 실제 OBJ 파일을 Rust Mesh로 바꾸고 실패 시 기존 scene을 유지한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "asset_failure" },
    { type: "steps", description: "28" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page);

  await page.evaluate(() =>
    window.__softRasterizer.uploadTextureRgba(2, 2, [
      255, 40, 30, 255, 30, 255, 80, 255, 30, 80, 255, 255, 245, 225, 80, 255,
    ]),
  );
  await page.locator("#texture-sampling").check();
  await page.locator("#lighting-enabled").check();

  const pyramidObj = [
    "# LH +X right, +Y up, +Z forward",
    "v -1 -1 0",
    "v 1 -1 0",
    "v 1 1 0",
    "v -1 1 0",
    "v 0 0 1",
    "vt 0 0",
    "vt 1 0",
    "vt 1 1",
    "vt 0 1",
    "vt 0.5 0.5",
    "f 1/1 4/4 3/3 2/2",
    "f 1/1 2/2 5/5",
    "f 2/1 3/2 5/5",
    "f 3/1 4/2 5/5",
    "f -2/1 -5/2 -1/5",
    "",
  ].join("\n");
  await page.evaluate((source) => {
    const transfer = new DataTransfer();
    transfer.items.add(new File([source], "pyramid.obj", { type: "text/plain" }));
    const input = document.querySelector("#mesh-file");
    input.files = transfer.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  }, pyramidObj);
  await expect
    .poll(async () => page.evaluate(() => window.__softRasterizer.snapshot().meshStatus))
    .toMatchObject({
      activeId: 1,
      sourcePositions: 5,
      sourceFaces: 5,
      triangles: 6,
      successes: 1,
      failures: 0,
      sourceMin: [-1, -1, 0],
      sourceMax: [1, 1, 1],
    });
  const loaded = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(loaded.meshStatus.internalVertices).toBeGreaterThan(5);
  expect(loaded).toMatchObject({
    textureSamplingEnabled: true,
    lightingEnabled: true,
    camera: { mode: 0, eye: [0, 0, -3] },
  });
  expect(loaded.stats.inputVertices).toBe(loaded.meshStatus.internalVertices);
  expect(loaded.stats.inputTriangles).toBe(6);
  expect(loaded.stats.textureSamples).toBe(loaded.stats.shadedSamples);
  expect(loaded.stats.lightingSamples).toBe(loaded.stats.shadedSamples);
  expect(loaded.stats.shadedSamples).toBeGreaterThan(0);
  expect(loaded.pixelHash).toBe("0091a2b0");
  await expect(page.locator("#mesh-status")).toContainText("LH +X/+Y/+Z profile");

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter21-obj-import.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter21-obj-import", {
    path: screenshotPath,
    contentType: "image/png",
  });

  const malformedObj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 99\n";
  await page.evaluate((source) => {
    const transfer = new DataTransfer();
    transfer.items.add(new File([source], "broken.obj", { type: "text/plain" }));
    const input = document.querySelector("#mesh-file");
    input.files = transfer.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  }, malformedObj);
  await expect(page.locator("#error")).toContainText("범위를 벗어났습니다");
  const afterMalformed = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(afterMalformed.meshStatus).toMatchObject({
    activeId: 1,
    successes: 1,
    failures: 1,
  });
  expect(afterMalformed.pixelHash).toBe(loaded.pixelHash);
  expect(afterMalformed.stats.inputVertices).toBe(loaded.stats.inputVertices);

  const oversized = await page.evaluate(() => window.__softRasterizer.testOversizedObjGuard());
  expect(oversized.bufferRead).toBe(false);
  expect(oversized.error).toContain("8 MiB");
  const invalidSize = await page.evaluate(() =>
    window.__softRasterizer.validateObjFileSize(Number.NaN),
  );
  expect(invalidSize.bytes).toBeNull();
  expect(invalidSize.error).toContain("안전한 정수");

  const latest = await page.evaluate(() =>
    window.__softRasterizer.testLatestObjSelectionWins(),
  );
  expect(latest.afterSecond.meshStatus).toMatchObject({
    activeId: 2,
    sourcePositions: 3,
    sourceFaces: 1,
    internalVertices: 3,
    triangles: 1,
    successes: 2,
    failures: 1,
  });
  expect(latest.afterFirst.meshStatus).toEqual(latest.afterSecond.meshStatus);
  expect(latest.afterFirst.pixelHash).toBe(latest.afterSecond.pixelHash);
  expect(latest.afterFirst.pixelHash).toBe("ce7e6853");
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, loaded, 0, browserLog, screenshotPath, {
    malformedError: afterMalformed.meshStatus.text,
    oversized,
    loadedMeshStatus: loaded.meshStatus,
    latestSelectionHash: latest.afterFirst.pixelHash,
    latestSelectionMeshStatus: latest.afterFirst.meshStatus,
  });
});

test("transparency: cutout depth와 sorted linear blend를 Rust framebuffer에서 합성한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "transparency" },
    { type: "steps", description: "34" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, {
    cullMode: 1,
    windingDebugMode: 0,
    alphaCutoff: 2,
  });
  await expect(page.locator("#error")).toContainText("0..1");
  await expect(page.locator("#alpha-cutoff")).toHaveValue("0.5");

  await page.locator("#alpha-mode").selectOption("1");
  await expect
    .poll(async () => page.evaluate(() => window.__softRasterizer.snapshot().transparency.alphaMode))
    .toBe(1);
  await page.locator("#alpha-cutoff").fill("0.4");
  await page.locator("#alpha-cutoff").dispatchEvent("change");
  await expect(page.locator("#error")).toHaveText("");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__softRasterizer.snapshot().transparency.alphaCutoff),
    )
    .toBeCloseTo(0.4);

  await page.locator("#transparency-debug").check();
  const loadedWhileFixture = await page.evaluate(() =>
    window.__softRasterizer.uploadObjText(
      "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 3 2\n",
      "transparency-reset.obj",
    ),
  );
  expect(loadedWhileFixture.error).toBeNull();
  expect(loadedWhileFixture.snapshot.transparency.debugEnabled).toBe(false);
  expect(loadedWhileFixture.snapshot.stats.inputTriangles).toBe(1);
  await expect(page.locator("#transparency-debug")).not.toBeChecked();
  await page.locator("#transparency-debug").check();
  const sorted = await page.evaluate(() => window.__softRasterizer.snapshot());

  expect(sorted.transparency).toEqual({
    debugEnabled: true,
    alphaMode: 1,
    alphaCutoff: Math.fround(0.4),
    sortEnabled: true,
    blendColorSpace: 0,
  });
  expect(sorted.stats).toMatchObject({
    inputVertices: 16,
    inputTriangles: 8,
    generatedTriangles: 8,
    invalidTriangles: 0,
    invalidDepthSamples: 0,
    invalidInterpolationSamples: 0,
  });
  expect(sorted.stats.alphaDiscardedSamples).toBeGreaterThan(0);
  expect(sorted.stats.depthWrittenSamples).toBeGreaterThan(0);
  expect(sorted.stats.blendedSamples).toBeGreaterThan(0);
  expect(sorted.stats.depthWrittenSamples).toBeLessThan(sorted.stats.depthPassedSamples);
  expect(sorted.pixelHash).toBe("2fbad03f");
  await expect(page.locator("#transparency-status")).toContainText("view +Z descending");
  await expect(page.locator("#transparency-status")).toContainText("Linear correct");

  await page.locator("#alpha-cutoff").fill("2");
  await page.locator("#alpha-cutoff").dispatchEvent("change");
  await expect(page.locator("#error")).toContainText("0..1");
  await expect(page.locator("#alpha-cutoff")).toHaveValue("0.4");
  await page.locator("#alpha-cutoff").dispatchEvent("change");
  await expect(page.locator("#error")).toHaveText("");

  await page.locator("#transparent-sort").uncheck();
  const unsorted = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(unsorted.pixelHash).not.toBe(sorted.pixelHash);
  expect(unsorted.stats.coveredSamples).toBe(sorted.stats.coveredSamples);
  expect(unsorted.stats.depthWrittenSamples).toBe(sorted.stats.depthWrittenSamples);

  await page.locator("#transparent-sort").check();
  await page.locator("#blend-color-space").selectOption("1");
  const wrongSpace = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(wrongSpace.pixelHash).not.toBe(sorted.pixelHash);
  expect(wrongSpace.stats.coveredSamples).toBe(sorted.stats.coveredSamples);

  await page.locator("#blend-color-space").selectOption("0");
  const restored = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(restored.pixelHash).toBe(sorted.pixelHash);
  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter22-transparency.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter22-transparency", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, restored, 0, browserLog, screenshotPath, {
    sortedHash: sorted.pixelHash,
    unsortedHash: unsorted.pixelHash,
    encodedWrongWayHash: wrongSpace.pixelHash,
    intersectingGeometryLimitation:
      "primitive 평균 view +Z 정렬은 교차하는 두 quad의 모든 fragment 순서를 해결하지 못한다",
  });
});

test("antialiasing_mipmap: 2x SSAA linear resolve와 perspective nearest mip를 비교한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "antialiasing_mipmap" },
    { type: "steps", description: "45" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, {
    cullMode: 1,
    windingDebugMode: 0,
    qualityMode: 1,
    mipmapEnabled: true,
    mipDebugEnabled: true,
  });
  const restoredBoot = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(restoredBoot.internalSize).toEqual([960, 540]);
  expect(restoredBoot.renderSize).toEqual([1920, 1080]);
  expect(restoredBoot.framebufferLength).toBe(960 * 540 * 4);
  expect(restoredBoot.quality).toEqual({
    mode: 1,
    mipmapEnabled: true,
    mipDebugEnabled: true,
    mipLevels: 2,
  });
  expect(restoredBoot.textureSamplingEnabled).toBe(true);
  await expect(page.locator("#quality-mode")).toHaveValue("1");
  await expect(page.locator("#mipmap-enabled")).toBeChecked();
  await expect(page.locator("#mip-debug")).toBeChecked();
  await expect(page.locator("#texture-sampling")).toBeChecked();
  expect(restoredBoot.pixelHash).toBe("eb133363");

  await page.locator("#quality-mode").selectOption("0");
  await page.locator("#mip-debug").uncheck();
  await page.locator("#mipmap-enabled").uncheck();
  const initial = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(initial.quality).toEqual({
    mode: 0,
    mipmapEnabled: false,
    mipDebugEnabled: false,
    mipLevels: 2,
  });
  expect(initial.renderSize).toEqual(initial.internalSize);
  const oversizedMip = await page.evaluate(() =>
    window.__softRasterizer.validateDecodedTextureSize(4096, 4096),
  );
  expect(oversizedMip.pixelCount).toBeNull();
  expect(oversizedMip.error).toContain("mip texel");

  const uploaded = await page.evaluate(() => {
    const extent = 512;
    const pixels = new Uint8Array(extent * extent * 4);
    for (let y = 0; y < extent; y += 1) {
      for (let x = 0; x < extent; x += 1) {
        const value = (x + y) % 2 === 0 ? 245 : 12;
        const index = 4 * (y * extent + x);
        pixels[index] = value;
        pixels[index + 1] = value;
        pixels[index + 2] = value;
        pixels[index + 3] = 255;
      }
    }
    return window.__softRasterizer.uploadTextureRgba(extent, extent, pixels);
  });
  expect(uploaded.error).toBeNull();
  expect(uploaded.snapshot.textureStatus.mipLevels).toBe(10);
  await page.locator("#texture-sampling").check();
  await page.evaluate(() => window.__softRasterizer.setModelRotationY(0.72));
  const noAa = await page.evaluate(() => window.__softRasterizer.snapshot());

  await page.locator("#quality-mode").selectOption("1");
  const ssaa = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(ssaa.quality.mode).toBe(1);
  expect(ssaa.renderSize).toEqual([1920, 1080]);
  expect(ssaa.framebufferLength).toBe(noAa.framebufferLength);
  expect(ssaa.stats.renderScale).toBe(2);
  expect(ssaa.stats.resolvedPixels).toBe(960 * 540);
  expect(ssaa.stats.shadedSamples).toBeGreaterThan(noAa.stats.shadedSamples * 3);
  expect(ssaa.pixelHash).not.toBe(noAa.pixelHash);
  expect(noAa.pixelHash).toBe("b28f8f60");
  expect(ssaa.pixelHash).toBe("716b8058");

  await page.locator("#mipmap-enabled").check();
  const mip = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(mip.quality.mipmapEnabled).toBe(true);
  expect(mip.stats.mipSamples).toBeGreaterThan(0);
  expect(mip.stats.maxMipLevel).toBeGreaterThan(0);
  expect(mip.stats.invalidLodSamples).toBe(0);
  expect(mip.pixelHash).not.toBe(ssaa.pixelHash);
  expect(mip.pixelHash).toBe("49b2f480");

  await page.locator("#mip-debug").check();
  const debug = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(debug.quality).toMatchObject({
    mode: 1,
    mipmapEnabled: true,
    mipDebugEnabled: true,
    mipLevels: 10,
  });
  expect(debug.stats.minMipLevel).toBeLessThanOrEqual(debug.stats.maxMipLevel);
  expect(debug.pixelHash).toBe("cb576630");
  await expect(page.locator("#quality-status")).toContainText("2x SSAA");
  await expect(page.locator("#quality-status")).toContainText("10 mip levels");

  await page.locator("#clip-debug").check();
  await expect(page.locator("#mip-debug")).not.toBeChecked();
  expect(
    await page.evaluate(() => window.__softRasterizer.snapshot().quality.mipDebugEnabled),
  ).toBe(false);
  await page.locator("#clip-debug").uncheck();
  await page.locator("#mip-debug").check();
  await page.locator("#texture-sampling").uncheck();
  await expect(page.locator("#mip-debug")).not.toBeChecked();
  await page.locator("#texture-sampling").check();
  await page.locator("#mip-debug").check();
  const resynchronized = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(resynchronized.pixelHash).toBe(debug.pixelHash);
  expect(resynchronized.quality.mipDebugEnabled).toBe(true);

  const invalid = await page.evaluate(() => window.__softRasterizer.setQualityMode(9));
  expect(invalid.error).toContain("quality mode");
  expect(invalid.snapshot.quality.mode).toBe(1);

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter23-antialiasing-mipmap.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter23-antialiasing-mipmap", {
    path: screenshotPath,
    contentType: "image/png",
  });

  await page.locator("#quality-mode").selectOption("0");
  await page.locator("#mip-debug").uncheck();
  await page.locator("#mipmap-enabled").uncheck();
  const restored = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(restored.renderSize).toEqual(restored.internalSize);
  expect(restored.stats.renderScale).toBe(1);
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, resynchronized, 0, browserLog, screenshotPath, {
    noAaHash: noAa.pixelHash,
    ssaaHash: ssaa.pixelHash,
    mipHash: mip.pixelHash,
    mipDebugHash: debug.pixelHash,
    shadedSamples: {
      noAa: noAa.stats.shadedSamples,
      ssaa: ssaa.stats.shadedSamples,
    },
  });
});

test("diagnostics_profiling: UV/overdraw view와 release p50/p95 report를 연결한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "diagnostics_profiling" },
    { type: "steps", description: "47" },
  );
  const browserLog = observeBrowserLog(page);
  await openReadyPage(page, { cullMode: 0, windingDebugMode: 0 });
  const baseline = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(baseline).toMatchObject({
    pipelineDebugMode: 0,
    stats: { overdrawnPixels: 0, maxOverdraw: 0 },
  });
  const invariantCounts = (snapshot) => ({
    inputTriangles: snapshot.stats.inputTriangles,
    generatedTriangles: snapshot.stats.generatedTriangles,
    submittedTriangles: snapshot.stats.submittedTriangles,
    culledTriangles: snapshot.stats.culledTriangles,
    coveredSamples: snapshot.stats.coveredSamples,
    depthPassedSamples: snapshot.stats.depthPassedSamples,
    depthFailedSamples: snapshot.stats.depthFailedSamples,
  });

  await page.locator("#pipeline-debug-mode").selectOption("12");
  const uv = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(uv.pipelineDebugMode).toBe(12);
  expect([uv.stats.overdrawnPixels, uv.stats.maxOverdraw]).toEqual([0, 0]);
  expect(uv.pixelHash).not.toBe(baseline.pixelHash);
  expect(invariantCounts(uv)).toEqual(invariantCounts(baseline));
  expect(uv.pixelHash).toBe("170555a9");

  await page.locator("#pipeline-debug-mode").selectOption("13");
  const overdraw = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(overdraw.pipelineDebugMode).toBe(13);
  expect(overdraw.stats.overdrawnPixels).toBeGreaterThan(0);
  expect(overdraw.stats.maxOverdraw).toBeGreaterThan(1);
  await expect(page.locator("#overdraw-stats")).toContainText("pixels · max");
  expect(overdraw.pixelHash).not.toBe(uv.pixelHash);
  expect(invariantCounts(overdraw)).toEqual(invariantCounts(baseline));
  expect(overdraw.pixelHash).toBe("3769fdf6");

  await page.locator("#mip-debug").check();
  await expect(page.locator("#pipeline-debug-mode")).toHaveValue("0");
  await expect(page.locator("#mip-debug")).toBeChecked();
  expect(await page.evaluate(() => window.__softRasterizer.snapshot())).toMatchObject({
    pipelineDebugMode: 0,
    quality: { mipDebugEnabled: true },
    stats: { overdrawnPixels: 0, maxOverdraw: 0 },
  });
  await page.locator("#pipeline-debug-mode").selectOption("13");
  await expect(page.locator("#mip-debug")).not.toBeChecked();
  expect(
    await page.evaluate(() => window.__softRasterizer.snapshot().pixelHash),
  ).toBe(overdraw.pixelHash);

  await page.locator("#texture-debug").check();
  await expect(page.locator("#pipeline-debug-mode")).toHaveValue("0");
  await expect(page.locator("#texture-debug")).toBeChecked();
  await page.locator("#pipeline-debug-mode").selectOption("12");
  await expect(page.locator("#texture-debug")).not.toBeChecked();
  expect(
    await page.evaluate(() => window.__softRasterizer.snapshot().pixelHash),
  ).toBe(uv.pixelHash);

  await page.locator("#transparency-debug").check();
  await expect(page.locator("#pipeline-debug-mode")).toHaveValue("0");
  await expect(page.locator("#transparency-debug")).toBeChecked();
  await page.locator("#pipeline-debug-mode").selectOption("13");
  await expect(page.locator("#transparency-debug")).not.toBeChecked();
  expect(
    await page.evaluate(() => window.__softRasterizer.snapshot().pixelHash),
  ).toBe(overdraw.pixelHash);

  await page.locator("#pipeline-debug-mode").selectOption("0");
  const restored = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(restored.pixelHash).toBe(baseline.pixelHash);
  expect([restored.stats.overdrawnPixels, restored.stats.maxOverdraw]).toEqual([0, 0]);

  const invalidBenchmark = await page.evaluate(() => {
    const errors = [];
    for (const args of [[-1, 1, 0], [0, 0, 0], [0, 1, 0.2]]) {
      try {
        window.__softRasterizer.runBenchmark(...args);
        errors.push(null);
      } catch (error) {
        errors.push(error instanceof Error ? error.message : String(error));
      }
    }
    return errors;
  });
  expect(invalidBenchmark[0]).toContain("warm-up");
  expect(invalidBenchmark[1]).toContain("sample");
  expect(invalidBenchmark[2]).toContain("fixed dt");
  const percentileFixture = summarizeFrameTimings(
    [9, 1, 7, 3, 5].map((value) => ({ updateMs: value, presentMs: value, totalMs: value })),
  );
  expect(percentileFixture).toEqual({
    count: 5,
    updateMs: { p50: 5, p95: 9 },
    presentMs: { p50: 5, p95: 9 },
    totalMs: { p50: 5, p95: 9 },
  });
  const wrappedRing = new FrameTimingRing(3);
  for (const value of [1, 2, 100, 3]) {
    wrappedRing.push({ updateMs: value, presentMs: value, totalMs: value });
  }
  expect(wrappedRing.summary()).toEqual({
    count: 3,
    updateMs: { p50: 3, p95: 100 },
    presentMs: { p50: 3, p95: 100 },
    totalMs: { p50: 3, p95: 100 },
  });

  const benchmark = await page.evaluate(() => window.__softRasterizer.runBenchmark(3, 7, 0));
  expect(benchmark.buildMode).toContain("release Wasm");
  expect(benchmark).toMatchObject({
    warmupFrames: 3,
    sampleFrames: 7,
    fixedDtSeconds: 0,
  });
  expect(benchmark.resolution).toEqual([960, 540]);
  expect(benchmark.logicalResolution).toEqual([960, 540]);
  expect({
    triangles: benchmark.triangles,
    coveredSamples: benchmark.coveredSamples,
    shadedSamples: benchmark.shadedSamples,
  }).toEqual({
    triangles: baseline.stats.inputTriangles,
    coveredSamples: baseline.stats.coveredSamples,
    shadedSamples: baseline.stats.shadedSamples,
  });
  expect(benchmark.timings.count).toBe(7);
  for (const stage of ["updateMs", "presentMs", "totalMs"]) {
    const timing = benchmark.timings[stage];
    expect([
      Number.isFinite(timing.p50),
      timing.p50 >= 0,
      Number.isFinite(timing.p95),
      timing.p95 >= timing.p50,
    ]).toEqual([true, true, true, true]);
  }
  const afterBenchmark = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(afterBenchmark.pixelHash).toBe(baseline.pixelHash);
  expect(afterBenchmark.timingWindow.count).toBeGreaterThanOrEqual(7);

  await page.locator("#pipeline-debug-mode").selectOption("13");
  const finalOverdraw = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(finalOverdraw.pipelineDebugMode).toBe(13);
  expect(finalOverdraw.pixelHash).toBe(overdraw.pixelHash);
  await expect(page.locator("#timing-window")).toContainText("p50");

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter24-diagnostics-profiling.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter24-diagnostics-profiling", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, finalOverdraw, 0, browserLog, screenshotPath, {
    baselineHash: baseline.pixelHash,
    uvHash: uv.pixelHash,
    overdrawHash: overdraw.pixelHash,
    benchmark,
  });
});

test("capstone_tiled: scalar와 disjoint 16x16 tile 경로의 exact image와 fallback을 검증한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "capstone_tiled" },
    { type: "steps", description: "42" },
  );
  const browserLog = observeBrowserLog(page);
  expect(
    resolveRasterPath(0, {
      crossOriginIsolated: false,
      wasmSharedMemory: false,
      parallelSchedulerBuilt: false,
    }),
  ).toMatchObject({ actualMode: 0, usedFallback: false });
  expect(
    resolveRasterPath(0, {
      crossOriginIsolated: true,
      wasmSharedMemory: true,
      parallelSchedulerBuilt: true,
    }),
  ).toMatchObject({ actualMode: 0, usedFallback: false });
  expect(
    resolveRasterPath(1, {
      crossOriginIsolated: true,
      wasmSharedMemory: true,
      parallelSchedulerBuilt: true,
    }),
  ).toMatchObject({ actualMode: 1, usedFallback: false });
  expect(() =>
    resolveRasterPath(3, {
      crossOriginIsolated: false,
      wasmSharedMemory: false,
      parallelSchedulerBuilt: false,
    }),
  ).toThrow("raster path");
  expect(() => resolveRasterPath(0, {})).toThrow("capability");
  expect(() =>
    resolveRasterPath(2, {
      crossOriginIsolated: true,
      wasmSharedMemory: true,
      parallelSchedulerBuilt: true,
    }),
  ).toThrow("single-thread capstone resolver");
  const schedulerWithoutIsolation = resolveRasterPath(2, {
    crossOriginIsolated: false,
    wasmSharedMemory: true,
    parallelSchedulerBuilt: true,
  });
  expect(schedulerWithoutIsolation).toMatchObject({ actualMode: 1, usedFallback: true });
  expect(schedulerWithoutIsolation.reason).toContain("crossOriginIsolated=false");
  const schedulerWithoutSharedBuild = resolveRasterPath(2, {
    crossOriginIsolated: true,
    wasmSharedMemory: false,
    parallelSchedulerBuilt: true,
  });
  expect(schedulerWithoutSharedBuild).toMatchObject({ actualMode: 1, usedFallback: true });
  expect(schedulerWithoutSharedBuild.reason).toContain("shared-memory build");
  const unavailable = resolveRasterPath(2, {
    crossOriginIsolated: false,
    wasmSharedMemory: false,
    parallelSchedulerBuilt: false,
  });
  expect(unavailable).toMatchObject({ actualMode: 1, usedFallback: true });
  expect(unavailable.reason).toContain("crossOriginIsolated=false");
  const isolatedWithoutSharedBuild = resolveRasterPath(2, {
    crossOriginIsolated: true,
    wasmSharedMemory: false,
    parallelSchedulerBuilt: false,
  });
  expect(isolatedWithoutSharedBuild.reason).not.toContain("crossOriginIsolated=false");
  expect(isolatedWithoutSharedBuild.reason).toContain("shared-memory build");

  await openReadyPage(page, { cullMode: 0, rasterPath: 0 });
  const scalar = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(scalar.rasterPath).toMatchObject({
    requestedMode: 0,
    actualMode: 0,
    usedFallback: false,
  });
  expect(scalar.pixelHash).toBe("10cf841e");
  expect({
    inputTriangles: scalar.stats.inputTriangles,
    coveredSamples: scalar.stats.coveredSamples,
    shadedSamples: scalar.stats.shadedSamples,
  }).toEqual({ inputTriangles: 12, coveredSamples: 125572, shadedSamples: 75292 });
  expect([scalar.stats.tiledRasterizedTriangles, scalar.stats.tileVisits]).toEqual([0, 0]);
  const invariantCounts = (snapshot) => ({
    inputTriangles: snapshot.stats.inputTriangles,
    generatedTriangles: snapshot.stats.generatedTriangles,
    submittedTriangles: snapshot.stats.submittedTriangles,
    culledTriangles: snapshot.stats.culledTriangles,
    rasterizedTriangles: snapshot.stats.rasterizedTriangles,
    coveredSamples: snapshot.stats.coveredSamples,
    shadedSamples: snapshot.stats.shadedSamples,
    depthPassedSamples: snapshot.stats.depthPassedSamples,
    depthFailedSamples: snapshot.stats.depthFailedSamples,
  });

  await page.locator("#raster-path").selectOption("1");
  const tiled = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(tiled.rasterPath).toMatchObject({
    requestedMode: 1,
    actualMode: 1,
    usedFallback: false,
  });
  expect(tiled.pixelHash).toBe(scalar.pixelHash);
  expect(invariantCounts(tiled)).toEqual(invariantCounts(scalar));
  expect(tiled.stats.tiledRasterizedTriangles).toBe(tiled.stats.rasterizedTriangles);
  expect(tiled.stats.tileVisits).toBeGreaterThanOrEqual(tiled.stats.tiledRasterizedTriangles);
  await expect(page.locator("#raster-status")).toContainText("16×16 tiled");

  await page.locator("#raster-path").selectOption("2");
  const fallback = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(fallback.rasterPath).toMatchObject({
    requestedMode: 2,
    actualMode: 1,
    usedFallback: true,
  });
  expect(fallback.rasterPath.reason).toContain("crossOriginIsolated=false");
  await expect(page.locator("#parallel-capability")).toContainText("fallback");
  expect(fallback.pixelHash).toBe(scalar.pixelHash);

  const invalidRequest = await page.evaluate(() => {
    try {
      window.__softRasterizer.setRasterPath(3);
      return null;
    } catch (error) {
      return error instanceof Error ? error.message : String(error);
    }
  });
  expect(invalidRequest).toContain("raster path");
  expect(
    await page.evaluate(() => window.__softRasterizer.snapshot().rasterPath.actualMode),
  ).toBe(1);

  await page.evaluate(() => window.__softRasterizer.setRasterPath(0));
  const scalarBenchmark = await page.evaluate(() =>
    window.__softRasterizer.runBenchmark(30, 120, 0),
  );
  expect(scalarBenchmark).toMatchObject({
    warmupFrames: 30,
    sampleFrames: 120,
    fixedDtSeconds: 0,
    rasterPath: { actualMode: 0 },
    timings: { count: 120 },
  });
  expect(scalarBenchmark.memory.estimatedRendererTargetsMiB).toBeCloseTo(3.955078125);
  const scalarAfterBenchmark = await page.evaluate(() => window.__softRasterizer.snapshot());

  await page.evaluate(() => window.__softRasterizer.setRasterPath(1));
  const tiledBenchmark = await page.evaluate(() =>
    window.__softRasterizer.runBenchmark(30, 120, 0),
  );
  expect(tiledBenchmark).toMatchObject({
    warmupFrames: 30,
    sampleFrames: 120,
    fixedDtSeconds: 0,
    rasterPath: { actualMode: 1 },
    timings: { count: 120 },
  });
  const tiledAfterBenchmark = await page.evaluate(() => window.__softRasterizer.snapshot());
  expect(tiledAfterBenchmark.pixelHash).toBe(scalarAfterBenchmark.pixelHash);
  expect(invariantCounts(tiledAfterBenchmark)).toEqual(invariantCounts(scalarAfterBenchmark));
  for (const benchmark of [scalarBenchmark, tiledBenchmark]) {
    for (const stage of ["updateMs", "presentMs", "totalMs"]) {
      expect([
        Number.isFinite(benchmark.timings[stage].p50),
        benchmark.timings[stage].p50 >= 0,
        Number.isFinite(benchmark.timings[stage].p95),
        benchmark.timings[stage].p95 >= benchmark.timings[stage].p50,
      ]).toEqual([true, true, true, true]);
    }
  }

  const screenshotDirectory = path.resolve("artifacts/e2e/screenshots");
  await mkdir(screenshotDirectory, { recursive: true });
  const screenshotPath = path.join(
    screenshotDirectory,
    `${EXECUTION_MODE}-${testInfo.project.name}-chapter25-capstone-tiled.png`,
  );
  await page.locator("main").screenshot({ path: screenshotPath });
  await testInfo.attach("chapter25-capstone-tiled", {
    path: screenshotPath,
    contentType: "image/png",
  });
  expect(browserLog.errors).toEqual([]);
  recordEvidence(testInfo, tiledAfterBenchmark, 0, browserLog, screenshotPath, {
    scalarHash: scalar.pixelHash,
    tiledHash: tiled.pixelHash,
    fallback: fallback.rasterPath,
    scalarBenchmark,
    tiledBenchmark,
  });
});
