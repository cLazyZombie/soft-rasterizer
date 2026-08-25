# CPU로 만드는 소프트웨어 래스터라이저

WebGL이나 WebGPU에 픽셀 생성을 맡기지 않고, Rust/WebAssembly가 만든 RGBA8 프레임버퍼를 Canvas 2D로 표시하는 소프트웨어 래스터라이저 프로젝트입니다.

현재 저장소는 25장짜리 구현 교재와 확정된 렌더링 계약을 따라 장별로 구현합니다. 7장까지 Rust가 소유한 RGBA8/깊이 버퍼와 수학 계층에 열벡터 MVP, LH/+Z look-at, zero-to-one 원근 투영, perspective divide와 y-down viewport를 구현했습니다. 회전하는 wireframe 큐브는 이 실제 Rust 파이프라인을 거쳐 Canvas 2D에 표시됩니다.

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

개발 서버는 dev Wasm을 빌드한 뒤 Vite를 실행합니다.

```bash
pnpm run dev
```

터미널에 표시된 주소(기본값 `http://127.0.0.1:5173`)를 브라우저에서 엽니다. Canvas 내부 버퍼는 물리 픽셀 수가 아니라 CSS 논리 해상도를 사용합니다. 예를 들어 화면 폭이 물리 1920px이고 `devicePixelRatio`가 2라면 내부 폭은 960px입니다. 창 크기나 화면 배율이 바뀌면 `Renderer::resize`로 색/깊이 버퍼를 함께 다시 만들고 오래된 Wasm `TypedArray` view를 버립니다.

release Wasm과 정적 웹 파일은 다음 명령으로 `dist/`에 만듭니다.

```bash
pnpm run build
pnpm exec vite preview --config vite.config.js
```

주요 검증 명령은 다음과 같습니다.

```bash
pnpm run format:check
pnpm run check
pnpm run lint
pnpm run test
pnpm run e2e:smoke
pnpm run e2e
pnpm run e2e:headed
pnpm run check:duplication
pnpm run coverage
pnpm run verify
```

`pnpm run verify`는 frozen install부터 format, 전체 headless E2E, Rust 중복 검사, 마지막 clean coverage까지 저장소 표준 순서로 실행합니다. 실제 창 lifecycle과 DPR 경계를 확인할 때는 test automation 빌드까지 포함하는 `pnpm run e2e:headed`를 사용합니다.

Rust coverage 제외는 컴파일러 attribute를 사용하지 않고 source marker로만 표현합니다. 한 줄은 `LCOV_EXCL_LINE`, 최소 범위는 `LCOV_EXCL_START`/`LCOV_EXCL_STOP`, 파일 전체는 첫 번째 비어 있지 않은 줄의 `LCOV_EXCL_FILE`을 사용합니다. `LINE`, `START`, `FILE` marker에는 같은 줄의 사유가 필요합니다. 자세한 계약은 [coverage 제외 결정](doc/decisions/coverage-exclusions.md)을 따릅니다.

## 먼저 읽기

- [전체 과정 소개와 목차](doc/00-들어가며.md)
- [좌표·카메라·깊이 결정](doc/decisions/coordinates.md)
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

## 부록과 원본

- [부록 A · 코딩 에이전트와 장별로 일하는 방법](doc/appendix-a-코딩-에이전트와-장별로-일하는-방법.md)
- [부록 B · 최소 공개 계약과 데이터 구조](doc/appendix-b-최소-공개-계약과-데이터-구조.md)
- [부록 C · 수학과 알고리즘 빠른 참조](doc/appendix-c-수학과-알고리즘-빠른-참조.md)
- [부록 D · 화면 증상으로 찾는 오류 단계](doc/appendix-d-화면-증상으로-찾는-오류-단계.md)
- [부록 E · 최종 Capstone 평가표](doc/appendix-e-최종-capstone-평가표.md)
- [부록 F · 공식 참고자료](doc/appendix-f-공식-참고자료.md)
- [교재 원본 DOCX](doc/software_rasterizer_curriculum_ko.docx)

## 권장 구현 구조

```text
renderer-core/    순수 Rust 수학·장면·클리핑·래스터화·프레임버퍼
renderer-wasm/    renderer-core를 감싸는 얇은 wasm-bindgen adapter
web/              Canvas 2D 표시, rAF, 입력, 파일·이미지 디코딩
tests/            결정적 fixture, golden, browser 통합 검사
doc/decisions/    결과 전체에 영향을 주는 확정 규약
```

구현과 검증 명령은 [저장소 작업 지침](AGENTS.md)의 canonical `pnpm` 진입점에 연결되어 있습니다.
