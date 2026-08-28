# CPU로 만드는 소프트웨어 래스터라이저

WebGL이나 WebGPU에 픽셀 생성을 맡기지 않고, Rust/WebAssembly가 만든 RGBA8 프레임버퍼를 Canvas 2D로 표시하는 소프트웨어 래스터라이저 프로젝트입니다.

현재 저장소는 26장짜리 구현 교재와 확정된 렌더링 계약을 모두 구현합니다. Rust가 소유한 RGBA8/깊이 버퍼와 수학 계층에 열벡터 MVP, LH/+Z 카메라, indexed mesh, homogeneous clipping, 고정소수점 coverage, strict 깊이, perspective-correct texture/lighting, Orbit/Fly 입력, OBJ/GLB import, node animation과 skinning, 투명도 queue, 2x SSAA, mipmap, 단계별 진단과 scalar/tiled reference를 조립했습니다. `PipelineState`의 모든 debug view와 외부 mesh는 같은 Rust coverage/depth 경로를 사용하고 Canvas 2D는 완성된 논리 해상도 framebuffer만 표시합니다.

## 실행 방법

필요한 도구는 Rust stable/nightly, `wasm32-unknown-unknown` target, `wasm-pack`, Node.js와 `pnpm`입니다. 처음 한 번 의존성과 E2E용 Chromium을 설치합니다.

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
pnpm install --frozen-lockfile
pnpm exec playwright install chromium
```

전체 검증까지 실행하려면 coverage와 Rust 중복 검사 도구도 설치합니다.

```bash
rustup toolchain install nightly --component llvm-tools-preview
cargo install cargo-llvm-cov
cargo install --git https://github.com/cLazyZombie/lcov_filter --force
brew install corca-ai/tap/nose
```

기본 개발 명령은 1–26장의 실제 commit을 갤러리로 빌드한 뒤 포트 5173에서 서비스합니다. 첫 실행은 과거 Wasm을 모두 빌드하므로 시간이 걸립니다.

```bash
pnpm run dev
```

터미널에 표시된 주소(기본값 `http://127.0.0.1:5173`)를 브라우저에서 열고 `/?chapter=16`처럼 장을 선택합니다. 현재 HEAD의 dev Wasm과 최신 장 앱만 단독으로 실행하려면 다음 명령을 사용합니다.

```bash
pnpm run dev:current
```

두 명령은 모두 포트 5173을 사용하므로 동시에 실행하지 않습니다.

최신 장 앱의 Canvas 내부 버퍼는 물리 픽셀 수가 아니라 CSS 논리 해상도를 사용합니다. 예를 들어 화면 폭이 물리 1920px이고 `devicePixelRatio`가 2라면 내부 폭은 960px입니다. 창 크기나 화면 배율이 바뀌면 `Renderer::resize`로 색/깊이 버퍼를 함께 다시 만들고 오래된 Wasm `TypedArray` view를 버립니다.

1–26장의 실제 commit을 각각 release Wasm 정적 앱으로 빌드하고 iframe 런처와 함께 `dist/`에 만들려면 다음 명령을 사용합니다. 빌드는 현재 working tree를 checkout하지 않으며 각 revision의 lockfile을 사용합니다.

```bash
pnpm run build
pnpm exec vite preview --config vite.gallery.config.js
```

`/?chapter=16`처럼 직접 접근할 수 있고 각 장의 standalone URL은 `/chapters/16/` 형식입니다. 현재 HEAD만 단독으로 release 빌드하려면 `pnpm run build:current`를 사용하며 출력은 `dist-current/`에 생성됩니다.

주요 검증 명령은 다음과 같습니다.

```bash
pnpm run format:check
pnpm run check:assets
pnpm run check
pnpm run lint
pnpm run test
pnpm run e2e:smoke
pnpm run e2e
pnpm run e2e:chapters
pnpm run e2e:headed
pnpm run check:duplication
pnpm run coverage
pnpm run verify
```

