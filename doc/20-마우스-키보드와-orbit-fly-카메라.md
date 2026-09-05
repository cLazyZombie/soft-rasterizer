# 20장. 마우스/키보드와 Orbit/Fly 카메라

> _DOM 이벤트를 발생할 때마다 렌더러에 보내지 않고, 브라우저에서 상태를 모아 프레임당 한 개의 입력 스냅샷으로 만든다._

> **이번 장의 눈에 보이는 결과**  드래그/휠로 orbit하고 WASD 또는 방향키로 fly할 수 있으며, focus를 잃어도 키가 stuck되지 않고 고주사율에서도 속도가 일정하다.

## 왜 필요한가

pointermove는 한 프레임 사이에 여러 번 발생할 수 있고 keyboard event는 자동 반복된다. 이벤트마다 Wasm을 호출하면 순서와 호출 횟수가 프레임률에 종속된다. JS collector가 현재 상태와 누적 delta를 보관하고 rAF 직전에 snapshot을 만든다.

카메라 제어는 렌더러를 실제 장치와 연결하는 첫 상호작용이다. dt, key layout, pointer capture, CSS 좌표, focus/blur를 다루면서도 카메라 수학 자체는 Rust에서 테스트 가능하게 유지한다.

## 배경지식

- <strong>Pointer Events</strong>는 mouse, pen, touch를 하나의 모델로 다룬다. drag 시작에서 setPointerCapture를 사용하면 포인터가 Canvas 밖으로 나가도 이동을 계속 받을 수 있다.
- <strong>KeyboardEvent.code</strong>는 게임 입력에 유용한 물리 키 위치를 나타내지만 일부 환경 지원과 키보드 레이아웃 설명에 주의한다. 재매핑 가능한 설정과 key fallback을 둔다.
- <strong>input snapshot</strong>은 held flags, pressed/released edges, pointer dx/dy, wheel delta, buttons, modifiers를 가진다.
- <strong>dt</strong>는 rAF timestamp 차이를 초로 바꾸고 background tab 복귀 시 큰 점프를 막기 위해 예를 들어 0.1초로 clamp한다.
- <strong>orbit</strong>는 target, radius, yaw, pitch로 카메라의 world forward를 만들고 target 반대쪽에 camera position을 둔다. yaw=0, pitch=0의 forward는 내부 왼손 규약의 +Z다.

## 핵심 식과 불변조건

```text
forward = normalize(cos(pitch)*sin(yaw), sin(pitch), cos(pitch)*cos(yaw))
right = normalize(cross(world_up, forward)), up = cross(forward, right)
camera_position = target - radius * forward
pitch = clamp(pitch + dy*sensitivity, -π/2+ε, π/2-ε)
radius = clamp(radius * exp(wheel * zoom_speed), min_r, max_r)
fly movement = speed * dt * normalize(input_right*right + input_up*up + input_forward*forward)
```

## 방향·속도·입력 단위를 연결하기

이 교재의 왼손 카메라는 yaw=0,pitch=0에서 +Z를 본다. 각도는 라디안이다.

```text
F=(sin(yaw)*cos(pitch), sin(pitch), cos(yaw)*cos(pitch))
R=normalize(cross((0,1,0),F))
U=cross(F,R)
orbit eye=target-radius*F
```

yaw=pitch=0이면 F=(0,0,1), R=(1,0,0)이다. target=(0,0,0), radius=3이면 eye=(0,0,-3)이다. yaw=π/2,pitch=0이면 F=(1,0,0), eye=(-3,0,0)이 된다. pitch는 up과 평행해지는 극점까지 도달하지 않도록 제한한다.

현재 fly speed=3 units/s다. W를 dt=1/60초 유지하면 `3*(1/60)=0.05`만큼 이동한다. W+D는 F+R=(1,0,1)을 정규화한 뒤 0.05를 곱하므로 총 이동 거리도 0.05다. 정규화하지 않으면 대각선 속도가 √2배가 된다. 입력 방향이 0이면 정규화하지 않고 위치를 유지한다.

