# 소프트웨어 래스터라이저 프로젝트 지침

이 파일은 저장소 전체에 적용한다. 이 프로젝트의 목표는 WebGL/WebGPU에 렌더링을 위임하지 않고, Rust/WebAssembly가 만든 RGBA 프레임버퍼를 Canvas 2D로 표시하는 설명 가능하고 검증 가능한 소프트웨어 래스터라이저를 단계적으로 구현하는 것이다.

## 기준 문서

- 전체 구조와 장별 링크: `doc/00-들어가며.md`
- 현재 작업 장의 계약: `doc/01-*.md`부터 `doc/25-*.md`
- 에이전트 작업 범위선: `doc/appendix-a-코딩-에이전트와-장별로-일하는-방법.md`
- 핵심 데이터 구조: `doc/appendix-b-최소-공개-계약과-데이터-구조.md`
- 수학/알고리즘 요약: `doc/appendix-c-수학과-알고리즘-빠른-참조.md`
- 증상별 진단 순서: `doc/appendix-d-화면-증상으로-찾는-오류-단계.md`
- 최종 평가 기준: `doc/appendix-e-최종-capstone-평가표.md`

작업을 시작하기 전에 현재 장과 직접 관련된 문서를 읽는다. 문서 전체를 한 번에 구현 범위로 간주하지 않는다. 현재 코드, 테스트 또는 `docs/decisions/`의 확정된 결정이 교재와 다르면 조용히 한쪽에 맞추지 말고 차이와 영향 범위를 먼저 밝힌다.

## 작업 시작 원칙

1. 현재 브랜치, 작업 트리, 관련 코드, 테스트, 결정 문서와 현재 장의 완료 조건을 확인한다.
2. 이번 작업의 눈에 보이는 산출물, 수정 허용 범위, 금지 범위와 검증 방법을 정한다.
3. 가능하면 실패하는 수치 테스트, 회귀 fixture 또는 관찰 가능한 debug scene을 먼저 만든다.
4. 현재 장의 계약을 만족하는 최소 구현을 한다. 다음 장 기능을 편의상 미리 넣지 않는다.
5. 네이티브 테스트, 결정적 golden, 브라우저 경계 검증 순으로 확인한다.
6. golden과 `FrameStats`가 기준 역할을 할 수 있게 된 뒤에만 구조 변경이나 최적화를 한다.

관련 없는 사용자 변경은 보존한다. 요청 범위 밖의 대규모 재작성, 새 그래픽 API, 불필요한 dependency 또는 버전 고정은 피한다.

## 툴체인과 명령 진입점

- JavaScript/TypeScript package manager와 전체 작업 orchestration에는 `pnpm`만 사용한다. `npm`, `npx`, `yarn`, `bun` 또는 별도 lockfile을 섞지 않는다.
- `package.json`의 `packageManager`와 저장소의 `pnpm-lock.yaml`을 authoritative dependency 계약으로 유지한다. CI와 clean 검증은 `pnpm install --frozen-lockfile`을 사용한다.
- 외부 CLI는 `npx` 대신 `pnpm exec`으로 실행한다.
- 표준 진입점은 `pnpm run format:check`, `pnpm run build`, `pnpm run check`, `pnpm run lint`, `pnpm run test`, `pnpm run e2e:smoke`, `pnpm run e2e`, `pnpm run check:duplication`, `pnpm run coverage`, `pnpm run verify`로 통일한다. 각 script가 필요한 Cargo, Wasm, browser 도구를 호출한다.
- 직접 `cargo` 명령은 focused 진단과 개발 중 반복에 사용할 수 있지만, 완료 판정은 같은 후보에서 canonical pnpm script로 다시 검증한다.
- 초기 scaffold에서 위 script와 실제 경로를 함께 만든다. script나 구현이 아직 없으면 실행했다고 주장하지 말고, 현재 존재하는 계약만 검증한다.

## 아키텍처 계약

권장 workspace 책임은 다음과 같다.

