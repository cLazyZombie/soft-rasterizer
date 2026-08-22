# 23장. Antialiasing과 Mipmap

> _계단 현상과 texture shimmer는 모두 한 픽셀로 대표하기에 신호가 너무 빠르게 변할 때 생기는 샘플링 문제다._

> **이번 장의 눈에 보이는 결과**  no-AA, 2x SSAA 또는 4-sample MSAA 비교와 mip level debug가 가능하고, 멀리 있는 checker texture의 shimmer가 줄어든다.

## 왜 필요한가

픽셀 중앙 하나만 검사하면 얇은 edge가 프레임마다 나타났다 사라진다. 여러 subpixel sample의 coverage를 평균내면 기하 edge를 부드럽게 표현할 수 있다. 가장 쉬운 기준은 높은 해상도에 렌더한 뒤 줄이는 SSAA다.

texture가 축소될 때 한 화면 픽셀이 많은 texel을 덮는다. base level에서 몇 texel만 고르면 카메라 이동마다 선택이 크게 바뀐다. mipmap은 미리 저해상도 texture를 만들어 footprint에 맞는 level을 선택한다.

## 배경지식

- <strong>2x SSAA</strong>는 가로/세로 2배, 총 4배 픽셀을 렌더하고 2x2를 평균한다. 간단하지만 변환 이후 모든 fragment 비용이 4배에 가깝다.
- <strong>4x MSAA</strong>는 픽셀 안 네 sample마다 coverage/depth를 저장하고 색을 resolve한다. 교육용 reference는 sample별 shading을 해도 되지만 비용은 SSAA와 비슷하다.
- <strong>rotated-grid sample 예</strong>: (0.375,0.125), (0.875,0.375), (0.125,0.625), (0.625,0.875). fixed-point edge 식에 각 offset을 넣는다.
- <strong>mip chain</strong>은 W,H에서 max(1,W/2), max(1,H/2)를 반복해 1x1까지 만든다. sRGB base color는 texel을 linear로 decode해 2x2 평균 후 저장/encode한다.
- <strong>LOD</strong>는 화면 x/y로 한 픽셀 움직였을 때 UV가 texture에서 얼마나 변하는지로 추정한다.

## 핵심 식과 불변조건

```text
SSAA resolve linear color = (c00+c10+c01+c11)/4
mip next(x,y) = average of valid 2x2 source texels in linear space
rho = max(length(dUVdx * texture_size), length(dUVdy * texture_size))
lod = log2(max(rho, epsilon))
trilinear = lerp(sample(level floor(lod)), sample(level ceil(lod)), fract(lod))
```

## 알고리즘과 구현 순서

1. 먼저 2x SSAA를 RenderScale 옵션으로 구현한다. 내부 target을 2배로 렌더하고 linear 평균으로 실제 Canvas 해상도 RGBA8를 만든다.
1. 선택 확장으로 픽셀당 네 sample의 fixed offsets, depth, coverage/color를 가진 MSAA target을 만든다. 각 sample에 동일 top-left edge 규칙을 적용한다.
1. texture upload 시 1x1까지 mip chain을 생성한다. 홀수 크기는 존재하는 source texel만 평균한다.
1. 한 fragment의 UV를 p, p+(1,0), p+(0,1)에서 같은 rational perspective 식으로 평가해 finite difference dUVdx/dUVdy를 얻는다.
1. rho와 lod를 계산해 nearest mip 또는 trilinear를 선택한다. mip level을 색으로 표시하는 debug view를 추가한다.

```text
evaluate_uv_at(edge_values):
  lambdas = edge_values / area
  q = sum(lambda[i] * inv_w[i])
  return sum(lambda[i] * uv_over_w[i]) / q

uv   = evaluate_uv_at(E)
uv_x = evaluate_uv_at(E + edge_step_x)
uv_y = evaluate_uv_at(E + edge_step_y)
dudx = uv_x - uv
dudy = uv_y - uv
rho = max(length(dudx * tex_size), length(dudy * tex_size))
lod = clamp(log2(max(rho, EPS)), 0, max_level)
```

## JS-Wasm 경계

JS는 quality preset과 내부 render scale을 선택하고 Canvas 표시 크기를 유지한다. 모든 sample coverage, mip 생성, LOD, resolve는 Rust에서 수행한다. JS/CSS의 image smoothing은 최종 framebuffer 확대일 뿐 scene antialiasing 대체물이 아니다.

## 코딩 에이전트 작업 명세

- 필수 구현은 2x SSAA와 mip nearest 선택으로 제한하고, MSAA/trilinear는 독립 확장으로 둔다.
- resolve와 mip downsample을 linear RGB에서 수행하는 테스트를 작성한다.
- edge sample 위치를 상수 배열로 정의하고 sample count가 hot loop 전체에 흩어지지 않게 한다.
- quality별 frame time과 shaded sample 수를 overlay에 표시한다.

## 검증 기준

- 단색 영역은 no-AA와 SSAA resolve 후 정확히 같은 색이어야 한다.
- edge pixel의 coverage가 0..1 사이이고 네 sample 중 통과 수와 resolve alpha/색이 일치해야 한다.
- 4x4 texture mip chain이 4x4,2x2,1x1 크기이며 1x1이 전체 linear 평균이어야 한다.
- texture가 멀어질수록 선택 mip level이 단조롭게 증가하는 대표 scene을 검사한다.
- affine가 아닌 perspective UV 평가로 dUV를 구해 기울어진 면에서도 LOD seam이 줄어드는지 확인한다.

### 자주 생기는 오류

- sRGB 바이트를 직접 평균하면 mip과 SSAA edge가 어두워진다. linear에서 평균한다.
- render scale 2는 픽셀 2배가 아니라 4배다. 메모리/시간 UI에 실제 비율을 표시한다.
- MSAA를 구현하면서 sample마다 전체 texture/lighting을 수행하면 비용이 예상보다 크다. 먼저 정확한 reference를 만들고 shade-once 최적화는 뒤에 한다.
