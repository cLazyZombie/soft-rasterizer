# CPU로 만드는 소프트웨어 래스터라이저

WebGL이나 WebGPU에 픽셀 생성을 맡기지 않고, Rust/WebAssembly가 만든 RGBA8 프레임버퍼를 Canvas 2D로 표시하는 소프트웨어 래스터라이저 프로젝트입니다.

현재 저장소는 25장짜리 구현 교재와 확정된 렌더링 계약으로 시작합니다. 각 장의 수식, 불변조건, 테스트 기준을 먼저 이해한 뒤 `renderer-core`, `renderer-wasm`, `web`을 단계적으로 구현합니다.

## 먼저 읽기

- [전체 과정 소개와 목차](doc/00-들어가며.md)
- [좌표·카메라·깊이 결정](docs/decisions/coordinates.md)
- [코딩 에이전트와 장별로 일하는 방법](doc/appendix-a-코딩-에이전트와-장별로-일하는-방법.md)
- [최소 공개 계약과 데이터 구조](doc/appendix-b-최소-공개-계약과-데이터-구조.md)
- [수학과 알고리즘 빠른 참조](doc/appendix-c-수학과-알고리즘-빠른-참조.md)
- [저장소 작업 지침](AGENTS.md)

## 고정 렌더링 규약

- 열벡터와 `p_clip = P * V * M * p_object`
- 왼손 object/world/view 공간: `+X` 오른쪽, `+Y` 위, 카메라 전방 `+Z`
- homogeneous clip: `-w <= x <= w`, `-w <= y <= w`, `0 <= z <= w`
- NDC 깊이 `0..1`, depth clear `+infinity`, strict `<`
- screen y-down, `orient2d > 0` front face, 고정소수점 top-left coverage
- Rust/Wasm이 픽셀을 만들고 Canvas 2D는 완성된 RGBA8 이미지만 표시

세부 수식과 외부 에셋 변환 규칙은 [좌표계 결정 문서](docs/decisions/coordinates.md)를 기준으로 합니다.

## 교재

### 1부 · 픽셀, 메모리, 최소 수학

1. [소프트웨어 래스터라이저가 하는 일](doc/01-소프트웨어-래스터라이저가-하는-일.md)
2. [Rust-Wasm 프로젝트와 역할 경계](doc/02-rust-wasm-프로젝트와-역할-경계.md)
3. [프레임버퍼, 색, Canvas 표시](doc/03-프레임버퍼-색-canvas-표시.md)
4. [픽셀 좌표와 선 그리기](doc/04-픽셀-좌표와-선-그리기.md)
5. [벡터와 행렬을 필요한 만큼만](doc/05-벡터와-행렬을-필요한-만큼만.md)

### 2부 · 3D 변환과 가시성

6. [좌표 공간과 MVP 변환](doc/06-좌표-공간과-mvp-변환.md)
7. [카메라, 원근 투영, 동차좌표 w](doc/07-카메라-원근-투영-동차좌표-w.md)
8. [Mesh, 인덱스, 정점 속성](doc/08-mesh-인덱스-정점-속성.md)
9. [Winding, 퇴화 삼각형, Backface Culling](doc/09-winding-퇴화-삼각형-backface-culling.md)
10. [동차 Clip 공간에서 삼각형 자르기](doc/10-동차-clip-공간에서-삼각형-자르기.md)

### 3부 · 삼각형 래스터라이저의 핵심

11. [Edge 함수, 픽셀 중심, Top-left 규칙](doc/11-edge-함수-픽셀-중심-top-left-규칙.md)
12. [Barycentric 좌표와 속성 보간](doc/12-barycentric-좌표와-속성-보간.md)
13. [깊이 버퍼와 가려짐](doc/13-깊이-버퍼와-가려짐.md)
14. [Perspective-correct 보간](doc/14-perspective-correct-보간.md)
15. [파이프라인 조립과 컬러 3D 큐브](doc/15-파이프라인-조립과-컬러-3d-큐브.md)

### 4부 · 텍스처, 조명, 실제 입력

16. [웹 이미지 입력과 Texture 메모리](doc/16-웹-이미지-입력과-texture-메모리.md)
17. [UV 주소화, Nearest, Bilinear 샘플링](doc/17-uv-주소화-nearest-bilinear-샘플링.md)
18. [법선 변환과 Lambert 조명](doc/18-법선-변환과-lambert-조명.md)
19. [Blinn-Phong과 sRGB/Linear 색 공간](doc/19-blinn-phong과-srgb-linear-색-공간.md)
20. [마우스/키보드와 Orbit/Fly 카메라](doc/20-마우스-키보드와-orbit-fly-카메라.md)

### 5부 · 에셋, 품질, 검증, 성능

21. [외부 모델 로딩: OBJ 기준선과 glTF 확장](doc/21-외부-모델-로딩-obj-기준선과-gltf-확장.md)
22. [투명도, Alpha Test, Blending 순서](doc/22-투명도-alpha-test-blending-순서.md)
23. [Antialiasing과 Mipmap](doc/23-antialiasing과-mipmap.md)
24. [디버그 뷰, 테스트, 프로파일링](doc/24-디버그-뷰-테스트-프로파일링.md)
25. [타일링, Worker, SIMD와 최종 Capstone](doc/25-타일링-worker-simd와-최종-capstone.md)

## 부록과 원본

- [부록 A · 코딩 에이전트와 장별로 일하는 방법](doc/appendix-a-코딩-에이전트와-장별로-일하는-방법.md)
- [부록 B · 최소 공개 계약과 데이터 구조](doc/appendix-b-최소-공개-계약과-데이터-구조.md)
- [부록 C · 수학과 알고리즘 빠른 참조](doc/appendix-c-수학과-알고리즘-빠른-참조.md)
- [부록 D · 화면 증상으로 찾는 오류 단계](doc/appendix-d-화면-증상으로-찾는-오류-단계.md)
- [부록 E · 최종 Capstone 평가표](doc/appendix-e-최종-capstone-평가표.md)
- [부록 F · 공식 참고자료](doc/appendix-f-공식-참고자료.md)
- [교재 원본 DOCX](doc/software_rasterizer_curriculum_ko.docx)

## 권장 구현 구조

```text
renderer-core/    순수 Rust 수학·장면·클리핑·래스터화·프레임버퍼
renderer-wasm/    renderer-core를 감싸는 얇은 wasm-bindgen adapter
web/              Canvas 2D 표시, rAF, 입력, 파일·이미지 디코딩
tests/            결정적 fixture, golden, browser 통합 검사
docs/decisions/   결과 전체에 영향을 주는 확정 규약
```

구현과 검증 명령은 scaffold가 추가되는 장에서 [저장소 작업 지침](AGENTS.md)의 canonical `pnpm` 진입점에 연결합니다.
