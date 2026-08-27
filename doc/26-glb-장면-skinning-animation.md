# 26장. GLB 장면, Skinning과 Animation

25장까지는 렌더 파이프라인의 정확성과 scalar/tiled 동등성을 완성했다. 이번 장은 그 파이프라인에 실제 GLB 2.0 장면을 연결한다. 화면의 픽셀은 여전히 Rust가 만들고 Canvas 2D는 완성된 RGBA8 프레임버퍼만 표시한다.

## 이번 장의 눈에 보이는 결과

- 앱을 열면 애니메이션이 포함된 Fox 모델이 기본 장면으로 나타난다.
- clip을 `Survey`, `Walk`, `Run` 중에서 선택하고 재생, 일시정지, 반복, seek를 조작할 수 있다.
- 사용자가 `.obj` 또는 `.glb` 파일을 선택하면 같은 Rust raster/depth/texture 경로로 렌더링한다.
- 잘못된 GLB나 이미지 decode 실패는 현재 유효한 장면을 없애지 않는다.

## 범위선

이번 장은 binary container인 `.glb`만 지원한다. JSON `.gltf`, 외부 `.bin`, 외부 image URI는 지원하지 않는다. GLB 안의 image는 PNG 또는 JPEG여야 한다.

| 지원 | 이번 장에서 제외 |
| --- | --- |
| default scene, node 계층, TRS/matrix node | glTF camera와 light |
| 여러 mesh/primitive/material | points, lines, strip, fan |
| indexed/non-indexed triangles | morph target과 weight animation |
| POSITION, NORMAL, TEXCOORD_0, COLOR_0 | sparse accessor, TANGENT, TEXCOORD_1, JOINTS_1/WEIGHTS_1 |
| JOINTS_0/WEIGHTS_0, inverse bind matrix | dual quaternion skinning |
| STEP, LINEAR, CUBICSPLINE TRS animation | clip blending과 cross-fade |
| baseColorFactor/texture, alpha, double-sided, unlit | metallic-roughness/normal/occlusion/emissive shading |
| repeat, clamp, mirrored repeat, nearest/bilinear | 진짜 trilinear filtering |

## GLB를 Rust에서 읽는 이유

브라우저는 파일 선택, HTTP fetch와 PNG/JPEG decode에 강하다. 반면 accessor 형식, index 범위, node 계층, skin joint와 animation 보간은 렌더러의 장면 계약이다. 프로젝트 importer가 strict chunk 순서·개수·정렬·padding을 먼저 검사하고, `gltf` crate로 document/BIN을 parse한 뒤 같은 crate의 JSON validator를 명시적으로 호출한다. 이 계약을 JavaScript와 Rust 양쪽에 나누면 같은 오류 검사를 두 번 만들거나 어느 쪽도 완전히 책임지지 않는 상태가 된다.

그래서 Rust importer가 GLB container와 장면 계약을 검증하고, `gltf` crate는 document parse와 accessor reader를 담당한다. JavaScript는 Rust가 추출한 encoded image만 브라우저 API로 RGBA8로 바꿔 다시 넘긴다.

```text
GLB bytes
  -> Rust: header/document/accessor/node/skin/animation 검증
  -> JavaScript: embedded PNG/JPEG만 RGBA8 decode
  -> Rust: texture 검증과 scene commit
  -> 매 frame: animation -> node global -> joint palette -> raster
```

`renderer-core`의 dependency는 다음처럼 제한한다.

```toml
gltf = { version = "1.4.1", default-features = false,
         features = ["utils", "names", "KHR_materials_unlit"] }
```

`import` feature를 켜지 않으므로 core가 파일 시스템이나 image decoder에 의존하지 않는다.

## 좌표계를 한 번만 바꾸기

glTF의 오른손 좌표를 내부 왼손 좌표로 옮길 때 X축을 반사한다.

