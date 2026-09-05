# 12장. Barycentric 좌표와 속성 보간

> _세 edge 값을 전체 면적으로 나누면 픽셀이 세 정점의 영향을 얼마나 받는지 나타내는 좌표가 된다._

> **이번 장의 눈에 보이는 결과**  삼각형 세 정점을 빨강/초록/파랑으로 두었을 때 내부가 부드러운 barycentric 색으로 채워지고, 각 정점에서 정확한 원래 색이 나온다.

## 왜 필요한가

coverage만 알면 단색 삼각형만 그릴 수 있다. 실제 렌더링은 정점의 색, UV, 법선, world position을 픽셀마다 계산해야 한다. barycentric 좌표는 삼각형 내부의 점을 세 정점의 가중합으로 표현한다.

이 장에서는 screen 공간에서 선형인 값을 affine 보간한다. 화면에 정의한 debug 색 그라데이션과 z_ndc는 이 방식으로 보간한다. 3D 표면의 vertex color는 14장부터 일반 속성과 함께 원근 보정한다. UV와 world position처럼 투영 전 공간에서 선형인 값은 14장에서 1/w를 사용해 보정한다.

## 배경지식

- <strong>λ0, λ1, λ2</strong>는 각각 반대편 edge의 signed area 비율이다. 내부에서는 0..1이고 합은 1이다.
- <strong>정점에서의 성질</strong>: v0 위치에서는 λ0=1, 나머지는 0이다. 따라서 보간 결과가 정확히 정점 속성으로 돌아온다.
- <strong>affine interpolation</strong>은 a = λ0\*a0 + λ1\*a1 + λ2\*a2다. 스칼라, 벡터, 색에 같은 식을 성분별 적용한다.
- <strong>edge 값 재사용</strong>: coverage에서 이미 계산한 e0,e1,e2를 area로 나누면 별도의 좌표 계산이 필요 없다.
- <strong>정규화 오차</strong>: fixed-point edge에서 얻은 λ의 합은 매우 가깝게 1이지만 f32 변환 오차가 있다. 디버그 assertion에 합 오차를 기록한다.

## 핵심 식과 불변조건

```text
λ0 = E(v1,v2,p) / E(v0,v1,v2)
λ1 = E(v2,v0,p) / E(v0,v1,v2)
λ2 = E(v0,v1,p) / E(v0,v1,v2)
λ0 + λ1 + λ2 = 1
a_affine = λ0*a0 + λ1*a1 + λ2*a2
```

## edge 값을 색의 비율로 바꾸기

11장의 같은 삼각형과 p=(0.5,0.5)를 사용하면 area=16, edge=(12,2,2)였다.

```text
λ0=12/16=3/4
λ1= 2/16=1/8
λ2= 2/16=1/8
λ0+λ1+λ2=1
```

내부의 점 p로 나눈 작은 세 삼각형의 면적 합이 원래 면적이므로 `e0+e1+e2=area`다. 따라서 비율의 합도 1이다. 정점 색을 순서대로 빨강 (1,0,0), 초록 (0,1,0), 파랑 (0,0,1)로 정하면:

```text
C=λ0*C0+λ1*C1+λ2*C2=(0.75,0.125,0.125)
debug 직접 양자화 round(255*C)=(191,32,32)
```

이 마지막 값은 12장의 화면 debug 색 인코딩 예제다. linear 색을 sRGB로 바꾸는 19장의 출력값과 같다고 가정하면 안 된다. 또한 표면 색을 보간하는 최신 기본 경로에는 14장의 원근 보정을 적용한다.

## 알고리즘과 구현 순서

1. triangle setup에서 inv_area = 1.0 / area를 f32로 한 번 계산한다.
1. coverage를 통과한 픽셀의 i64 edge 값을 f32로 바꾸고 inv_area를 곱해 λ를 얻는다.
1. 정점 color를 λ 가중합하고 0..1 clamp 뒤 RGBA8로 변환한다.
1. debug 모드에서 속성 대신 (λ0, λ1, λ2)를 RGB로 써서 방향과 정점 매핑을 시각화한다.
1. 색 이외 속성은 FragmentInput 구조에 모으되, UV/normal/world_pos에는 아직 perspective-correct 경로가 필요하다는 표시를 둔다.

