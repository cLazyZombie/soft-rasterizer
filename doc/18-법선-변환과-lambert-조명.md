# 18장. 법선 변환과 Lambert 조명

> _조명의 첫 단계는 밝기 식이 아니라, 법선과 광원 방향을 같은 공간에 놓고 길이를 1로 만드는 것이다._

> **이번 장의 눈에 보이는 결과**  회전하는 textured cube에 방향광의 명암이 생기고 normal/ndotl debug view로 계산 과정을 확인한다.

## 왜 필요한가

법선은 위치와 같은 방식으로 변환되지 않는다. 특히 비균일 scale이 있으면 model 3x3을 그대로 곱한 법선이 표면에 수직이 아니게 된다. inverse-transpose normal matrix가 수직 관계를 보존한다.

Lambert diffuse는 단위 법선 N과 표면에서 빛으로 향하는 단위 벡터 L의 dot을 0 이상으로 제한한 값이다. 단순하지만 면 방향, 공간, normalize 오류를 매우 잘 드러낸다.

## 배경지식

- <strong>flat normal</strong>은 triangle 두 변의 cross로 하나의 면 법선을 쓴다. <strong>smooth normal</strong>은 vertex normal을 보간하고 픽셀마다 normalize한다.
- <strong>normal matrix</strong>는 world-space lighting이면 inverse(transpose(model upper 3x3))다. 정확한 표기는 transpose(inverse(M3))다.
- <strong>directional light</strong>는 위치 없이 방향만 가진다. 구조체의 direction이 빛이 진행하는 방향인지, 표면에서 빛으로 향하는 방향인지 이름으로 구분한다.
- 이 교재는 L을 <strong>surface_to_light</strong>로 정의한다. 태양 광선 진행 방향을 저장했다면 L=-ray_direction이다.
- <strong>ambient term</strong>은 간단한 상수로 완전히 검은 뒷면을 피한다. 물리적인 global illumination 모델은 아니다.

## 핵심 식과 불변조건

```text
N_world = normalize(transpose(inverse(M3)) * N_object)
L = normalize(surface_to_light)
diffuse = max(dot(N, L), 0)
lit_rgb = albedo * (ambient + light_color * intensity * diffuse)
```

## 알고리즘과 구현 순서

1. 모델 행렬이 바뀔 때 normal matrix를 계산한다. 역행렬이 없으면 명시적 오류 또는 geometric normal fallback을 사용한다.
1. vertex normal을 world space로 변환하고 ClipVertex 속성으로 보낸다.
1. pixel에서 perspective-correct normal을 복원하고 normalize한다.
1. directional light의 surface_to_light와 dot해 ndotl을 만들고 ambient+diffuse를 albedo에 곱한다.
1. normal을 RGB=(N\*0.5+0.5), ndotl을 grayscale로 표시하는 debug mode를 추가한다.

```text
fragment_lambert(fragment, material, light):
  N = normalize(fragment.normal_world)
  L = normalize(light.surface_to_light)
  ndotl = max(dot(N, L), 0)
  albedo = sample_texture(fragment.uv) * material.base_color
  rgb = albedo.rgb * (material.ambient
        + light.color * light.intensity * ndotl)
  return rgba(rgb, albedo.a)
```

## JS-Wasm 경계

JS UI는 light 방향/세기와 debug mode를 값으로 보낸다. normal matrix, 보간, normalize, dot, 최종 색은 Rust에서 계산한다. 브라우저 CSS filter나 Canvas globalCompositeOperation으로 조명을 흉내 내지 않는다.

## 코딩 에이전트 작업 명세

- Mat3 inverse/transpose 또는 Mat4에서 normal matrix를 얻는 최소 구현과 singular 처리 정책을 만든다.
- flat/smooth normal mode, normal RGB, ndotl debug view를 구현한다.
- uniform scale에서는 M3와 normal matrix 결과 방향이 같고 non-uniform scale에서는 수직 조건이 보존되는 테스트를 작성한다.
- direction 명칭을 ray_direction 또는 surface_to_light로 명확히 하고 변환 위치를 한 곳으로 제한한다.

## 검증 기준

- N=L이면 diffuse=1, N과 L이 직각이면 0, 반대면 clamp 후 0이어야 한다.
- 모델을 회전하면 highlight가 표면과 함께 이동하고 카메라 이동만으로 Lambert 명암이 바뀌지 않아야 한다.
- non-uniform scale 뒤 transformed tangent와 transformed normal의 dot이 0에 가까워야 한다.
- normal debug 색이 표면 전체에서 유한하고 복원 normal 길이가 1에 가까워야 한다.

### 자주 생기는 오류

- 빛의 진행 방향을 L로 그대로 쓰면 밝은 면과 어두운 면이 뒤집힌다.
- 보간한 normal을 normalize하지 않으면 삼각형 중앙이 부당하게 어두워질 수 있다.
- singular scale(축 값 0)의 normal matrix inverse를 억지로 계산하면 NaN이 퍼진다. 입력을 거부하거나 명시적 fallback을 쓴다.
