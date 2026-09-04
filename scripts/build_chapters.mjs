import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const SCRIPT_DIRECTORY = path.dirname(fileURLToPath(import.meta.url));
const REPOSITORY_ROOT = path.resolve(SCRIPT_DIRECTORY, "..");
const FULL_COMMIT_SHA = /^[0-9a-f]{40}$/;
const CHAPTER_NUMBER = /^(0[1-9]|1[0-9]|2[0-6])$/;
const UI_ID = /^[a-z][a-z0-9-]*$/;
const UI_REGION = /^[#.][a-z][a-z0-9-]*$/;
const GENERATED_OUTPUT_DIRECTORY = /^dist(?:-[a-z0-9][a-z0-9._-]*)?$/i;
const ROOT_ASSET_REFERENCE = /(?:src|href)=(?:"|')\/(?!\/)/;

function fail(message) {
  throw new Error(message);
}

export function validateManifest(manifest) {
  if (manifest === null || typeof manifest !== "object" || Array.isArray(manifest)) {
    fail("chapter manifest는 JSON object여야 합니다.");
  }
  if (manifest.schemaVersion !== 1) {
    fail("chapter manifest schemaVersion은 1이어야 합니다.");
  }
  if (!CHAPTER_NUMBER.test(manifest.defaultChapter)) {
    fail("chapter manifest defaultChapter가 유효하지 않습니다.");
  }
  if (!Array.isArray(manifest.chapters)) {
    fail("chapter manifest chapters는 배열이어야 합니다.");
  }
  if (manifest.chapters.length !== 26) {
    fail(`chapter manifest는 26개 장을 가져야 합니다: ${manifest.chapters.length}`);
  }

  const numbers = new Set();
  for (const chapter of manifest.chapters) {
    if (chapter === null || typeof chapter !== "object" || Array.isArray(chapter)) {
      fail("각 chapter 항목은 JSON object여야 합니다.");
    }
    if (!CHAPTER_NUMBER.test(chapter.number)) {
      fail(`유효하지 않은 장 번호입니다: ${String(chapter.number)}`);
    }
    if (numbers.has(chapter.number)) {
      fail(`중복된 장 번호입니다: ${chapter.number}`);
    }
    numbers.add(chapter.number);
    if (typeof chapter.title !== "string" || chapter.title.trim() === "") {
      fail(`${chapter.number}장의 title이 비어 있습니다.`);
    }
    if (!FULL_COMMIT_SHA.test(chapter.commit)) {
      fail(`${chapter.number}장의 commit은 전체 40자리 소문자 SHA여야 합니다.`);
    }
    if (!new Set(["exact", "integrated"]).has(chapter.reproduction)) {
      fail(`${chapter.number}장의 reproduction 값이 유효하지 않습니다.`);
    }
    if (
      chapter.reproduction === "integrated" &&
      (typeof chapter.note !== "string" || chapter.note.trim() === "")
    ) {
      fail(`${chapter.number}장의 통합 상태에는 note가 필요합니다.`);
    }
  }

  for (let number = 1; number <= 26; number += 1) {
    const padded = String(number).padStart(2, "0");
    if (!numbers.has(padded)) fail(`chapter manifest에 ${padded}장이 없습니다.`);
  }
  if (!numbers.has(manifest.defaultChapter)) {
    fail("chapter manifest의 기본 장이 chapters에 없습니다.");
  }
  return manifest;
}

function validateUniqueStrings(values, description, pattern) {
  if (!Array.isArray(values)) fail(`${description}는 배열이어야 합니다.`);
  const unique = new Set();
  for (const value of values) {
    if (typeof value !== "string" || !pattern.test(value)) {
      fail(`${description}에 유효하지 않은 값이 있습니다: ${String(value)}`);
    }
    if (unique.has(value)) fail(`${description}에 중복된 값이 있습니다: ${value}`);
    unique.add(value);
  }
  return unique;
}

export function validateUiPolicy(policy) {
  if (policy === null || typeof policy !== "object" || Array.isArray(policy)) {
    fail("chapter UI 정책은 JSON object여야 합니다.");
  }
  if (policy.schemaVersion !== 1) {
    fail("chapter UI 정책 schemaVersion은 1이어야 합니다.");
  }
  const knownRegions = validateUniqueStrings(policy.regions, "chapter UI 공통 regions", UI_REGION);
  if (!Array.isArray(policy.chapters) || policy.chapters.length !== 26) {
    fail(`chapter UI 정책은 26개 장을 가져야 합니다: ${policy.chapters?.length ?? 0}`);
  }

  const numbers = new Set();
  for (const chapter of policy.chapters) {
    if (chapter === null || typeof chapter !== "object" || Array.isArray(chapter)) {
      fail("각 chapter UI 항목은 JSON object여야 합니다.");
    }
    if (!CHAPTER_NUMBER.test(chapter.number)) {
      fail(`chapter UI 장 번호가 유효하지 않습니다: ${String(chapter.number)}`);
    }
    if (numbers.has(chapter.number)) {
      fail(`chapter UI 장 번호가 중복되었습니다: ${chapter.number}`);
    }
    numbers.add(chapter.number);
    validateUniqueStrings(chapter.controls, `${chapter.number}장 controls`, UI_ID);
    validateUniqueStrings(chapter.stats, `${chapter.number}장 stats`, UI_ID);
    const regions = validateUniqueStrings(
      chapter.regions,
      `${chapter.number}장 regions`,
      UI_REGION,
    );
    for (const region of regions) {
      if (!knownRegions.has(region)) {
        fail(`${chapter.number}장의 알 수 없는 UI region입니다: ${region}`);
      }
    }
  }

  for (let number = 1; number <= 26; number += 1) {
    const padded = String(number).padStart(2, "0");
    if (!numbers.has(padded)) fail(`chapter UI 정책에 ${padded}장이 없습니다.`);
  }
  return policy;
}

function selectUiPolicy(policy, selectedNumbers) {
  const selected = new Set(selectedNumbers);
  return {
    ...structuredClone(policy),
    chapters: policy.chapters.filter((chapter) => selected.has(chapter.number)),
  };
}

export function selectManifestChapters(manifest, requestedNumbers) {
  if (requestedNumbers === undefined) return structuredClone(manifest);
  if (requestedNumbers.length === 0) fail("--chapters는 한 개 이상의 장을 선택해야 합니다.");

  const requested = new Set(requestedNumbers);
  if (requested.size !== requestedNumbers.length) fail("--chapters에 중복된 장이 있습니다.");
  for (const number of requested) {
    if (!CHAPTER_NUMBER.test(number)) fail(`--chapters의 장 번호가 유효하지 않습니다: ${number}`);
    if (!manifest.chapters.some((chapter) => chapter.number === number)) {
      fail(`--chapters의 장이 manifest에 없습니다: ${number}`);
    }
  }

  const chapters = manifest.chapters.filter((chapter) => requested.has(chapter.number));
  return {
    ...structuredClone(manifest),
    defaultChapter: requested.has(manifest.defaultChapter)
      ? manifest.defaultChapter
      : chapters.at(-1).number,
    chapters,
  };
}

export function parseArguments(argumentsList) {
  const options = {
    outDir: "dist",
    testOutDir: undefined,
    chapters: undefined,
  };
  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    const value = argumentsList[index + 1];
    if (argument === "--out-dir" || argument === "--test-out-dir" || argument === "--chapters") {
      if (value === undefined || value.startsWith("--")) fail(`${argument} 값이 필요합니다.`);
      index += 1;
      if (argument === "--out-dir") options.outDir = value;
      if (argument === "--test-out-dir") options.testOutDir = value;
      if (argument === "--chapters") {
        options.chapters = value.split(",").map((number) => number.padStart(2, "0"));
      }
    } else {
      fail(`알 수 없는 인자입니다: ${argument}`);
    }
  }
  return options;
}

export function assertSafeOutputDirectory(repositoryRoot, outputDirectory) {
  const resolved = path.resolve(repositoryRoot, outputDirectory);
  const relative = path.relative(repositoryRoot, resolved);
  if (
    relative === "" ||
    relative === "." ||
    relative.startsWith("..") ||
    path.isAbsolute(relative) ||
    path.dirname(relative) !== "." ||
    !GENERATED_OUTPUT_DIRECTORY.test(relative)
  ) {
    fail(`출력 디렉터리는 저장소 최상위의 dist 또는 dist-* 경로여야 합니다: ${resolved}`);
  }
  return resolved;
}

export function assertIndependentOutputDirectories(first, second) {
  const firstToSecond = path.relative(first, second);
  const secondToFirst = path.relative(second, first);
  const isInside = (relative) =>
    relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
  if (isInside(firstToSecond) || isInside(secondToFirst)) {
    fail("production과 test 출력 디렉터리는 서로 겹치지 않아야 합니다.");
  }
}

export function cargoTargetDirectory(temporaryRoot, commit) {
  if (!FULL_COMMIT_SHA.test(commit)) fail("Cargo target을 만들 commit SHA가 유효하지 않습니다.");
  return path.join(temporaryRoot, "cargo-targets", commit);
}

export function installTemporaryDirectorySignalCleanup(
  temporaryRoot,
  processObject = process,
  preserve = false,
  remove = rmSync,
) {
  const exitCodes = new Map([
    ["SIGINT", 130],
    ["SIGTERM", 143],
  ]);
  const handlers = new Map();
  const dispose = () => {
    for (const [signal, handler] of handlers) processObject.off(signal, handler);
  };
  for (const [signal, exitCode] of exitCodes) {
    const handler = () => {
      dispose();
      if (!preserve) remove(temporaryRoot, { recursive: true, force: true });
      processObject.exit(exitCode);
    };
    handlers.set(signal, handler);
    processObject.once(signal, handler);
  }
  return dispose;
}

function run(command, argumentsList, options = {}) {
  const result = spawnSync(command, argumentsList, {
    cwd: options.cwd,
    env: options.env ?? process.env,
    stdio: "inherit",
  });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) {
    fail(`${command} ${argumentsList.join(" ")} 명령이 종료 코드 ${result.status}로 실패했습니다.`);
  }
}