```text
if covered:
  l0 = float(e0) * inv_area
  l1 = float(e1) * inv_area
  l2 = float(e2) * inv_area

  color = l0*c0 + l1*c1 + l2*c2
  write_rgba8(x, y, encode_debug_color(color))
```

## JS-Wasm 경계

보간과 색 변환은 Rust에서 수행한다. JS UI는 barycentric RGB, triangle ID, solid color 같은 debug shader mode를 enum 값으로 선택할 수 있다. 픽셀 속성 배열을 JS로 내보내지 않는다.

12장 학습 단계의 구현은 `TriangleSetup`에서 positive fixed-point area의 역수를 한 번 만들고, coverage callback이 받은 세 edge 값을 그대로 `BarycentricCoordinates`로 바꾼다. 공개 생성자는 세 edge가 각각 `0..=area`이고 합이 정확히 area인 경우만 허용해 가중치 invariant를 타입에 고정한다. 이 단계의 `FragmentInput`은 barycentric과 affine color만 담으며 UV, world position과 normal은 14장 전까지 넣지 않는다. 12장의 vertex-color mode는 세 `ClipVertex.color`를 screen 공간에서 affine 보간하고, barycentric debug mode는 `(lambda0,lambda1,lambda2)`를 RGB로 기록한다.

색 채널은 유한한 값만 0..1로 clamp해 마지막에 RGBA8로 반올림한다. 범용 변환 함수에 직접 들어온 NaN/Inf 채널은 0으로 고정하지만, 렌더 경로의 `FragmentInput`은 non-finite 정점/보간 결과를 거부해 framebuffer를 쓰지 않고 `invalid_values`에 기록한다. `FrameStats.max_barycentric_sum_error`는 프레임의 모든 covered sample에서 최대 `|lambda0+lambda1+lambda2-1|`를 기록한다. R/G/B 단일 triangle fixture는 source affine color와 barycentric debug가 같은 이미지를 만드는 불변조건을 Rust-Wasm-Canvas 2D 경로에서 확인한다. 네 정점 색을 하나의 screen-space affine 함수로 정한 quad는 어느 대각선으로 나누어도 모든 내부 sample의 owner가 1이고 RGBA8 이미지가 exact match하는지 네이티브 invariant로 고정한다.

## 코딩 에이전트 작업 명세

- FragmentInput 또는 Interpolants 구조를 만들고 12장에서는 affine color와 barycentric만 채운다.
- 정점, 변의 중점, 무게중심에서 알려진 λ와 색을 반환하는 순수 함수 테스트를 작성한다.
- barycentric debug view와 λ 합 오차의 최대값 통계를 추가한다.
- color float-&gt;u8 변환에서 NaN, 음수, 1 초과 값을 안전하게 처리하는 정책을 만든다.

## 검증 기준

- v0, v1, v2 샘플에서 대응 λ가 1이고 다른 두 값이 0이어야 한다. fixed quantization 때문에 정확한 정점 샘플이 없으면 수학 함수 단위 테스트로 검사한다.
- 삼각형 무게중심에서 세 λ가 약 1/3이고 RGB가 비슷해야 한다.
- 내부 모든 샘플에서 λ 합이 정한 epsilon 안에서 1이어야 한다.
- edge setup을 incremental로 바꿔도 barycentric golden image가 동일해야 한다.

### 자주 생기는 오류

- e0를 v0 반대 edge가 아닌 다른 edge와 연결하면 색 꼭짓점이 뒤바뀐다. λ-정점 대응을 명시적으로 테스트한다.
- area 부호가 음수인 삼각형을 그대로 받으면 내부 λ 부호와 coverage 규칙이 흔들린다. positive winding 입력을 보장한다.
- RGBA8에서 직접 정수 보간하면 밴딩과 overflow가 생긴다. 계산은 f32 0..1, 마지막에만 u8로 바꾼다.

12장 실행본은 화면 debug 색을 affine 보간하는 학습 단계다. 3D 표면에 붙은 vertex color는 UV처럼 일반 속성이므로 14장 이후 기본 경로에서는 perspective-correct 보간한다. 최신 `FragmentInput`은 world position, normal, UV, color를 함께 복원하며, 12장의 affine 설명은 최신 전체 경로의 기본값을 뜻하지 않는다.
