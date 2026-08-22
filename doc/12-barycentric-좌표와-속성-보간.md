# 12장. Barycentric 좌표와 속성 보간

> _세 edge 값을 전체 면적으로 나누면 픽셀이 세 정점의 영향을 얼마나 받는지 나타내는 좌표가 된다._

> **이번 장의 눈에 보이는 결과**  삼각형 세 정점을 빨강/초록/파랑으로 두었을 때 내부가 부드러운 barycentric 색으로 채워지고, 각 정점에서 정확한 원래 색이 나온다.

## 왜 필요한가

coverage만 알면 단색 삼각형만 그릴 수 있다. 실제 렌더링은 정점의 색, UV, 법선, world position을 픽셀마다 계산해야 한다. barycentric 좌표는 삼각형 내부의 점을 세 정점의 가중합으로 표현한다.

이 장에서는 screen 공간에서 선형인 값을 affine 보간한다. 색 그라데이션과 z_ndc는 이 방식으로 충분하다. UV와 world position처럼 투영 전 공간에서 선형인 값은 14장에서 1/w를 사용해 보정한다.

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

## 코딩 에이전트 작업 명세

- FragmentInput 또는 Interpolants 구조를 만들고 현재는 affine color와 barycentric만 채운다.
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
