# 8장. Mesh, 인덱스, 정점 속성

> _래스터라이저의 입력은 삼각형 목록이지만, 실제 모델은 정점을 공유한다. 데이터 구조가 이후 캐시, 클리핑, 보간의 기준이 된다._

> **이번 장의 눈에 보이는 결과**  인덱스 기반 큐브 mesh가 12개 삼각형으로 구성되고, 위치/색/UV/법선 속성을 검증한 뒤 wireframe으로 표시된다.

## 왜 필요한가

정점 세 개를 삼각형마다 복제하면 구현은 단순하지만 모델이 커지고 같은 위치를 반복 변환한다. 인덱스 버퍼는 정점 배열의 세 인덱스로 삼각형을 표현한다. 다만 같은 위치라도 면마다 법선이나 UV가 다르면 별도 정점이어야 한다.

정점 속성은 래스터 단계에서 보간할 값의 출발점이다. position만 특별히 clip 공간으로 변환되고, color/UV/normal/world_pos는 클리핑과 보간을 따라 함께 이동한다.

## 배경지식

- <strong>Vertex</strong>의 최소 필드는 position Vec3, normal Vec3, uv Vec2, color Vec4다. 초기 큐브는 color를 사용하고 texture 장부터 uv를 사용한다.
- <strong>Index</strong>는 세 개씩 읽어 triangle을 만든다. 16비트와 32비트 선택보다 먼저 범위 검증과 3의 배수 검사를 한다.
- <strong>정점 공유의 한계</strong>: 큐브 모서리는 위치는 같아도 여섯 면의 평평한 법선과 UV seam 때문에 보통 24개 정점을 쓴다.
- <strong>AoS와 SoA</strong>: 교육용 기준은 Vertex 구조체 배열(AoS)이다. 프로파일 후 transform hot loop에서 SoA를 검토할 수 있다.
- <strong>Material/Primitive 분리</strong>: mesh geometry와 texture/lighting 설정을 분리하면 같은 mesh를 다른 재질로 그릴 수 있다.

## 핵심 식과 불변조건

```text
triangle_count = indices.len / 3
모든 index i에 대해 0 <= i < vertices.len
triangle = (vertices[idx[3k]], vertices[idx[3k+1]], vertices[idx[3k+2]])
```

## 큐브가 8개가 아닌 24정점을 쓰는 이유

모서리의 위치는 8개지만 정점은 위치만이 아니다. 면마다 법선과 UV가 다르면 같은 위치에도 다른 정점이 필요하다. 이 큐브는 `6면*4정점=24정점`, `6면*2삼각형*3인덱스=36인덱스`를 사용한다.

카메라 eye=(0,0,-3) 쪽을 향한 면을 보자.

```text
A=(-1,-1,-1), B=(-1,1,-1), C=(1,1,-1), D=(1,-1,-1)
triangle 1=(A,B,C), triangle 2=(A,C,D)
B-A=(0,2,0), C-A=(2,2,0)
cross(B-A,C-A)=(0,0,-4)
normalize → outward normal=(0,0,-1)
```

index는 이 정점 배열의 위치를 참조한다. 인덱스가 유효해도 normal·UV·color가 NaN이면 이후 보간이 실패하므로 Mesh 업로드는 position을 포함한 모든 정점 속성의 유한성도 검사한다.

## 알고리즘과 구현 순서

1. Vertex, Mesh, Primitive 또는 DrawItem의 최소 구조를 정의한다. 아직 복잡한 scene graph는 만들지 않는다.
1. Mesh 생성 시 indices 길이, index 범위, 유한한 position, nonzero normal 여부를 검증한다.
1. 정육면체를 면별 네 정점과 두 삼각형으로 만든다. 각 면의 winding과 법선을 일관되게 둔다.
1. 정점 변환 결과를 인덱스별로 캐시해 공유 정점을 한 프레임에 한 번만 변환한다.
1. 삼각형마다 캐시된 ClipVertex 세 개를 모아 다음 culling/clipping 단계로 넘긴다.

```text
transformed = array(mesh.vertices.len)
for i, vertex in enumerate(mesh.vertices):
  transformed[i] = vertex_stage(vertex, M, V, P)

for tri in mesh.indices.chunks_exact(3):
  a = transformed[tri[0]]
  b = transformed[tri[1]]
  c = transformed[tri[2]]
  submit_triangle(a, b, c, material_id)
```

## JS-Wasm 경계

기본 큐브는 Rust에서 생성한다. 나중에 JS가 파일 바이트를 읽어도 외부 포맷을 내부 Vertex/Mesh로 바꾸고 검증하는 책임은 Rust에 둔다. JS에서 수천 정점을 객체 배열로 만들어 하나씩 넘기는 경로는 만들지 않는다.

## 코딩 에이전트 작업 명세

- Vertex, Mesh, MaterialId, DrawItem의 최소 구조와 validation 오류 enum을 만든다.
- 24정점/36인덱스 큐브를 생성하고 각 면 winding, normal, UV를 테스트한다.
- 정점 변환 캐시를 추가하고 transformed vertex 수와 submitted triangle 수를 통계에 넣는다.
- invalid index, indices not multiple of 3, NaN position을 거부하는 테스트를 작성한다.

## 검증 기준

- 큐브는 6면, 12삼각형, 36인덱스이며 면별 normal이 단위 벡터여야 한다.
- 각 삼각형의 geometric normal과 저장된 vertex normal의 dot이 양수인지 확인해 winding/normal 일관성을 검사한다.
- 기본 왼손 카메라 `eye=(0,0,-3)`, `target=(0,0,0)`에서 카메라 쪽 큐브 면의 outward normal이 `-Z`이고, 투영 뒤 screen y-down `orient2d&gt;0` front-face가 되는 수치 fixture를 둔다.
- 공유 가능한 정점이 한 프레임에 중복 vertex stage를 통과하지 않는지 통계로 확인한다.
- 빈 mesh와 퇴화 삼각형이 있어도 전체 frame이 panic하지 않는다.

### 자주 생기는 오류

- 큐브를 8개 위치 정점만으로 만들고 면 법선을 공유하면 모서리가 둥글게 보인다. 평면 음영에는 면별 정점 분리가 필요하다.
- index를 신뢰하면 외부 파일 하나가 Wasm 메모리 경계 오류나 panic을 만들 수 있다. 업로드 시 검증한다.
- 클리핑 뒤 새 정점은 원래 index buffer에 없으므로 이후 단계는 ClipVertex 값 자체를 다뤄야 한다.
