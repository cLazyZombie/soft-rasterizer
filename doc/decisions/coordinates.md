# 좌표, 카메라, 깊이 규약

상태: 확정

이 결정은 object/world/view 공간, 카메라, projection, clipping, screen winding과 외부 asset 변환의 공통 기준이다. 좌표 규약을 바꿀 때는 이 문서, 수학 테스트, golden과 browser fixture를 같은 변경에서 갱신한다.

## 기본 좌표와 행렬

- 열벡터를 사용하고 `p_clip = P * V * M * p_object` 순서로 변환한다.
- object/world/view는 왼손 좌표계다. `+X`는 오른쪽, `+Y`는 위, `+Z`는 전방이다.
- view 공간에서 카메라는 원점에 있고 `+Z`를 본다. 카메라 앞의 유한한 점은 `z_view>0`이다.
- 통상적인 algebraic cross 식과 `cross(X,Y)=Z`를 유지한다. handedness를 이유로 cross 결과를 음수화하지 않는다.
- 양의 축 회전은 `Rx(+π/2)*Y=Z`, `Ry(+π/2)*Z=X`, `Rz(+π/2)*X=Y`로 고정한다.
- 벡터 정규화는 길이 제곱이 `1e-12` 이하이거나 NaN/Inf이면 `None`을 반환한다. 실패를 0벡터나 임의 방향으로 조용히 바꾸지 않는다.
- `Mat4` 저장 순서는 공개 계약이 아니다. 외부 코드는 논리적 `get(row, column)`과 열벡터 곱만 사용한다.

## Look-at

```text
F = normalize(target - eye)
R = normalize(cross(world_up, F))
U = cross(F, R)

V rows:
[R.x, R.y, R.z, -dot(R, eye)]
[U.x, U.y, U.z, -dot(U, eye)]
[F.x, F.y, F.z, -dot(F, eye)]
[0,   0,   0,    1]
```

`eye=(0,0,0)`, `target=(0,0,1)`, `world_up=(0,1,0)`이면 V는 identity다. `eye+k*F`, `k>0`은 view `(0,0,k)`로 간다. F와 world_up이 거의 평행하면 명시한 대체 up을 사용하고 0벡터 normalize를 허용하지 않는다.

## Perspective와 깊이

`q=1/tan(fov_y/2)`, `0<n<f`일 때 left-handed zero-to-one projection은 다음과 같다.

```text
P rows:
[q/aspect, 0, 0,             0]
[0,        q, 0,             0]
[0,        0, f/(f-n), -f*n/(f-n)]
[0,        0, 1,             0]
```

- `w_clip=z_view`다. 앞의 점은 w가 양수이고 뒤의 `z_view<0` 점은 w가 음수다.
- `z_view=n`은 `z_ndc=0`, `z_view=f`는 `z_ndc=1`이다.
- clip 범위는 `-w<=x<=w`, `-w<=y<=w`, `0<=z<=w`다. plane distance는 `x+w`, `w-x`, `y+w`, `w-y`, `z`, `w-z`를 유지한다.
- depth clear는 `+infinity`, 통과는 유한한 `0..1` 후보에 대한 strict `<`다. 작은 NDC 깊이가 가깝다.

## Homogeneous clipping

- perspective divide 전에 left, right, bottom, top, near, far 순서로 여섯 평면을 처리한다.
- 내부 판정은 정확히 `distance >= 0`이며 임의 epsilon으로 평면 소유를 바꾸지 않는다.
- crossing edge 교점은 `t=dA/(dA-dB)`로 만들고 `ClipVertex`의 clip position, world position, normal, UV, color를 모두 같은 `t`로 보간한다.
- 경계(`distance == 0`) endpoint는 교점으로 중복 생성하지 않고 한 번만 출력한다. 그 밖의 crossing에서 두 거리의 차가 0이거나 non-finite이면 잘못된 입력으로 관찰하고 버린다. 좌표 규모에 비례하는 ULP tolerance는 생성 정점의 debug postcondition에만 사용한다.
- 삼각형에서 시작한 convex polygon은 평면마다 정점이 최대 하나 늘 수 있으므로 scratch capacity 상한은 9다. 두 polygon buffer와 fan output buffer는 프레임 사이 재사용한다.
- source 통계와 fan output 통계를 분리하며 정상 fan 출력은 `generated = submitted + culled + degenerate + invalid`를 만족한다.

