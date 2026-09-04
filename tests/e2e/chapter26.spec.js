import { expect, test } from "@playwright/test";

const EXECUTION_MODE = process.env.SOFT_RASTERIZER_E2E_MODE ?? "unspecified";

async function openReadyPage(page) {
  await page.addInitScript(() => {
    const requested = [];
    const originalGetContext = HTMLCanvasElement.prototype.getContext;
    Object.defineProperty(window, "__chapter26RequestedContexts", { value: requested });
    HTMLCanvasElement.prototype.getContext = function auditedGetContext(kind, ...args) {
      requested.push(String(kind));
      return originalGetContext.call(this, kind, ...args);
    };
  });
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

function observeErrors(page) {
  const errors = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  return errors;
}

test("glb_scene: Fox GLB가 skin animation과 transactional failure를 렌더링한다", async ({ page }, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "glb_scene" },
    { type: "steps", description: "22" },
  );
  const errors = observeErrors(page);
  await openReadyPage(page);

  const frameRate = await page.evaluate(() =>
    window.__softRasterizer.testFrameRateTimestamps([0, 20, 40, 60]),
  );
  expect(frameRate).toEqual({
    summary: { count: 3, fps: 50 },
    text: "50.0 FPS",
  });
  await expect(page.locator("#current-fps")).toHaveText("50.0 FPS");

  const fallback = await page.evaluate(() => window.__softRasterizer.testBundledFoxFetchFailure());
  expect(fallback.error).toContain("HTTP 503");
  expect(fallback.snapshot.glbStatus.active).toBe(false);
  expect(fallback.snapshot.stats.inputVertices).toBe(24);

  const restoredControls = await page.evaluate(() => {
    window.__softRasterizer.setShaderMode(2);
    window.__softRasterizer.setShaderMode(0);
    return {
      snapshot: window.__softRasterizer.snapshot(),
      shaderSelect: document.querySelector("#shader-mode").value,
      lightingChecked: document.querySelector("#lighting-enabled").checked,
    };
  });
  expect(restoredControls.snapshot).toMatchObject({
    shaderMode: 0,
    lightingEnabled: false,
  });
  expect(restoredControls.shaderSelect).toBe("2");
  expect(restoredControls.lightingChecked).toBe(false);

  const loaded = await page.evaluate(() => window.__softRasterizer.loadBundledFox());
  expect(loaded.error).toBeNull();
  expect(loaded.snapshot).toMatchObject({
    shaderMode: 2,
    lightingEnabled: true,
  });
  await expect(page.locator("#shader-mode")).toHaveValue("2");
  await expect(page.locator("#lighting-enabled")).toBeChecked();
  await page.evaluate(() => window.__softRasterizer.setShaderMode(1));
  expect(loaded.snapshot.glbStatus).toMatchObject({
    active: true,
    drawItems: 1,
    nodes: 26,
    skins: 1,
    joints: 24,
    vertices: 1728,
    triangles: 576,
    clips: 3,
    samplerDowngrades: 1,
  });
  expect(loaded.snapshot.animation).toMatchObject({
    selectedClip: 1,
    selectedName: "Walk",
    playing: true,
    looping: true,
  });
  expect(loaded.snapshot.stats).toMatchObject({
    inputVertices: 1728,
    inputTriangles: 576,
    transformedVertices: 1728,
    sceneDrawItems: 1,
    animatedNodes: 20,
    skinnedVertices: 1728,
    jointMatrices: 24,
    samplerDowngrades: 1,
    invalidValues: 0,
  });
  expect(loaded.snapshot.stats.shadedSamples).toBeGreaterThan(0);
  const attribution = page.locator("#fox-attribution");
  for (const creator of ["PixelMannen", "tomkranis", "AsoboStudio", "scurest"]) {
    await expect(attribution).toContainText(creator);
  }
  await expect(attribution.locator('a[rel="license"]').nth(0)).toHaveAttribute(
    "href",
    "https://creativecommons.org/publicdomain/zero/1.0/",
  );
  await expect(attribution.locator('a[rel="license"]').nth(1)).toHaveAttribute(
    "href",
    "https://creativecommons.org/licenses/by/4.0/",
  );
  await expect(attribution.locator('a[rel~="noopener"]')).toHaveAttribute(
    "href",
    "https://github.com/KhronosGroup/glTF-Sample-Assets/tree/2d97dcc2463db123ed5203598cffedf8b6cf1683/Models/Fox",
  );
  await expect(page.locator("#texture-filter")).toHaveValue("2");
  await expect(page.locator("#texture-sampler")).toContainText("GLB material별 imported sampler");
  await expect(page.locator("#quality-status")).toContainText("GLB texture별 mip chain");

  const latestSelection = await page.evaluate(() =>
    window.__softRasterizer.testLatestGlbSelectionWins(),
  );
  expect(latestSelection.afterSecond.glbStatus.active).toBe(true);
  expect(latestSelection.afterSecond.glbStatus.pendingId).toBe(0);
  expect(latestSelection.afterSecond.glbStatus.successes).toBe(
    latestSelection.successesBeforeRace + 1,
  );
  expect(latestSelection.afterSecond.glbStatus.text).toContain("latest.glb");
  expect(latestSelection.afterSecond.animation).toMatchObject({
    selectedClip: 0,
    selectedName: "Survey",
  });
  expect(latestSelection.afterStale.pixelHash).toBe(latestSelection.afterSecond.pixelHash);
  expect(latestSelection.afterStale.glbStatus.text).toBe(latestSelection.afterSecond.glbStatus.text);
  expect(latestSelection.afterStale.glbStatus.successes).toBe(
    latestSelection.afterSecond.glbStatus.successes,
  );
  expect(latestSelection.afterStale.animation.selectedClip).toBe(
    latestSelection.afterSecond.animation.selectedClip,
  );
  expect(latestSelection.afterStale.glbStatus.failures).toBe(
    latestSelection.failuresBeforeCancel,
  );

  const decodeFailure = await page.evaluate(() =>
    window.__softRasterizer.testBundledFoxDecodeFailure(),
  );
  expect(decodeFailure.error).toContain("injected Fox image decode failure");
  expect(decodeFailure.snapshot.glbStatus.active).toBe(true);
  expect(decodeFailure.snapshot.glbStatus.failures).toBe(loaded.snapshot.glbStatus.failures + 1);
  expect(decodeFailure.snapshot.glbStatus.lastFailure).toContain("image decode failure");
  expect(decodeFailure.snapshot.pixelHash).toBe(latestSelection.afterStale.pixelHash);

  const paused = await page.evaluate(() => window.__softRasterizer.setGlbAnimationPlaying(false));
  const pausedHash = paused.pixelHash;
  const pausedAdvance = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.1));
  expect(pausedAdvance.animation.timeSeconds).toBeCloseTo(paused.animation.timeSeconds, 6);
  expect(pausedAdvance.pixelHash).toBe(pausedHash);

  const seekUiFailure = await page.evaluate(() =>
    window.__softRasterizer.testGlbSeekControlFailure(),
  );
  expect(seekUiFailure.errorText).toContain("injected animation seek failure");
  expect(seekUiFailure.inputValue).toBeCloseTo(
    seekUiFailure.snapshot.animation.timeSeconds,
    6,
  );

  await page.locator("#animation-time").evaluate((input) => {
    input.focus();
    input.value = "0.2";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await page.evaluate(() => window.__softRasterizer.setGlbAnimationPlaying(true));
  const domSeekAdvance = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.1));
  expect(Number(await page.locator("#animation-time").inputValue())).toBeCloseTo(
    domSeekAdvance.animation.timeSeconds,
    3,
  );
  await page.evaluate(() => window.__softRasterizer.setGlbAnimationPlaying(false));

  const sought = await page.evaluate(() => window.__softRasterizer.seekGlbAnimation(0.35));
  expect(sought.animation.timeSeconds).toBeCloseTo(0.35, 5);
  expect(sought.pixelHash).not.toBe(pausedHash);
  const run = await page.evaluate(() => window.__softRasterizer.setGlbClip(2));
  expect(run.animation.selectedName).toBe("Run");
  const runFrame = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.2));
  expect(runFrame.pixelHash).not.toBe(sought.pixelHash);

  await page.evaluate(() => window.__softRasterizer.setGlbAnimationLooping(false));
  const nearEnd = await page.evaluate(() => {
    const current = window.__softRasterizer.snapshot();
    window.__softRasterizer.seekGlbAnimation(current.animation.durationSeconds - 0.01);
    return window.__softRasterizer.setGlbAnimationPlaying(true);
  });
  const ended = await page.evaluate(() => window.__softRasterizer.advanceFrame(0.1));
  expect(ended.animation.timeSeconds).toBeCloseTo(nearEnd.animation.durationSeconds, 5);
  expect(ended.animation.playing).toBe(false);

  const beforeInvalid = ended;
  const invalid = await page.evaluate(() =>
    window.__softRasterizer.uploadGlbBytes([0x67, 0x6c, 0x54, 0x46], "broken.glb"),
  );
  expect(invalid.error).toContain("header");
  expect(invalid.snapshot.glbStatus.active).toBe(true);
  expect(invalid.snapshot.pixelHash).toBe(beforeInvalid.pixelHash);
  expect(invalid.snapshot.glbStatus.failures).toBe(beforeInvalid.glbStatus.failures + 1);
  expect(invalid.snapshot.glbStatus.lastFailure).toContain("header");

  const grown = await page.evaluate(() => window.__softRasterizer.growMemory(1));
  expect(grown.bufferChanged).toBe(true);
  const afterGrowth = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(afterGrowth.typedArrayViewRebuilds).toBeGreaterThan(loaded.snapshot.typedArrayViewRebuilds);
  expect(afterGrowth.pixelHash).toBe("d5071ee3");

  const contexts = await page.evaluate(() => window.__chapter26RequestedContexts);
  expect(contexts).toContain("2d");
  expect(contexts).not.toContain("webgl");
  expect(contexts).not.toContain("webgl2");
  expect(contexts).not.toContain("webgpu");
  const screenshotPath = testInfo.outputPath("fox-run.png");
  await page.locator("#framebuffer").screenshot({ path: screenshotPath });
  testInfo.annotations.push({
    type: "evidence",
    description: JSON.stringify({
      executionMode: EXECUTION_MODE,
      fixedDtSeconds: 0.1,
      internalSize: afterGrowth.internalSize,
      frameStats: afterGrowth.stats,
      pixelHash: afterGrowth.pixelHash,
      screenshotPath,
      consoleErrors: errors,
    }),
  });
  expect(errors).toEqual([]);
});