- `renderer-core/`: DOM과 브라우저 타입을 모르는 순수 Rust 라이브러리. 수학, 장면, 변환, 클리핑, 래스터화, 텍스처, 조명과 프레임버퍼를 소유한다. 네이티브 `cargo test`가 가능해야 하며 `web-sys`와 `wasm-bindgen`에 의존하지 않는다.
- `renderer-wasm/`: `renderer-core`를 감싸는 얇은 `wasm-bindgen` adapter. 생성자, 프레임 단위 호출, resize, asset upload와 작은 getter만 노출한다.
- `web/`: DOM/Canvas, 파일과 이미지 디코딩, `requestAnimationFrame`, Pointer/Keyboard 이벤트, UI와 표시를 담당한다. 픽셀 생성이나 삼각형 채우기를 대신하지 않는다.
- `tests/`: 작은 결정적 scene, golden 결과와 브라우저 통합 검사를 둔다.
- `docs/decisions/`: 좌표, 깊이, 색, 샘플링처럼 결과 전체에 영향을 주는 확정 규약을 기록한다.

다음 경계는 유지한다.

- WebGL/WebGPU context를 생성하거나 해당 API를 렌더링 결과 생성에 사용하지 않는다.
- Canvas 2D는 Wasm이 완성한 RGBA8 이미지를 `ImageData`/`putImageData`로 표시하는 장치 계층이다.
- JS-Wasm 호출 수는 프레임 수에 비례해야 한다. 픽셀별 또는 삼각형별 왕복 호출을 만들지 않는다.
- 프레임의 고수준 진입점은 `update_and_render(dt, input)` 같은 단일 호출을 기본으로 한다.
- JS는 rAF timestamp로 `dt`를 계산하고 비정상적으로 큰 값은 제한한다. 한 프레임의 입력은 작은 `InputSnapshot`으로 압축한다.
- `Renderer`가 색/깊이 버퍼를 소유한다. JS의 TypedArray는 빌린 뷰이며 `memory.buffer`, pointer 또는 length가 바뀌면 반드시 다시 만든다.
- resize나 Wasm memory growth 뒤 오래된 pointer/view를 계속 사용하지 않는다.
- 큰 연속 메모리와 작은 통계 snapshot을 사용한다. `FrameStats`에 큰 배열이나 문자열을 넣지 않는다.

## 고정 렌더링 규약

규약을 바꾸려면 관련 결정 문서, 수학 테스트, golden과 브라우저 검증을 같은 변경에서 갱신한다.

### 프레임버퍼와 화면

- 색 버퍼는 row-major RGBA8이다.
- `pixel_index = y * width + x`, `byte_index = 4 * pixel_index`를 사용한다.
- 불투명 clear와 출력의 alpha는 255다. alpha에 sRGB transfer function을 적용하지 않는다.
- CSS 크기와 내부 렌더 해상도를 구분한다. 내부 해상도 변경은 명시적 resize 경로를 거친다.
- `width * height`, `pixel_count * 4`의 overflow와 최대 허용 픽셀 수를 할당 전에 검사한다.

### 좌표와 행렬

- 열벡터를 사용하며 변환 순서는 `p_clip = P * V * M * p_object`다.
- object/world/view 공간은 왼손 좌표계다. `+X`는 오른쪽, `+Y`는 위, `+Z`는 전방이며 view 공간에서 카메라는 `+Z` 방향을 본다.
- 통상적인 algebraic cross 식과 `cross(X, Y) = Z`는 유지한다. 왼손 look-at 기저는 `forward = normalize(target - eye)`, `right = normalize(cross(world_up, forward))`, `up = cross(forward, right)` 순서로 만든다.
- 원근 투영은 left-handed zero-to-one 규약이다. `w_clip = z_view`, `z_clip = f/(f-n) * z_view - f*n/(f-n)`이며 `z_view=near`와 `z_view=far`를 각각 NDC 깊이 0과 1로 보낸다.
- homogeneous clip 범위는 `-w <= x <= w`, `-w <= y <= w`, `0 <= z <= w`다.
- perspective divide 뒤 NDC 깊이는 `0..1`이다.
- 화면은 y-down이고 `screen_y = (0.5 - 0.5 * y_ndc) * height`다.
- object/world/view/clip/NDC/screen 값을 필드명이나 타입으로 구분한다. 서로 다른 공간의 벡터를 암묵적으로 연산하지 않는다.
- NaN/Inf는 조용히 버리지 말고 개발 assertion 또는 오류 통계로 관찰 가능하게 한다.

