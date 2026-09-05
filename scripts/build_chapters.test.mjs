import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import { mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  applyChapterUiScope,
  assertIndependentOutputDirectories,
  assertSafeOutputDirectory,
  cargoTargetDirectory,
  installTemporaryDirectorySignalCleanup,
  parseArguments,
  replaceOutputDirectories,
  selectManifestChapters,
  validateBuiltChapter,
  validateManifest,
  validateUiPolicy,
  writeBuildReport,
} from "./build_chapters.mjs";

const FULL_SHA = "0123456789abcdef0123456789abcdef01234567";

function completeManifest() {
  return {
    schemaVersion: 1,
    defaultChapter: "26",
    chapters: Array.from({ length: 26 }, (_, index) => ({
      number: String(index + 1).padStart(2, "0"),
      title: `${index + 1}장`,
      commit: FULL_SHA,
      reproduction: "exact",
    })),
  };
}

function completeUiPolicy() {
  return {
    schemaVersion: 1,
    regions: ["#coordinate-debug", ".space-legend", "#fox-attribution"],
    chapters: Array.from({ length: 26 }, (_, index) => ({
      number: String(index + 1).padStart(2, "0"),
      controls: [],
      stats: [],
      regions: [],
    })),
  };
}

test("manifest는 1–26장과 전체 SHA를 검증한다", () => {
  const manifest = completeManifest();
  assert.equal(validateManifest(manifest), manifest);

  assert.throws(
    () => validateManifest({ ...manifest, chapters: manifest.chapters.slice(1) }),
    /26개 장/,
  );
  assert.throws(
    () =>
      validateManifest({
        ...manifest,
        chapters: manifest.chapters.map((chapter, index) =>
          index === 1 ? { ...chapter, number: "01" } : chapter,
        ),
      }),
    /중복된 장 번호/,
  );
  assert.throws(
    () =>
      validateManifest({
        ...manifest,
        chapters: manifest.chapters.map((chapter, index) =>
          index === 0 ? { ...chapter, commit: "a05da7a" } : chapter,
        ),
      }),
    /전체 40자리/,
  );
  assert.throws(
    () =>
      validateManifest({
        ...manifest,
        chapters: manifest.chapters.map((chapter, index) =>
          index === 2
            ? { ...chapter, reproduction: "integrated", note: undefined }
            : chapter,
        ),
      }),
    /note가 필요/,
  );
});

test("장별 UI 정책은 1–26장의 control, stat과 region 계약을 검증한다", () => {
  const policy = completeUiPolicy();
  assert.equal(validateUiPolicy(policy), policy);

  assert.throws(
    () => validateUiPolicy({ ...policy, chapters: policy.chapters.slice(1) }),
    /26개 장/,
  );
  assert.throws(
    () =>
      validateUiPolicy({
        ...policy,
        chapters: policy.chapters.map((chapter, index) =>
          index === 0 ? { ...chapter, controls: ["bad id"] } : chapter,
        ),
      }),
    /유효하지 않은 값/,
  );
  assert.throws(
    () =>
      validateUiPolicy({
        ...policy,
        chapters: policy.chapters.map((chapter, index) =>
          index === 0 ? { ...chapter, regions: ["#unknown"] } : chapter,
        ),
      }),
    /알 수 없는 UI region/,
  );
});

