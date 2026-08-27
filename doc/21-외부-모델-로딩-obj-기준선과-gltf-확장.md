# 21장. 외부 모델 로딩: OBJ 기준선과 glTF 확장

> **PART 5 · 에셋, 품질, 검증, 성능**
>
> 외부 데이터를 안전하게 받아들이고, 화질 옵션과 회귀 테스트를 더한 뒤 근거가 있는 최적화로 마무리한다.

> _파일 읽기는 JS가, 포맷 해석과 내부 Mesh 검증은 Rust가 맡는다. 먼저 작은 OBJ 부분집합으로 경계를 익히고 glTF는 별도 확장으로 다룬다._

> **이번 장의 눈에 보이는 결과**  사용자가 선택한 외부 OBJ 모델이 internal Mesh로 변환되어 texture/lighting과 함께 렌더링되고, 잘못된 파일은 안전한 오류로 거부된다.

## 왜 필요한가

하드코딩 큐브는 파이프라인 검증에는 좋지만 실제 모델의 index seam, 누락 normal, 큰 좌표, 비정상 데이터 문제를 드러내지 않는다. 외부 모델 로더는 렌더러와 asset 경계를 실전 수준으로 만든다.

OBJ는 텍스트 구조가 단순해 position/UV/normal index 조합을 내부 vertex로 만드는 알고리즘을 배우기 좋다. glTF 2.0은 scene, binary buffer, material, texture 색 공간까지 포함한 실전 포맷이므로 전체를 직접 재구현하기보다 명시한 subset 또는 유지되는 parser를 쓰는 것이 좋다.

## 배경지식

- <strong>JS 파일 경계</strong>: input File을 arrayBuffer로 읽고 Uint8Array bytes와 format hint를 한 번 Wasm에 넘긴다.
- <strong>OBJ 레코드</strong>: v는 position, vt는 UV, vn은 normal, f는 position/uv/normal index tuple 목록이다.
- <strong>OBJ 좌표 규약</strong>: OBJ 자체는 handedness와 up/forward 축을 고정하지 않는다. 교육용 baseline은 입력이 이미 내부 LH `+X` right, `+Y` up, `+Z` forward라고 명시하고, 다른 도구의 OBJ는 명시적인 import profile 없이는 추측해 변환하지 않는다.
- <strong>OBJ index</strong>는 양수 1-based이고 음수는 현재 목록 끝 기준 상대 index가 될 수 있다. 0은 유효하지 않다.
- <strong>vertex dedup key</strong>는 (position_index, uv_index?, normal_index?) tuple이다. 위치가 같아도 UV/normal index가 다르면 다른 internal Vertex다.
- **n-gon** fan triangulation은 볼록 polygon에만 안전하다. 교육용 기준선은 triangle/convex face로 제한하거나 명시적으로 거부한다.
- <strong>glTF</strong>는 오른손 `+Y` up, `+Z` forward, `-X` right다. 내부 왼손 `+X` right, `+Y` up, `+Z` forward로 옮길 때 `C=diag(-1,1,1,1)`로 X축을 한 번 반사한다. accessor/bufferView와 node transform, primitive material, baseColor sRGB와 data texture linear 의미도 함께 해석해야 한다.

## 핵심 식과 불변조건

```text
resolve positive OBJ index i -> i-1
resolve negative OBJ index i -> current_count + i
dedup[(pi,ti,ni)] -> internal vertex index
convex fan: (v0,v1,v2), (v0,v2,v3), ...
glTF basis conversion: C = diag(-1,1,1,1)
p_lh = C*p_gltf, n_lh = normalize(C3*n_gltf), M_lh = C*M_gltf*C
triangle list winding: (i0,i1,i2) -> (i0,i2,i1)
tangent_lh = (C3*tangent.xyz, -tangent.w)
```

## 알고리즘과 구현 순서

1. JS가 파일 크기 상한을 먼저 확인하고 ArrayBuffer를 Wasm load_mesh에 전달한다.
1. Rust가 UTF-8 또는 허용 텍스트 정책으로 OBJ line을 읽고 v/vt/vn/f만 파싱한다. 지원하지 않는 문법은 무시/오류 정책을 명시한다.
1. 각 face token을 index tuple로 파싱하고 양/음 index를 현재 배열 길이에 대해 resolve하며 범위를 검사한다.
1. tuple dedup map으로 internal Vertex를 만들고 face를 triangle로 변환해 index buffer에 추가한다.
1. normal이 없으면 triangle geometric normal을 면적 가중으로 vertex accumulator에 더하고 마지막에 normalize한다. hard edge 정책은 smoothing group 지원 전까지 문서화한다.
1. Mesh validation을 다시 수행하고 bounding box/center/scale을 계산해 카메라 framing에 사용한다.
1. glTF 확장에서는 position/normal/tangent와 morph delta를 C로 변환하고 triangle winding을 한 번 뒤집는다. node matrix와 inverse bind matrix는 `C*M*C`로 바꾸며, TRS animation도 같은 basis에서 행렬 동등성을 검증한다. triangle strip/fan은 topology별로 처리하거나 triangle list로 확장한다.
1. glTF camera를 지원한다면 source camera의 world eye/forward/up을 계산해 C로 옮긴 뒤 내부 `look_at_lh`를 만든다. glTF camera의 local -Z lens를 내부 camera에 그대로 복사하지 않는다.