### 클리핑

- perspective divide 전에 homogeneous clip 공간에서 여섯 평면을 모두 처리한다.
- 평면 거리는 `x+w`, `w-x`, `y+w`, `w-y`, `z`, `w-z`이며 내부는 `distance >= 0`이다.
- 교점은 `t = dA / (dA - dB)`로 구한다.
- `clip_pos`뿐 아니라 world position, normal, UV, color 등 `ClipVertex` 전체를 같은 `t`로 보간한다.
- `inv_w`와 `attribute_over_w`는 clipping을 끝낸 뒤 만든다.
- clipping 결과 polygon은 triangle fan으로 분해한다. 생성된 모든 정점이 여섯 평면 조건을 만족하는지 검사한다.

### Winding, coverage와 top-left

- screen y-down 공간에서 `orient2d(v0, v1, v2) > 0`을 front face로 사용한다. 화면에서 보면 시계 방향이다.
- 양자화 뒤 area가 0인 퇴화 삼각형은 버린다.
- 픽셀 샘플 위치는 `(x + 0.5, y + 0.5)`다.
- equality가 중요한 triangle setup/edge 연산은 고정소수점을 사용한다. 기본 subpixel scale을 바꾸면 결정 문서와 fixture를 갱신한다.
- y-down positive winding에서 포함 edge는 `dy < 0 || (dy == 0 && dx > 0)`이다.
- 임의의 float epsilon으로 top-left 소유 규칙을 흉내 내지 않는다.
- 두 삼각형으로 만든 quad의 모든 내부 샘플 owner count가 정확히 1이어야 한다.

### 보간과 깊이

- barycentric은 `lambda_i = edge_i / area`로 구한다.
- 일반 속성은 `(sum(lambda_i * attribute_i / w_i)) / sum(lambda_i / w_i)`로 복원한다.
- normal은 perspective-correct 보간 뒤 다시 normalize한다.
- `z_ndc`는 화면 공간에서 affine 보간하며 일반 속성처럼 다시 perspective 보정하지 않는다.
- 깊이 버퍼는 색 버퍼와 같은 pixel index를 쓰는 `f32` 배열이다.
- 깊이 clear 값은 `+infinity`, 통과 조건은 유한한 `0..1` 후보에 대한 strict `candidate < stored`다.
- 깊이 검사를 비싼 texture/lighting보다 먼저 한다. NaN 또는 범위를 크게 벗어난 깊이는 `invalid_depth` 같은 통계에 기록한다.

### 텍스처와 색

- 내부 이미지 메모리는 top-to-bottom row-major RGBA8이다.
- 내부 UV는 `u=0`이 왼쪽, `v=0`이 위쪽이다. 다른 에셋 규약은 importer에서 한 번만 변환한다.
- repeat는 `u - floor(u)`를 사용해 음수 UV도 일관되게 처리한다.
- bilinear의 texel-center 규약은 `x = u * width - 0.5`다.
- base-color/sRGB texture는 texel을 linear로 decode한 뒤 filter와 lighting을 수행한다.
- lighting, blending과 MSAA resolve는 linear 공간에서 수행하고 최종 framebuffer 쓰기 직전에 sRGB로 encode한다.
- normal/data texture를 sRGB로 decode하지 않는다. texture/material에 color-space 의미를 보존한다.

## 데이터와 오류 처리