## Viewport, winding과 coverage

- `screen_x=(0.5+0.5*x_ndc)*width`, `screen_y=(0.5-0.5*y_ndc)*height`다.
- screen y-down에서 `orient2d(v0,v1,v2)>0`을 front face로 쓴다. 화면에서는 시계 방향이다.
- handedness 변경만으로 screen area, edge 또는 top-left 부호를 뒤집지 않는다.
- 9장 wireframe 제출 단계는 non-finite area나 screen projection 실패를 `invalid`, 이름 붙인 최소 float epsilon 이하의 area를 `degenerate`로 조기 거부한다. 10장부터는 source triangle이 fan triangle 여러 개를 만들 수 있으므로 `generated = submitted + culled + degenerate + invalid`로 fan 출력을 완전히 분류한다. 11장 coverage의 최종 퇴화 판정은 고정소수점 양자화 뒤 `area==0`이며 float epsilon을 top-left equality에 사용하지 않는다.
- culling은 `none`, `back`, `front`를 지원한다. culling을 통과한 back face는 정점 순서를 바꿔 이후 단계에 positive winding으로 제출한다.
- 포함 edge는 `dy<0 || (dy==0 && dx>0)`이며 sample 위치는 `(x+0.5,y+0.5)`다.

## 카메라 입력

Orbit/Fly의 yaw=0, pitch=0 world forward는 `+Z`다.

```text
forward = normalize(cos(pitch)*sin(yaw), sin(pitch), cos(pitch)*cos(yaw))
right = normalize(cross(world_up, forward))
up = cross(forward, right)
orbit_position = target - radius * forward
```

Fly에서 W/S는 `+forward/-forward`, D/A는 `+right/-right`다. 양의 pointer dx는 yaw를 늘려 forward를 `+X` 쪽으로 돌린다.

## 외부 asset

- OBJ는 handedness/up/forward가 포맷에 고정되지 않는다. baseline profile은 이미 내부 LH 축을 사용한다고 선언하며, 다른 입력은 명시적 profile 없이 추측해 변환하지 않는다.
- glTF는 오른손 `+Y` up, `+Z` forward, `-X` right다. 내부 축 의미를 보존하는 변환은 `C=diag(-1,1,1,1)`이다.
- glTF position/normal/morph delta에는 C를, node/skin 행렬에는 `C*M*C`를 적용한다. triangle list는 `(i0,i1,i2)->(i0,i2,i1)`로 winding을 뒤집는다.
- tangent는 `(C3*tangent.xyz,-tangent.w)`로 변환한다. animation rotation은 변환한 행렬이 `C*R*C`와 같은지 검증한다.
- UV는 좌표 handedness 때문에 뒤집지 않는다. glTF camera를 지원하면 source eye/forward/up을 world에서 C로 변환한 뒤 내부 `look_at_lh`를 만든다.

## 필수 fixture

- canonical `look_at_lh` identity와 `eye+k*F -> (0,0,k)`
- `perspective_lh_zo`의 `+near -> 0`, `+far -> 1`, `w_clip=z_view`
- 앞점 `w>0`, 뒤점 `w<0`과 clipping-before-divide
- LH 큐브의 outward normal, screen `area>0`과 culling 일치
- orbit yaw=0, pointer yaw 부호와 W/A/S/D 이동
- transparent `view_depth=z_view` descending
- glTF의 `C*(M*p)==(C*M*C)*(C*p)`, winding/normal/tangent/rotation 변환
