# 2장. Rust-Wasm 프로젝트와 역할 경계

> _좋은 경계는 언어를 나누는 것이 아니라 자주 변하는 장치 코드와 결정적인 렌더링 코어를 나눈다._

> **이번 장의 눈에 보이는 결과**  순수 Rust 단위 테스트와 브라우저 통합 실행이 같은 renderer-core를 사용하고, JS-Wasm 왕복 횟수가 프레임당 소수 회로 보인다.

## 왜 필요한가

wasm32-unknown-unknown은 브라우저용으로 흔히 쓰는 최소 WebAssembly 대상이지만 파일 시스템, 일반적인 thread 생성, 터미널 출력 같은 호스트 기능을 당연히 제공하지 않는다. 따라서 브라우저를 장치 드라이버처럼 취급하고 필요한 바이트와 입력을 명시적으로 건네는 구조가 자연스럽다.

수학과 래스터 알고리즘을 web-sys에 묶으면 모든 테스트가 브라우저를 필요로 한다. core를 순수 Rust로 유지하면 작은 삼각형, 클리핑 경계, 깊이 순서를 네이티브에서 빠르게 반복 검증할 수 있다. wasm-bindgen은 소유권과 타입을 번역하는 얇은 어댑터로 제한한다.

## 배경지식

- <strong>WebAssembly 선형 메모리</strong>는 큰 바이트 배열과 비슷하다. Rust의 Vec도 결국 이 메모리 안에 놓이고 JS는 memory.buffer 위에 TypedArray 뷰를 만들 수 있다.
- <strong>ABI 경계</strong>에서는 숫자와 연속 배열이 가장 단순하고 빠르다. 복잡한 JS 객체를 픽셀 또는 정점 단위로 반복 전달하지 않는다.
- **target web** 출력은 브라우저에서 ES module로 직접 초기화할 수 있다. 이 과정의 기준 경로는 vanilla JS + ES module이며 번들러는 필수가 아니다.
- <strong>core와 adapter 분리</strong>: core는 Renderer, math, mesh, raster를 소유한다. adapter는 JS가 호출할 생성자와 포인터/길이 getter만 노출한다.

## 핵심 식과 불변조건

```text
경계 호출 비용의 원칙: O(프레임 수) 호출은 허용, O(픽셀 수) 또는 O(삼각형 수) JS-Wasm 호출은 금지
메모리 소유권: Rust owns Vec<u8>; JS borrows a temporary view; JS never frees or stores the pointer as permanent truth
```

## 알고리즘과 구현 순서

1. renderer-core의 공개 API를 브라우저 타입 없이 설계한다. 입력도 f32, bool, 작은 Rust 구조체 또는 고정 배열로 표현한다.
1. renderer-wasm에서 JS용 Renderer 래퍼를 만들고 core Renderer를 내부에 소유한다.
1. JS 초기화에서 Wasm module exports와 Renderer 인스턴스를 한 번 보관한다.
1. 프레임버퍼 pointer, length, width, height를 읽어 TypedArray 뷰를 만든다. buffer identity, pointer, length 중 하나가 바뀌면 뷰를 새로 만든다.
1. 오류는 panic에만 의존하지 말고 생성/업로드 단계에서 명시적인 Result 또는 오류 코드로 바꿔 JS가 화면에 표시할 수 있게 한다.

```text
Rust core API:
  new(width, height) -> Result<Renderer>
  resize(width, height) -> Result
  update_and_render(dt, InputSnapshot)
  framebuffer() -> &[u8]
  stats() -> FrameStats

Wasm adapter API:
  constructor(width, height)
  resize(width, height)
  frame(dt, packed_input)
  framebuffer_ptr() -> usize
  framebuffer_len() -> usize
```

## JS-Wasm 경계

파일 선택, 이미지 디코딩, Canvas 크기, PointerEvent, KeyboardEvent, requestAnimationFrame은 JS에 남긴다. mesh 변환, 클리핑, 래스터화, 텍스처 샘플링, 깊이 검사, 조명은 Rust에 남긴다. 모델 파일 파싱은 장치 연결이 아니라 데이터 해석이므로 21장에서 Rust core 쪽에 둘 수 있다.

## 코딩 에이전트 작업 명세

- core가 web-sys와 wasm-bindgen에 의존하지 않는지 Cargo dependency graph와 소스 import로 검사한다.
- Wasm adapter의 공개 메서드를 최소화하고, 픽셀/정점 단위 export를 만들지 않는다.
- JS 프레임 루프에 호출 횟수와 프레임 단계 시간을 표시하는 개발용 overlay를 추가한다.
- Rust panic hook은 개발 빌드에서만 활성화하고, release에서는 사용자에게 보여 줄 오류 경로를 별도로 유지한다.

## 검증 기준

- cargo test가 브라우저 없이 renderer-core 테스트를 통과한다.
- wasm-pack의 web target 빌드 결과를 ES module로 초기화하고 로컬 HTTP 서버에서 실행한다. file:// 직접 열기는 기준 경로로 사용하지 않는다.
- 한 프레임에서 high-level render 호출은 1회이고, pointer/통계 getter를 포함해 호출 수가 해상도나 삼각형 수에 비례하지 않는다.
- 브라우저 resize 후 오래된 TypedArray를 계속 쓰지 않고 새 memory.buffer 뷰가 생성되는지 테스트한다.

### 자주 생기는 오류

- Vec을 반환해 JS 배열로 복사하는 편리한 API와, pointer로 메모리를 빌리는 API를 섞으면 어느 쪽이 복사인지 불명확해진다. 프레임버퍼는 명시적으로 borrowed view 경로를 쓴다.
- Wasm memory가 grow되면 기존 ArrayBuffer 뷰가 분리될 수 있다. pointer가 같아 보여도 memory.buffer가 바뀌면 뷰를 다시 만든다.
- core가 Date, Window, HtmlCanvasElement를 알게 되면 테스트 경계가 무너진다. 시간과 입력은 값으로 주입한다.
