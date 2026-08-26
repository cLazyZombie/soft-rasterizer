# 9장. Winding, 퇴화 삼각형, Backface Culling

> _보이지 않는 뒷면을 버리는 최적화이기 전에, winding은 edge 함수의 부호와 top-left 규칙을 하나로 묶는 정확성 규약이다._

> **이번 장의 눈에 보이는 결과**  회전 큐브의 앞면 wireframe만 보이며 culling on/off와 front-face 색상 debug 모드를 즉시 전환할 수 있다.

## 왜 필요한가

삼각형 정점 순서가 시계인지 반시계인지에 따라 면 법선과 signed area의 부호가 달라진다. 화면 y를 뒤집는 viewport는 부호를 한 번 더 바꾼다. 이 규약을 명시하지 않으면 절반의 면이 사라지거나 카메라가 뒤집힐 때 결과가 달라진다.

면의 뒷면은 닫힌 불투명 모델에서 보이지 않으므로 raster 비용을 줄일 수 있다. 그러나 양면 재질, 투명 재질, 잘못된 mesh를 디버깅할 때는 끌 수 있어야 한다.

## 배경지식

- <strong>signed area</strong>는 2D orient 함수와 같다. 절댓값이 거의 0이면 점들이 한 선 위에 있어 픽셀 면적이 없는 퇴화 삼각형이다.
- <strong>화면 y-down 규약</strong>에서 이 교재는 orient2d(v0,v1,v2)&gt;0을 front face로 정한다. 화면에서 보면 시계 방향이다.
- 이 screen-space 판정은 왼손/+Z view 전환 뒤에도 그대로다. 통상적인 cross로 만든 카메라 쪽 outward normal과 기본 LH 카메라의 투영 결과가 `area2&gt;0`으로 일치해야 하며, handedness만 보고 area 부호를 뒤집지 않는다.
- <strong>backface culling 시점</strong>은 clipping과 projection 뒤 screen 공간에서 정하면 raster edge 부호와 정확히 같은 규약을 공유하기 쉽다.
- <strong>early reject</strong>로 view 공간 geometric normal과 `surface_to_camera=-view_pos`를 dot할 수도 있지만 clipping과 비균일 scale, winding 규약을 함께 다뤄야 하므로 기본 구현은 screen area를 사용한다.

## 핵심 식과 불변조건

```text
orient2d(a,b,c) = (b.x-a.x)*(c.y-a.y) - (b.y-a.y)*(c.x-a.x)
abs(area2) <= epsilon -> degenerate
screen y-down 기준: area2 > 0 -> front, area2 < 0 -> back
```

## 알고리즘과 구현 순서

1. clip과 perspective divide를 통과한 세 screen 위치로 area2를 계산한다.
1. area2가 유한하지 않으면 invalid 삼각형 통계를 올리고 버린다.
1. area2가 0에 가까우면 퇴화 삼각형 통계를 올리고 버린다.
1. material이 double_sided가 아니고 area2&lt;0이면 culled 통계를 올리고 버린다.
1. raster 단계에는 area2&gt;0인 삼각형만 넘긴다. 필요하면 v1과 v2를 교환해 positive winding으로 정규화한다.
1. debug 모드에서 front는 초록, back은 빨강 wireframe으로 그려 규약을 확인한다.

```text
area2 = orient2d(s0, s1, s2)
if not finite(area2):
  stats.invalid += 1
  reject

if abs(area2) <= AREA_EPS:
  stats.degenerate += 1
  reject

if material.cull_backfaces and area2 < 0:
  stats.backface_culled += 1
  reject

if area2 < 0:
  swap(s1, s2)  # only for double-sided normalized raster input
```

## JS-Wasm 경계

culling toggle과 double-sided 옵션은 JS UI가 상태를 바꿀 수 있지만 판정은 Rust에서 한다. JS는 triangle 목록을 줄이거나 winding을 고치지 않는다. 통계만 받아 overlay에 submitted/cull/degenerate/invalid 수를 표시한다. 한 프레임의 입력 삼각형은 `input = submitted + culled + degenerate + invalid`로 완전히 분류되어야 한다.

## 코딩 에이전트 작업 명세

- orient2d를 raster 모듈의 단일 함수로 만들고 culling과 coverage가 같은 구현을 사용하게 한다.
- screen y-down에서 positive front라는 결정을 문서와 테스트에 고정한다.
- culling off, backface on, frontface on 또는 double-sided debug 옵션을 최소 enum으로 만든다.
- 큐브의 각 회전 각도에서 submitted, culled 수를 기록하는 통합 검사를 추가한다.

## 검증 기준

- 정점 순서를 뒤집으면 area 부호가 정확히 반대로 바뀌어야 한다.
- 같은 직선 위 세 점은 퇴화로 거부되고 깊이/색 버퍼를 바꾸지 않아야 한다.
- non-finite area 또는 screen projection 실패는 invalid로 거부되고 다른 분류와 중복 집계되지 않아야 한다.
- 모든 입력 삼각형은 submitted, culled, degenerate, invalid 중 정확히 하나로 집계되어야 한다.
- 닫힌 큐브를 일반 시점에서 볼 때 culling on이 off보다 raster submitted 수를 줄여야 한다.
- 기본 LH 카메라 앞의 outward-normal triangle은 `area2&gt;0`, 정점 순서를 뒤집은 triangle은 `area2&lt;0`이어야 한다.
- double-sided 재질에서는 뒤집힌 삼각형도 positive winding으로 정규화되어 같은 coverage 규칙을 사용해야 한다.

### 자주 생기는 오류

- world/view 공간의 CCW 설명을 screen 공간에 그대로 적용하면 viewport y flip 때문에 부호가 반대가 된다.
- 너무 큰 area epsilon은 멀리 있는 작지만 보이는 삼각형을 없앤다. screen 좌표와 고정소수점 단위에 맞춰 정한다.
- culling이 켜진 상태에서 mesh winding 오류를 숨기지 않는다. debug 모드에서 양면 색을 먼저 확인한다.
