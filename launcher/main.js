const select = document.querySelector("#chapter-select");
const title = document.querySelector("#chapter-title");
const commit = document.querySelector("#chapter-commit");
const note = document.querySelector("#chapter-note");
let frame = document.querySelector("#chapter-frame");
const frameShell = document.querySelector(".frame-shell");
const standaloneLink = document.querySelector("#standalone-link");
const errorOutput = document.querySelector("#launcher-error");
const readingPanel = document.querySelector("#reading-panel");
const resultButton = document.querySelector("#result-view");
const readingButton = document.querySelector("#reading-view");
let documentFrame = document.querySelector("#document-frame");
const documentLink = document.querySelector("#document-link");
let documentation;

function setReadingView(reading, updateHistory = true) {
  frameShell.hidden = reading;
  readingPanel.hidden = !reading;
  resultButton.setAttribute("aria-pressed", String(!reading));
  readingButton.setAttribute("aria-pressed", String(reading));
  if (updateHistory) {
    const url = new URL(window.location.href);
    if (reading) url.searchParams.set("view", "reading");
    else url.searchParams.delete("view");
    window.history.replaceState(window.history.state, "", url);
  }
}

function chapterLabel(chapter) {
  const suffix = chapter.reproduction === "integrated" ? ` — ${chapter.note}` : "";
  return `${Number(chapter.number)}장 · ${chapter.title}${suffix}`;
}

function chapterUrl(number) {
  return `./chapters/${number}/`;
}

function normalizeRequestedNumber(number) {
  if (typeof number !== "string" || !/^\d{1,2}$/.test(number)) return number;
  const parsed = Number(number);
  return parsed >= 1 && parsed <= 26 ? String(parsed).padStart(2, "0") : number;
}

function replaceChapterFrame(url, frameTitle) {
  const nextFrame = frame.cloneNode(false);
  nextFrame.src = url;
  nextFrame.title = frameTitle;
  frame.replaceWith(nextFrame);
  frame = nextFrame;
}

function selectChapter(manifest, requestedNumber, historyMode = "replace") {
  const normalizedNumber = normalizeRequestedNumber(requestedNumber);
  const chapter =
    manifest.chapters.find((candidate) => candidate.number === normalizedNumber) ??
    manifest.chapters.find((candidate) => candidate.number === manifest.defaultChapter);

  if (chapter === undefined) {
    throw new Error("기본 장을 manifest에서 찾을 수 없습니다.");
  }

  const url = chapterUrl(chapter.number);
  select.value = chapter.number;
  title.textContent = `${Number(chapter.number)}장 · ${chapter.title}`;
  commit.textContent = chapter.commit;
  replaceChapterFrame(
    url,
    `소프트웨어 래스터라이저 ${Number(chapter.number)}장 · ${chapter.title}`,
  );
  standaloneLink.href = url;
  const chapterDocument = documentation.chapters.find((entry) => entry.number === chapter.number);
  if (!chapterDocument) throw new Error(`${chapter.number}장 교재를 찾을 수 없습니다.`);
  const nextDocumentFrame = documentFrame.cloneNode(false);
  nextDocumentFrame.src = chapterDocument.href;
  nextDocumentFrame.title = chapterDocument.title;
  documentFrame.replaceWith(nextDocumentFrame);
  documentFrame = nextDocumentFrame;
  documentLink.href = chapterDocument.href;
  setReadingView(new URL(window.location.href).searchParams.get("view") === "reading", false);

  if (chapter.note === undefined) {
    note.hidden = true;
    note.textContent = "";
  } else {
    note.hidden = false;
    note.textContent = `${Number(chapter.number)}장 — ${chapter.note}`;
  }

  const nextUrl = new URL(window.location.href);
  nextUrl.searchParams.set("chapter", chapter.number);
  window.history[`${historyMode}State`]({ chapter: chapter.number }, "", nextUrl);
  document.documentElement.dataset.selectedChapter = chapter.number;
}

async function bootstrap() {
  const response = await fetch("./chapter-manifest.json");
  if (!response.ok) {
    throw new Error(`chapter manifest를 읽지 못했습니다: HTTP ${response.status}`);
  }

  const manifest = await response.json();
  const docsResponse = await fetch("./chapter-docs.json");
  if (!docsResponse.ok) throw new Error(`교재 목록을 읽지 못했습니다: HTTP ${docsResponse.status}`);
  documentation = await docsResponse.json();
  document.querySelector("#curriculum-link").href = documentation.index.href;
  select.replaceChildren(
    ...manifest.chapters.map((chapter) => {
      const option = document.createElement("option");
      option.value = chapter.number;
      option.textContent = chapterLabel(chapter);
      return option;
    }),
  );
  select.disabled = false;

  const requestedNumber = new URL(window.location.href).searchParams.get("chapter");
  selectChapter(manifest, requestedNumber, "replace");

  select.addEventListener("change", () => selectChapter(manifest, select.value, "push"));
  resultButton.addEventListener("click", () => setReadingView(false));
  readingButton.addEventListener("click", () => setReadingView(true));
  window.addEventListener("popstate", () => {
    const number = new URL(window.location.href).searchParams.get("chapter");
    selectChapter(manifest, number, "replace");
  });

  document.documentElement.dataset.ready = "true";
}

bootstrap().catch((error) => {
  errorOutput.textContent = error instanceof Error ? error.message : String(error);
  errorOutput.hidden = false;
  frameShell.hidden = true;
  readingPanel.hidden = true;
  document.querySelector(".view-toolbar").hidden = true;
  document.documentElement.dataset.ready = "error";
});
