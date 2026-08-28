import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

function commandOutput(command, args) {
  try {
    return execFileSync(command, args, { encoding: "utf8" }).trim();
  } catch {
    return "unavailable";
  }
}

function commandBuffer(command, args) {
  try {
    return execFileSync(command, args);
  } catch {
    return Buffer.from("unavailable");
  }
}

function fileHash(file) {
  try {
    return createHash("sha256").update(readFileSync(file)).digest("hex");
  } catch {
    return "unavailable";
  }
}

function candidateDiffHash() {
  const hash = createHash("sha256");
  hash.update(commandBuffer("git", ["status", "--porcelain=v1", "-z"]));
  hash.update(commandBuffer("git", ["diff", "--binary", "--no-ext-diff"]));
  hash.update(commandBuffer("git", ["diff", "--cached", "--binary", "--no-ext-diff"]));
  const untrackedFiles = commandBuffer("git", ["ls-files", "--others", "--exclude-standard", "-z"])
    .toString("utf8")
    .split("\0")
    .filter(Boolean)
    .sort();
  for (const file of untrackedFiles) {
    hash.update(file);
    hash.update("\0");
    hash.update(readFileSync(file));
  }
  return hash.digest("hex");
}

export default class ChapterReporter {
  constructor() {
    this.startedAt = new Date().toISOString();
    this.results = [];
  }

  onTestEnd(test, result) {
    const annotation = (type) => test.annotations.find((entry) => entry.type === type)?.description;
    const evidenceDescription = annotation("evidence");
    this.results.push({
      scenario: annotation("scenario") ?? test.title,
      steps: Number(annotation("steps") ?? 0),
      project: test.parent.project()?.name ?? "unknown",
      status: result.status,
      durationMs: result.duration,
      evidence: evidenceDescription === undefined ? null : JSON.parse(evidenceDescription),
      errors: result.errors.map((error) => error.message),
      artifacts: result.attachments.map((attachment) => ({
        name: attachment.name,
        path: attachment.path ?? null,
        contentType: attachment.contentType,
      })),
    });
  }

  onEnd(result) {
    const executionMode = process.env.SOFT_RASTERIZER_E2E_MODE ?? "unspecified";
    const report = {
      schemaVersion: 1,
      candidate: {
        head: commandOutput("git", ["rev-parse", "HEAD"]),
        diffSha256: candidateDiffHash(),
        cargoLockSha256: fileHash("Cargo.lock"),
        pnpmLockSha256: fileHash("pnpm-lock.yaml"),
        chapterManifestSha256: fileHash("chapter-manifest.json"),
        chapterBuildReportSha256: fileHash("dist-chapters-test/build-report.json"),
        rustToolchain: commandOutput("rustc", ["--version"]),
      },
      browser: "Playwright Chromium",
      executionMode,
      startedAt: this.startedAt,
      finishedAt: new Date().toISOString(),
      status: result.status,
      scenarioRuns: this.results.length,
      stepRuns: this.results.reduce((sum, entry) => sum + entry.steps, 0),
      reports: this.results,
    };
    const reportDirectory = path.resolve(
      process.env.SOFT_RASTERIZER_E2E_REPORT_DIR ?? "artifacts/e2e",
    );
    mkdirSync(reportDirectory, { recursive: true });
    const serializedReport = `${JSON.stringify(report, null, 2)}\n`;
    writeFileSync(path.join(reportDirectory, `report-${executionMode}.json`), serializedReport);
    writeFileSync(path.join(reportDirectory, "report.json"), serializedReport);
  }
}