```text
for line in obj_lines:
  tag, fields = split_ascii_whitespace(line)
  if tag == "v":  positions.push(parse_vec3(fields))
  if tag == "vt": uvs.push(parse_vec2(fields))
  if tag == "vn": normals.push(parse_vec3(fields))
  if tag == "f":
    face = []
    for token in fields:
      key = resolve_obj_tuple(token, current_counts)
      idx = dedup.get_or_insert(key, make_internal_vertex(key))
      face.push(idx)
    require supported_convex_face(face)
    for i in 1 .. face.len-1:
      indices.extend([face[0], face[i], face[i+1]])
```

## JS-Wasm 경계

JS는 File/drag-and-drop, 파일명, bytes 획득과 오류 UI를 맡는다. Rust는 포맷 파싱, index resolve, dedup, normal 생성, validation을 맡는다. 수천 개 JS object로 vertex를 만들지 않는다. glTF 확장에서는 JS가 URI resource를 모아 bytes를 제공할 수 있지만 accessor 해석은 한 계층에서만 수행한다.

## 코딩 에이전트 작업 명세

- 지원 OBJ subset과 제한을 README에 명시하고 parser에 입력 크기/정점/face 수 상한을 둔다.
- OBJ baseline의 LH 축 profile과, profile이 없는 다른 축 입력을 추측하지 않는 정책을 README와 UI에 명시한다.
- 양수/음수 index, missing vt/vn, seam tuple, invalid token fixture를 단위 테스트한다.
- load progress와 parse 오류를 JS UI에 표시하고 기존 scene을 오류 상태로 덮어쓰지 않는다.
- glTF는 별도 milestone로 두고 Khronos glTF 2.0 규약을 따르는 parser/library를 선택하되 전체 spec 수제 구현을 기본 과제로 만들지 않는다. 지원하는 mesh/node/animation/camera 범위마다 X reflection과 winding 보정을 한 adapter에 모은다.

이 milestone의 실제 GLB 2.0 범위와 staged image/scene commit은 [26장 GLB 장면, Skinning과 Animation](26-glb-장면-skinning-animation.md)에서 구현한다.

## 검증 기준

- 동일 position에 서로 다른 UV가 있는 face가 internal vertex를 분리해 texture seam을 보존해야 한다.
- 음수 OBJ index가 당시 배열 끝 기준으로 정확히 resolve되어야 한다.
- 범위 밖 index, NaN/Inf 숫자, 비정상적으로 큰 count가 panic이나 무제한 allocation 없이 오류가 되어야 한다.
- normal이 없는 단순 plane에서 생성 normal이 winding과 일치하고 단위 길이여야 한다.
- glTF fixture에서 `C*(M*p) == (C*M*C)*(C*p)`가 성립하고, 반사와 winding swap 뒤 geometric normal과 변환 normal의 dot이 양수여야 한다.
- tangent를 지원한다면 변환된 bitangent가 source bitangent를 C로 옮긴 결과와 같아야 한다. quaternion/TRS animation을 지원한다면 변환 rotation matrix가 `C*R*C`와 같아야 한다.
- 모델 bounding box를 이용한 frame camera가 극단적으로 크거나 작은 모델도 화면에 맞춰야 한다.

### 자주 생기는 오류

- OBJ position index만 dedup하면 UV와 normal seam이 사라진다. 전체 tuple을 key로 쓴다.
- 모든 n-gon을 fan으로 자르면 오목 polygon이 잘못된다. subset 제한 또는 ear-clipping/검증된 parser를 쓴다.
- glTF position만 반사하고 winding, normal, tangent handedness 또는 node/skin transform을 빠뜨리면 culling과 조명이 서로 다른 좌표계를 사용한다. 변환은 importer의 단일 경계에서 원자적으로 적용한다.
- 파일 전체를 신뢰하고 reserve하면 악의적 count가 메모리를 고갈시킬 수 있다. 상한과 checked arithmetic을 둔다.
