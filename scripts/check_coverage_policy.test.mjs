import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  collectOwnedRustFiles,
  validateCoveragePolicy,
  workspacePackageDirectories,
} from "./check_coverage_policy.mjs";

test("marker가 없는 Rust source를 허용한다", () => {
  const result = validateCoveragePolicy("pub fn covered() {}\n", "src/lib.rs");

  assert.deepEqual(result.issues, []);
  assert.deepEqual(result.counts, { files: 0, lines: 0, sections: 0 });
});

test("사유가 있는 line과 section marker를 허용한다", () => {
  const source = `
pub fn boundary() {
    unreachable!(); // LCOV_EXCL_LINE -- rustc가 실행 불가능한 region을 만든다.
    // LCOV_EXCL_START -- 실제 OS window가 필요한 경계다.
    platform_call();
    // LCOV_EXCL_STOP
}
`;
  const result = validateCoveragePolicy(source, "src/boundary.rs");

  assert.deepEqual(result.issues, []);
  assert.deepEqual(result.counts, { files: 0, lines: 1, sections: 1 });
});

test("첫 줄의 사유가 있는 file marker를 허용한다", () => {
  const source = "// LCOV_EXCL_FILE -- 생성된 platform glue다.\nfn main() {}\n";
  const result = validateCoveragePolicy(source, "src/generated.rs");

  assert.deepEqual(result.issues, []);
  assert.deepEqual(result.counts, { files: 1, lines: 0, sections: 0 });
});

test("coverage off attribute를 multiline과 cfg_attr 변형까지 거부한다", () => {
  for (const source of [
    "#[coverage(off)]\nfn hidden() {}\n",
    "#![cfg_attr(coverage_nightly, coverage( off ))]\n",
    "#[coverage(\n    off\n)]\nfn hidden() {}\n",
    "#![cfg_attr(coverage_nightly, coverage(\n    off\n))]\n",
    "#[coverage(/* reason */ off)]\nfn hidden() {}\n",
    "#[coverage /* reason */ (off)]\nfn hidden() {}\n",
    "#![cfg_attr(coverage_nightly, coverage(/* reason */ off))]\n",
    "#[coverage(r#off)]\nfn hidden() {}\n",
    "#![cfg_attr(coverage_nightly, coverage(r#off))]\n",
  ]) {
    const result = validateCoveragePolicy(source, "src/lib.rs");
    assert.ok(result.issues.some((issue) => issue.includes("coverage(off)")));
  }
});

test("문자열과 주석의 coverage(off) 문구는 attribute로 오인하지 않는다", () => {
  for (const source of [
    'const NOTE: &str = "coverage(off)";\n',
    "// coverage(off)는 금지다.\npub fn covered() {}\n",
    "/* coverage(off)는 금지다. */\npub fn covered() {}\n",
  ]) {
    const result = validateCoveragePolicy(source, "src/lib.rs");
    assert.ok(!result.issues.some((issue) => issue.includes("coverage(off)")));
  }
});

test("line marker의 문자열 위장과 독립 주석을 거부한다", () => {
  const stringMarker = validateCoveragePolicy(
    'const NOTE: &str = "LCOV_EXCL_LINE -- reason";\n',
    "src/string.rs",
  );
  const standalone = validateCoveragePolicy(
    "// LCOV_EXCL_LINE -- 다음 줄을 제외하지 않는다.\nunreachable!();\n",
    "src/standalone.rs",
  );

  assert.ok(stringMarker.issues.some((issue) => issue.includes("실제 // 주석")));
  assert.ok(standalone.issues.some((issue) => issue.includes("같은 줄")));
});

test("char와 byte-char 뒤의 실제 line marker를 허용한다", () => {
  for (const source of [
    `let quote = '"'; // LCOV_EXCL_LINE -- OS 경계다.\n`,
    `let quote = b'"'; // LCOV_EXCL_LINE -- OS 경계다.\n`,
    `let quote = '\\''; // LCOV_EXCL_LINE -- OS 경계다.\n`,
  ]) {
    const result = validateCoveragePolicy(source, "src/char.rs");
    assert.deepEqual(result.issues, []);
    assert.equal(result.counts.lines, 1);
  }
});

test("file marker의 위치와 사유와 narrow marker 혼용을 거부한다", () => {
  const late = validateCoveragePolicy(
    "fn before() {}\n// LCOV_EXCL_FILE -- 너무 늦다.\n",
    "src/late.rs",
  );
  const noReason = validateCoveragePolicy("// LCOV_EXCL_FILE\nfn main() {}\n", "src/no_reason.rs");
  const mixed = validateCoveragePolicy(
    "// LCOV_EXCL_FILE -- 전체 파일이다.\nfn main() {} // LCOV_EXCL_LINE -- 중복이다.\n",
    "src/mixed.rs",
  );

  assert.ok(late.issues.some((issue) => issue.includes("첫 번째 비어 있지 않은 줄")));
  assert.ok(noReason.issues.some((issue) => issue.includes("-- 사유")));
  assert.ok(mixed.issues.some((issue) => issue.includes("함께 사용할 수 없다")));
});