```text
C = diag(-1, 1, 1, 1)
p_lh = C * p_gltf
M_lh = C * M_gltf * C
(i0, i1, i2) -> (i0, i2, i1)
q_lh = (qx, -qy, -qz, qw)
```

예를 들어 glTF position `(2, 3, 4)`는 내부에서 `(-2, 3, 4)`가 된다. 위치만 반사하면 winding이 뒤집히므로 index의 두 번째와 세 번째 값도 맞바꾼다. 이 두 변환을 함께 해야 변환된 geometric normal과 imported normal이 같은 방향을 본다.

UV는 좌표계 반사 때문에 뒤집지 않는다. 내부 image 메모리도 glTF와 같은 top-to-bottom 규약을 사용한다.

## Accessor에서 Mesh로

primitive마다 `POSITION`을 읽고 나머지 속성 수가 position 수와 같은지 확인한다. index accessor가 없으면 `0..vertex_count`를 만들어 non-indexed triangle을 indexed `Mesh`로 바꾼다.

normal이 없으면 각 triangle의 면적 벡터를 세 정점에 더한 뒤 정규화한다.

```text
for triangle(a, b, c):
    area_normal = cross(p[b] - p[a], p[c] - p[a])
    sum[a] += area_normal
    sum[b] += area_normal
    sum[c] += area_normal

normal[i] = normalize(sum[i])
```

면적이 큰 면이 smooth normal에 더 큰 영향을 주므로, 작은 삼각형이 결과를 과도하게 흔들지 않는다.

## Node 계층과 animation

node의 local matrix는 열벡터 규약에 맞게 `T * R * S`다. global matrix는 부모에서 자식 순서로 계산한다.

```text
global[root] = local[root]
global[child] = global[parent] * local[child]
```

animation을 평가할 때 먼저 모든 node를 bind/base pose로 되돌린다. 선택한 clip의 channel만 현재 시간에 샘플링해 translation, rotation, scale을 바꾼 뒤 global matrix를 다시 만든다. 이렇게 해야 이전 frame이나 이전 clip의 값이 다음 평가에 남지 않는다. global matrix는 재귀 호출 대신 재사용하는 명시적 node stack으로 계산하므로 허용 상한인 4,096-depth 계층도 Wasm call stack을 소모하지 않는다.

clip 변경, seek, frame update가 지원하지 않는 pose를 만들면 선택 clip, 시간, 재생 상태와 평가 vertex를 이전 유효 상태로 되돌린 뒤 오류를 반환한다. UI도 이 오류를 표시하고 실제 runtime 값으로 control을 다시 맞춘다.

- `STEP`: 왼쪽 key 값을 유지한다.
- translation/scale `LINEAR`: 두 값을 선형 보간한다.
- rotation `LINEAR`: quaternion 부호를 확인한 뒤 최단 경로 slerp를 사용한다.
- `CUBICSPLINE`: key 사이 시간으로 tangent를 스케일한 Hermite 식을 쓰고 rotation 결과를 정규화한다.

시간이 0.25초이고 key가 0초와 1초라면 보간 비율은 `0.25`다. 1초짜리 clip을 반복 재생하면서 현재 시간이 1.25초가 되면 `rem_euclid(1.0)`으로 0.25초로 돌아간다. 반복을 끄면 마지막 시간에 멈추고 재생 상태도 꺼진다.

## Linear blend skinning

각 vertex는 최대 네 joint index와 weight를 가진다. weight 합은 importer에서 1로 정규화한다.

```text
joint_matrix[j] = joint_global[j] * inverse_bind[j]

skinned_position = sum(weight[k] * joint_matrix[joint[k]] * position)
skinned_normal   = normalize(sum(weight[k] * normal_matrix[joint[k]] * normal))
```

