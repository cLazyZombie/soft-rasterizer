# 진단과 성능 측정 기준선

## 목적

24장의 성능 수치는 이미지 정확성을 대신하는 합격 기준이 아니다. 같은 release Wasm 후보와 결정적 scene에서 warm-up을 분리한 뒤 p50/p95를 기록하고, 전후 pixel hash와 `FrameStats` count가 같을 때만 비교 자료로 사용한다.

## 고정 측정 계약

- production build와 E2E test build 모두 Rust는 `wasm-pack --release`로 만든다. 자동 benchmark API는 Vite의 test automation build에만 존재한다.
- JS는 `input snapshot`, Rust `update/render`, Canvas `present`, 전체 frame 단계를 `performance.now()`로 측정한다. UI overlay 갱신 시간은 frame 단계 수치에 포함하지 않는다.
- 최근 120 frame은 고정 크기 ring에 보관한다. p50/p95는 정렬한 표본의 nearest-rank 값이며 평균 FPS로 대체하지 않는다.
- 자동 회귀 fixture는 960x540 논리 해상도, 내장 12-triangle cube, 고정 `dt=0`, warm-up 3 frame, 측정 7 frame을 사용한다. 이 작은 표본은 경계와 report schema를 검증하기 위한 것이며 성능 예산은 아니다.
- 성능 변경 비교에는 warm-up 30 frame 이상, 측정 120 frame을 사용하고 browser/user agent, hardware concurrency, device memory, DPR, 논리/실제 render 해상도, triangle 수, covered/shaded sample 수를 함께 기록한다.
- benchmark 전후 pixel hash가 다르거나 `FrameStats` count가 다르면 그 측정은 무효다.
- raster path 비교는 Scalar와 Tiled16을 같은 model/input/fixed `dt=0` 상태에서 각각 warm-up 30/표본 120 frame 이상 실행한다. tile 전용 count를 제외한 pipeline count와 pixel hash가 exact match해야 한다.
- memory 표본은 logical/supersample color RGBA8와 `f32` depth target만 계산하며 Wasm heap 전체 사용량으로 표현하지 않는다.

## Debug view 계약

- `Solid`, `Wireframe`, `TriangleId`, `Barycentric`, `Depth`, `Normal`, `UV`, `NdotL`, `MipLevel`, `Overdraw`는 같은 transform/clip/cull/setup/coverage/depth 경로를 공유한다.
- UV view는 perspective-correct로 복원한 내부 top-left UV를 표시한다.
- Overdraw는 depth test 전에 각 covered sample의 owner 횟수를 센 뒤, 모든 geometry 제출이 끝난 post-view에서 색을 덮는다. 따라서 최종 제출 순서와 무관하게 count를 표시하면서 기존 depth buffer와 단계별 통계는 보존한다.
- overdraw storage는 해당 mode에서만 render target 크기로 할당하고 frame clear 때 재사용한다. 일반 경로에는 per-fragment allocation을 추가하지 않는다.

## 성능 변경 체크리스트

- [ ] 기준과 변경 후보가 같은 release build/scene/해상도/input/`dt`를 사용한다.
- [ ] warm-up과 측정 frame 수를 기록한다.
- [ ] p50/p95 update, present, total ms와 triangle/covered/shaded count를 기록한다.
- [ ] 기준/변경 pixel hash가 같고 golden/E2E가 통과한다.
- [ ] 환경 메타데이터와 headless/headed 여부를 기록한다.
- [ ] 이미지나 count가 달라졌다면 성능 수치를 폐기하고 correctness 차이를 먼저 설명한다.