test("잘못된 section 구조를 거부한다", () => {
  const unclosed = validateCoveragePolicy(
    "// LCOV_EXCL_START -- 닫히지 않는다.\nplatform_call();\n",
    "src/unclosed.rs",
  );
  const orphan = validateCoveragePolicy("// LCOV_EXCL_STOP\n", "src/orphan.rs");
  const nested = validateCoveragePolicy(
    "// LCOV_EXCL_START -- 바깥이다.\n// LCOV_EXCL_START -- 안쪽이다.\n// LCOV_EXCL_STOP\n",
    "src/nested.rs",
  );

  assert.ok(unclosed.issues.some((issue) => issue.includes("닫히지 않은")));
  assert.ok(orphan.issues.some((issue) => issue.includes("시작 marker 없는")));
  assert.ok(nested.issues.some((issue) => issue.includes("중첩")));
});

test("workspace package의 custom target을 포함하고 생성 경로는 제외한다", async (context) => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "soft-rasterizer-coverage-policy-"));
  context.after(() => rm(temporaryDirectory, { recursive: true, force: true }));
  const ownedDirectory = path.join(temporaryDirectory, "owned");
  const dependencyDirectory = path.join(temporaryDirectory, "dependency");
  await mkdir(path.join(ownedDirectory, "src", "nested"), { recursive: true });
  await mkdir(path.join(ownedDirectory, "src", "target"), { recursive: true });
  await mkdir(path.join(ownedDirectory, "tests"), { recursive: true });
  await mkdir(path.join(ownedDirectory, "examples"), { recursive: true });
  await mkdir(path.join(ownedDirectory, "lib"), { recursive: true });
  await mkdir(path.join(ownedDirectory, "tools"), { recursive: true });
  await mkdir(path.join(ownedDirectory, "scripts"), { recursive: true });
  await mkdir(path.join(ownedDirectory, "target"), { recursive: true });
  await mkdir(path.join(ownedDirectory, ".git", "hooks"), { recursive: true });
  await mkdir(path.join(dependencyDirectory, "src"), { recursive: true });
  await writeFile(path.join(ownedDirectory, "Cargo.toml"), "[package]\nname='owned'\nversion='0.1.0'\n");
  await writeFile(path.join(dependencyDirectory, "Cargo.toml"), "[package]\nname='dep'\nversion='0.1.0'\n");
  await writeFile(path.join(ownedDirectory, "src", "lib.rs"), "pub fn owned() {}\n");
  await writeFile(path.join(ownedDirectory, "src", "nested", "mod.rs"), "pub fn nested() {}\n");
  await writeFile(path.join(ownedDirectory, "src", "target", "platform.rs"), "pub fn platform() {}\n");
  await writeFile(path.join(ownedDirectory, "tests", "integration.rs"), "#[test] fn test_it() {}\n");
  await writeFile(path.join(ownedDirectory, "examples", "demo.rs"), "fn main() {}\n");
  await writeFile(path.join(ownedDirectory, "build.rs"), "fn main() {}\n");
  await writeFile(path.join(ownedDirectory, "lib", "custom.rs"), "pub fn custom() {}\n");
  await writeFile(path.join(ownedDirectory, "tools", "custom_bin.rs"), "fn main() {}\n");
  await writeFile(path.join(ownedDirectory, "scripts", "custom_build.rs"), "fn main() {}\n");
  await writeFile(path.join(ownedDirectory, "target", "generated.rs"), "fn generated() {}\n");
  await writeFile(path.join(ownedDirectory, ".git", "hooks", "ignored.rs"), "fn ignored() {}\n");
  await writeFile(path.join(dependencyDirectory, "src", "lib.rs"), "pub fn dependency() {}\n");
  const metadata = {
    workspace_members: ["owned 0.1.0"],
    packages: [
      { id: "owned 0.1.0", manifest_path: path.join(ownedDirectory, "Cargo.toml") },
      { id: "dep 0.1.0", manifest_path: path.join(dependencyDirectory, "Cargo.toml") },
    ],
  };

  assert.deepEqual(workspacePackageDirectories(metadata), [ownedDirectory]);
  const files = await collectOwnedRustFiles(metadata);
  assert.deepEqual(files, [
    path.join(ownedDirectory, "build.rs"),
    path.join(ownedDirectory, "examples", "demo.rs"),
    path.join(ownedDirectory, "lib", "custom.rs"),
    path.join(ownedDirectory, "scripts", "custom_build.rs"),
    path.join(ownedDirectory, "src", "lib.rs"),
    path.join(ownedDirectory, "src", "nested", "mod.rs"),
    path.join(ownedDirectory, "src", "target", "platform.rs"),
    path.join(ownedDirectory, "tests", "integration.rs"),
    path.join(ownedDirectory, "tools", "custom_bin.rs"),
  ]);
});
