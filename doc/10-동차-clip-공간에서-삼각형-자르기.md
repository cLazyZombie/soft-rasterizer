# 10장. 동차 Clip 공간에서 삼각형 자르기

> _클리핑은 화면 밖을 버리는 최적화가 아니라, 카메라 뒤의 점을 w로 나누어 거대한 잘못된 삼각형을 만드는 것을 막는 안전장치다._

> **이번 장의 눈에 보이는 결과**  near plane과 화면 모서리를 가로지르는 삼각형이 튀거나 뒤집히지 않고 부드럽게 잘려 wireframe으로 보인다.

## 왜 필요한가

왼손 projection에서는 `w_clip=z_view`다. 세 정점 중 하나가 카메라 뒤의 `z_view&lt;0`에 있으면 w가 음수일 수 있다. 이를 바로 나누면 화면 좌표가 반전되고 거의 무한대가 되어 bounding box가 전체 화면을 덮는다. 삼각형을 버리기만 하면 near plane을 통과할 때 갑자기 사라진다.

정확한 방법은 clip space에서 여섯 평면을 차례로 적용해 내부 다각형을 얻는 것이다. Sutherland-Hodgman은 다각형의 각 변이 평면을 통과하는지 보고 교점을 추가한다. 결과 다각형은 triangle fan으로 다시 삼각형화한다.

## 배경지식

- <strong>plane distance</strong>를 내부에서 0 이상이 되도록 정하면 모든 평면을 같은 코드로 처리할 수 있다.
- <strong>여섯 평면</strong>: left x+w, right w-x, bottom y+w, top w-y, near z, far w-z.
- <strong>교점 t</strong>는 두 signed distance의 선형 보간이 0이 되는 지점이다. t=dA/(dA-dB).
- <strong>속성도 함께 보간</strong>한다. clip position뿐 아니라 world position, normal, UV, color를 같은 t로 lerp한다. perspective 보정용 1/w는 clipping 뒤 계산한다.
- <strong>fan triangulation</strong>은 결과 polygon [v0,v1,...]을 (v0,v1,v2), (v0,v2,v3)처럼 나눈다.

## 핵심 식과 불변조건

```text
inside(v, plane) iff distance(v) >= 0
left: x+w, right: w-x, bottom: y+w, top: w-y, near: z, far: w-z
t = d_prev / (d_prev - d_curr)
intersection = lerp(prev, curr, t)
```

## 알고리즘과 구현 순서

1. 입력 triangle을 작은 Vec&lt;ClipVertex&gt; polygon으로 시작한다.
1. 각 clip plane마다 기존 polygon의 모든 prev-&gt;curr edge를 순환한다.
1. prev와 curr가 모두 내부면 curr를 출력한다. 내부-&gt;외부면 교점만, 외부-&gt;내부면 교점과 curr를 출력한다. 둘 다 외부면 아무것도 출력하지 않는다.
1. 한 평면 뒤 결과가 비면 즉시 fully clipped로 종료한다. 내부 판정이 서로 다를 때만 교점을 만들며, 이때 유한한 `d_prev-d_curr`는 0일 수 없다. 경계(`distance == 0`) endpoint는 교점으로 중복 출력하지 않는다.
1. 여섯 평면을 통과한 polygon을 fan으로 삼각형화하고, 그때 처음 perspective divide와 viewport를 수행한다.

```text
clip_polygon_against_plane(input, distance):
  output = []
  prev = input.last
  d_prev = distance(prev.clip_pos)
  for curr in input:
    d_curr = distance(curr.clip_pos)
    prev_in = d_prev >= 0
    curr_in = d_curr >= 0
    if prev_in and curr_in:
      output.push(curr)
    else if prev_in and not curr_in:
      if d_prev != 0:
        output.push(lerp_vertex(prev, curr, d_prev/(d_prev-d_curr)))
    else if not prev_in and curr_in:
      if d_curr != 0:
        output.push(lerp_vertex(prev, curr, d_prev/(d_prev-d_curr)))
      output.push(curr)
    prev, d_prev = curr, d_curr
  return output
```

## JS-Wasm 경계

클리핑은 전적으로 Rust core에 있다. JS resize는 aspect와 projection을 바꿀 뿐 clip 규칙은 바꾸지 않는다. clipping 통계로 input triangles, fully clipped, generated triangles, max polygon vertex 수를 JS overlay에 전달한다.

구현에서는 내부 판정을 `distance >= 0`으로 유지한다. 실제 crossing edge는 두 거리의 부호가 다르므로 유한한 분모는 0이 아니고 `t`는 `0..1`이다. 경계 endpoint는 이미 정확한 교점이므로 그대로 한 번만 출력하고, 그 밖의 crossing에서 분모가 0이거나 non-finite이면 invalid로 분류한다. 임의 epsilon으로 inside/outside를 바꾸지 않고, 좌표 규모에 비례하는 ULP tolerance는 생성 정점의 debug postcondition에만 사용한다.

통계 단계는 source와 fan 출력을 섞지 않는다. `input_triangles`는 source triangle, `fully_clipped_triangles`와 `clip_invalid_triangles`는 source 단계, `generated_triangles`와 submitted/culled/degenerate/invalid는 fan 출력 단계다. 따라서 정상 fan 출력은 `generated = submitted + culled + degenerate + invalid`를 만족한다.

## 코딩 에이전트 작업 명세

- ClipVertex 전체를 lerp하는 단일 함수를 만들고 새 속성이 추가될 때 누락되지 않도록 테스트한다.
- 여섯 plane을 enum 또는 함수 테이블로 표현해 중복 분기 코드를 없앤다.
- near, left, corner를 가로지르는 수작업 triangle fixture와 모든 정점이 밖인 fixture를 만든다.
- clip 뒤 모든 정점이 여섯 distance에서 작은 음의 epsilon보다 크거나 같은지 assertion/debug 검사한다.

## 검증 기준

- 완전히 내부인 triangle은 값과 삼각형 수가 바뀌지 않아야 한다.
- `z_view&gt;0`인 카메라 앞 정점은 양의 w를, `z_view&lt;0`인 뒤 정점은 음의 w를 만들며 뒤쪽 정점이 포함된 fixture도 divide 전에 안전하게 clip되어야 한다.
- 한 정점만 near 밖인 triangle은 일반적으로 quad가 되어 두 triangle로 나온다.
- 완전히 한 평면 밖인 triangle은 0개가 되고 색/깊이 버퍼를 건드리지 않는다.
- 생성된 교점의 UV, color, world position이 같은 t의 lerp 값인지 검사한다.
- 카메라를 triangle을 통과해 움직일 때 screen 좌표에 NaN/Inf가 생기지 않는다.

### 자주 생기는 오류

- NDC로 나눈 뒤 2D에서 자르면 w와 원근 속성의 관계를 잃는다. 반드시 homogeneous clip space에서 자른다.
- position만 자르고 UV/normal을 원래 정점에서 가져오면 clip 경계에서 텍스처와 조명이 찢어진다.
- distance가 매우 작은 값을 무조건 0으로 만들면 plane을 따라 움직일 때 깜빡일 수 있다. 경계 소유는 exact `distance >= 0`으로 유지하고, 좌표 규모에 비례하는 tolerance는 생성 정점의 debug postcondition에만 사용한다.