- `RenderTarget`은 크기가 일치하는 color/depth storage를 함께 소유한다.
- `Mesh`는 업로드 시 index 범위와 정점의 유한성을 검증한 뒤 immutable geometry로 사용한다.
- `Texture`는 크기, `width * height * 4`, 입력 byte length와 최대 허용 크기를 검증한다.
- 외부 좌표 규약은 importer에서 내부 왼손 규약으로 한 번만 변환한다. OBJ는 포맷 자체에 handedness가 없으므로 지원 profile을 명시하고, glTF의 오른손 `+Y` up, `+Z` forward, `-X` right 입력은 `C=diag(-1,1,1,1)`로 변환하며 triangle winding과 tangent handedness도 함께 보정한다.
- asset upload byte는 호출 중에만 빌리거나 Rust가 복사해 소유한다. 비동기 JS buffer를 장기 Rust pointer로 보관하지 않는다.
- 잘못된 Mesh/Texture ID는 명시적 오류다. stale handle을 막을 수 있는 ID 정책을 사용한다.
- 업로드 실패는 기존 유효 scene을 파괴하지 않고 UI와 asset status/통계에 원인을 남긴다.
- hot path에서 triangle/fragment별 heap allocation을 만들지 않는다. 임시 polygon, transformed vertex와 tile bin의 capacity를 재사용한다.
- `unsafe`나 SIMD 최적화에는 먼저 동작하는 safe scalar reference, 범위 증명과 pixel diff 증거가 있어야 한다.

## 단계별 범위선

- 1-5장: 프레임버퍼, JS-Wasm 경계, 선/그리드와 최소 수학만 구현한다.
- 6-10장: 변환, 카메라, Mesh, winding과 homogeneous clipping을 구현한다.
- 11-15장: edge/top-left, barycentric, depth, perspective 보간을 조립해 scalar 컬러 큐브를 완성한다.
- 16-20장: 이미지 입력, sampler, 조명, 색 공간과 입력 카메라를 추가한다.
- 21-25장: 외부 에셋, 투명도, AA/mipmap, 진단/프로파일링과 최적화를 추가한다.

특히 다음 gate를 지킨다.

- 11장 전에는 texture/lighting을 구현하지 않는다.
- 15장의 scalar 컬러 큐브와 golden이 통과하기 전 worker/SIMD를 추가하지 않는다.
- 외부 asset parser가 생겨도 core 테스트를 파일 시스템이나 DOM에 의존시키지 않는다.
- thread/SIMD/tiled 경로를 추가해도 scalar reference와 fallback을 제거하지 않는다.
- Shared memory/threads는 `crossOriginIsolated`와 Wasm shared-memory 빌드를 모두 확인하고, 조건이 없으면 안전하게 single-thread 경로로 fallback한다.

## 테스트와 관찰 가능성

테스트는 네 층으로 유지한다.

1. 네이티브 unit test: Vec/Mat, clip distance/intersection, edge/top-left, barycentric, depth, sampler와 color transfer 같은 순수 함수.
2. property/invariant test: 공간/범위/소유 규칙과 제출 순서 독립성, 대각선 선택 독립성, scalar-optimized 동등성.
3. golden test: 외부 asset에 의존하지 않는 32x32 또는 64x64 triangle/quad/cube scene의 RGBA와 필요시 depth/hash.
4. browser smoke/E2E: Wasm 초기화, rAF 입력, resize, stale memory view 재생성, Canvas 표시와 오류 UI.

변경에 맞는 최소 fixture만 돌리고 끝내지 말고, 영향받는 계층까지 검증한다. 테스트 filter를 썼다면 실제 테스트가 하나 이상 선택됐는지 확인한다.

- 테스트를 주석 처리하거나 skip, 더미 데이터 또는 항상 참인 assertion으로 무효화하지 않는다.
- 실제 sleep, 짧은 timeout, 임의 polling 횟수처럼 CPU 부하, 브라우저 속도와 coverage 계측 여부에 흔들리는 테스트를 만들지 않는다. 명시적인 ready/frame/tick signal을 기다린다.
- flaky 실패는 원인을 제거한다. 재실행 우연으로 통과한 결과를 완료 증거로 사용하지 않는다.
- 고정 seed, 내부 해상도, 입력 sequence와 `dt`를 fixture에 기록해 결과를 재현 가능하게 만든다.

필수 회귀 사례에는 다음을 포함한다.