`pnpm run verify`는 frozen install부터 format, 최신장 전체 headless E2E, 1–26장 갤러리 E2E, Rust 중복 검사, 마지막 clean coverage까지 저장소 표준 순서로 실행합니다. 실제 창 lifecycle과 DPR 경계를 확인할 때는 test automation 빌드까지 포함하는 `pnpm run e2e:headed`를 사용합니다. 장별 빌드 구조와 3장 통합 구현 예외는 [장별 정적 실행본 결정](doc/decisions/chapter-static-gallery.md)에 기록되어 있습니다.

Rust coverage 제외는 컴파일러 attribute를 사용하지 않고 source marker로만 표현합니다. 한 줄은 `LCOV_EXCL_LINE`, 최소 범위는 `LCOV_EXCL_START`/`LCOV_EXCL_STOP`, 파일 전체는 첫 번째 비어 있지 않은 줄의 `LCOV_EXCL_FILE`을 사용합니다. `LINE`, `START`, `FILE` marker에는 같은 줄의 사유가 필요합니다. 자세한 계약은 [coverage 제외 결정](doc/decisions/coverage-exclusions.md)을 따릅니다.

## 먼저 읽기

- [전체 과정 소개와 목차](doc/00-들어가며.md)
- [좌표·카메라·깊이 결정](doc/decisions/coordinates.md)
- [최종 Capstone 구현과 성능 보고서](doc/capstone-report.md)
- [GLB runtime loading 구현 가이드](doc/glb-runtime-loading.md)
- [장별 정적 실행본과 iframe 런처 결정](doc/decisions/chapter-static-gallery.md)
- [코딩 에이전트와 장별로 일하는 방법](doc/appendix-a-코딩-에이전트와-장별로-일하는-방법.md)
- [최소 공개 계약과 데이터 구조](doc/appendix-b-최소-공개-계약과-데이터-구조.md)
- [수학과 알고리즘 빠른 참조](doc/appendix-c-수학과-알고리즘-빠른-참조.md)
- [저장소 작업 지침](AGENTS.md)

## 고정 렌더링 규약

- 열벡터와 `p_clip = P * V * M * p_object`
- 왼손 object/world/view 공간: `+X` 오른쪽, `+Y` 위, 카메라 전방 `+Z`
- homogeneous clip: `-w <= x <= w`, `-w <= y <= w`, `0 <= z <= w`
- NDC 깊이 `0..1`, depth clear `+infinity`, strict `<`
- screen y-down, `orient2d > 0` front face, 고정소수점 top-left coverage
- Rust/Wasm이 픽셀을 만들고 Canvas 2D는 완성된 RGBA8 이미지만 표시

세부 수식과 외부 에셋 변환 규칙은 [좌표계 결정 문서](doc/decisions/coordinates.md)를 기준으로 합니다.

## 21장 OBJ import 범위

- 파일은 UTF-8 OBJ, 최대 8 MiB다. JS는 크기와 비동기 파일 읽기를 맡고 Rust가 text parsing과 Mesh 검증을 소유한다.
- 입력 좌표 profile은 이미 내부와 같은 LH `+X` right, `+Y` up, `+Z` forward다. 다른 DCC 축을 추측해서 바꾸지 않는다.
- `v x y z`, `vt u v`, `vn x y z`, `f`를 지원한다. `o`, `g`, `s`, `usemtl`, `mtllib`은 metadata로 무시하며 그 밖의 record는 오류다.
- face token은 `v`, `v/vt`, `v//vn`, `v/vt/vn`과 양수 1-based/음수 상대 index를 지원한다. 최대 8정점의 planar strict-convex face만 fan으로 삼각분할하고 오목·자기교차·비평면 face는 거부한다.
- vertex dedup key는 position/UV/normal index tuple 전체다. OBJ의 texture V는 importer에서 내부 top-left 규약으로 한 번 뒤집는다.
- normal이 없으면 source position 단위로 면적 가중 normal을 생성한다. smoothing group은 아직 해석하지 않으므로 누락 normal은 같은 position에서 smooth하다는 것이 baseline hard-edge 정책이다.
- 참조된 source bounds를 이용해 geometry를 중심 기준 `[-0.75, 0.75]` 범위로 정규화하므로 아주 크거나 작은 유한 모델도 기본 카메라에 들어온다. 원본 bounds는 asset status에 남는다.
- glTF runtime parser는 이 장의 baseline에 포함하지 않는다. 확장 경계에는 Khronos glTF 2.0의 X reflection, triangle winding swap, normal/tangent handedness와 `C*M*C` 행렬 adapter 및 수학 fixture만 둔다.

