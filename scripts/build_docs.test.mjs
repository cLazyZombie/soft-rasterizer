import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { buildDocumentation } from "./build_docs.mjs";

function fixture(run) {
  const root = mkdtempSync(path.join(os.tmpdir(), "rasterizer-docs-"));
  try {
    mkdirSync(path.join(root, "doc", "assets"), { recursive: true });
    writeFileSync(path.join(root, "doc", "00-목차.md"), "# 교재 목차\n\n[첫 장](01-첫-장.md)\n");
    writeFileSync(path.join(root, "doc", "assets", "example.png"), "image bytes");
    const chapter = path.join(root, "doc", "01-첫-장.md");
    writeFileSync(chapter, "# 첫 장\n\n## 예제\n\n```rust\nlet x = 1 < 2;\n```\n\n| x | y |\n| - | - |\n| 1 | 2 |\n\n![그림](assets/example.png)\n\n[목차](00-목차.md#교재-목차)\n");
    run(root, chapter, path.join(root, "dist"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("문서 하나를 수정해 재빌드하면 HTML과 source hash가 함께 바뀐다", () => fixture((root, chapter, output) => {
  const first = buildDocumentation(root, output, [{ number: "01" }]);
  const htmlPath = path.join(output, "docs", "01-첫-장.html");
  const firstHtml = readFileSync(htmlPath, "utf8");
  assert.match(firstHtml, /<table>/);
  assert.match(firstHtml, /let x = 1 &lt; 2;/);
  assert.match(firstHtml, /00-%EB%AA%A9%EC%B0%A8.html#%EA%B5%90%EC%9E%AC-%EB%AA%A9%EC%B0%A8/);
  assert.equal(readFileSync(path.join(output, "docs/assets/example.png"), "utf8"), "image bytes");
  const updated = "# 바뀐 제목\n\n원본만 수정한 새 설명입니다.\n";
  writeFileSync(chapter, updated);
  const second = buildDocumentation(root, output, [{ number: "01" }]);
  assert.match(readFileSync(htmlPath, "utf8"), /원본만 수정한 새 설명/);
  assert.doesNotMatch(readFileSync(htmlPath, "utf8"), /let x = 1/);
  assert.equal(second.chapters[0].title, "바뀐 제목");
  assert.equal(second.chapters[0].sourceSha256, createHash("sha256").update(updated).digest("hex"));
  assert.notEqual(second.chapters[0].sourceSha256, first.chapters[0].sourceSha256);
  assert.equal(readFileSync(path.join(output, "docs", "01-첫-장.md"), "utf8"), updated);
}));

test("누락되거나 중복된 장 문서와 깨진 링크는 빌드를 실패시킨다", () => fixture((root, chapter, output) => {
  assert.throws(() => buildDocumentation(root, output, [{ number: "02" }]), /02장 문서/);
  writeFileSync(path.join(root, "doc", "01-중복.md"), "# 중복\n");
  assert.throws(() => buildDocumentation(root, output, [{ number: "01" }]), /01장 문서/);
  rmSync(path.join(root, "doc", "01-중복.md"));
  writeFileSync(chapter, "# 첫 장\n\n[없는 문서](missing.md)\n");
  assert.throws(() => buildDocumentation(root, output, [{ number: "01" }]), /깨진 문서 링크/);
  writeFileSync(chapter, "# 첫 장\n\n[바깥](../private.txt)\n");
  assert.throws(() => buildDocumentation(root, output, [{ number: "01" }]), /doc\/ 밖/);
}));

test("중첩 문서의 상대 링크와 한국어 heading ID를 보존한다", () => fixture((root, chapter, output) => {
  mkdirSync(path.join(root, "doc", "decisions"));
  writeFileSync(path.join(root, "doc", "decisions", "rule.md"), "# 규약\n\n[첫 장](../01-첫-장.md)\n\n[공식 문서](https://example.com/docs)\n\n## 같은 제목\n\n## 같은 제목\n");
  buildDocumentation(root, output, [{ number: "01" }]);
  const html = readFileSync(path.join(output, "docs", "decisions", "rule.html"), "utf8");
  assert.match(html, /href="\.\.\/\.\.\/docs.css"/);
  assert.match(html, /href="\.\.\/01-/);
  assert.match(html, /id="같은-제목"/);
  assert.match(html, /id="같은-제목-1"/);
  assert.match(html, /href="https:\/\/example.com\/docs" target="_blank" rel="noopener noreferrer"/);
}));