function runCapture(command, argumentsList, options = {}) {
  return execFileSync(command, argumentsList, {
    cwd: options.cwd,
    encoding: "utf8",
  }).trim();
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function sha256File(file) {
  return sha256(readFileSync(file));
}

function matches(html, pattern) {
  return [...html.matchAll(pattern)].map((match) => match[1]);
}

function validateArchivedUiId(id, chapterNumber) {
  if (!UI_ID.test(id)) {
    fail(`${chapterNumber}장 archive에 CSS로 안전하게 처리할 수 없는 UI id가 있습니다: ${id}`);
  }
  return id;
}

function regionExists(html, selector) {
  const [prefix, name] = [selector[0], selector.slice(1)];
  if (prefix === "#") {
    return new RegExp(`\\bid=["']${name}["']`).test(html);
  }
  return matches(html, /\bclass=["']([^"']+)["']/g).some((classes) =>
    classes.split(/\s+/).includes(name),
  );
}

function escapeHtml(text) {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

export function applyChapterUiScope(sourceDirectory, chapter, uiChapter, knownRegions) {
  const indexPath = path.join(sourceDirectory, "web", "index.html");
  let html = readFileSync(indexPath, "utf8");
  if (html.includes("data-chapter-ui-scope=")) {
    fail(`${chapter.number}장 archive에 chapter UI scope가 이미 적용되어 있습니다.`);
  }

  const controlIds = new Set(
    [
      ...matches(html, /<label\b[^>]*\bfor=["']([^"']+)["'][^>]*>/g),
      ...matches(html, /<button\b[^>]*\bid=["']([^"']+)["'][^>]*>/g),
    ].map((id) => validateArchivedUiId(id, chapter.number)),
  );
  const statIds = new Set(
    matches(html, /<dd\b[^>]*\bid=["']([^"']+)["'][^>]*>/g).map((id) =>
      validateArchivedUiId(id, chapter.number),
    ),
  );
  const visibleControls = new Set(uiChapter.controls);
  const visibleStats = new Set(uiChapter.stats);
  const visibleRegions = new Set(uiChapter.regions);

  for (const id of visibleControls) {
    if (!controlIds.has(id)) fail(`${chapter.number}장 archive에 control #${id}가 없습니다.`);
  }
  for (const id of visibleStats) {
    if (!statIds.has(id)) fail(`${chapter.number}장 archive에 stat #${id}가 없습니다.`);
  }
  for (const selector of visibleRegions) {
    if (!regionExists(html, selector)) {
      fail(`${chapter.number}장 archive에 UI region ${selector}가 없습니다.`);
    }
  }

  const hiddenSelectors = [];
  for (const id of controlIds) {
    if (!visibleControls.has(id)) hiddenSelectors.push(`label[for="${id}"]`, `#${id}`);
  }
  for (const id of statIds) {
    if (!visibleStats.has(id)) hiddenSelectors.push(`dt:has(+ dd#${id})`, `dd#${id}`);
  }
  for (const selector of knownRegions) {
    if (!visibleRegions.has(selector)) hiddenSelectors.push(selector);
  }

  const rules = [
    "/* 현재 장의 학습 계약 밖에 있는 누적 UI만 숨긴다. DOM은 과거 JS 연결을 위해 보존한다. */",
  ];
  if (hiddenSelectors.length > 0) {
    rules.push(`${hiddenSelectors.join(",\n")} {\n  display: none !important;\n}`);
  }
  if (visibleControls.size === 0) rules.push(".controls {\n  display: none !important;\n}");
  if (visibleStats.size === 0) rules.push("dl {\n  display: none !important;\n}");
  const scopedStyle = `  <style id="chapter-ui-scope">\n${rules.join("\n\n")}\n  </style>\n`;

  html = html.replace("</head>", `${scopedStyle}</head>`);
  html = html.replace(
    /<html\b/,
    `<html data-chapter-ui-scope="${chapter.number}"`,
  );
  const heading = `${Number(chapter.number)}장 · ${chapter.title}`;
  html = html.replace(/<title>[\s\S]*?<\/title>/, `<title>${escapeHtml(heading)}</title>`);
  html = html.replace(/<h1>[\s\S]*?<\/h1>/, `<h1>${escapeHtml(heading)}</h1>`);
  writeFileSync(indexPath, html);

  return {
    visibleControls: [...visibleControls],
    visibleStats: [...visibleStats],
    visibleRegions: [...visibleRegions],
  };
}

function filesRecursively(directory, relativeDirectory = "") {
  const absoluteDirectory = path.join(directory, relativeDirectory);
  return readdirSync(absoluteDirectory, { withFileTypes: true })
    .flatMap((entry) => {
      const relative = path.join(relativeDirectory, entry.name);
      return entry.isDirectory() ? filesRecursively(directory, relative) : [relative];
    })
    .sort();
}

function directorySummary(directory) {
  const hash = createHash("sha256");
  let bytes = 0;
  const files = filesRecursively(directory);
  for (const file of files) {
    const contents = readFileSync(path.join(directory, file));
    hash.update(file);
    hash.update("\0");
    hash.update(contents);
    bytes += contents.byteLength;
  }
  return { fileCount: files.length, bytes, sha256: hash.digest("hex") };
}

export function validateBuiltChapter(directory) {
  const indexPath = path.join(directory, "index.html");
  if (!existsSync(indexPath)) fail(`장 산출물에 index.html이 없습니다: ${directory}`);
  const html = readFileSync(indexPath, "utf8");
  if (ROOT_ASSET_REFERENCE.test(html)) {
    fail(`장 산출물 index.html에 루트 절대 asset 경로가 남았습니다: ${directory}`);
  }
  const files = filesRecursively(directory);
  if (!files.some((file) => file.endsWith(".wasm"))) {
    fail(`장 산출물에 Wasm binary가 없습니다: ${directory}`);
  }
  return directorySummary(directory);
}

function verifyManifestCommits(manifest, repositoryRoot) {
  for (const chapter of manifest.chapters) {
    const resolved = runCapture("git", ["rev-parse", `${chapter.commit}^{commit}`], {
      cwd: repositoryRoot,
    });
    if (resolved !== chapter.commit) {
      fail(`${chapter.number}장의 commit object가 manifest SHA와 다릅니다: ${resolved}`);
    }
  }
}

function prepareOutput(stageDirectory, manifest, uiPolicy, repositoryRoot) {
  mkdirSync(stageDirectory, { recursive: true });
  cpSync(path.join(repositoryRoot, "launcher"), stageDirectory, { recursive: true });
  copyFileSync(path.join(repositoryRoot, "web", "icon.png"), path.join(stageDirectory, "icon.png"));
  writeFileSync(
    path.join(stageDirectory, "chapter-manifest.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  writeFileSync(
    path.join(stageDirectory, "chapter-ui.json"),
    `${JSON.stringify(uiPolicy, null, 2)}\n`,
  );
  mkdirSync(path.join(stageDirectory, "chapters"), { recursive: true });
}

function extractCommit(repositoryRoot, commit, sourceDirectory, archivePath) {
  mkdirSync(sourceDirectory, { recursive: true });
  run("git", ["archive", "--format=tar", `--output=${archivePath}`, commit], {
    cwd: repositoryRoot,
  });
  run("tar", ["-xf", archivePath, "-C", sourceDirectory]);
  rmSync(archivePath);
}

function assertArchivedBuildContract(sourceDirectory, chapterNumber) {
  for (const relativePath of ["package.json", "pnpm-lock.yaml", "Cargo.lock", "vite.config.js"]) {
    if (!existsSync(path.join(sourceDirectory, relativePath))) {
      fail(`${chapterNumber}장 archive에 ${relativePath} 파일이 없습니다.`);
    }
  }
  const packageJson = JSON.parse(readFileSync(path.join(sourceDirectory, "package.json"), "utf8"));
  if (typeof packageJson.scripts?.["wasm:release"] !== "string") {
    fail(`${chapterNumber}장 package.json에 wasm:release script가 없습니다.`);
  }
}

function buildViteMode(sourceDirectory, outputDirectory, mode, environment) {
  const argumentsList = [
    "exec",
    "vite",
    "build",
    "--config",
    "vite.config.js",
    "--base",
    "./",
    "--outDir",
    outputDirectory,
  ];
  if (mode === "test") argumentsList.push("--mode", "test");
  run("pnpm", argumentsList, { cwd: sourceDirectory, env: environment });
}

const DEFAULT_DIRECTORY_OPERATIONS = {
  exists: existsSync,
  make: mkdirSync,
  move: renameSync,
  remove: rmSync,
};

export function replaceOutputDirectories(
  replacements,
  temporaryRoot,
  operations = DEFAULT_DIRECTORY_OPERATIONS,
) {
  const backups = [];
  const installed = [];
  try {
    for (const [index, replacement] of replacements.entries()) {
      operations.make(path.dirname(replacement.target), { recursive: true });
      if (operations.exists(replacement.target)) {
        const backup = path.join(temporaryRoot, `previous-output-${index}`);
        operations.move(replacement.target, backup);
        backups.push({ backup, target: replacement.target });
      }
    }
    for (const replacement of replacements) {
      operations.move(replacement.stage, replacement.target);
      installed.push(replacement.target);
    }
  } catch (error) {
    for (const target of installed) {
      operations.remove(target, { recursive: true, force: true });
    }
    for (const backup of backups.reverse()) {
      if (operations.exists(backup.backup)) operations.move(backup.backup, backup.target);
    }
    throw error;
  }

  for (const backup of backups) {
    operations.remove(backup.backup, { recursive: true, force: true });
  }
}

export function writeBuildReport(stageDirectory, mode, chapterReports) {
  const report = {
    schemaVersion: 1,
    mode,
    manifestSha256: sha256File(path.join(stageDirectory, "chapter-manifest.json")),
    uiPolicySha256: sha256File(path.join(stageDirectory, "chapter-ui.json")),
    chapterCount: chapterReports.length,
    chapters: chapterReports,
  };
  writeFileSync(path.join(stageDirectory, "build-report.json"), `${JSON.stringify(report, null, 2)}\n`);
}

export function buildChapterGallery(options = {}) {
  const repositoryRoot = options.repositoryRoot ?? REPOSITORY_ROOT;
  const manifestPath = path.join(repositoryRoot, "chapter-manifest.json");
  const sourceManifest = validateManifest(JSON.parse(readFileSync(manifestPath, "utf8")));
  const manifest = selectManifestChapters(sourceManifest, options.chapters);
  const sourceUiPolicy = validateUiPolicy(
    JSON.parse(readFileSync(path.join(repositoryRoot, "chapter-ui.json"), "utf8")),
  );
  const uiPolicy = selectUiPolicy(
    sourceUiPolicy,
    manifest.chapters.map((chapter) => chapter.number),
  );
  const uiByChapter = new Map(uiPolicy.chapters.map((chapter) => [chapter.number, chapter]));
  verifyManifestCommits(manifest, repositoryRoot);

  const productionTarget = assertSafeOutputDirectory(
    repositoryRoot,
    options.outDir ?? "dist",
  );
  const testTarget =
    options.testOutDir === undefined
      ? undefined
      : assertSafeOutputDirectory(repositoryRoot, options.testOutDir);
  if (testTarget !== undefined) {
    assertIndependentOutputDirectories(productionTarget, testTarget);
  }

  const temporaryParent = path.join(repositoryRoot, ".tmp");
  mkdirSync(temporaryParent, { recursive: true });
  const temporaryRoot = mkdtempSync(path.join(temporaryParent, "chapter-gallery-"));
  const keepTemporaryRoot = process.env.SOFT_RASTERIZER_KEEP_CHAPTER_BUILD_TMP === "1";
  const disposeSignalCleanup = installTemporaryDirectorySignalCleanup(
    temporaryRoot,
    process,
    keepTemporaryRoot,
  );
  const productionStage = path.join(temporaryRoot, "production-output");
  const testStage = testTarget === undefined ? undefined : path.join(temporaryRoot, "test-output");
  const productionReports = [];
  const testReports = [];

  try {
    prepareOutput(productionStage, manifest, uiPolicy, repositoryRoot);
    if (testStage !== undefined) prepareOutput(testStage, manifest, uiPolicy, repositoryRoot);

    for (const chapter of manifest.chapters) {
      process.stdout.write(`\n[chapter ${chapter.number}] ${chapter.commit}\n`);
      const sourceDirectory = path.join(temporaryRoot, `source-${chapter.number}`);
      const archivePath = path.join(temporaryRoot, `chapter-${chapter.number}.tar`);
      extractCommit(repositoryRoot, chapter.commit, sourceDirectory, archivePath);
      assertArchivedBuildContract(sourceDirectory, chapter.number);
      const ui = applyChapterUiScope(
        sourceDirectory,
        chapter,
        uiByChapter.get(chapter.number),
        uiPolicy.regions,
      );

      const environment = {
        ...process.env,
        CARGO_TARGET_DIR: cargoTargetDirectory(temporaryRoot, chapter.commit),
      };
      run("pnpm", ["install", "--frozen-lockfile"], { cwd: sourceDirectory, env: environment });
      run("pnpm", ["run", "wasm:release"], { cwd: sourceDirectory, env: environment });

      const productionChapterDirectory = path.join(
        productionStage,
        "chapters",
        chapter.number,
      );
      buildViteMode(sourceDirectory, productionChapterDirectory, "production", environment);
      productionReports.push({
        number: chapter.number,
        commit: chapter.commit,
        pnpmLockSha256: sha256File(path.join(sourceDirectory, "pnpm-lock.yaml")),
        cargoLockSha256: sha256File(path.join(sourceDirectory, "Cargo.lock")),
        ui,
        output: validateBuiltChapter(productionChapterDirectory),
      });

      if (testStage !== undefined) {
        const testChapterDirectory = path.join(testStage, "chapters", chapter.number);
        buildViteMode(sourceDirectory, testChapterDirectory, "test", environment);
        testReports.push({
          number: chapter.number,
          commit: chapter.commit,
          pnpmLockSha256: sha256File(path.join(sourceDirectory, "pnpm-lock.yaml")),
          cargoLockSha256: sha256File(path.join(sourceDirectory, "Cargo.lock")),
          ui,
          output: validateBuiltChapter(testChapterDirectory),
        });
      }
      rmSync(sourceDirectory, { recursive: true, force: true });
    }

    writeBuildReport(productionStage, "production", productionReports);
    const replacements = [{ stage: productionStage, target: productionTarget }];
    if (testStage !== undefined) {
      writeBuildReport(testStage, "test", testReports);
      replacements.push({ stage: testStage, target: testTarget });
    }
    replaceOutputDirectories(replacements, temporaryRoot);
  } finally {
    disposeSignalCleanup();
    if (keepTemporaryRoot) {
      process.stdout.write(`장별 빌드 임시 디렉터리를 보존했습니다: ${temporaryRoot}\n`);
    } else {
      rmSync(temporaryRoot, { recursive: true, force: true });
    }
  }

  return {
    productionTarget,
    testTarget,
    chapterCount: manifest.chapters.length,
  };
}

function main() {
  const options = parseArguments(process.argv.slice(2));
  const result = buildChapterGallery(options);
  process.stdout.write(
    `\n${result.chapterCount}개 장의 production 갤러리를 만들었습니다: ${result.productionTarget}\n`,
  );
  if (result.testTarget !== undefined) {
    process.stdout.write(`test automation 갤러리를 만들었습니다: ${result.testTarget}\n`);
  }
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.stack : String(error)}${os.EOL}`);
    process.exitCode = 1;
  }
}