- clear pattern, buffer 길이, alpha와 resize
- M/V/P 순차 적용과 합성 MVP의 동등성, `look_at_lh`의 기저와 `perspective_lh_zo`의 near/far 매핑
- 각 clip plane의 완전 내부/외부/교차 triangle과 속성 보간
- winding, 퇴화 triangle, 화면 경계와 여러 방향의 quad owner count
- triangle 제출 순서를 바꾼 동일 depth 결과
- affine/perspective 비교와 서로 다른 quad 대각선의 seam 부재
- 2x2 texture의 네 모서리, 음수/repeat/clamp UV와 nearest/bilinear
- sRGB encode/decode 기준점과 round-trip
- 왼손 큐브의 outward normal, screen `orient2d > 0` front-face와 culling의 일치
- orbit yaw=0, fly forward/right와 glTF 좌표/winding/normal/tangent 변환
- blur, visibility change와 pointer cancel 뒤 입력 상태 해제

debug view는 최종 화면과 같은 Rust-Wasm-Canvas 경로를 사용한다. 가능한 진단에는 wireframe, triangle ID, barycentric, depth, normal, UV, mip/overdraw와 단계별 `FrameStats`가 있다. debug mode 변경이 의도 없이 geometry/depth count를 바꾸지 않아야 한다.

golden 변경은 자동 승인하지 않는다. 픽셀 수, 위치, 최대 채널 차이와 원인을 설명하고, 의도한 계약 변경일 때만 기준을 갱신한다.

## 브라우저 자동화와 E2E

- browser E2E는 선택 검증이 아니라 코드, asset, build 설정 변경의 필수 완료 gate다. 변경 범위 scenario를 먼저 실행하고 안정화 뒤 전체 headless E2E를 실행한다.
- E2E runner가 local HTTP server, Wasm build, browser process의 시작과 정상 종료, 임시 profile과 artifact 정리를 소유한다. `file://` 실행은 기준 경로로 사용하지 않는다.
- automation 기능은 test-only feature/build에서만 활성화하고 production build에 제어 endpoint를 노출하지 않는다.
- automation API는 deterministic frame advance, 고정 `dt`/seed/내부 해상도, keyboard/pointer 입력, resize, fixture asset upload, debug mode와 quality mode 변경을 제공한다.
- 입력 주입은 실제 JS input collector와 `InputSnapshot`을 통과해야 한다. 카메라나 Renderer 내부 상태를 직접 수정하지 않는다.
- 화면 검증은 실제 `update_and_render -> Wasm memory view -> ImageData -> putImageData` 경로를 통과해야 한다. 테스트 전용 Canvas 채우기나 별도 renderer로 결과를 만들지 않는다.
- 상태 조회는 작은 automation DTO, `FrameStats`, framebuffer generation, pixel/digest hash로 제한한다. 내부 대형 배열을 일반 제어 응답으로 복사하지 않는다.
- console error, page error, unhandled rejection, Wasm panic, 잘못된 framebuffer 범위와 browser process 비정상 종료는 즉시 E2E 실패다.
- 앱이 WebGL/WebGPU context를 요청하지 않았고 최종 표시는 Canvas 2D를 사용했는지 검증한다.
- E2E report에는 실행 후보 식별자, browser/profile, scenario와 step 수, seed, `dt`, 내부/CSS 해상도, `FrameStats`, pixel hash, screenshot/diff 경로와 console log를 기록한다.
- filtered E2E는 scenario와 step이 하나 이상 선택됐는지 확인한다. 전체 실행 결과는 fresh report 하나를 기준으로 판정하며 여러 E2E process가 같은 report/artifact 디렉터리에 동시에 쓰지 않게 한다.
- screenshot baseline은 자동 갱신하지 않는다. 실제 diff와 의도한 contract 변화를 확인한 뒤 관련 baseline만 갱신하고 전체 E2E를 다시 실행한다.

최소 browser scenario는 다음을 포함한다.

