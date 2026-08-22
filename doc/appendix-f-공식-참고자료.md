# 부록 F. 공식 참고자료

도구와 브라우저 API는 바뀔 수 있으므로 exact crate 버전은 교재에 고정하지 않았다. 구현 시점에는 아래 공식 문서와 현재 저장소의 호환 범위를 다시 확인한다. 수학/알고리즘 규약은 이 교재의 테스트가 기준이다. 확인일: 2026-08-22.

Rust rustc book - wasm32-unknown-unknown target - 브라우저에 흔히 쓰는 최소 Wasm target의 host 기능 제한과 특성.

wasm-bindgen Guide - Deploying Rust and WebAssembly - web target, bundler/비번들러 배포 방식.

wasm-bindgen Guide - Without a Bundler - ES module 초기화와 wasm-pack build --target web 흐름.

wasm-bindgen Guide - Number Slices - 숫자 slice와 JavaScript TypedArray 표현.

MDN - ImageData constructor - TypedArray 기반 ImageData 생성과 worker 사용.

MDN - CanvasRenderingContext2D.putImageData - RGBA ImageData를 Canvas에 표시하는 API.

MDN - requestAnimationFrame - timestamp 기반 animation loop와 background tab 동작.

MDN - Pointer events - mouse/pen/touch 통합 이벤트 모델.

MDN - KeyboardEvent.code - 물리 키 위치 입력의 의미와 호환성 주의.

MDN - WebAssembly.Memory.grow - memory growth 뒤 기존 ArrayBuffer view detachment.

MDN - crossOriginIsolated - SharedArrayBuffer와 COOP/COEP 기반 cross-origin isolation 조건.

Microsoft Learn - Rasterization Rules - 픽셀 중심과 top-left triangle fill 규칙.

Microsoft Learn - XMMatrixLookAtLH / XMMatrixPerspectiveFovLH - 왼손 view와 perspective projection 생성 규약.

Khronos - glTF 2.0 Specification - glTF의 오른손 좌표, 축 방향, winding, geometry/material/texture와 base color sRGB 규약.

wasm-bindgen-test documentation - 브라우저에서 실행하는 Rust/Wasm 테스트 도구.

> **마지막 원칙**  최적화된 렌더러보다 먼저 필요한 것은 설명 가능한 렌더러다. 한 픽셀이 왜 그 색과 깊이를 가졌는지 edge, barycentric, 1/w, texture, lighting 순서로 재현할 수 있으면 이후 기능은 안전하게 확장할 수 있다.