키 상태는 시간에 따른 속도이므로 dt를 곱한다. 반면 pointer delta는 이미 지난 프레임 이후의 이동량이므로 감도만 곱하고 dt를 다시 곱하지 않는다. wheel의 지수 zoom `r'=r*exp(k*delta)`는 clamp에 닿지 않는 범위에서 두 입력 d1,d2가 `exp(k*d1)*exp(k*d2)=exp(k*(d1+d2))`로 합쳐져 이벤트 분할에 일관된다.

## 알고리즘과 구현 순서

1. JS InputCollector가 keydown/up Set, pointerdown/move/up, wheel을 듣는다. Canvas에 focus 가능 tabindex를 주고 blur/visibilitychange에서 held keys를 비운다.
1. pointer delta와 wheel은 이벤트마다 누적하고 snapshot 후 0으로 초기화한다. held key는 초기화하지 않는다.
1. snapshot을 작은 고정 배열 또는 packed 구조로 adapter에 한 번 전달한다.
1. Rust OrbitController는 yaw/pitch/radius/target을 갱신하고 position을 계산해 `look_at_lh` view를 만든다.
1. FlyController는 camera basis의 forward/right/up과 입력 축을 조합한다. W/S는 `+forward/-forward`, D/A는 `+right/-right`로 고정하고 diagonal 속도가 빨라지지 않게 방향을 normalize한다.

```text
JS each frame:
  snapshot = {
    held_bits,
    pressed_bits,
    pointer_dx: accumulated_dx,
    pointer_dy: accumulated_dy,
    wheel: accumulated_wheel,
    dragging
  }
  reset accumulated deltas and pressed edges
  renderer.frame(dt, snapshot)

Rust orbit update:
  if dragging:
    yaw   += dx * rotate_speed
    pitch = clamp(pitch + dy*rotate_speed, limits)
  radius = clamp(radius * exp(wheel*zoom_speed), limits)
  forward = spherical_forward(yaw,pitch)  # yaw=0, pitch=0 -> +Z
  position = target - radius * forward
```

## JS-Wasm 경계

JS는 DOM 이벤트 수집, focus, pointer capture, preventDefault 범위를 담당한다. Rust는 입력 값을 camera state에 적용하고 view matrix를 만든다. JS가 행렬을 만들지 않으며, Rust가 이벤트 listener를 등록하지 않는다.

## 코딩 에이전트 작업 명세

- web/input.js에 collector를 만들고 dispose 시 listener를 해제할 수 있게 한다.
- InputSnapshot의 packed layout과 비트 의미를 문서화하고 adapter에서 길이/범위를 검증한다.
- OrbitController와 FlyController를 core에 구현해 합성 입력과 dt 독립성 단위 테스트를 작성한다.
- focus/blur, pointercancel, visibilitychange, resize를 브라우저 통합 테스트에 포함한다.

## 검증 기준

- 같은 총 시간과 held input에 대해 60개의 작은 dt와 30개의 큰 dt가 거의 같은 이동 거리를 만들어야 한다.
- 대각 입력을 normalize해 한 축 이동보다 빠르지 않아야 한다.
- yaw=0, pitch=0에서 orbit forward는 +Z, camera position은 target의 -Z 쪽이어야 한다. 양의 pointer dx가 yaw를 늘려 forward를 +X 쪽으로 돌리는 UX 규약을 fixture로 고정한다.
- Fly에서 W는 +forward, D는 `cross(world_up,forward)`로 얻은 +right 방향으로 이동해야 한다.
- pitch가 극점 제한을 넘지 않고 look_at basis가 0벡터 cross를 만들지 않아야 한다.
- Canvas 밖으로 drag해도 pointer capture 동안 회전하고 release/cancel 뒤 멈춰야 한다.
- 창 focus를 잃었다 돌아왔을 때 held key가 남지 않아야 한다.

### 자주 생기는 오류

- keydown repeat 횟수로 이동하면 OS 반복 설정과 프레임률에 따라 속도가 달라진다. held state \* dt를 사용한다.
- pointer client 좌표와 Canvas 내부 픽셀 좌표를 혼동한다. orbit은 delta를 쓰고, 절대 좌표가 필요하면 bounding rect로 변환한다.
- wheel에서 페이지 스크롤을 무조건 막으면 접근성이 나빠진다. Canvas가 활성 상호작용 상태일 때만 preventDefault한다.
