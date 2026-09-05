# 부록 D. 화면 증상으로 찾는 오류 단계

| **증상** | **가장 먼저 볼 단계** | **확인 순서** |
| --- | --- | --- |
| Canvas가 완전히 투명/검정 | 초기화/표시 | alpha=255, ImageData 길이, Wasm init, pointer/view, clear pattern부터 확인 |
| resize 뒤 화면이 멈춤 | 메모리 경계 | memory.buffer identity, pointer/len 변경 뒤 TypedArray 재생성 |
| 모든 면이 사라짐 | culling/winding | culling off, screen area 부호, viewport y flip |
| 카메라 근처에서 화면 폭발 | clipping/w | divide가 clip보다 앞인지, LH에서 w_clip=z_view인지, 앞 z_view&gt;0/뒤 z_view&lt;0인지, clip 공간 near distance=z_clip과 w 유한성 |
| 삼각형 사이 한 픽셀 틈 | coverage | top-left 식, fixed-point 양자화, 공유 edge 방향, quad owner count |
| 멀리 있는 것이 앞에 보임 | depth | z_ndc 0..1, less 비교, clear=Inf, buffer index |
| 텍스처가 대각선에서 꺾임 | 보간 | uv_over_w와 inv_w 분모, clipping 뒤 over_w 생성 |
| 텍스처 상하 반전 | asset/UV | row0/UV v=0 규약, importer 변환, 2x2 모서리 fixture |
| 조명이 카메라 이동에 따라 뒤집힘 | 공간/방향 | normal/L/V의 공간, light 방향 부호, normalize |
| 외부 모델의 면/조명이 반전 | asset 좌표 변환 | OBJ import profile, glTF X reflection, triangle winding, normal/tangent/node 변환을 한 경계에서 적용했는지 |
| 중간톤/edge가 너무 어두움 | 색 공간 | decode -&gt; filter/light/blend -&gt; encode 순서 |
| 키가 계속 눌린 상태 | 입력 | blur/visibilitychange/pointercancel에서 state clear |
| release만 이미지가 다름 | UB/수치 | unsafe 범위, NaN, 초기화 누락, 최적화 전 scalar golden |
