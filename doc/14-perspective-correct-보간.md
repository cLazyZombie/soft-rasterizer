# 14장. Perspective-correct 보간

> _화면에서 반듯한 보간과 3D 표면에서 반듯한 보간은 다르다. 1/w가 그 차이를 복원한다._

> **이번 장의 눈에 보이는 결과**  기울어진 사각형의 체크무늬가 affine 모드에서는 휘고 perspective-correct 모드에서는 원근에 맞게 곧게 보이는 비교 화면이 나온다.

## 왜 필요한가

screen barycentric으로 UV를 그대로 보간하면 카메라에 가까운 부분과 먼 부분이 같은 screen 비율로 섞인다. 실제 3D 표면에서의 선형 관계가 투영으로 비선형이 되었기 때문에 텍스처가 헤엄치거나 대각선 seam이 보인다.

각 정점 속성을 w로 나눈 값과 1/w를 screen에서 affine 보간한 뒤 다시 나누면 투영 전 선형 속성을 복원할 수 있다. 같은 분모를 UV, normal, color, world position에 재사용한다.

## 배경지식

- <strong>inv_w = 1/w_clip</strong>는 clipping과 perspective divide 뒤 정점마다 계산한다.
- <strong>attribute over w</strong>: uv_over_w = uv \* inv_w처럼 저장한다.
- **분모** q = λ0\*inv_w0 + λ1\*inv_w1 + λ2\*inv_w2다.
- **복원** a = (Σ λi \* ai \* inv_wi) / q다.
- <strong>normal</strong>도 perspective-correct 보간한 뒤 반드시 normalize한다. 선형 보간은 단위 길이를 보존하지 않는다.
- <strong>z_ndc 예외</strong>: depth는 13장의 screen-affine z_ndc를 유지한다. clip z를 일반 속성처럼 중복 보정하지 않는다.

## 핵심 식과 불변조건

```text
q = λ0/w0 + λ1/w1 + λ2/w2
a = (λ0*a0/w0 + λ1*a1/w1 + λ2*a2/w2) / q
uv = uv_over_w_interpolated / inv_w_interpolated
normal = normalize(normal_over_w_interpolated / inv_w_interpolated)
```

## 알고리즘과 구현 순서

1. clip을 마친 각 ScreenVertex에 inv_w와 모든 perspective 속성의 attr_over_w를 준비한다.
1. coverage 픽셀에서 λ로 inv_w를 보간해 q를 얻는다. q가 0에 가깝거나 유한하지 않으면 fragment를 거부하고 통계를 올린다.
1. UV, normal, world position, 선택적으로 vertex color의 over_w 값을 λ로 보간하고 q로 나눈다.
1. normal은 복원 뒤 normalize를 호출한다. 길이가 0이거나 non-finite라 실패하면 fragment를 거부하고 오류 통계에 기록한다.
1. affine/perspective 비교 모드를 남겨 이후 texture와 lighting 오류를 빠르게 구분한다.

```text
q = l0*s0.inv_w + l1*s1.inv_w + l2*s2.inv_w
if abs(q) <= EPS or not finite(q):
  reject_fragment

uv_num = l0*s0.uv_over_w + l1*s1.uv_over_w + l2*s2.uv_over_w
n_num  = l0*s0.normal_over_w + l1*s1.normal_over_w + l2*s2.normal_over_w

fragment.uv = uv_num / q
fragment.normal = normalize(n_num / q)
fragment.z_ndc = affine_depth(l0,l1,l2)
```

## JS-Wasm 경계

JS는 affine/perspective 비교 toggle만 바꾼다. inv_w와 attr_over_w는 내부 중간값이므로 JS API에 노출하지 않는다. debug overlay에는 q의 최소/최대와 invalid 수만 작은 통계로 전달한다.

## 현재 구현

- `ScreenVertex::from_clip_vertex`는 여섯 평면 clipping과 perspective divide/viewport가 끝난 정점에서만 `inv_w`, `world_position_over_w`, `normal_over_w`, `uv_over_w`, `color_over_w`를 만든다. affine 비교의 bit-stable 입력을 위해 같은 immutable 정점 안에 원본 속성도 보존한다.
- `FragmentInput::from_screen_vertices`가 한 번 계산한 `q=Σ(λ/w)`를 모든 일반 속성 복원에 공유한다. `q`가 유한하지 않거나 `1e-8` 이하면 색과 깊이를 기록하지 않고 `invalid_interpolation_samples`와 `invalid_values`를 올린다.
- 기본 모드는 perspective-correct이고 affine은 교재 비교 모드로 유지한다. 두 모드는 같은 coverage와 screen-affine `z_ndc` 깊이를 사용한다.
- 기울어진 4정점/2삼각형 fixture는 복원된 UV로 Rust 안에서 8×8 procedural checker를 그린다. 실제 이미지 입력과 sampler는 16~17장 범위라 이번 장에는 추가하지 않는다.
- `FrameStats`는 복원에 성공한 sample 수, invalid 수와 프레임의 최소/최대 `q`만 공개한다. UV나 fragment 배열은 Wasm 경계를 넘기지 않는다.

## 코딩 에이전트 작업 명세

- ClipVertex와 ScreenVertex를 분리하고 ScreenVertex 생성 시 inv_w/over_w를 준비한다.
- perspective 속성을 한 곳에서 복원하는 Interpolants 함수를 만들어 새 속성 누락을 막는다.
- 한 평면 quad를 서로 다른 대각선으로 삼각형화한 두 결과가 perspective 모드에서 같은지 golden test를 만든다.
- affine UV debug 모드를 의도적으로 유지해 교재 비교가 가능하게 한다.

## 검증 기준

- 모든 w가 같으면 perspective-correct 결과가 affine 결과와 같아야 한다.
- 정점 위치에서는 복원 속성이 원래 정점 속성과 같아야 한다.
- 기울어진 quad의 대각선 선택을 바꿔도 체크무늬 seam이 없어야 한다.
- 복원된 normal 길이는 normalize 뒤 약 1이어야 하고 NaN이 없어야 한다.

### 자주 생기는 오류

- clipping 전에 uv/w를 만들어 그것을 선형 clipping하면 새 교점 속성이 잘못될 수 있다. 원래 속성을 clip t로 보간하고 그 뒤 over_w를 만든다.
- 분모로 나누지 않고 uv_over_w만 사용하면 더 심한 왜곡이 생긴다.
- normal을 보간 후 normalize하지 않으면 밝기가 triangle 내부 위치에 따라 부당하게 변한다.
