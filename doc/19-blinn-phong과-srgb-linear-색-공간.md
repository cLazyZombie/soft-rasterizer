# 19장. Blinn-Phong과 sRGB/Linear 색 공간

> _조명 수식이 맞아도 sRGB 바이트를 그대로 곱하고 평균내면 중간톤이 너무 어둡다. 계산 공간과 저장 공간을 분리한다._

> **이번 장의 눈에 보이는 결과**  diffuse와 specular highlight가 있는 모델을 linear 공간에서 계산하고, sRGB encode 전후 비교 화면으로 차이를 확인한다.

## 왜 필요한가

일반 이미지의 RGB 바이트는 밝기에 선형인 값이 아니라 sRGB transfer function으로 encode된 값이다. 이 값을 그대로 bilinear하고 조명과 곱하면 물리적 에너지 관계가 깨지고 어두운 경계가 생긴다.

Blinn-Phong은 시선 V와 광원 L의 중간 벡터 H를 사용해 specular highlight를 만든다. 최신 PBR은 아니지만 normal, view direction, shininess와 색 공간의 상호작용을 배우기에 충분하다.

## 배경지식

- <strong>decode</strong>는 sRGB 0..1을 linear 0..1 근사 밝기로 바꾼다. texture base color와 UI에서 받은 sRGB 색에 적용한다.
- <strong>filtering 순서</strong>는 각 texel을 linear로 decode한 뒤 bilinear lerp하는 것이 정확하다. 네 encoded 값을 먼저 평균내지 않는다.
- <strong>V</strong>는 surface에서 camera로 향하는 단위 벡터, <strong>H=normalize(L+V)</strong>다.
- <strong>specular</strong>는 pow(max(dot(N,H),0), shininess)이며 ndotl&gt;0일 때만 더한다.
- <strong>encode</strong>는 최종 linear RGB를 clamp 또는 간단한 tone policy 뒤 sRGB RGBA8로 바꾼다. alpha는 transfer function을 적용하지 않는다.

## 핵심 식과 불변조건

```text
sRGB decode: c<=0.04045 ? c/12.92 : ((c+0.055)/1.055)^2.4
sRGB encode: c<=0.0031308 ? 12.92*c : 1.055*c^(1/2.4)-0.055
V = normalize(camera_world - fragment_world), H = normalize(L + V)
spec = ndotl>0 ? pow(max(dot(N,H),0), shininess) : 0
linear = ambient*albedo + light*(albedo*ndotl + specular_color*spec)*intensity
```

## byte에서 조명 계산을 거쳐 byte로 돌아오기

sRGB byte b를 먼저 `c=b/255`로 0..1에 맞춘 뒤 아래 decode에 넣는다. 출력 byte를 만들 때만 encode와 양자화를 한다.

```text
D(c) = c/12.92                         (c<=0.04045)
       ((c+0.055)/1.055)^2.4            (그 밖)
E(l) = 12.92*l                         (l<=0.0031308)
       1.055*l^(1/2.4)-0.055           (그 밖)
output_byte = round(255*E(clamp(linear,0,1)))
```

검정 b=0은 linear 0, 흰색 b=255는 linear 1이다. 빛의 양을 반씩 섞으면 0.5다. 이를 E에 넣으면 약 0.73536, byte로 약 188이다. encoded byte부터 평균낸 128은 그보다 어두운 빛의 양을 뜻한다. texture filtering·blending·AA resolve가 모두 linear에서 계산되어야 하는 같은 이유다. alpha는 빛의 밝기 인코딩이 아니므로 D/E를 적용하지 않는다.

### Blinn–Phong 방향과 지수의 의미

L은 표면→광원, V는 표면→카메라, N은 표면 법선이다. 모두 같은 world 공간에서 단위 길이로 만든다. H=normalize(L+V)는 광원과 시선의 중간 방향이다. `max(dot(N,H),0)^shininess`가 하이라이트의 세기를 정한다.

```text
N·H=0.5일 때:
shininess=2 → 0.5^2=0.25
shininess=8 → 0.5^8=0.00390625
```

지수가 크면 N과 H가 거의 나란한 좁은 영역만 밝게 남는다. N·L<=0이면 뒷면 광원이므로 specular도 0이다. L+V=0 또는 카메라와 fragment 위치가 같아 방향을 정규화할 수 없는 경우 현재 구현은 specular=0으로 처리해 NaN을 만들지 않는다.

## 알고리즘과 구현 순서

1. Texture에 color_space를 표시한다. BaseColor는 SRGB, normal/metallic 같은 데이터 texture는 Linear로 분리할 준비를 한다.
1. nearest는 선택 texel RGB를 decode한다. bilinear는 네 texel RGB를 각각 decode한 뒤 linear에서 lerp한다.
1. world position으로 V를 만들고 L, V, N을 normalize한 뒤 H와 spec을 계산한다.
1. ambient/diffuse/specular를 linear에서 합친다. 음수를 0으로 제한하고 교육용으로 1보다 큰 값은 clamp하거나 간단한 exposure를 선택한다.
1. 최종 RGB만 sRGB encode해 u8로 쓰고 alpha는 linear coverage 값으로 0..255 변환한다.

```text
sample_base_color_linear(texture, uv):
  texels, weights = gather_for_filter(texture, uv)
  linear_rgb = sum(weight[i] * srgb_decode(texel[i].rgb))
  alpha = sum(weight[i] * texel[i].a)
  return (linear_rgb, alpha)

shade:
  N,L,V = normalized directions
  ndotl = max(dot(N,L), 0)
  H = normalize(L+V)
  spec = ndotl > 0 ? pow(max(dot(N,H),0), shininess) : 0
  out_linear = ambient*albedo + intensity*light*(albedo*ndotl + specColor*spec)
  out_rgba8 = (srgb_encode(clamp(out_linear)), alpha)
```

## JS-Wasm 경계

색 공간 메타데이터와 shading은 Rust Material/Texture가 소유한다. HTML color input이 sRGB 문자열을 제공하면 JS adapter가 0..1 숫자로 파싱해 한 번 전달하고, Rust가 decode한다. Canvas에는 이미 sRGB encode된 최종 RGBA8를 전달한다.

## 코딩 에이전트 작업 명세

- srgb_decode/encode scalar 함수와 round-trip/기준점 테스트를 만든다.
- Sampler가 filter 전에 texel color space를 decode할 수 있게 구조를 조정한다.
- Lambert, Blinn-Phong, unlit을 Material shader mode로 분리하되 동적 trait dispatch보다 명확한 enum match를 우선한다.
- linear/sRGB wrong-way 비교 debug view와 diffuse/specular 단독 view를 추가한다.

## 검증 기준

- decode(0)=0, decode(1)=1, encode(decode(c))가 8비트 양자화 오차 안에서 원래 c와 같아야 한다.
- 검정/흰색 texel의 50% linear 평균을 encode한 값이 encoded 값 0.5와 다르다는 테스트로 순서를 고정한다.
- 카메라를 움직이면 specular highlight는 움직이고 Lambert diffuse는 그대로여야 한다.
- N=L=V이면 spec이 최대이며, ndotl&lt;=0이면 spec을 더하지 않아야 한다.

### 자주 생기는 오류

- 모든 texture를 sRGB로 decode하면 normal/data texture가 망가진다. color_space를 material 의미와 함께 둔다.
- bilinear한 뒤 decode하면 경계가 너무 어둡다. decode each texel -&gt; interpolate 순서를 지킨다.
- pow의 밑에 음수를 넣거나 H normalize 실패를 무시하면 NaN이 생긴다. clamp와 degenerate 방향 처리를 둔다.
