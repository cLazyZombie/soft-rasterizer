const select = document.querySelector("#chapter-select");
const title = document.querySelector("#chapter-title");
const commit = document.querySelector("#chapter-commit");
const note = document.querySelector("#chapter-note");
let frame = document.querySelector("#chapter-frame");
const frameShell = document.querySelector(".frame-shell");
const standaloneLink = document.querySelector("#standalone-link");
const errorOutput = document.querySelector("#launcher-error");

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
  document.documentElement.dataset.ready = "error";
});
