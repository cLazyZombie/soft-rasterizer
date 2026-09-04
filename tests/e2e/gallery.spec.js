import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";

const manifest = JSON.parse(readFileSync("chapter-manifest.json", "utf8"));
const uiPolicy = JSON.parse(readFileSync("chapter-ui.json", "utf8"));
const uiByChapter = new Map(uiPolicy.chapters.map((chapter) => [chapter.number, chapter]));
const REPRESENTATIVE_CHAPTERS = ["01", "04", "10", "15", "16", "20", "21", "25", "26"];
const REPRESENTATIVE_HASHES = {
  "01": "7c64ddc5",
  "04": "1cd4e722",
  "10": "f9fb1bdc",
  "15": "5943c536",
  "16": "5943c536",
  "20": "10cf841e",
  "21": "10cf841e",
  "25": "10cf841e",
  "26": "a64de05c",
};
const RECORD_HASHES = process.env.SOFT_RASTERIZER_RECORD_CHAPTER_HASHES === "1";

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

async function installCanvasContextAudit(page) {
  await page.addInitScript(() => {
    const requested = [];
    const originalGetContext = HTMLCanvasElement.prototype.getContext;
    Object.defineProperty(window, "__chapterRequestedContexts", { value: requested });
    HTMLCanvasElement.prototype.getContext = function auditedGetContext(kind, ...args) {
      requested.push(String(kind));
      return originalGetContext.call(this, kind, ...args);
    };
  });
}

async function openStandaloneChapter(page, number) {
  await page.goto(`/chapters/${number}/`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#framebuffer")).toBeVisible();
}

async function deterministicSnapshot(page, number) {
  if (number === "26") {
    return page.evaluate(async () => {
      const loaded = await window.__softRasterizer.loadBundledFox();
      if (loaded.error !== null) throw new Error(loaded.error);
      window.__softRasterizer.setGlbAnimationPlaying(false);
      window.__softRasterizer.setGlbClip(2);
      window.__softRasterizer.seekGlbAnimation(0.35);
      return window.__softRasterizer.advanceFrame(0);
    });
  }
  return page.evaluate(() => window.__softRasterizer.advanceFrame(0));
}

async function visibleChapterUi(page) {
  return page.evaluate((knownRegions) => {
    const visible = (element) => {
      const style = getComputedStyle(element);
      return (
        style.display !== "none" &&
        style.visibility !== "hidden" &&
        element.getClientRects().length > 0
      );
    };
    const controls = [
      ...[...document.querySelectorAll("label[for]")]
        .filter(visible)
        .map((label) => label.htmlFor),
      ...[...document.querySelectorAll("button[id]")]
        .filter(visible)
        .map((button) => button.id),
    ].sort();
    const stats = [...document.querySelectorAll("dd[id]")]
      .filter(visible)
      .map((entry) => entry.id)
      .sort();
    const regions = knownRegions
      .filter((selector) => {
        const element = document.querySelector(selector);
        return element !== null && visible(element);
      })
      .sort();
    return {
      scope: document.documentElement.dataset.chapterUiScope,
      controls,
      stats,
      regions,
    };
  }, uiPolicy.regions);
}

test("chapter_launcher: manifest, 직접 접근과 iframe metadata가 일치한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "chapter_launcher" },
    { type: "steps", description: "24" },
  );
  const errors = observeErrors(page);
  await installCanvasContextAudit(page);
  await page.goto("/?chapter=16");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#chapter-select option")).toHaveCount(26);
  await expect(page.locator("#chapter-select")).toHaveValue("16");
  await expect(page.locator("#chapter-title")).toHaveText("16장 · 웹 이미지 입력과 Texture 메모리");
  await expect(page.locator("#chapter-commit")).toHaveText(
    "65956c7551d6d8c3e0f2b2a3a6338444c827c980",
  );
  await expect(page.locator("#chapter-frame")).toHaveAttribute("src", "./chapters/16/");
  await expect(page.frameLocator("#chapter-frame").locator("html")).toHaveAttribute(
    "data-ready",
    "true",
  );
  const launcherScreenshotPath = testInfo.outputPath("launcher-chapter-16.png");
  await page.screenshot({ path: launcherScreenshotPath, fullPage: true });

  await page.locator("#chapter-select").selectOption("03");
  await expect(page).toHaveURL(/\?chapter=03$/);
  await expect(page.locator("#chapter-note")).toHaveText("3장 — 4장과 통합된 구현");
  await expect(page.locator("#chapter-frame")).toHaveAttribute("src", "./chapters/03/");
  await expect(page.frameLocator("#chapter-frame").locator("html")).toHaveAttribute(
    "data-ready",
    "true",
  );

  await page.evaluate(() => window.history.back());
  await expect(page).toHaveURL(/\?chapter=16$/);
  await expect(page.locator("#chapter-select")).toHaveValue("16");
  await expect(page.locator("#chapter-commit")).toHaveText(
    "65956c7551d6d8c3e0f2b2a3a6338444c827c980",
  );
  await expect(page.locator("#chapter-frame")).toHaveAttribute("src", "./chapters/16/");
  await expect(page.frameLocator("#chapter-frame").locator("html")).toHaveAttribute(
    "data-ready",
    "true",
  );
  await page.evaluate(() => window.history.forward());
  await expect(page).toHaveURL(/\?chapter=03$/);
  await expect(page.locator("#chapter-select")).toHaveValue("03");
  await expect(page.locator("#chapter-frame")).toHaveAttribute("src", "./chapters/03/");
  await expect(page.frameLocator("#chapter-frame").locator("html")).toHaveAttribute(
    "data-ready",
    "true",
  );

  await page.reload();
  await expect(page.locator("#chapter-select")).toHaveValue("03");
  await page.goto("/?chapter=3");
  await expect(page.locator("#chapter-select")).toHaveValue("03");
  await expect(page).toHaveURL(/\?chapter=03$/);
  await expect(page.locator("#chapter-note")).toHaveText("3장 — 4장과 통합된 구현");
  await page.goto("/?chapter=99");
  await expect(page.locator("#chapter-select")).toHaveValue("26");
  await expect(page).toHaveURL(/\?chapter=26$/);
  await expect(page.frameLocator("#chapter-frame").locator("html")).toHaveAttribute(
    "data-ready",
    "true",
  );

  const requestedContexts = await page
    .frameLocator("#chapter-frame")
    .locator("html")
    .evaluate(() => window.__chapterRequestedContexts);
  expect(requestedContexts).toContain("2d");
  expect(requestedContexts).not.toContain("webgl");
  expect(requestedContexts).not.toContain("webgl2");
  expect(requestedContexts).not.toContain("webgpu");
  expect(errors).toEqual([]);
  testInfo.annotations.push({
    type: "evidence",
    description: JSON.stringify({
      chapterCount: manifest.chapters.length,
      directChapter: "16",
      integratedChapter: "03",
      normalizedDirectChapter: "3 -> 03",
      historyNavigation: "16 -> 03 -> back 16 -> forward 03",
      invalidFallbackChapter: "26",
      screenshotPath: launcherScreenshotPath,
      requestedContexts,
      consoleErrors: errors,
    }),
  });
});

