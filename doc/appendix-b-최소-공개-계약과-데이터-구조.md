# 부록 B. 최소 공개 계약과 데이터 구조

아래는 완성 소스가 아니라 모듈 사이의 책임을 고정하는 설계 스케치다. 실제 이름은 저장소 스타일에 맞춰 바꿀 수 있지만, 소유권과 호출 빈도는 유지한다.

## Renderer 상태

- <strong>RenderTarget</strong>: width, height, rgba8 color, f32 depth 또는 선택적 multisample storage.
- <strong>Camera</strong>: LH/+Z position/target 또는 yaw/pitch, fov_y, near, far, `look_at_lh`/`perspective_lh_zo` cache.
- <strong>Scene</strong>: DrawItem 목록. DrawItem은 MeshId, MaterialId, model transform을 참조.
- <strong>GLB Scene</strong>: node/skin/animation과 여러 primitive/material을 소유한다. primitive의 immutable index/기본 vertex와 frame마다 재사용하는 평가 vertex를 분리한다.
- <strong>Mesh</strong>: Vertex 배열과 index 배열. 업로드 시 validation을 마친 immutable geometry.
- <strong>Texture</strong>: mip level 배열, 크기, RGBA8, color space. Material이 SamplerState와 함께 참조.
- <strong>PipelineState</strong>: cull mode, depth test/write, alpha mode, debug mode, quality mode.
- <strong>FrameStats</strong>: 단계별 count와 오류 count. 큰 배열이나 문자열을 포함하지 않는 작은 snapshot.

## 단계별 핵심 타입

```text
Vertex:
  position_object, normal_object, uv, color

ClipVertex:
  clip_pos Vec4
  world_pos, normal_world, uv, color

ScreenVertex:
  x_screen, y_screen, z_ndc
  inv_w
  world_pos_over_w, normal_over_w, uv_over_w, color_over_w

TriangleSetup:
  fixed-point vertices
  bbox, area, edge coefficients, top-left flags
  three ScreenVertex values and material reference

FragmentInput:
  x, y, z_ndc, barycentric
  perspective-correct world_pos, normal, uv, color
```

## 메모리 수명 규칙

- Renderer가 살아 있는 동안 framebuffer Vec을 소유한다. JS view는 빌린 뷰이며 resize/memory growth 뒤 폐기한다.
- asset upload bytes는 호출 중에만 빌리거나 Rust가 복사해 소유한다. 비동기 JS buffer를 Rust pointer로 장기간 참조하지 않는다.
- GLB upload는 prepare, embedded image RGBA 공급, commit generation으로 나눈다. 모든 image와 runtime 구성이 성공하기 전에는 활성 scene과 texture store를 바꾸지 않는다.
- GLB animation은 매 frame base pose에서 다시 시작해 channel을 적용한다. skinned primitive는 mesh node transform을 무시하고 `joint_global * inverse_bind`만 position/normal에 적용한다.
- frame hot path의 temporary polygon, transformed vertices, tile bins는 capacity를 재사용한다.
- Mesh/Texture ID는 index+generation 같은 stale-handle 방지 방식을 선택할 수 있다. 최소 구현에서도 invalid ID는 오류가 되어야 한다.