## 22장 투명도 기준선

- `Material::alpha_mode`는 `Opaque`, `Mask`, `Blend`로 나뉘며 각각 opaque, cutout, transparent queue로 분류한다.
- Opaque와 threshold를 통과한 Mask fragment만 strict depth test 뒤 깊이를 쓴다. Mask discard는 texture alpha를 얻은 뒤 색·깊이를 모두 바꾸지 않는다.
- Blend triangle은 clipping 뒤 보존한 LH view-space `+Z` 대표 깊이를 기준으로 큰 값부터 안정 정렬한다. opaque/cutout 깊이에 대해서는 test하지만 깊이를 쓰지 않는다.
- texture와 material은 straight alpha다. source-over는 destination RGBA8을 linear RGB로 decode해 합성한 뒤 sRGB로 encode하며 framebuffer alpha는 255를 유지한다.
- debug fixture는 cutout checker와 서로 교차하는 두 반투명 quad를 함께 표시한다. primitive 평균 깊이 정렬은 교차 geometry의 fragment 순서를 완전히 해결하지 못하며 OIT는 이 장의 범위가 아니다.
- `FrameStats`는 alpha discard, depth write, blended sample을 분리한다. UI의 encoded-sRGB wrong-way 비교는 같은 coverage/depth와 올바른 linear 경로의 수치·화면 차이를 고정한다.

## 23장 Antialiasing과 Mipmap 기준선

- `NoAa`는 논리 해상도를 그대로 렌더하고 `Ssaa2x`는 Rust 내부에서 가로·세로 2배인 4배 sample target을 사용한다. 공개 Wasm framebuffer 크기와 Canvas/CSS 크기는 논리 해상도로 유지한다.
- 2x2 SSAA sample은 저장된 sRGB RGB를 linear로 decode해 평균하고 최종 RGBA8 쓰기 직전에 다시 encode한다. resolve 깊이는 네 sample의 최솟값이며 출력 alpha는 255다.
- texture upload는 base부터 ceil-half인 `max(1, (width+1)/2) × max(1, (height+1)/2)`를 반복해 1x1까지 mip chain을 Rust가 소유한다. 홀수 extent의 마지막 행/열을 보존하고 base-color RGB는 linear에서 평균한다. base와 모든 mip level의 합이 texture당 최대 texel 수를 넘기 전에 거부한다.
- LOD는 현재 fragment와 screen x/y 한 픽셀 이웃에서 같은 perspective rational UV 식을 평가해 구한다. `rho`의 log2를 유효 mip 범위에 clamp하고 필수 범위인 nearest mip만 선택한다.
- mip debug는 선택 level을 색으로 표시한다. `FrameStats`는 render scale/resolved pixel, mip sample, 최소·최대 level과 invalid LOD를 별도로 보고한다. MSAA와 trilinear는 후속 확장 범위다.

## 24장 진단과 프로파일링 기준선