for (const failure of [
  {
    scenario: "chapter_launcher_http_error",
    fulfill: { status: 500, contentType: "application/json", body: "{}" },
    message: "chapter manifest를 읽지 못했습니다: HTTP 500",
  },
  {
    scenario: "chapter_launcher_invalid_json",
    fulfill: { status: 200, contentType: "application/json", body: "{" },
    message: /JSON|Unexpected|expected/i,
  },
]) {
  test(`${failure.scenario}: manifest 실패를 빈 iframe 대신 표시한다`, async ({
    page,
  }, testInfo) => {
    testInfo.annotations.push(
      { type: "scenario", description: failure.scenario },
      { type: "steps", description: "4" },
    );
    await page.route("**/chapter-manifest.json", (route) => route.fulfill(failure.fulfill));
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-ready", "error");
    await expect(page.locator("#launcher-error")).toBeVisible();
    await expect(page.locator("#launcher-error")).toHaveText(failure.message);
    await expect(page.locator(".frame-shell")).toBeHidden();
  });
}

for (const chapter of manifest.chapters) {
  test(`chapter_${chapter.number}_boot: standalone Wasm과 Canvas 2D가 준비된다`, async ({
    page,
  }, testInfo) => {
    testInfo.annotations.push(
      { type: "scenario", description: `chapter_${chapter.number}_boot` },
      { type: "steps", description: "6" },
    );
    const errors = observeErrors(page);
    await installCanvasContextAudit(page);
    await openStandaloneChapter(page, chapter.number);
    const snapshot = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
    const contexts = await page.evaluate(() => window.__chapterRequestedContexts);
    const visibleUi = await visibleChapterUi(page);
    const expectedUi = uiByChapter.get(chapter.number);

    expect(snapshot.internalSize[0]).toBeGreaterThan(0);
    expect(snapshot.internalSize[1]).toBeGreaterThan(0);
    expect(snapshot.pixelHash).toMatch(/^[0-9a-f]{8}$/);
    expect(contexts).toContain("2d");
    expect(contexts).not.toContain("webgl");
    expect(contexts).not.toContain("webgl2");
    expect(contexts).not.toContain("webgpu");
    expect(visibleUi).toEqual({
      scope: chapter.number,
      controls: [...expectedUi.controls].sort(),
      stats: [...expectedUi.stats].sort(),
      regions: [...expectedUi.regions].sort(),
    });
    await expect(page.locator("h1")).toHaveText(
      `${Number(chapter.number)}장 · ${chapter.title}`,
    );
    expect(errors).toEqual([]);

    testInfo.annotations.push({
      type: "evidence",
      description: JSON.stringify({
        chapter: chapter.number,
        commit: chapter.commit,
        internalSize: snapshot.internalSize,
        pixelHash: snapshot.pixelHash,
        visibleUi,
        requestedContexts: contexts,
        consoleErrors: errors,
      }),
    });
  });
}

