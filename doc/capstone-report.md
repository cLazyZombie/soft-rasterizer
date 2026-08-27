# 최종 Capstone 구현과 성능 보고서

이 보고서는 25장 baseline의 범위, correctness 증거와 성능 측정 계약을 기록한다. 범용 FPS나 다른 기기의 성능을 주장하지 않는다.

## 병목 가설과 선택한 변경

기본 cube의 12 input triangle에 비해 125,572 covered sample과 75,292 shaded sample이 있다는 점에서 fragment 구간의 locality가 병목일 수 있다고 가정했다. 현재 profiler는 transform, clip/setup, raster/depth와 shade를 분리하지 않고 Rust update/render 전체를 재므로 이 가설은 단계별 timing으로 확인되지 않았다. 이번 장은 성능 향상을 전제하지 않고 같은 `TriangleSetup`을 서로 겹치지 않는 16×16 pixel 범위로 나누는 single-thread tiled reference를 추가해 가설을 비교 가능하게 만들었다.

- Scalar reference는 기존 row-major incremental edge 경로 그대로다.
- Tiled16은 triangle setup을 한 번만 만들고 bbox가 겹치는 tile을 row-major로 방문한다. 각 tile은 자기 범위만 쓰며 sample의 누락이나 중복이 없다.
- triangle 제출 순서는 바꾸지 않아 strict depth, transparency source order와 wireframe 결과를 보존한다.
- worker, shared Wasm memory와 SIMD는 구현하지 않았다. 요청 UI는 `crossOriginIsolated`, shared-memory build와 scheduler 조건을 설명하고 single-thread tiled로 fallback한다.

## Correctness 증거

- native invariant는 tile 경계 triangle의 scalar/tiled covered sample 집합과 owner 수를 비교한다.
- 64×48 SSAA, texture, mipmap과 lighting cube는 scalar/tiled의 RGBA8, depth와 tile 전용 필드를 제외한 `FrameStats`가 exact match다.
- cutout Mask와 sorted/unsorted Blend fixture도 scalar/tiled의 RGBA8, depth, discard/write/blend count가 exact match다.
- browser 960×540, cull none에서 scalar/tiled/fallback pixel hash는 모두 `10cf841e`다.
- 두 경로 모두 input 12 triangles, covered 125,572, shaded 75,292 sample을 유지한다.
- 1×와 2× DPR project는 같은 내부 960×540 해상도와 hash를 사용한다.

## 성능 측정과 판정

조건은 release Wasm + test automation web, browser/user agent와 device 정보를 report에 기록하고, DPR별 960×540 cull-none cube, fixed `dt=0`, warm-up 30 frame 제외 뒤 120 frame이다. `capstone_tiled`는 다음 p50/p95를 매 fresh candidate의 `artifacts/e2e/report-headless.json`에 기록한다.

| 경로 | update | present | total |
| --- | --- | --- | --- |
| Scalar reference | p50 / p95 | p50 / p95 | p50 / p95 |
| Single-thread Tiled16 | p50 / p95 | p50 / p95 | p50 / p95 |

한 번의 순차 wall-clock 표본은 실행 순서, browser scheduling과 warm cache에 민감하므로 어느 경로가 빠르다는 결론에 사용하지 않는다. 반복·교차 순서 측정과 단계별 timing이 없으므로 기본값은 correctness 기준인 Scalar로 유지한다. Tiled16은 exact reference와 향후 tile-bin/worker 확장의 경계이며 성능 수치를 위해 correctness나 scalar fallback을 제거하지 않는다.

logical color+depth target의 추정 저장 용량은 3.955078125 MiB다. 이 값은 RGBA8 4 byte와 `f32` depth 4 byte만 포함하며 Wasm heap, mesh, texture/mip, temporary clip scratch는 포함하지 않는다. 2× SSAA에서는 별도 4배 color+depth target이 추가된다.

## 재현

```bash
pnpm run e2e
```

`capstone_tiled` scenario가 1×/2× project 각각 scalar와 tiled를 30/120 조건으로 측정하고 `artifacts/e2e/report-headless.json`에 조건, memory estimate, p50/p95, hash와 `FrameStats`를 기록한다. 전체 correctness와 coverage는 `pnpm run verify`로 재현한다.

## 남은 한계

- 현재 tiled 구현은 triangle-local tile traversal이며 frame 전체 triangle ID bin을 만들지 않는다.
- SharedArrayBuffer/Wasm threads, dedicated rendering Worker와 SIMD path는 없다.
- 실제 병렬 경로를 추가할 때는 shared-memory build와 COOP/COEP startup gate, worker 수별 opaque exact image, scalar fallback, tile 독점 write 증거가 먼저 필요하다.
- 25장 baseline 당시 외부 모델은 21장의 OBJ subset이었다. 26장에서 GLB scene/node/material/TRS animation과 skinning을 추가했지만 morph target, PBR, OIT, MSAA와 진짜 trilinear filtering은 여전히 구현 범위 밖이다.