- perspective-correct UV와 covered-sample Overdraw를 pipeline debug view에 추가했다. Overdraw는 depth test 전에 count하고 geometry 제출이 끝난 뒤 post-view로 표시하므로 geometry/depth count를 바꾸지 않는다.
- Overdraw storage는 해당 mode에서만 target 크기로 할당해 frame마다 재사용한다. `FrameStats`는 `overdrawn_pixels`와 `max_overdraw`를 보고하고 기존 단계 관계식과 함께 검증한다.
- 브라우저는 최근 120 frame의 update/present/total ms를 ring에 보관하고 nearest-rank p50/p95를 표시한다. test-only release benchmark는 warm-up을 제외한 표본과 build/browser/device/DPR/해상도/triangle/sample count를 E2E JSON report에 기록한다.
- 자동 3 warm-up/7 sample은 report regression fixture다. 성능 변경 비교에는 30 warm-up/120 sample 이상과 동일 pixel hash/`FrameStats`를 요구한다. 상세 계약은 [진단과 성능 측정 기준선](doc/decisions/profiling.md)에 있다.

## 25장 Tiled Raster와 Capstone 기준선

- 기본 `Scalar`와 선택 가능한 single-thread `Tiled16`을 모두 safe Rust로 유지한다. Tiled16은 같은 setup을 16×16의 서로 겹치지 않는 pixel 범위로 나누며 triangle 제출 순서, top-left, strict depth와 shading 순서를 바꾸지 않는다.
- UI의 `Shared threads 요청`은 현재 병렬 경로가 아니다. `crossOriginIsolated`, shared-memory Wasm build와 scheduler 조건을 표시하고 어느 하나라도 없으면 Tiled16으로 안전하게 fallback한다.
- `capstone_tiled` E2E는 960×540, cull none, fixed `dt=0`에서 scalar/tiled/fallback exact pixel hash와 단계 count를 비교하고 각 경로를 warm-up 30/표본 120 frame으로 측정한다.
- 현재 측정은 반복·교차 순서나 단계별 timing으로 재현 가능한 speedup을 입증하지 않으므로 Scalar가 기본이다. worker, SIMD와 frame-wide tile bin은 후속 범위이며 자세한 측정 계약과 한계는 [최종 Capstone 보고서](doc/capstone-report.md)에 기록했다.

## 26장 GLB scene과 animation 기준선

- `gltf` crate가 GLB 2.0의 scene/node/mesh/material/skin/animation을 Rust에서 검증한다. JSON `.gltf`, 외부 buffer/image URI와 morph target은 범위 밖이다.
- embedded PNG/JPEG는 브라우저가 RGBA8로 decode한다. prepare/image supply/commit generation을 나누어 실패하거나 stale인 upload가 기존 장면을 파괴하지 않게 한다.
- glTF 오른손 좌표는 importer에서 X reflection, winding swap, `C*M*C`와 quaternion 부호 변환으로 내부 LH/+Z 규약에 맞춘다.
- STEP/LINEAR/CUBICSPLINE node TRS와 4-weight linear blend skinning을 frame마다 평가한다. material별 base color, sampler, double-sided, alpha와 `KHR_materials_unlit`을 기존 pipeline에 연결한다.
- 제품 시작 장면은 attribution과 SHA-256을 고정한 animated Fox GLB다. test automation build는 1–25장 golden을 위해 cube로 시작하고 26장 scenario가 Fox를 명시적으로 로드한다.

## 교재

### 1부 · 픽셀, 메모리, 최소 수학

1. [소프트웨어 래스터라이저가 하는 일](doc/01-소프트웨어-래스터라이저가-하는-일.md)
2. [Rust-Wasm 프로젝트와 역할 경계](doc/02-rust-wasm-프로젝트와-역할-경계.md)
3. [프레임버퍼, 색, Canvas 표시](doc/03-프레임버퍼-색-canvas-표시.md)
4. [픽셀 좌표와 선 그리기](doc/04-픽셀-좌표와-선-그리기.md)
5. [벡터와 행렬을 필요한 만큼만](doc/05-벡터와-행렬을-필요한-만큼만.md)

### 2부 · 3D 변환과 가시성

