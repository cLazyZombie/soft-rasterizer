# 장별 정적 실행본과 iframe 런처 결정

## 상태

확정. 장별 구현을 최신 코드의 feature flag로 흉내 내지 않고, 각 장을 완성한 Git commit을 독립된 정적 웹 앱으로 빌드한다.

## 결정

`chapter-manifest.json`은 1–26장의 표시 제목, 전체 40자리 commit SHA와 재현 상태를 기록한다. 현재 빌드는 SHA를 authoritative input으로 사용한다. 이 선택은 아래의 tag + lock 전환안을 적용하기 전까지 유지한다.

`chapter-ui.json`은 현재 저장소가 소유하는 별도 표시 계약이다. 각 장에서 보일 control ID, 통계 `dd` ID와 큰 진단 region만 allowlist로 기록한다. 빌더는 과거 archive의 DOM과 JS 연결을 제거하지 않고 CSS scope를 주입해 계약 밖의 누적 UI만 숨긴다. 따라서 장별 렌더러 source와 lockfile은 그 시점 그대로 재현하면서, 교육용 표시 범위는 과거 26개 commit을 다시 쓰지 않고 한 파일에서 수정할 수 있다. standalone `<title>`과 `<h1>`도 manifest의 장 번호와 제목으로 맞춘다.

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
├── chapter-ui.json
├── build-report.json
└── chapters/01..26/

dist-chapters-test/
└── production과 같은 구조, __AUTOMATION__이 활성화된 test-mode 앱
```

기존 최신장 개발과 회귀 검증은 별도 출력인 `dist-current/`, `dist-current-test/`를 사용한다. 따라서 최신장 E2E가 `/`에서 단일 앱을 여는 계약과 장별 런처의 `/`가 충돌하지 않는다.

## iframe 경계

런처는 `?chapter=16`처럼 장을 선택하고 `./chapters/16/`을 iframe으로 연다. 장마다 달라지는 DOM, event listener, 전역 automation object와 Wasm API는 iframe document 안에 남는다. 런처는 manifest metadata와 iframe URL만 소유한다. 빌더는 Vite 실행 전에 archive의 `web/index.html`에 현재 장의 표시 scope와 제목만 주입하며 JavaScript, Wasm 또는 렌더 결과는 바꾸지 않는다.

모든 경로는 정적 호스팅의 하위 경로에서도 동작하도록 상대 경로를 사용한다. 과거 source의 `/icon.png` 같은 루트 절대 참조는 source를 수정하는 대신 Vite `--base ./` 변환으로 해결하며, 변환 뒤 루트 절대 `src`/`href`가 남으면 빌드를 실패시킨다.

## 3장 예외

3장과 4장은 `afdee4744ebcd70ba063d01d8973d8d573a11b6e` 한 commit에 함께 들어 있다. 별도 snapshot commit이 생기기 전까지 manifest는 3장을 `integrated`로 기록하고 런처에 `3장 — 4장과 통합된 구현`이라고 표시한다. 3장 snapshot은 `8a981041360d9b50b89b1b6f59946e94dfa47ff2` 이후에 3장 framebuffer/Canvas 계약만 적용한 별도 commit으로 만들며 main history를 재작성하지 않는다.

3장 standalone의 제목과 통계 UI는 `chapter-ui.json`에 따라 3장 범위로 보정한다. 다만 같은 commit의 4장 선 렌더링까지 포함되는 한계는 숨기지 않고 `integrated` note를 유지한다.

## revision 이름과 수정 흐름 권고

장별 source를 사람이 다루기 쉽게 만드는 최종 형태는 **움직이는 branch 또는 tag 단독이 아니라, immutable annotated tag + 해석된 SHA lock** 조합이다.

- `chapter-01-v1`, `chapter-01-v2` 같은 annotated tag는 공개된 장 실행본의 이름이자 변경되지 않는 milestone로 쓴다. 기존 tag를 다른 commit으로 강제 이동하지 않는다.
- manifest에는 사람이 수정하기 쉬운 `ref: chapter-01-v2`를 기록하고, 별도 lock에는 그 ref가 해석된 40자리 commit을 기록한다. 빌드는 `ref^{commit}`과 lock의 SHA가 정확히 일치할 때만 진행한다.
- 장 구현을 고칠 때만 해당 tag에서 `maint/chapter-01` 같은 임시 branch를 만들고, 수정 후 새 버전 tag와 lock을 만든다. 26개 장의 장기 branch를 항상 유지하지 않는다.
- branch를 빌드 입력으로 직접 사용하지 않는다. 같은 이름이 새 commit으로 이동해 어제와 오늘의 결과가 달라질 수 있기 때문이다.
- UI 노출 범위만 고치는 일은 역사 source branch를 만들지 않고 현재 `chapter-ui.json`에서 처리한다.

이번 변경에서는 branch/tag 생성이나 manifest schema 전환을 수행하지 않는다. tag 생성, lock migration과 원격 push는 별도 승인 작업이다.

## 검증과 배포 조건

- manifest에는 1–26장이 정확히 한 번씩 있어야 하며 모든 revision은 로컬 Git object로 존재해야 한다.
- UI 정책에도 1–26장이 정확히 한 번씩 있어야 하며 allowlist의 control/stat/region은 해당 archive에 실제로 존재해야 한다.
- 모든 standalone 장은 Wasm 초기화, `data-ready=true`, Canvas 2D 표시, console/page/network error 부재와 WebGL/WebGPU 미사용을 통과해야 한다.
- 모든 standalone 장은 표시된 control/stat/region 집합이 `chapter-ui.json`과 정확히 같아야 한다.
- 대표 경계 장은 고정 viewport와 `dt=0`에서 pixel hash를 고정한다. 26장은 bundled Fox를 명시적으로 로드하고 animation을 고정한 뒤 검사한다.
- 갤러리 report와 최신장 report/artifact 디렉터리는 분리한다.
- 배포 checkout은 manifest의 과거 object를 포함해야 한다. CI에서 shallow clone을 사용한다면 `fetch-depth: 0` 또는 모든 manifest revision을 가져오는 동등한 fetch가 필요하다.
- tag 생성과 원격 push는 빌드 구현과 별도의 승인 작업이다.

## 거부한 대안

- 최신 코드의 장별 feature flag: 이후 장의 데이터 구조와 runtime이 남아 있어 실제 장 재현이 아니다.
- 선택할 때 현재 working tree checkout: 사용자 변경을 위험하게 만들고 정적 배포가 불가능하다.
- 장별 server/port: iframe 정적 산출물보다 운영 복잡도가 크다.
