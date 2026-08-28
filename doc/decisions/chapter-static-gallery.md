# 장별 정적 실행본과 iframe 런처 결정

## 상태

확정. 장별 구현을 최신 코드의 feature flag로 흉내 내지 않고, 각 장을 완성한 Git commit을 독립된 정적 웹 앱으로 빌드한다.

## 결정

`chapter-manifest.json`은 1–26장의 표시 제목, 전체 40자리 commit SHA와 재현 상태를 기록한다. 빌드는 tag가 아니라 SHA를 authoritative input으로 사용한다. 사람이 찾기 위한 `chapter-NN` tag는 나중에 추가할 수 있지만, 빌더는 tag 이동이나 누락의 영향을 받지 않는다.

`pnpm run build`는 현재 작업 트리를 checkout하지 않는다. 각 SHA를 `git archive`로 임시 디렉터리에 풀고 그 revision의 `pnpm-lock.yaml`, `Cargo.lock`, `package.json`과 Vite 설정으로 빌드한다. 모든 archive는 다음 명령 경계를 유지한다.

```text
pnpm install --frozen-lockfile
pnpm run wasm:release
pnpm exec vite build --config vite.config.js --base ./ --outDir <장별 staging>
```

Cargo output은 commit SHA별 디렉터리로 격리한다. 서로 다른 archive가 동일한 crate 이름과 버전을 가지므로 하나의 `CARGO_TARGET_DIR`를 공유하면 과거 source 대신 앞 장의 artifact가 재사용될 수 있다. pnpm content-addressable store는 공유할 수 있지만, Rust build artifact는 revision 사이에 공유하지 않는다.

production과 E2E test-mode 장을 같은 archive에서 연속으로 Vite build한다. Wasm은 한 번만 만든다. 모든 장이 성공하고 상대 asset 경로·`index.html`·Wasm binary 검사가 끝난 뒤에만 완성된 staging을 다음 경로로 옮긴다.

```text
dist/
├── index.html
├── chapter-manifest.json
├── build-report.json
└── chapters/01..26/

dist-chapters-test/
└── production과 같은 구조, __AUTOMATION__이 활성화된 test-mode 앱
```

기존 최신장 개발과 회귀 검증은 별도 출력인 `dist-current/`, `dist-current-test/`를 사용한다. 따라서 최신장 E2E가 `/`에서 단일 앱을 여는 계약과 장별 런처의 `/`가 충돌하지 않는다.

## iframe 경계

런처는 `?chapter=16`처럼 장을 선택하고 `./chapters/16/`을 iframe으로 연다. 장마다 달라지는 DOM, event listener, 전역 automation object와 Wasm API는 iframe document 안에 남는다. 런처는 manifest metadata와 iframe URL만 소유하며 과거 HTML, `<title>`, JavaScript 또는 Wasm 산출물을 후처리하지 않는다.

모든 경로는 정적 호스팅의 하위 경로에서도 동작하도록 상대 경로를 사용한다. 과거 source의 `/icon.png` 같은 루트 절대 참조는 source를 수정하는 대신 Vite `--base ./` 변환으로 해결하며, 변환 뒤 루트 절대 `src`/`href`가 남으면 빌드를 실패시킨다.

## 3장 예외

3장과 4장은 `afdee4744ebcd70ba063d01d8973d8d573a11b6e` 한 commit에 함께 들어 있다. 별도 snapshot commit이 생기기 전까지 manifest는 3장을 `integrated`로 기록하고 런처에 `3장 — 4장과 통합된 구현`이라고 표시한다. 3장 snapshot은 `8a981041360d9b50b89b1b6f59946e94dfa47ff2` 이후에 3장 framebuffer/Canvas 계약만 적용한 별도 commit으로 만들며 main history를 재작성하지 않는다.

## 검증과 배포 조건

- manifest에는 1–26장이 정확히 한 번씩 있어야 하며 모든 revision은 로컬 Git object로 존재해야 한다.
- 모든 standalone 장은 Wasm 초기화, `data-ready=true`, Canvas 2D 표시, console/page/network error 부재와 WebGL/WebGPU 미사용을 통과해야 한다.
- 대표 경계 장은 고정 viewport와 `dt=0`에서 pixel hash를 고정한다. 26장은 bundled Fox를 명시적으로 로드하고 animation을 고정한 뒤 검사한다.
- 갤러리 report와 최신장 report/artifact 디렉터리는 분리한다.
- 배포 checkout은 manifest의 과거 object를 포함해야 한다. CI에서 shallow clone을 사용한다면 `fetch-depth: 0` 또는 모든 manifest revision을 가져오는 동등한 fetch가 필요하다.
- tag 생성과 원격 push는 빌드 구현과 별도의 승인 작업이다.

## 거부한 대안

- 최신 코드의 장별 feature flag: 이후 장의 데이터 구조와 runtime이 남아 있어 실제 장 재현이 아니다.
- 선택할 때 현재 working tree checkout: 사용자 변경을 위험하게 만들고 정적 배포가 불가능하다.
- 장별 server/port: iframe 정적 산출물보다 운영 복잡도가 크다.