6. [좌표 공간과 MVP 변환](doc/06-좌표-공간과-mvp-변환.md)
7. [카메라, 원근 투영, 동차좌표 w](doc/07-카메라-원근-투영-동차좌표-w.md)
8. [Mesh, 인덱스, 정점 속성](doc/08-mesh-인덱스-정점-속성.md)
9. [Winding, 퇴화 삼각형, Backface Culling](doc/09-winding-퇴화-삼각형-backface-culling.md)
10. [동차 Clip 공간에서 삼각형 자르기](doc/10-동차-clip-공간에서-삼각형-자르기.md)

### 3부 · 삼각형 래스터라이저의 핵심

11. [Edge 함수, 픽셀 중심, Top-left 규칙](doc/11-edge-함수-픽셀-중심-top-left-규칙.md)
12. [Barycentric 좌표와 속성 보간](doc/12-barycentric-좌표와-속성-보간.md)
13. [깊이 버퍼와 가려짐](doc/13-깊이-버퍼와-가려짐.md)
14. [Perspective-correct 보간](doc/14-perspective-correct-보간.md)
15. [파이프라인 조립과 컬러 3D 큐브](doc/15-파이프라인-조립과-컬러-3d-큐브.md)

### 4부 · 텍스처, 조명, 실제 입력

16. [웹 이미지 입력과 Texture 메모리](doc/16-웹-이미지-입력과-texture-메모리.md)
17. [UV 주소화, Nearest, Bilinear 샘플링](doc/17-uv-주소화-nearest-bilinear-샘플링.md)
18. [법선 변환과 Lambert 조명](doc/18-법선-변환과-lambert-조명.md)
19. [Blinn-Phong과 sRGB/Linear 색 공간](doc/19-blinn-phong과-srgb-linear-색-공간.md)
20. [마우스/키보드와 Orbit/Fly 카메라](doc/20-마우스-키보드와-orbit-fly-카메라.md)

### 5부 · 에셋, 품질, 검증, 성능

21. [외부 모델 로딩: OBJ 기준선과 glTF 확장](doc/21-외부-모델-로딩-obj-기준선과-gltf-확장.md)
22. [투명도, Alpha Test, Blending 순서](doc/22-투명도-alpha-test-blending-순서.md)
23. [Antialiasing과 Mipmap](doc/23-antialiasing과-mipmap.md)
24. [디버그 뷰, 테스트, 프로파일링](doc/24-디버그-뷰-테스트-프로파일링.md)
25. [타일링, Worker, SIMD와 최종 Capstone](doc/25-타일링-worker-simd와-최종-capstone.md)
26. [GLB 장면, Skinning과 Animation](doc/26-glb-장면-skinning-animation.md)

## 부록과 원본

- [부록 A · 코딩 에이전트와 장별로 일하는 방법](doc/appendix-a-코딩-에이전트와-장별로-일하는-방법.md)
- [부록 B · 최소 공개 계약과 데이터 구조](doc/appendix-b-최소-공개-계약과-데이터-구조.md)
- [부록 C · 수학과 알고리즘 빠른 참조](doc/appendix-c-수학과-알고리즘-빠른-참조.md)
- [부록 D · 화면 증상으로 찾는 오류 단계](doc/appendix-d-화면-증상으로-찾는-오류-단계.md)
- [부록 E · 최종 Capstone 평가표](doc/appendix-e-최종-capstone-평가표.md)
- [부록 F · 공식 참고자료](doc/appendix-f-공식-참고자료.md)
- [1–25장 교재 원본 DOCX](doc/software_rasterizer_curriculum_ko.docx) — 26장은 위 Markdown 장과 runtime guide로 추가했습니다.

## 권장 구현 구조

```text
renderer-core/    순수 Rust 수학·장면·클리핑·래스터화·프레임버퍼
renderer-wasm/    renderer-core를 감싸는 얇은 wasm-bindgen adapter
web/              Canvas 2D 표시, rAF, 입력, 파일·이미지 디코딩
tests/            결정적 fixture, golden, browser 통합 검사
doc/decisions/    결과 전체에 영향을 주는 확정 규약
```

구현과 검증 명령은 [저장소 작업 지침](AGENTS.md)의 canonical `pnpm` 진입점에 연결되어 있습니다.