test("과거 장 archive에는 현재 장의 UI만 보이는 scope를 주입한다", () => {
  const temporaryRoot = mkdtempSync(path.join(os.tmpdir(), "chapter-ui-scope-test-"));
  try {
    const webDirectory = path.join(temporaryRoot, "web");
    mkdirSync(webDirectory);
    writeFileSync(
      path.join(webDirectory, "index.html"),
      `<!doctype html>
<html lang="ko">
  <head><title>과거 제목</title></head>
  <body>
    <h1>과거 제목</h1>
    <div class="controls">
      <label for="current-control"><input id="current-control" /></label>
      <label for="old-control"><input id="old-control" /></label>
      <button id="old-button">누적 버튼</button>
    </div>
    <p class="space-legend">현재 범례</p>
    <pre id="coordinate-debug">누적 진단</pre>
    <dl><dt>현재</dt><dd id="current-stat">1</dd><dt>과거</dt><dd id="old-stat">2</dd></dl>
  </body>
</html>`,
    );
    const summary = applyChapterUiScope(
      temporaryRoot,
      { number: "12", title: "Barycentric 좌표와 속성 보간" },
      {
        number: "12",
        controls: ["current-control"],
        stats: ["current-stat"],
        regions: [".space-legend"],
      },
      ["#coordinate-debug", ".space-legend", "#fox-attribution"],
    );
    const html = readFileSync(path.join(webDirectory, "index.html"), "utf8");

    assert.deepEqual(summary, {
      visibleControls: ["current-control"],
      visibleStats: ["current-stat"],
      visibleRegions: [".space-legend"],
    });
    assert.match(html, /data-chapter-ui-scope="12"/);
    assert.match(html, /<title>12장 · Barycentric 좌표와 속성 보간<\/title>/);
    assert.match(html, /<h1>12장 · Barycentric 좌표와 속성 보간<\/h1>/);
    assert.match(html, /label\[for="old-control"\]/);
    assert.match(html, /#old-button/);
    assert.match(html, /dd#old-stat/);
    assert.match(html, /#coordinate-debug/);
    assert.doesNotMatch(html, /label\[for="current-control"\]/);
    assert.doesNotMatch(html, /dd#current-stat/);
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

test("focused 장 선택은 manifest 순서와 안전한 기본 장을 유지한다", () => {
  const selected = selectManifestChapters(completeManifest(), ["01", "16"]);
  assert.deepEqual(
    selected.chapters.map((chapter) => chapter.number),
    ["01", "16"],
  );
  assert.equal(selected.defaultChapter, "16");
  assert.throws(() => selectManifestChapters(completeManifest(), []), /한 개 이상/);
  assert.throws(
    () => selectManifestChapters(completeManifest(), ["01", "01"]),
    /중복된 장/,
  );
  assert.throws(
    () => selectManifestChapters(completeManifest(), ["27"]),
    /유효하지 않습니다/,
  );
});

test("CLI 인자는 출력과 focused 장을 명시적으로 해석한다", () => {
  assert.deepEqual(
    parseArguments([
      "--out-dir",
      "dist-gallery",
      "--test-out-dir",
      "dist-gallery-test",
      "--chapters",
      "1,04,26",
    ]),
    {
      outDir: "dist-gallery",
      testOutDir: "dist-gallery-test",
      chapters: ["01", "04", "26"],
    },
  );
  assert.throws(() => parseArguments(["--unknown"]), /알 수 없는 인자/);
  assert.throws(() => parseArguments(["--out-dir"]), /값이 필요/);
});

test("출력 경로는 저장소 최상위의 generated dist 디렉터리로 제한한다", () => {
  const root = path.resolve("/tmp/example-repository");
  assert.equal(assertSafeOutputDirectory(root, "dist"), path.join(root, "dist"));
  assert.equal(
    assertSafeOutputDirectory(root, "dist-gallery-test"),
    path.join(root, "dist-gallery-test"),
  );
  for (const unsafe of [".", "../outside", ".git", "doc", "renderer-core", "dist/nested"]) {
    assert.throws(() => assertSafeOutputDirectory(root, unsafe), /dist 또는 dist-\*/);
  }

  assert.doesNotThrow(() =>
    assertIndependentOutputDirectories(path.join(root, "dist"), path.join(root, "dist-test")),
  );
  assert.throws(
    () =>
      assertIndependentOutputDirectories(
        path.join(root, "dist"),
        path.join(root, "dist", "test"),
      ),
    /겹치지 않아야/,
  );
});

test("출력 교체 성공 뒤 backup 정리 실패가 새 산출물을 rollback하지 않는다", () => {
  const temporaryRoot = mkdtempSync(path.join(os.tmpdir(), "chapter-replace-test-"));
  try {
    const productionTarget = path.join(temporaryRoot, "dist");
    const testTarget = path.join(temporaryRoot, "dist-test");
    const productionStage = path.join(temporaryRoot, "production-stage");
    const testStage = path.join(temporaryRoot, "test-stage");
    for (const [directory, contents] of [
      [productionTarget, "old-production"],
      [testTarget, "old-test"],
      [productionStage, "new-production"],
      [testStage, "new-test"],
    ]) {
      mkdirSync(directory);
      writeFileSync(path.join(directory, "identity.txt"), contents);
    }

    let injected = false;
    const operations = {
      exists: (target) => {
        try {
          readFileSync(path.join(target, "identity.txt"));
          return true;
        } catch {
          return false;
        }
      },
      make: mkdirSync,
      move: renameSync,
      remove: (target, options) => {
        if (!injected && target.endsWith("previous-output-1")) {
          injected = true;
          throw new Error("injected backup cleanup failure");
        }
        rmSync(target, options);
      },
    };

    assert.throws(
      () =>
        replaceOutputDirectories(
          [
            { stage: productionStage, target: productionTarget },
            { stage: testStage, target: testTarget },
          ],
          temporaryRoot,
          operations,
        ),
      /injected backup cleanup failure/,
    );
    assert.equal(
      readFileSync(path.join(productionTarget, "identity.txt"), "utf8"),
      "new-production",
    );
    assert.equal(readFileSync(path.join(testTarget, "identity.txt"), "utf8"), "new-test");
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

test("build report는 실제 staged manifest의 hash를 기록한다", () => {
  const temporaryRoot = mkdtempSync(path.join(os.tmpdir(), "chapter-report-test-"));
  try {
    const manifestBytes = `${JSON.stringify({ chapters: [{ number: "16" }] })}\n`;
    const uiPolicyBytes = `${JSON.stringify({ chapters: [{ number: "16" }] })}\n`;
    writeFileSync(path.join(temporaryRoot, "chapter-manifest.json"), manifestBytes);
    writeFileSync(path.join(temporaryRoot, "chapter-ui.json"), uiPolicyBytes);
    writeFileSync(path.join(temporaryRoot, "chapter-docs.json"), "{}\n");
    writeBuildReport(temporaryRoot, "production", [{ number: "16" }]);
    const report = JSON.parse(readFileSync(path.join(temporaryRoot, "build-report.json"), "utf8"));
    assert.equal(report.chapterCount, 1);
    assert.equal(report.documentationSha256, createHash("sha256").update("{}\n").digest("hex"));
    assert.equal(
      report.manifestSha256,
      createHash("sha256").update(manifestBytes).digest("hex"),
    );
    assert.equal(
      report.uiPolicySha256,
      createHash("sha256").update(uiPolicyBytes).digest("hex"),
    );
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

test("Cargo target은 commit별로 격리한다", () => {
  const temporaryRoot = path.resolve("/tmp/chapter-gallery-example");
  const first = cargoTargetDirectory(temporaryRoot, FULL_SHA);
  const secondSha = `1${FULL_SHA.slice(1)}`;
  const second = cargoTargetDirectory(temporaryRoot, secondSha);
  assert.notEqual(first, second);
  assert.equal(first, path.join(temporaryRoot, "cargo-targets", FULL_SHA));
  assert.throws(() => cargoTargetDirectory(temporaryRoot, "a05da7a"), /유효하지 않습니다/);
});

test("SIGINT와 SIGTERM은 활성 장별 빌드 임시 디렉터리를 정리한다", () => {
  for (const [signal, expectedExitCode] of [
    ["SIGINT", 130],
    ["SIGTERM", 143],
  ]) {
    const temporaryRoot = mkdtempSync(path.join(os.tmpdir(), "chapter-signal-test-"));
    writeFileSync(path.join(temporaryRoot, "incomplete.txt"), "partial build");
    const processObject = new EventEmitter();
    let exitCode = null;
    processObject.exit = (code) => {
      exitCode = code;
    };

    installTemporaryDirectorySignalCleanup(temporaryRoot, processObject);
    processObject.emit(signal);

    assert.equal(exitCode, expectedExitCode);
    assert.equal(processObject.listenerCount("SIGINT"), 0);
    assert.equal(processObject.listenerCount("SIGTERM"), 0);
    assert.throws(() => readFileSync(path.join(temporaryRoot, "incomplete.txt")), /ENOENT/);
  }
});

test("장 산출물은 index, 상대 asset 경로와 Wasm을 요구한다", () => {
  const temporaryRoot = mkdtempSync(path.join(os.tmpdir(), "chapter-build-test-"));
  try {
    const valid = path.join(temporaryRoot, "valid");
    mkdirSync(path.join(valid, "assets"), { recursive: true });
    const html = '<script src="./assets/app.js"></script>';
    writeFileSync(path.join(valid, "index.html"), html);
    writeFileSync(path.join(valid, "assets", "renderer.wasm"), "wasm");
    const summary = validateBuiltChapter(valid);
    assert.equal(summary.fileCount, 2);
    assert.equal(summary.bytes, Buffer.byteLength(html) + 4);
    assert.match(summary.sha256, /^[0-9a-f]{64}$/);

    const absoluteAsset = path.join(temporaryRoot, "absolute");
    mkdirSync(absoluteAsset);
    writeFileSync(absoluteAsset + "/index.html", '<img src="/icon.png">');
    writeFileSync(absoluteAsset + "/renderer.wasm", "wasm");
    assert.throws(() => validateBuiltChapter(absoluteAsset), /루트 절대 asset/);

    const missingWasm = path.join(temporaryRoot, "missing-wasm");
    mkdirSync(missingWasm);
    writeFileSync(missingWasm + "/index.html", "<!doctype html>");
    assert.throws(() => validateBuiltChapter(missingWasm), /Wasm binary/);
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
});
