import { spawnSync } from "node:child_process";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const FILE_MARKER = "LCOV_EXCL_FILE";
export const LINE_MARKER = "LCOV_EXCL_LINE";
export const START_MARKER = "LCOV_EXCL_START";
export const STOP_MARKER = "LCOV_EXCL_STOP";

const MARKERS = [FILE_MARKER, LINE_MARKER, START_MARKER, STOP_MARKER];

function markerAtCommentStart(comment, marker) {
  if (!comment.startsWith(marker)) {
    return false;
  }
  const next = comment[marker.length];
  return next === undefined || !/[A-Za-z0-9_]/.test(next);
}

function hasReason(comment, marker) {
  return markerAtCommentStart(comment, marker) && /^\s*--\s*\S/.test(comment.slice(marker.length));
}

function sourceLineNumber(source, index) {
  return source.slice(0, index).split(/\r?\n/).length;
}

function charLiteralEnd(source, start) {
  for (let index = start + 1; index < source.length && source[index] !== "\n"; index += 1) {
    if (source[index] === "\\") {
      index += 1;
    } else if (source[index] === "'") {
      return index;
    }
  }
  return -1;
}

function rustCodeForPolicy(source) {
  const code = [...source];
  let blockDepth = 0;
  let inLineComment = false;
  let inString = false;
  let rawHashes = null;
  let charEnd = -1;

  const blank = (index) => {
    if (code[index] !== "\n" && code[index] !== "\r") {
      code[index] = " ";
    }
  };

  for (let index = 0; index < source.length; index += 1) {
    if (inLineComment) {
      if (source[index] === "\n") {
        inLineComment = false;
      } else {
        blank(index);
      }
      continue;
    }
    if (blockDepth > 0) {
      if (source.startsWith("/*", index)) {
        blank(index);
        blank(index + 1);
        blockDepth += 1;
        index += 1;
      } else if (source.startsWith("*/", index)) {
        blank(index);
        blank(index + 1);
        blockDepth -= 1;
        index += 1;
      } else {
        blank(index);
      }
      continue;
    }
    if (rawHashes !== null) {
      const terminator = `"${"#".repeat(rawHashes)}`;
      blank(index);
      if (source.startsWith(terminator, index)) {
        for (let offset = 1; offset < terminator.length; offset += 1) {
          blank(index + offset);
        }
        rawHashes = null;
        index += terminator.length - 1;
      }
      continue;
    }
    if (inString) {
      blank(index);
      if (source[index] === "\\") {
        blank(index + 1);
        index += 1;
      } else if (source[index] === '"') {
        inString = false;
      }
      continue;
    }
    if (charEnd >= index) {
      blank(index);
      if (index === charEnd) {
        charEnd = -1;
      }
      continue;
    }

    if (source.startsWith("//", index)) {
      blank(index);
      blank(index + 1);
      inLineComment = true;
      index += 1;
    } else if (source.startsWith("/*", index)) {
      blank(index);
      blank(index + 1);
      blockDepth = 1;
      index += 1;
    } else if (source[index] === "r") {
      const rawStart = source.slice(index).match(/^r(#+)?"/);
      if (rawStart) {
        rawHashes = rawStart[1]?.length ?? 0;
        for (let offset = 0; offset < rawStart[0].length; offset += 1) {
          blank(index + offset);
        }
        index += rawStart[0].length - 1;
      }
    } else if (source[index] === '"') {
      blank(index);
      inString = true;
    } else if (source[index] === "'") {
      const end = charLiteralEnd(source, index);
      if (end >= 0) {
        blank(index);
        charEnd = end;
      }
    }
  }
  return code.join("");
}

function rustLineCommentIndexes(source) {
  const lines = source.split(/\r?\n/);
  const indexes = [];
  let blockDepth = 0;
  let inString = false;
  let rawHashes = null;

  for (const line of lines) {
    let lineComment = -1;
    for (let index = 0; index < line.length; index += 1) {
      if (blockDepth > 0) {
        if (line.startsWith("/*", index)) {
          blockDepth += 1;
          index += 1;
        } else if (line.startsWith("*/", index)) {
          blockDepth -= 1;
          index += 1;
        }
        continue;
      }

      if (rawHashes !== null) {
        const terminator = `"${"#".repeat(rawHashes)}`;
        if (line.startsWith(terminator, index)) {
          rawHashes = null;
          index += terminator.length - 1;
        }
        continue;
      }

      if (inString) {
        if (line[index] === "\\") {
          index += 1;
        } else if (line[index] === '"') {
          inString = false;
        }
        continue;
      }

      if (line.startsWith("//", index)) {
        lineComment = index;
        break;
      }
      if (line.startsWith("/*", index)) {
        blockDepth = 1;
        index += 1;
        continue;
      }
      if (line[index] === "r") {
        const rawStart = line.slice(index).match(/^r(#+)?"/);
        if (rawStart) {
          rawHashes = rawStart[1]?.length ?? 0;
          index += rawStart[0].length - 1;
          continue;
        }
      }
      if (line[index] === "'") {
        const end = charLiteralEnd(line, index);
        if (end >= 0) {
          index = end;
          continue;
        }
      }
      if (line[index] === '"') {
        inString = true;
      }
    }
    indexes.push(lineComment);
  }
  return indexes;
}

export function validateCoveragePolicy(source, filePath = "<source>") {
  const issues = [];
  const lines = source.split(/\r?\n/);
  const lineCommentIndexes = rustLineCommentIndexes(source);
  const firstNonEmptyLine = lines.findIndex((line) => line.trim().length > 0) + 1;
  const fileMarkerLines = [];
  let lineMarkerCount = 0;
  let sectionCount = 0;
  let openSectionLine = null;
  const policyCode = rustCodeForPolicy(source);

  for (const match of policyCode.matchAll(/\bcoverage\s*\(\s*(?:r#)?off\s*\)/gs)) {
    issues.push(
      `${filePath}:${sourceLineNumber(source, match.index)}: coverage(off)는 허용하지 않는다`,
    );
  }

  for (const [index, line] of lines.entries()) {
    const lineNumber = index + 1;
    const commentIndex = lineCommentIndexes[index];
    const comment = commentIndex >= 0 ? line.slice(commentIndex + 2).trimStart() : "";
    const hasFile = markerAtCommentStart(comment, FILE_MARKER);
    const hasLine = markerAtCommentStart(comment, LINE_MARKER);
    const hasStart = markerAtCommentStart(comment, START_MARKER);
    const hasStop = markerAtCommentStart(comment, STOP_MARKER);
    const mentionedMarkers = MARKERS.filter((marker) => line.includes(marker));

    if (mentionedMarkers.filter((marker) => marker !== FILE_MARKER).length > 1) {
      issues.push(`${filePath}:${lineNumber}: 한 줄에 narrow coverage marker를 둘 이상 쓸 수 없다`);
    }
    for (const marker of mentionedMarkers) {
      if (!markerAtCommentStart(comment, marker)) {
        issues.push(`${filePath}:${lineNumber}: ${marker}는 실제 // 주석 marker여야 한다`);
      }
    }
    if (hasFile) {
      fileMarkerLines.push(lineNumber);
      if (line.slice(0, commentIndex).trim().length > 0) {
        issues.push(`${filePath}:${lineNumber}: ${FILE_MARKER}은 독립된 주석이어야 한다`);
      }
      if (!hasReason(comment, FILE_MARKER)) {
        issues.push(`${filePath}:${lineNumber}: ${FILE_MARKER} 뒤에 -- 사유가 필요하다`);
      }
    }
    if (hasLine) {
      lineMarkerCount += 1;
      if (line.slice(0, commentIndex).trim().length === 0) {
        issues.push(`${filePath}:${lineNumber}: ${LINE_MARKER}은 제외할 코드와 같은 줄이어야 한다`);
      }
      if (!hasReason(comment, LINE_MARKER)) {
        issues.push(`${filePath}:${lineNumber}: ${LINE_MARKER} 뒤에 -- 사유가 필요하다`);
      }
      if (openSectionLine !== null) {
        issues.push(`${filePath}:${lineNumber}: section 안에서 ${LINE_MARKER}을 중복 사용했다`);
      }
    }
    if (hasStart) {
      if (line.slice(0, commentIndex).trim().length > 0) {
        issues.push(`${filePath}:${lineNumber}: ${START_MARKER}는 독립된 주석이어야 한다`);
      }
      if (!hasReason(comment, START_MARKER)) {
        issues.push(`${filePath}:${lineNumber}: ${START_MARKER} 뒤에 -- 사유가 필요하다`);
      }
      if (openSectionLine !== null) {
        issues.push(
          `${filePath}:${lineNumber}: ${START_MARKER}가 ${openSectionLine}행 section 안에 중첩됐다`,
        );
      } else {
        openSectionLine = lineNumber;
        sectionCount += 1;
      }
    }
    if (hasStop) {
      if (line.slice(0, commentIndex).trim().length > 0) {
        issues.push(`${filePath}:${lineNumber}: ${STOP_MARKER}는 독립된 주석이어야 한다`);
      }
      if (openSectionLine === null) {
        issues.push(`${filePath}:${lineNumber}: 시작 marker 없는 ${STOP_MARKER}다`);
      } else {
        openSectionLine = null;
      }
    }
  }

  if (openSectionLine !== null) {
    issues.push(`${filePath}:${openSectionLine}: 닫히지 않은 ${START_MARKER}다`);
  }
  if (fileMarkerLines.length > 1) {
    issues.push(`${filePath}: ${FILE_MARKER}은 파일당 한 번만 사용할 수 있다`);
  }
  if (fileMarkerLines.length === 1 && fileMarkerLines[0] !== firstNonEmptyLine) {
    issues.push(`${filePath}:${fileMarkerLines[0]}: ${FILE_MARKER}은 첫 번째 비어 있지 않은 줄이어야 한다`);
  }
  if (fileMarkerLines.length > 0 && (lineMarkerCount > 0 || sectionCount > 0)) {
    issues.push(`${filePath}: ${FILE_MARKER}과 line/section marker를 함께 사용할 수 없다`);
  }

  return {
    issues,
    counts: {
      files: fileMarkerLines.length,
      lines: lineMarkerCount,
      sections: sectionCount,
    },
  };
}

export function workspacePackageDirectories(metadata) {
  const workspaceMembers = new Set(metadata.workspace_members);
  return metadata.packages
    .filter((pkg) => workspaceMembers.has(pkg.id))
    .map((pkg) => path.dirname(pkg.manifest_path))
    .sort();
}

async function collectRustFilesUnder(directory, excludedDirectories) {
  const files = [];
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error?.code === "ENOENT") {
      return files;
    }
    throw error;
  }
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory() && excludedDirectories.has(entryPath)) {
      continue;
    }
    if (entry.isDirectory()) {
      files.push(...(await collectRustFilesUnder(entryPath, excludedDirectories)));
    } else if (entry.isFile() && entry.name.endsWith(".rs")) {
      files.push(entryPath);
    }
  }
  return files;
}

export async function collectOwnedRustFiles(metadata) {
  const files = new Set();
  for (const packageDirectory of workspacePackageDirectories(metadata)) {
    const excludedDirectories = new Set([
      path.join(packageDirectory, ".git"),
      path.join(packageDirectory, "target"),
    ]);
    for (const file of await collectRustFilesUnder(packageDirectory, excludedDirectories)) {
      files.add(file);
    }
  }
  return [...files].sort();
}

function loadCargoMetadata(repositoryRoot) {
  const result = spawnSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
    cwd: repositoryRoot,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(result.stderr.trim() || "cargo metadata 실행에 실패했다");
  }
  return JSON.parse(result.stdout);
}

export async function checkRepository(repositoryRoot) {
  const metadata = loadCargoMetadata(repositoryRoot);
  const files = await collectOwnedRustFiles(metadata);
  if (files.length === 0) {
    throw new Error("검사할 workspace Rust source가 없다");
  }

  const totals = { files: 0, lines: 0, sections: 0 };
  const issues = [];
  for (const file of files) {
    const source = await readFile(file, "utf8");
    const result = validateCoveragePolicy(source, path.relative(repositoryRoot, file));
    issues.push(...result.issues);
    totals.files += result.counts.files;
    totals.lines += result.counts.lines;
    totals.sections += result.counts.sections;
  }
  return { files, issues, totals };
}

async function main() {
  const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
  const repositoryRoot = path.resolve(scriptDirectory, "..");
  const result = await checkRepository(repositoryRoot);
  if (result.issues.length > 0) {
    for (const issue of result.issues) {
      console.error(issue);
    }
    process.exitCode = 1;
    return;
  }
  console.log(
    `coverage exclusion policy: ${result.files.length} Rust files, ` +
      `file=${result.totals.files}, line=${result.totals.lines}, sections=${result.totals.sections}`,
  );
}

if (path.resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  await main();
}