- `smoke_boot`: Wasm 초기화, clear pattern, Canvas 2D 표시와 WebGL/WebGPU 미사용
- `resize_memory_view`: resize/memory growth 뒤 stale TypedArray를 버리고 새 뷰를 사용하는지 확인
- `triangle_pipeline`: clip, winding, top-left, depth와 perspective fixture의 pixel/hash 확인
- `texture_color`: 2x2 texture, UV 방향, nearest/bilinear와 sRGB 경로 확인
- `input_camera`: keyboard/pointer, blur/cancel 해제와 orbit/fly 확인
- `asset_failure`: 잘못된 mesh/texture가 기존 유효 scene을 파괴하지 않는지 확인
- `debug_views`: debug mode가 의도 없이 geometry/depth count를 바꾸지 않는지 확인
- `determinism`: 동일 seed/input/frame sequence가 같은 `FrameStats`와 pixel hash를 만드는지 확인

headless browser는 Wasm/Canvas 통합과 결정적 pixel 검증의 기본 경로다. 실제 OS window, pointer 좌표, focus, devicePixelRatio, resize 또는 headed lifecycle이 관련된 변경은 별도 headed E2E를 실행한다. headless 결과를 headed 입력/화면의 증거로 보고하지 않고 두 scenario/step 수를 구분해 보고한다.

## Rust coverage

- coverage 대상은 이 저장소가 소유한 Rust production source 전체다. `renderer-core`, `renderer-wasm`과 이후 추가되는 모든 Rust crate는 100% line coverage를 유지해야 한다.
- JavaScript/TypeScript는 browser와 Wasm을 연결하는 interface 계층이므로 coverage 대상에서 제외한다. 대신 build, lint와 browser E2E로 실제 경계를 검증한다.
- authoritative 완료 표시는 Rust 대상의 `Missing Lines 0`이다. 반올림한 전체 백분율이나 JS/TS를 섞은 aggregate 수치로 100%를 주장하지 않는다.
- Wasm 전용 Rust glue라는 이유만으로 제외하지 않는다. 가능한 로직은 native-test 가능한 Rust로 분리하고, 남은 Rust 경계도 coverage 가능한 test path에서 실행한다.
- coverage는 format, build, lint, unit/property/golden, 전체 E2E와 중복 검사가 모두 끝난 뒤 마지막에 clean 상태로 실행한다. 이전 계측 결과를 재사용하는 `--no-clean`은 사용하지 않는다.
- `pnpm run coverage`는 `cargo +nightly llvm-cov` 기반 canonical script를 호출하고 pipeline 전체에 `pipefail`을 적용해야 한다.
- `Missing Lines`가 나오면 파일과 line을 보고하고 같은 변경에서 의미 있는 정상/실패 경로 테스트를 추가한다. runtime 분기나 오류 처리를 제거해 수치만 맞추지 않는다.
- coverage 제외 annotation은 기본적으로 허용하지 않는다. toolchain 생성 코드처럼 실제로 제어할 수 없는 예외가 필요하면 범위와 사유를 먼저 보고하고 명시적 승인을 받아 최소 범위에만 적용한다.

## Rust 코드 중복 검사

- `nose` 기반 `scripts/check_duplication.sh`를 canonical 검사로 사용하고 `pnpm run check:duplication`에서 호출한다.
- 검사는 프로젝트 소유 Rust crate의 `src/**/*.rs`와 `tests/**/*.rs`를 대상으로 한다. JavaScript/TypeScript, `target/`, 생성 코드, vendored dependency와 외부 checkout은 검사하지 않는다.
- 새 환경에서는 `brew install corca-ai/tap/nose`, 기존 환경에서는 `brew upgrade corca-ai/tap/nose`로 Homebrew stable 최신 `nose`를 사용하며 저장소에 binary 버전을 고정하지 않는다.
- Homebrew 조회가 실패하면 경고 후 로컬 `nose`로 계속할 수 있다. 버전 조회에 성공했는데 최신이 아니면 실패한다. CI 등에서 버전 조회 자체를 건너뛰는 명시적 escape hatch는 `SOFT_RASTERIZER_SKIP_NOSE_VERSION_CHECK=1`로 한정한다.
- 기본 판정은 DS와 같은 `syntax`, `min-size=24`, `min-members=3`, 평균 8줄 이상, 공통 6줄 이상 family 기준을 사용한다.
- `.nose-baseline.json`은 도입 시점의 기존 중복만 허용한다. 검사를 통과시키기 위한 baseline 갱신은 금지한다.
- 새 family 또는 기존 family 변경은 helper/공통 함수 추출로 해결한다. 유지가 불가피할 때만 `nose.ignore.json`에 full `family_id`와 `reason`을 기록하고, 가능하면 `note`, `owner`, `expires_at`도 남긴다.
- 감사 정보가 없는 inline ignore는 사용하지 않는다. baseline은 중복 제거 결과를 고정하거나 판정 규칙을 의도적으로 바꿀 때만 review 후 갱신한다.
- 검사 script 자체의 회귀 테스트도 pnpm script로 실행해 Rust 파일 탐색, 제외 경로, baseline과 version-check 동작을 고정한다. 이 script 테스트는 JS/TS coverage 대상은 아니다.

