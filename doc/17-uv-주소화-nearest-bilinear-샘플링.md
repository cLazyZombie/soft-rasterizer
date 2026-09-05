# 17장. UV 주소화, Nearest, Bilinear 샘플링

> _UV는 단순히 0..1을 픽셀 번호로 바꾸는 값이 아니다. 경계 밖 주소와 texel 중심을 어떻게 정의하는지가 sampler의 결과를 결정한다._

> **이번 장의 눈에 보이는 결과**  기울어진 큐브에 checker texture가 perspective-correct UV로 붙고 nearest/bilinear, repeat/clamp를 즉시 비교할 수 있다.

## 왜 필요한가

nearest는 한 texel을 고르고 bilinear는 주변 네 texel을 섞는다. 이 작은 차이는 확대 시 계단, 축소 시 shimmer, texture 경계 seam을 만든다. 정확한 sampler는 순수 함수라서 작은 texture로 완전히 테스트할 수 있다.

UV=1에서 width를 곱하면 index가 width가 되어 범위를 벗어난다. repeat의 음수 UV 처리, clamp 끝점, bilinear의 -0.5 texel 중심 규약을 명시해야 한다.

## 배경지식

- <strong>texel center</strong>: 정규화 UV에서 첫 texel 중심은 정확히 0.5/width다. bilinear는 x=u\*width-0.5로 두면 정수 texel 좌표의 중심과 맞는다.
- <strong>repeat</strong>는 fract(u)=u-floor(u)를 사용하면 음수에서도 0..1로 들어온다.
- <strong>clamp</strong>는 UV를 0..1에 제한한 뒤 texel index를 0..width-1로 clamp한다.
- <strong>nearest</strong>는 주소화된 UV에서 floor(u\*width)를 사용하고 마지막 index를 clamp한다.
- <strong>bilinear</strong>는 x0,y0의 네 이웃 c00,c10,c01,c11을 가져와 x와 y 방향으로 두 번 lerp한다.
- <strong>색 공간</strong>: 지금은 sampler 구조를 만든다. sRGB texture의 올바른 bilinear는 19장에서 네 texel을 linear로 decode한 뒤 섞는다.

## 핵심 식과 불변조건

```text
repeat(u) = u - floor(u)
nearest: x = min(floor(u * W), W-1), y = min(floor(v * H), H-1)
bilinear coordinates: x = u*W - 0.5, y = v*H - 0.5
cx0 = lerp(c00,c10,fx), cx1 = lerp(c01,c11,fx), c = lerp(cx0,cx1,fy)
```

## UV 한 점의 bilinear 계산 전체

`lerp(a,b,t)=(1-t)a+t*b`는 a에서 b로 t만큼 이동한 값이다. W=H=2, UV=(0.5,0.5)를 대입하자.

```text
x=u*W-0.5=0.5, y=v*H-0.5=0.5
x0=floor(x)=0, y0=floor(y)=0
fx=x-x0=0.5, fy=y-y0=0.5

c=(1-fx)*(1-fy)*c00 + fx*(1-fy)*c10
  +(1-fx)*fy*c01     + fx*fy*c11
 =(c00+c10+c01+c11)/4
```

네 가중치는 각각 0.25, 합은 1이다. 최신 sampler의 base-color texture에서는 네 texel을 **linear RGB로 decode한 뒤** 이 식에 넣는다. encoded RGBA8 숫자 네 개의 단순 평균과 같다고 가정하면 안 된다. alpha는 sRGB 변환을 하지 않고 선형으로 평균낸다. 19장에서 검정·흰색 평균으로 이 차이를 계산한다.

음수 repeat도 같은 규칙으로 계산한다. `repeat(-0.25)=-0.25-floor(-0.25)=-0.25-(-1)=0.75`다. u=1은 repeat에서 0으로 돌아가지만 clamp에서는 1이다. bilinear 경계의 각 이웃 index도 repeat/clamp 규칙으로 주소화하므로 반대편 texel 연결 여부가 달라진다.

## 알고리즘과 구현 순서

1. AddressMode enum으로 Repeat와 ClampToEdge를 만든다. u와 v에 각각 적용 가능하게 한다.
1. FilterMode enum으로 Nearest와 Bilinear를 만든다.
1. sample_nearest는 주소화, index 계산, RGBA8-&gt;float4 변환을 수행한다.
1. sample_bilinear는 texel 중심 좌표를 만들고 x0/x1/y0/y1 각각에 address mode를 적용해 네 색을 얻는다.
1. fragment shader에서 perspective-correct uv로 texture를 샘플하고 vertex color 또는 material factor와 곱한다.

```text
sample_bilinear(texture, uv, address):
  x = uv.x * W - 0.5
  y = uv.y * H - 0.5
  x0 = floor(x); y0 = floor(y)
  fx = x - x0; fy = y - y0
  c00 = fetch(address(x0),   address(y0))
  c10 = fetch(address(x0+1), address(y0))
  c01 = fetch(address(x0),   address(y0+1))
  c11 = fetch(address(x0+1), address(y0+1))
  return lerp(lerp(c00,c10,fx), lerp(c01,c11,fx), fy)
```

## JS-Wasm 경계

sampler state는 Rust Material의 일부다. JS UI가 filter/address enum을 변경할 수 있지만 texture pixel을 Canvas가 확대해 필터링하도록 맡기지 않는다. Canvas의 imageSmoothing은 최종 framebuffer 표시 확대에만 영향을 주고 3D texture 필터와는 별개다.

## 코딩 에이전트 작업 명세

- Texture::fetch와 Sampler::sample을 분리해 주소화와 filtering을 각각 테스트한다.
- 1x1, 2x2, non-power-of-two texture와 음수/1초과 UV fixture를 만든다.
- nearest/bilinear, repeat/clamp UI와 sampler 통계를 추가한다.
- UV checker debug view와 affine/perspective toggle을 함께 유지한다.

## 검증 기준

- 2x2 texture 중앙 UV에서 bilinear 결과가 네 linear 색의 평균이어야 한다. sRGB texture는 먼저 linear로 decode한 값을 평균한다.
- 1x1 texture는 어떤 UV와 filter에서도 같은 색을 반환해야 한다.
- repeat(-0.25)와 repeat(0.75)가 같은 texel을 선택해야 한다.
- clamp에서 UV=1과 매우 큰 값이 마지막 texel을 안전하게 반환해야 한다.
- quad의 대각선 선택을 바꿔도 texture seam이 없어야 한다.

### 자주 생기는 오류

- u\*width를 round하면 경계가 비대칭이고 UV=1에서 범위를 벗어날 수 있다.
- Rust의 나머지 연산을 음수 repeat에 그대로 쓰면 음수 결과가 남을 수 있다. u-floor(u)를 사용한다.
- bilinear에서 네 texel index를 먼저 clamp한 뒤 fx를 잘못 바꾸면 경계 색이 어두워진다. 좌표와 address 단계를 분리한다.
