# 부록 F. 공식 참고자료

도구와 브라우저 API는 바뀔 수 있으므로 일반 참고자료는 특정 버전에 묶지 않는다. 다만 재현성과 지원 profile에 직접 영향을 주는 dependency는 해당 장과 `Cargo.toml`에 exact 호환 버전을 기록한다. 구현 시점에는 아래 공식 문서와 현재 저장소의 호환 범위를 다시 확인한다. 수학/알고리즘 규약은 이 교재의 테스트가 기준이다. 링크 확인일: 2026-09-05.

[Rust rustc book - wasm32-unknown-unknown target](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html) - 브라우저에 흔히 쓰는 최소 Wasm target의 host 기능 제한과 특성.

[wasm-bindgen Guide - Deploying Rust and WebAssembly](https://wasm-bindgen.github.io/wasm-bindgen/reference/deployment.html) - web target, bundler/비번들러 배포 방식.

[wasm-bindgen Guide - Without a Bundler](https://wasm-bindgen.github.io/wasm-bindgen/examples/without-a-bundler.html) - ES module 초기화와 wasm-pack build --target web 흐름.

[wasm-bindgen Guide - Number Slices](https://wasm-bindgen.github.io/wasm-bindgen/reference/types/number-slices.html) - 숫자 slice와 JavaScript TypedArray 표현.

[MDN - ImageData constructor](https://developer.mozilla.org/en-US/docs/Web/API/ImageData/ImageData) - TypedArray 기반 ImageData 생성과 worker 사용.

[MDN - CanvasRenderingContext2D.putImageData](https://developer.mozilla.org/en-US/docs/Web/API/CanvasRenderingContext2D/putImageData) - RGBA ImageData를 Canvas에 표시하는 API.

[MDN - requestAnimationFrame](https://developer.mozilla.org/en-US/docs/Web/API/Window/requestAnimationFrame) - timestamp 기반 animation loop와 background tab 동작.

[MDN - Pointer events](https://developer.mozilla.org/en-US/docs/Web/API/Pointer_events) - mouse/pen/touch 통합 이벤트 모델.

[MDN - KeyboardEvent.code](https://developer.mozilla.org/en-US/docs/Web/API/KeyboardEvent/code) - 물리 키 위치 입력의 의미와 호환성 주의.

[MDN - WebAssembly.Memory.grow](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/Memory/grow) - memory growth 뒤 기존 ArrayBuffer view detachment.

[MDN - crossOriginIsolated](https://developer.mozilla.org/en-US/docs/Web/API/Window/crossOriginIsolated) - SharedArrayBuffer와 COOP/COEP 기반 cross-origin isolation 조건.

[Microsoft Learn - Rasterization Rules](https://learn.microsoft.com/en-us/windows/win32/direct3d11/d3d10-graphics-programming-guide-rasterizer-stage-rules) - 픽셀 중심과 top-left triangle fill 규칙.

[Microsoft Learn - XMMatrixLookAtLH](https://learn.microsoft.com/en-us/windows/win32/api/directxmath/nf-directxmath-xmmatrixlookatlh) / [XMMatrixPerspectiveFovLH](https://learn.microsoft.com/en-us/windows/win32/api/directxmath/nf-directxmath-xmmatrixperspectivefovlh) - 왼손 view와 perspective projection 생성 규약.

[Khronos - glTF 2.0 Specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html) - glTF의 오른손 좌표, 축 방향, winding, geometry/material/texture와 base color sRGB 규약.

[docs.rs - gltf 1.4.1](https://docs.rs/gltf/1.4.1/gltf/) - `Gltf::from_slice_without_validation`, 공개 `gltf::json::validation::Validate`와 mesh/skin/animation reader API. 26장 구현은 required extension/external buffer를 먼저 구분한 뒤 같은 crate의 JSON validator를 명시적으로 호출하며, default feature를 끄고 `utils`, `names`, `KHR_materials_unlit`만 사용한다.

[Khronos glTF Sample Assets - Fox](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Fox) - vendored animation GLB의 원본, 제작자 attribution과 CC0/CC BY 4.0 license 정보.

[wasm-bindgen-test documentation](https://wasm-bindgen.github.io/wasm-bindgen/wasm-bindgen-test/index.html) - 브라우저에서 실행하는 Rust/Wasm 테스트 도구.

> **마지막 원칙**  최적화된 렌더러보다 먼저 필요한 것은 설명 가능한 렌더러다. 한 픽셀이 왜 그 색과 깊이를 가졌는지 edge, barycentric, 1/w, texture, lighting 순서로 재현할 수 있으면 이후 기능은 안전하게 확장할 수 있다.
