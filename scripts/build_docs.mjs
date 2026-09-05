import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { Marked } from "marked";

function escapeHtml(value) {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;").replaceAll('"', "&quot;");
}

function filesUnder(directory, prefix = "") {
  return readdirSync(path.join(directory, prefix), { withFileTypes: true }).flatMap((entry) => {
    const relative = path.posix.join(prefix, entry.name);
    if (entry.isDirectory()) return filesUnder(directory, relative);
    if (!entry.isFile()) throw new Error(`문서는 일반 파일이어야 합니다: ${relative}`);
    return [relative];
  }).sort();
}

function encodePath(value) {
  return value.split("/").map(encodeURIComponent).join("/");
}

export function buildDocumentation(repositoryRoot, outputDirectory, chapters) {
  const sourceDirectory = path.join(repositoryRoot, "doc");
  const sourceFiles = new Set(filesUnder(sourceDirectory));
  const markdownFiles = [...sourceFiles].filter((file) => file.endsWith(".md"));
  const assets = new Set();
  const documents = [];
  const docsOutput = path.join(outputDirectory, "docs");
  mkdirSync(docsOutput, { recursive: true });

  for (const file of markdownFiles) {
    const markdown = readFileSync(path.join(sourceDirectory, file), "utf8");
    const heading = /^# (.+)$/m.exec(markdown)?.[1];
    if (!heading) throw new Error(`문서에 H1 제목이 없습니다: doc/${file}`);
    const ids = new Map();
    const parser = new Marked({
      gfm: true,
      walkTokens(token) {
        if (token.type !== "link" && token.type !== "image") return;
        const href = token.href;
        if (/^(?:https?:|mailto:)/i.test(href) || href.startsWith("#")) return;
        const resolved = new URL(href, `https://docs.invalid/doc/${encodePath(file)}`);
        if (resolved.origin !== "https://docs.invalid" || !resolved.pathname.startsWith("/doc/")) {
          throw new Error(`doc/ 밖의 상대 링크입니다: ${file} → ${href}`);
        }
        const target = decodeURIComponent(resolved.pathname.slice(5));
        if (!sourceFiles.has(target)) throw new Error(`깨진 문서 링크입니다: ${file} → ${href}`);
        const renderedTarget = target.endsWith(".md") ? target.replace(/\.md$/, ".html") : target;
        if (!target.endsWith(".md")) assets.add(target);
        token.href = encodePath(path.posix.relative(path.posix.dirname(file), renderedTarget))
          + resolved.search + resolved.hash;
      },
      renderer: {
        link({ href, title, tokens }) {
          const external = /^(?:https?:|mailto:)/i.test(href);
          const target = external ? ' target="_blank" rel="noopener noreferrer"' : "";
          const label = title ? ` title="${escapeHtml(title)}"` : "";
          return `<a href="${escapeHtml(href)}"${label}${target}>${this.parser.parseInline(tokens)}</a>`;
        },
        heading({ tokens, depth }) {
          const text = this.parser.parseInline(tokens);
          const base = text.replace(/<[^>]*>/g, "").toLowerCase()
            .replace(/[^\p{L}\p{N}_\s-]/gu, "").trim().replace(/\s+/g, "-") || "section";
          const count = ids.get(base) ?? 0;
          ids.set(base, count + 1);
          const id = count === 0 ? base : `${base}-${count}`;
          return `<h${depth} id="${escapeHtml(id)}">${text}</h${depth}>\n`;
        },
      },
    });
    const content = parser.parse(markdown);
    const outputFile = file.replace(/\.md$/, ".html");
    const root = path.posix.relative(path.posix.dirname(file), ".") || ".";
    const html = `<!doctype html>
<html lang="ko">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src 'self' https: data:; style-src 'self'; base-uri 'none'; form-action 'none'" />
  <title>${escapeHtml(heading)} · 소프트웨어 래스터라이저</title>
  <link rel="icon" type="image/png" href="${root}/../icon.png" />
  <link rel="stylesheet" href="${root}/../docs.css" />
</head>
<body>
  <nav aria-label="문서 탐색"><a href="${root}/../" target="_top">장별 실행본</a><a href="${encodePath(path.posix.basename(file))}" target="_blank" rel="noopener">Markdown 원문</a></nav>
  <main><article>${content}</article></main>
</body>
</html>
`;
    mkdirSync(path.dirname(path.join(docsOutput, outputFile)), { recursive: true });
    writeFileSync(path.join(docsOutput, outputFile), html);
    writeFileSync(path.join(docsOutput, file), markdown);
    documents.push({
      source: `doc/${file}`,
      title: heading,
      href: `./docs/${encodePath(outputFile)}`,
      sourceSha256: createHash("sha256").update(markdown).digest("hex"),
    });
  }
  for (const asset of assets) {
    const target = path.join(docsOutput, asset);
    mkdirSync(path.dirname(target), { recursive: true });
    copyFileSync(path.join(sourceDirectory, asset), target);
  }
  const chapterDocs = chapters.map(({ number }) => {
    const matches = documents.filter(({ source }) => source.startsWith(`doc/${number}-`));
    if (matches.length !== 1) throw new Error(`${number}장 문서는 doc/${number}-*.md 한 개여야 합니다.`);
    return { number, ...matches[0] };
  });
  const index = documents.find(({ source }) => source.startsWith("doc/00-"));
  if (!index) throw new Error("교재 목차 doc/00-*.md가 필요합니다.");
  const manifest = { schemaVersion: 1, index, chapters: chapterDocs, documents };
  writeFileSync(path.join(outputDirectory, "chapter-docs.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}
