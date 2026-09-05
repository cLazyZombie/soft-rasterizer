# 부록 A. 코딩 에이전트와 장별로 일하는 방법

이 교재에서 학습자의 일은 소스를 길게 입력하는 것이 아니라 <strong>규약을 선택하고 결과를 판정하는 것</strong>이다. 에이전트에게는 한 장의 범위만 주고, 다음 장 기능을 미리 넣지 않게 한다. 구현 결과를 받으면 설명보다 먼저 테스트와 debug 화면으로 계약을 확인한다.

## 권장 장별 사이클

1. 교재의 좌표/색/깊이 규약과 이번 장의 불변조건을 읽고 결정 문서에 반영한다.
1. 에이전트에게 수정 가능 모듈, 금지 범위, 공개 API, 테스트 fixture, 완료 보고 형식을 함께 준다.
1. 에이전트가 먼저 실패하는 테스트 또는 관찰 가능한 debug scene을 만들게 한다.
1. 최소 구현으로 테스트를 통과시키고 browser demo를 확인한다.
1. golden image와 FrameStats를 저장한 뒤에만 refactor 또는 최적화를 허용한다.
1. 다음 장으로 넘어가기 전에 이번 장의 흔한 오류 fixture도 회귀 테스트로 남긴다.

## 복사해 쓸 수 있는 에이전트 작업 요청 틀

```text
역할:
  기존 구조와 좌표 규약을 보존하는 구현 에이전트

이번 목표:
  [한 장의 눈에 보이는 산출물]

반드시 지킬 규약:
  [열벡터 / LH / +Z / z 0..1 / screen y-down 등]

수정 허용:
  [모듈과 파일]

이번 작업에서 금지:
  [다음 장 기능, 새 그래픽 API, 불필요한 dependency, 전체 재작성]

알고리즘 계약:
  [식, 포함 규칙, 입력/출력, 오류 처리]

필수 테스트:
  [수치 fixture, invariant, golden image, browser smoke]

완료 보고:
  바뀐 구조, 테스트 명령과 결과, debug 화면,
  남은 한계, 성능 수치가 있다면 측정 조건
```

> **에이전트 리뷰 질문**  어떤 파일을 얼마나 바꿨는가보다, 어떤 불변조건을 테스트로 고정했고 어떤 반례를 추가했는지 묻는다.

## 에이전트가 앞서 나가지 못하게 하는 범위선

- 11장 전에는 texture/lighting을 구현하지 않는다. 그라데이션·선·wireframe으로 각 단계를 확인하고, 11장에서 단색 coverage, 12장에서 barycentric 색을 추가한다.
- 15장의 scalar 컬러 큐브가 golden을 통과하기 전 worker/SIMD를 추가하지 않는다.
- 외부 asset parser가 생겨도 core 테스트가 파일/DOM에 의존하지 않게 한다.
- 26장 GLB parser는 binary scene/material/TRS animation/skinning까지만 다룬다. morph/PBR/cross-fade를 편의상 함께 구현하지 않고 embedded image decode는 브라우저 장치 계층에 남긴다.
- 성능 때문에 unsafe를 제안하면 먼저 safe reference와 pixel diff, 범위 proof를 요구한다.
- 좌표 규약을 바꾸는 refactor는 결정 문서, 수학 테스트, 모든 golden을 한 작업에서 갱신해야 한다.