glTF 규약에 따라 skinned mesh node 자체의 transform은 position에 다시 곱하지 않는다. joint global transform만 적용한다. mesh node transform까지 곱하면 model이 두 번 이동하는 흔한 오류가 생긴다. 따라서 skinned primitive의 mesh node가 음수 scale이어도 submit winding을 따로 뒤집지 않는다. 이 장의 선형 skin profile은 joint palette에 음수 determinant가 생기는 pose를 지원하지 않으며 명시적 오류로 거부한다.

장면의 중심과 크기를 처음 평가한 pose에서 한 번 계산해 고정 normalization matrix를 만든다. animation frame마다 bounds를 다시 맞추지 않으므로 모델이 움직일 때 camera가 확대·축소되는 것처럼 보이지 않는다.

## 재질과 sampler를 교육용 shader에 맞추기

`baseColorFactor`는 linear 값이다. 기존 `Material`은 저장된 base color를 sRGB로 간주하므로 importer에서 factor RGB를 한 번 encode한다. fragment shader가 다시 decode하면 원래 linear factor가 복원된다. alpha에는 transfer function을 적용하지 않는다.

- `doubleSided=true`인 primitive는 그 draw에서 culling을 끄고, 뒷면 fragment는 조명 계산 전에 보간 normal을 뒤집는다.
- `KHR_materials_unlit`은 UI에서 조명을 켜도 unlit을 유지한다.
- `OPAQUE`, `MASK`, `BLEND`는 22장의 depth/write/blend 정책을 그대로 사용한다.
- transparent triangle은 primitive 경계를 넘어 view `+Z` 내림차순으로 정렬한다.
- `MIRRORED_REPEAT`는 주기 2에서 좌우를 반사한다.

현재 runtime은 minification과 magnification에 하나의 texel filter를 사용하고 mip 선택은 nearest mip다. 두 glTF filter가 다르면 더 부드러운 쪽인 bilinear를 선택한다. `NEAREST_MIPMAP_LINEAR`나 `LINEAR_MIPMAP_LINEAR`처럼 mip 사이 선형 보간을 요구하는 경우도 nearest mip와 nearest/bilinear texel 조합으로 낮춘다. 두 경우 모두 `sampler_downgrades` 통계에 기록한다.

## 실패해도 기존 장면을 지키는 commit

GLB upload는 세 단계다.

1. `prepare_glb`: GLB 전체를 Rust가 검증하고 encoded image 목록을 보관한다.
2. `supply_glb_image_rgba`: JavaScript가 각 image를 decode해 RGBA8을 공급한다.
3. `commit_glb`: 모든 texture와 runtime scene을 만들 수 있을 때만 활성 장면을 교체한다.

각 prepare에는 증가하는 generation ID가 있다. 뒤에 선택한 파일이 새 ID를 만들면 느리게 끝난 이전 decode는 stale ID 오류를 받고 commit하지 못한다. parse, decode, texture 검증, runtime 구성 중 어느 단계가 실패해도 기존 cube, OBJ 또는 GLB 장면은 그대로 남는다.

새 GLB prepare나 OBJ load 시도가 시작되면 성공 여부와 관계없이 이전 pending GLB ID를 먼저 폐기한다. 오류 상태 문자열은 512자로 제한하고, document validation은 처음 8개 오류만 각각 256자까지 보관한 뒤 전체 오류 수만 덧붙여 외부 JSON이 진단 문자열 메모리를 증폭시키지 못하게 한다.

## 명시적 상한

| 항목 | 상한 |
| --- | ---: |
| GLB bytes | 32 MiB |
| embedded image encoded bytes 합 | 32 MiB |
| 전체 vertex / triangle | 각각 262,144 |
| node / primitive | 각각 4,096 |
| material / image | 512 / 64 |
| skin / skin당 joint | 128 / 256 |
| primitive별 joint palette 합 / frame | 65,536 matrices |
| animation / channel | 64 / 4,096 |
| 전체 keyframe | 1,048,576 |
| clipping 후 transparent triangle 최악치 | 65,536 |
| texture mip 포함 texel | 한 pending GLB 전체 16,777,216 |

