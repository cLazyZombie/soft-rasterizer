# 부록 E. 최종 Capstone 평가표

점수보다 중요한 것은 각 항목에 재현 가능한 증거가 있다는 점이다. 기능을 많이 넣었지만 좌표 규약과 테스트가 없는 결과보다, 제한된 기능을 정확히 구현하고 한계를 설명한 결과를 높게 평가한다.

| **평가 항목** | **필수 증거** | **비중** |
| --- | --- | --- |
| 파이프라인 정확성 | 6면 homogeneous clipping, top-left, depth, perspective 속성의 수치/golden 증거 | 25 |
| 시각 결과 | 외부 textured model, Lambert/Blinn, sRGB linear 경로, resize | 15 |
| 장치 연결 | rAF dt, pointer/keyboard snapshot, stale memory view 대응, 오류 UI | 10 |
| 에셋 안전성 | 크기/index/NaN 제한, 명시된 OBJ/glTF subset, texture 색 공간 | 10 |
| 디버깅성 | wireframe, triangle ID, barycentric, depth, normal, UV, mip/overdraw 중 핵심 뷰 | 10 |
| 테스트 | native unit/property, golden, browser smoke, 제출 순서/quad owner 회귀 | 15 |
| 성능 방법 | release p50/p95, 조건 기록, 병목 근거, 정확성 보존 전후 비교 | 10 |
| 설명과 한계 | 결정 문서, controls/build/test 재현법, transparency/threads 한계 | 5 |

## 완료 시 데모 시나리오

1. 앱 시작: checker cube가 기본 장면으로 보이고 overlay에 해상도/삼각형/frame ms가 표시된다.
1. 카메라: orbit/fly, resize, near plane 통과에서도 화면이 안정적이다.
1. 에셋: 외부 model/texture를 로드하고 오류 파일은 기존 장면을 유지한 채 설명한다.
1. 품질: unlit/Lambert/Blinn, nearest/bilinear/mip, no-AA/AA를 비교한다.
1. 정확성: clipping fixture, quad top-left fixture, depth order fixture를 debug menu에서 재생한다.
1. 성능: scalar와 선택한 최적화 경로의 동일 이미지와 p50/p95를 같은 조건으로 보여 준다.