test("chapter_23_mipmap: 표시된 control만으로 texture와 mipmap을 활성화한다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "chapter_23_mipmap" },
    { type: "steps", description: "4" },
  );
  const errors = observeErrors(page);
  await openStandaloneChapter(page, "23");
  await page.locator("#texture-sampling").check();
  await page.locator("#mipmap-enabled").check();
  const snapshot = await page.evaluate(() => window.__softRasterizer.advanceFrame(0));
  expect(snapshot.textureSamplingEnabled).toBe(true);
  expect(snapshot.stats.textureSamples).toBeGreaterThan(0);
  expect(snapshot.stats.mipSamples).toBeGreaterThan(0);
  expect(errors).toEqual([]);
  testInfo.annotations.push({
    type: "evidence",
    description: JSON.stringify({
      chapter: "23",
      internalSize: snapshot.internalSize,
      pixelHash: snapshot.pixelHash,
      stats: snapshot.stats,
      consoleErrors: errors,
    }),
  });
});

for (const number of REPRESENTATIVE_CHAPTERS) {
  test(`chapter_${number}_golden: 대표 경계 장의 pixel hash가 고정된다`, async ({
    page,
  }, testInfo) => {
    testInfo.annotations.push(
      { type: "scenario", description: `chapter_${number}_golden` },
      { type: "steps", description: number === "26" ? "8" : "4" },
    );
    const errors = observeErrors(page);
    await installCanvasContextAudit(page);
    await openStandaloneChapter(page, number);
    const snapshot = await deterministicSnapshot(page, number);
    const screenshotPath = testInfo.outputPath(`chapter-${number}.png`);
    await page.locator("#framebuffer").screenshot({ path: screenshotPath });

    if (RECORD_HASHES) {
      process.stdout.write(`CHAPTER_HASH ${number} ${snapshot.pixelHash}\n`);
    } else {
      expect(REPRESENTATIVE_HASHES[number], `${number}장의 golden hash가 기록되어야 합니다.`).toBeDefined();
      expect(snapshot.pixelHash).toBe(REPRESENTATIVE_HASHES[number]);
    }
    expect(errors).toEqual([]);

    testInfo.annotations.push({
      type: "evidence",
      description: JSON.stringify({
        chapter: number,
        commit: manifest.chapters.find((chapter) => chapter.number === number).commit,
        internalSize: snapshot.internalSize,
        pixelHash: snapshot.pixelHash,
        screenshotPath,
        consoleErrors: errors,
      }),
    });
  });
}

test("chapter_determinism: 같은 장을 다시 선택하면 같은 hash를 만든다", async ({
  page,
}, testInfo) => {
  testInfo.annotations.push(
    { type: "scenario", description: "chapter_determinism" },
    { type: "steps", description: "7" },
  );
  const errors = observeErrors(page);
  await page.goto("/?chapter=16");
  await expect(page.frameLocator("#chapter-frame").locator("html")).toHaveAttribute(
    "data-ready",
    "true",
  );
  const firstHash = await page
    .frameLocator("#chapter-frame")
    .locator("html")
    .evaluate(() => window.__softRasterizer.advanceFrame(0).pixelHash);

  await page.locator("#chapter-select").selectOption("17");
  await expect(page.frameLocator("#chapter-frame").locator("html")).toHaveAttribute(
    "data-ready",
    "true",
  );
  await page.locator("#chapter-select").selectOption("16");
  await expect(page.frameLocator("#chapter-frame").locator("html")).toHaveAttribute(
    "data-ready",
    "true",
  );
  const secondHash = await page
    .frameLocator("#chapter-frame")
    .locator("html")
    .evaluate(() => window.__softRasterizer.advanceFrame(0).pixelHash);

  expect(secondHash).toBe(firstHash);
  expect(errors).toEqual([]);
  testInfo.annotations.push({
    type: "evidence",
    description: JSON.stringify({
      chapter: "16",
      firstHash,
      secondHash,
      consoleErrors: errors,
    }),
  });
});