32 MiB 입력 상한은 document parse 전에 확인한다. 이 장의 container profile은 header와 JSON `asset.version`이 모두 2.0이어야 하고 `asset.minVersion`이 2.0보다 높아서는 안 된다. URI 없는 BIN chunk가 정확히 하나여야 하며 JSON/BIN 4-byte 정렬, 추가 chunk 부재와 JSON buffer 선언 대비 최대 3-byte BIN padding까지 직접 검사한다. encoded image가 같은 bufferView를 여러 번 가리켜도 Rust 소유 복사량이 증폭되지 않도록 모든 embedded image의 encoded byte 합도 32 MiB로 제한한다.

renderer가 node, primitive, vertex, skin, animation collection을 만들기 전에 각 count 상한을 확인한다. 같은 skin을 여러 primitive가 참조해도 실제로 생성할 position/normal palette의 joint matrix 합을 frame당 65,536개로 제한한 뒤 runtime을 할당한다. accessor는 reader를 만들기 전에 component/dimension/normalized profile, stride, offset, count와 bufferView 실제 범위를 검증하며 sparse accessor는 이 장의 profile에서 명시적으로 거부한다. scene root와 skin joint 목록도 collect 전에 제한하며, inverse bind matrix는 모든 성분이 유한한 affine 행렬이어야 한다. 같은 animation 안의 `(target node, property)` 중복과 비유한 animation `dt`도 명시적 오류다.

BLEND triangle은 clip 한 개가 최대 7개 fan triangle이 되는 최악치를 import 때 계산해 65,536개로 제한한다. 따라서 frame 정렬 scratch가 입력 index 수에 의해 수백 MiB로 증폭되지 않는다. image를 공급할 때는 base부터 1×1까지의 모든 mip texel을 pending GLB 전체로 합산하며, 교체 공급은 이전 slot의 texel을 빼고 다시 계산한다. commit은 GLB 전용 texture store를 한 번에 교체하므로 이전 GLB의 texture는 회수된다. 유효하지 않은 숫자, accessor 수 불일치, BIN 실제 길이, 음수·중복 skin influence, joint 범위 초과와 node cycle도 명시적 오류다. 개별 TRS가 유한해도 parent-child global matrix, joint palette, world position 또는 정규화 transform 합성이 범위를 넘으면 frame을 commit하지 않고 이전 pose로 rollback한다. renderer는 그 animation을 일시정지하고 최대 512자의 runtime 원인을 GLB status에 남긴다.

## 검증 장면

core 테스트는 메모리에서 작은 GLB binary를 만든다. 파일 시스템이나 DOM 없이 indexed/non-indexed triangle, 누락 normal, embedded image, 재질, 세 보간 방식, 2-joint skinning과 staged commit을 검사한다. 같은 pose를 scalar와 tiled path로 렌더한 RGBA/depth도 exact match해야 한다.

browser E2E는 vendored Fox를 실제 `fetch -> Wasm parse -> browser image decode -> Wasm commit -> update_and_render -> ImageData` 경로로 로드한다. 1×와 2× DPR project가 같은 내부 해상도와 pixel hash를 내고, pause/seek/loop/clip 변경, Wasm memory growth, invalid GLB와 image decode 실패의 원자성·원인 통계, 화면의 라이선스 attribution과 WebGL/WebGPU 미사용을 함께 확인한다.

## 이전 장과 다음 확장

이 장은 21장의 glTF 좌표 adapter를 실제 container와 accessor에 연결하고, 22장의 alpha queue, 23장의 mip/SSAA, 24장의 통계, 25장의 scalar/tiled 경로를 그대로 재사용한다.

후속 확장은 tangent/normal texture, metallic-roughness PBR, morph target, cross-fade, 진짜 trilinear filtering 순서가 적절하다. 각각 현재의 scalar reference와 pixel hash를 보존하는 별도 장으로 다뤄야 한다.
