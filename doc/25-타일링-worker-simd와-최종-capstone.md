# 25장. 타일링, Worker, SIMD와 최종 Capstone

> _정답 이미지를 보존한 상태에서 병목을 줄인다. 데이터 배치, 타일 독립성, 브라우저 보안 조건을 이해한 뒤 병렬화를 선택한다._

> **이번 장의 눈에 보이는 결과**  완성 렌더러가 correctness suite를 유지하면서 최적화 전후 성능 보고서를 만들고, 선택적으로 worker/tile/SIMD 경로를 비교한다.

## 왜 필요한가

software rasterizer는 픽셀 수가 많아 scalar 단일 스레드가 빠르게 한계에 닿는다. 그러나 JS-Wasm 호출, per-fragment allocation, 전체 화면 bounding box 같은 큰 낭비를 먼저 제거하지 않고 thread를 추가하면 복잡성만 늘어난다.

화면을 작은 tile로 나누면 color/depth의 메모리 지역성이 좋아지고, 각 tile을 한 작업자가 독점하게 해 race 없이 병렬화할 수 있다. opaque depth 렌더링은 triangle 제출 순서와 무관하므로 tile 작업 분할에 잘 맞는다.

## 배경지식

- <strong>release 최적화</strong>: opt-level, LTO, 적은 codegen units, panic abort, 선택적 wasm-opt를 적용하되 현재 toolchain과 호환성을 빌드에서 확인한다.
- <strong>allocation 제거</strong>: frame hot path의 Vec 재할당, triangle별 임시 heap, texture lookup 객체 생성을 먼저 없앤다.
- <strong>tile binning</strong>: 예를 들어 16x16 또는 32x32 tile에 triangle setup ID를 넣고 tile bbox 안의 픽셀만 처리한다.
- <strong>SharedArrayBuffer/Wasm threads</strong>는 cross-origin isolation이 필요하다. 일반적으로 COOP: same-origin과 COEP: require-corp 또는 credentialless 같은 응답 헤더 조건을 만족해야 한다.
- <strong>전용 Worker 대안</strong>: main thread가 입력을 전달하고 worker가 Wasm Renderer와 OffscreenCanvas를 소유하면 UI blocking을 줄일 수 있다. Shared Wasm threads보다 단순할 수 있다.
- <strong>SIMD</strong>는 v128로 여러 색/깊이/sample을 함께 처리한다. scalar reference와 bit/pixel 비교를 유지한 채 clear, resolve, color pack처럼 연속 데이터부터 적용한다.

## 핵심 식과 불변조건

```text
tile_x = x / TILE_W, tile_y = y / TILE_H
triangle overlaps tiles from floor(bbox_min/TILE) to floor(bbox_max/TILE)
parallel safety: each tile owns disjoint color/depth sample ranges
speedup은 Amdahl의 법칙에 제한: serial fraction이 남으면 worker 수만 늘려도 선형 증가하지 않는다
```

## 알고리즘과 구현 순서

1. 24장의 profiler로 transform, clip/setup, raster/depth, shade, present 중 실제 병목을 확인한다.
1. release flags와 hot path allocation 제거, incremental edge, early depth, cached matrices를 먼저 적용하고 결과/수치를 비교한다.
1. TriangleSetup을 한 번 만들고 bounding box가 겹치는 tile bin에 ID를 추가한다.
1. 단일 스레드 tile renderer로 scalar 전체 화면 renderer와 exact image가 같은지 확인한다.
1. 필요하면 worker별 tile range를 배정한다. Shared memory를 사용할 때 hosting header와 crossOriginIsolated 검사를 startup gate로 둔다.
1. SIMD는 clear/resolve 또는 4 sample coverage처럼 독립 lane이 자연스러운 곳에 제한적으로 적용하고 scalar fallback을 유지한다.

```text
prepare:
  setups = transform_clip_and_setup_all_triangles()
  bins = array(tile_count)
  for tri_id, setup in setups:
    for tile in tiles_overlapping(setup.bbox):
      bins[tile].push(tri_id)

render tiles:
  for assigned tile:
    for tri_id in bins[tile]:
      rasterize_triangle_clamped_to_tile(setups[tri_id], tile)

gate parallel path:
  if threads requested and not crossOriginIsolated:
    show actionable hosting error
    fall back to single-thread scalar/tiled path
```

## JS-Wasm 경계

main thread JS는 입력/UI와 hosting capability 검사를 맡는다. 전용 Worker 경로에서는 snapshot과 resize/asset 메시지를 보낸다. Wasm thread 경로에서는 Rust/tile scheduler가 shared color/depth를 다루고 JS는 worker bootstrap과 필요한 보안 헤더를 준비한다. 어느 경로든 최종 pixel 생성은 Wasm이다.

## 코딩 에이전트 작업 명세

- 최적화마다 scalar golden suite를 먼저 실행하고, 이미지가 달라지면 성능 수치를 폐기한다.
- single-thread tiled path를 먼저 구현한 뒤 worker 또는 SIMD를 별도 feature flag로 추가한다.
- crossOriginIsolated=false일 때 원인을 설명하고 안전하게 fallback하는 startup 진단을 만든다.
- 최종 README에 열벡터, LH/+Z view, NDC depth 0..1, screen y-down을 포함한 좌표/깊이 규약과 architecture, build/serve, controls, quality modes, test, benchmark 재현법을 정리한다.
- Capstone 보고서에 병목 가설, 변경, correctness 증거, 전후 p50/p95, 메모리, 남은 한계를 기록한다.

## 검증 기준

- scalar, tiled, worker/SIMD 선택 경로의 golden 이미지가 명시한 허용 범위에서 같아야 한다.
- tile 경계에 걸친 triangle이 균열/중복 없이 top-left와 depth 규칙을 유지해야 한다.
- worker 수가 달라도 opaque 결과가 결정적이어야 한다.
- cross-origin isolation이 없는 서버에서도 single-thread fallback으로 렌더러가 동작해야 한다.
- 최종 demo는 resize, 외부 model/texture, orbit/fly, texture/lighting, depth, clipping, debug views를 한 세션에서 보여야 한다.
- 성능 목표는 특정 기기에서 정한 scene/해상도의 p50/p95로 기록하며 보편적인 FPS를 과장하지 않는다.

### 자주 생기는 오류

- tile bin에 같은 triangle을 넣는 것은 정상이다. 하지만 각 tile이 자기 픽셀 범위만 쓰지 않으면 race가 생긴다.
- SharedArrayBuffer가 브라우저에서 보인다는 사실만으로 threads가 준비된 것이 아니다. crossOriginIsolated와 Wasm shared memory 빌드를 함께 확인한다.
- SIMD intrinsics로 scalar보다 복잡한 코드를 먼저 만들지 않는다. profiler가 가리킨 연속 데이터 loop에서 시작한다.
