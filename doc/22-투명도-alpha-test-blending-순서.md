# 22장. 투명도, Alpha Test, Blending 순서

> _깊이 버퍼만으로 투명 표면을 해결할 수 없다. 잘라내는 투명도와 섞는 투명도를 다른 파이프라인으로 취급한다._

> **이번 장의 눈에 보이는 결과**  잎사귀 같은 cutout texture와 반투명 quad가 opaque 모델 위에서 동작하고, 정렬 한계가 debug scene으로 드러난다.

## 왜 필요한가

alpha=0인 texel도 먼저 depth를 쓰면 뒤의 표면을 가린다. 반투명 표면이 depth를 쓰면 뒤쪽 투명 표면이 사라지고, 쓰지 않으면 제출 순서에 따라 색이 달라진다. 투명도는 단순 색 공식뿐 아니라 draw 순서와 depth write 정책의 문제다.

cutout은 threshold 아래 fragment를 버리고 나머지는 opaque처럼 depth를 쓸 수 있다. 연속 alpha blend는 opaque를 먼저 그리고 transparent를 대략 뒤에서 앞으로 정렬해 depth test는 하되 write는 끄는 기준선을 사용한다.

## 배경지식

- <strong>Alpha test/cutout</strong>: alpha&lt;threshold면 discard, 아니면 depth write를 포함한 opaque 처리다. 잎, 철망에 적합하다.
- <strong>Straight alpha source-over</strong>: C = Cs\*As + Cd\*(1-As). 최종 Canvas 배경을 opaque로 유지하면 출력 alpha를 1로 둘 수 있다.
- <strong>Premultiplied alpha</strong>에서는 Cs가 이미 As를 곱한 값이고 C=Cs+Cd\*(1-As)다. texture 저장 방식과 공식을 섞지 않는다.
- <strong>선형 색 공간</strong>에서 blend한다. 현재 RGBA8 sRGB 버퍼를 유지한다면 destination을 decode하고 blend 후 다시 encode하는 정확하지만 느린 기준 경로를 쓸 수 있다.
- <strong>정렬</strong>: transparent draw items 또는 triangles를 view-space depth 기준 back-to-front로 정렬한다. 이 교재의 LH view에서는 `view_depth=z_view&gt;0`이고 큰 값이 더 멀기 때문에 descending이 back-to-front다. 교차하는 geometry에는 완전한 해결이 아니다.

## 핵심 식과 불변조건

```text
cutout: if alpha < threshold -> discard before depth write
straight source-over linear RGB: Cout = Csrc*Asrc + Cdst*(1-Asrc)
Aout = Asrc + Adst*(1-Asrc)
transparent policy: depth_test=on, depth_write=off, order=back_to_front
```

## 알고리즘과 구현 순서

1. RenderQueue를 Opaque, Cutout, Transparent로 나눈다.
1. Opaque와 Cutout을 먼저 그린다. Cutout은 texture alpha를 얻은 뒤 threshold에서 discard하고 통과 fragment만 depth를 쓴다.
1. Transparent item/triangle에 `view_depth=z_view` 대표 깊이를 계산하고 큰 값부터 정렬한다.
1. transparent fragment는 opaque depth에 대해 test하지만 depth를 갱신하지 않는다.
1. source와 기존 destination을 linear RGB로 얻어 source-over blend하고 최종 sRGB RGBA8로 쓴다. 교육용 기본 배경은 opaque로 유지한다.

```text
render opaque and cutout queues with depth_write = true

sort transparent primitives by view_depth=z_view descending
for transparent triangle:
  rasterize with depth_test = true, depth_write = false
  fragment:
    src = shade_linear()
    dst = srgb_decode(framebuffer_rgb)
    out = src.rgb * src.a + dst * (1 - src.a)
    framebuffer_rgb = srgb_encode(out)
    framebuffer_alpha = 1
```

## JS-Wasm 경계

JS UI는 blend mode, cutout threshold, transparent sort debug를 바꿀 수 있다. Canvas globalAlpha나 CSS opacity는 사용하지 않는다. 모든 표면 조합은 Rust 색/깊이 정책 안에서 일어나야 screenshot 회귀 테스트가 가능하다.

## 코딩 에이전트 작업 명세

- Material alpha mode를 Opaque, Mask, Blend로 만들고 queue 분류를 구현한다.
- depth test와 depth write를 별도 상태로 분리한다.
- linear blend 기준 경로와 잘못된 sRGB blend 비교 debug scene을 만든다.
- 서로 교차하는 두 transparent quad를 제공해 정렬 기반 방식의 한계를 문서화한다.

## 검증 기준

- alpha=0 cutout texel이 depth를 쓰지 않아 뒤 opaque 표면이 보여야 한다.
- alpha=1 Blend 결과가 opaque source color와 같아야 하고 alpha=0은 destination을 바꾸지 않아야 한다.
- opaque 제출 순서는 여전히 결과와 무관해야 한다.
- transparent 순서를 뒤집으면 잘못된 결과가 나는 fixture와 정렬 후 기대 결과를 비교한다.
- blend가 linear 공간에서 수행된다는 중간 회색 수치 테스트를 둔다.

### 자주 생기는 오류

- transparent fragment가 depth를 쓰면 다음 투명 표면이 사라진다. test와 write를 분리한다.
- straight texture에 premultiplied 공식을 적용하면 가장자리가 어두워지거나 색이 샌다.
- object center 하나로 정렬하면 큰/교차 mesh에서 틀릴 수 있다. 기준선의 한계이며 OIT는 별도 고급 주제다.