## 검증 순서

코드, asset 또는 build 설정을 변경했으면 아래 canonical script를 각각 실행하고 결과를 분리해 확인한다. `pnpm run verify`도 이 순서를 보존하며 coverage를 마지막에 실행해야 한다.

```bash
pnpm install --frozen-lockfile
pnpm run format:check
git diff --check
pnpm run check
pnpm run lint
pnpm run test
pnpm run build
pnpm run e2e:smoke
pnpm run e2e
pnpm run check:duplication
pnpm run coverage
```

- 작업 중에는 focused Rust test, golden 또는 E2E scenario를 반복할 수 있지만 완료 전에 전체 순서를 fresh candidate에서 실행한다.
- 실제 OS window/input/DPI 경계가 바뀌면 관련 headed E2E를 `pnpm run e2e` 완료 뒤 추가한다.
- 문서만 변경했으면 Markdown 구조, 참조 경로와 `git diff --check`만 검증하고 코드 gate는 생략할 수 있다. 생략한 항목과 이유를 최종 보고에 적는다.
- 초기 scaffold 전에는 존재하지 않는 pnpm script, Cargo test, E2E 또는 coverage를 실행했다고 주장하지 않는다. scaffold 작업에서 이 검증 계약을 실제 script와 CI에 연결한다.

## 성능 작업

- 정확한 scalar 기준 구현과 profiler 증거 없이 최적화하지 않는다.
- release 빌드에서 warm-up을 분리하고, 해상도, scene/삼각형 수, sample 수, 빌드 모드와 환경을 함께 기록한다.
- 평균 FPS 하나 대신 frame/stage별 p50/p95 시간을 사용한다.
- transform, clip/setup, raster/depth, shade와 present 시간을 분리하고 관련 count를 같이 기록한다.
- 최적화 뒤 이미지가 기준과 달라지면 해당 성능 수치는 폐기한다.
- allocation 제거, incremental edge, early depth와 cached matrix처럼 단순한 병목부터 해결한다.
- tiled 경로는 먼저 single-thread로 scalar 결과와 exact image를 비교한 뒤 병렬화한다.
- worker 수가 달라도 opaque 결과는 결정적이어야 하며 각 tile은 서로 겹치지 않는 color/depth 범위만 쓴다.

## 완료 기준과 보고

구현 완료 보고에는 다음을 포함한다.

- 이번 장/작업에서 고정한 동작과 불변조건
- 바꾼 구조와 공개 API, JS-Wasm 경계의 변화
- 추가한 정상/반례 fixture와 debug scene
- 실행한 네이티브, golden, Wasm build, browser 검증 명령 및 결과
- headless/headed E2E의 scenario/step 수와 report/artifact 경로
- Rust coverage의 대상 line 수와 `Missing Lines 0` 결과
- Rust 중복 검사의 새/변경 family 수와 ignore/baseline 변화 여부
- golden 차이가 있다면 정량 diff와 승인 근거
- 성능 수치가 있다면 측정 조건과 correctness 보존 증거
- 의도적으로 남긴 한계와 다음 장에 넘긴 범위

설명이나 screenshot만으로 완료를 주장하지 않는다. 코드, asset 또는 build 설정 변경은 전체 E2E, Rust `Missing Lines 0`, Rust 중복 검사와 작업 위험에 맞는 실제 브라우저 증거가 모두 통과해야 완료다.
