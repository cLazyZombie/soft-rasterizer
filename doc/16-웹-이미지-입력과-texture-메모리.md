# 16장. 웹 이미지 입력과 Texture 메모리

> **PART 4 · 텍스처, 조명, 실제 입력**
>
> 브라우저 장치에서 들어온 이미지와 입력을 프레임 단위 데이터로 바꾸고, Wasm 안에서 표면의 최종 색을 계산한다.

> _이미지 파일을 읽고 디코딩하는 일과, 텍스처를 샘플링해 픽셀을 만드는 일은 서로 다른 책임이다._

> **이번 장의 눈에 보이는 결과**  JS에서 선택한 PNG/JPEG를 RGBA8로 디코딩해 한 번 Wasm에 업로드하고, Rust texture debug 화면으로 원본 픽셀을 표시한다.

## 왜 필요한가

브라우저는 파일 선택, fetch, 이미지 디코딩, CORS 같은 장치와 보안 문제를 잘 처리한다. 반면 텍스처 주소 계산과 필터링은 렌더링 알고리즘이므로 Rust에 있어야 한다. 이 경계를 분리하면 다양한 입력 형식을 지원하면서 sampler는 결정적으로 테스트할 수 있다.

매 프레임 JS 이미지 객체를 넘기거나 Canvas에서 색을 읽으면 큰 복사와 경계 호출이 반복된다. 업로드 시 한 번 RGBA8로 변환해 Rust가 소유하는 Texture로 복사하고, 프레임에서는 Wasm 메모리만 읽는다.

## 배경지식

- <strong>웹 디코딩 경로</strong>: File/Blob 또는 fetch Response -&gt; createImageBitmap -&gt; 임시 Canvas/OffscreenCanvas -&gt; getImageData -&gt; RGBA8.
- <strong>Texture 구조</strong>는 width, height, Vec&lt;u8&gt; pixels, color_space, sampler 설정을 가진다. row-major이며 한 행 stride는 width\*4다.
- <strong>내부 UV 규약</strong>은 이 과정에서 u=0 왼쪽, v=0 위쪽으로 정한다. OBJ/glTF importer가 다른 규약을 만나면 import 시 한 번 변환한다.
- <strong>업로드 검증</strong>: width/height가 0이 아니고 len=width\*height\*4이며 최대 texture 픽셀 수를 넘지 않는지 확인한다.
- <strong>CORS와 tainted Canvas</strong>: 허가되지 않은 cross-origin 이미지를 Canvas에 그린 뒤 getImageData하면 보안 예외가 날 수 있다. 로컬 파일이나 CORS 허용 리소스로 시작한다.

## 핵심 식과 불변조건

```text
texture_byte(x,y) = 4 * (y * texture_width + x)
expected_len = checked(texture_width * texture_height * 4)
업로드 비용은 O(texture pixels), 프레임 샘플 비용은 O(covered fragments)
```

## 알고리즘과 구현 순서

1. JS가 선택된 Blob을 ImageBitmap으로 디코딩하고 width/height를 얻는다.
1. 같은 크기의 임시 2D Canvas에 그려 ImageData RGBA8를 얻는다. 오류와 CORS 예외를 사용자에게 표시한다.
1. 한 번의 upload_texture_rgba 호출로 width, height, 연속 bytes를 Wasm에 전달한다. adapter 복사 여부를 문서화하고 업로드는 프레임 밖에서 한다.
1. Rust가 크기/길이를 검증해 Texture를 소유하고 TextureId를 반환한다.
1. texture debug pass에서 픽셀을 1:1 또는 nearest 확대해 framebuffer에 복사해 채널/행/방향을 확인한다.

```text
JS upload path:
  bitmap = await createImageBitmap(file_or_blob)
  temp_canvas.resize(bitmap.width, bitmap.height)
  temp_context.drawImage(bitmap, 0, 0)
  rgba = temp_context.getImageData(0,0,w,h).data
  texture_id = renderer.upload_texture_rgba(w, h, rgba)
  bitmap.close()

Rust validation:
  require len == checked(w*h*4)
  require 0 < w,h <= configured_limits
  store Texture { w, h, pixels = copy(bytes) }
```

## JS-Wasm 경계

JS는 이미지 바이트를 브라우저가 이해하는 RGBA로 디코딩한다. Rust는 업로드된 바이트의 유효성, 보관, UV 주소화, 필터링, 색 공간 처리를 담당한다. 사용자가 파일을 선택하는 동안 render loop는 기존 texture로 계속 동작하고, 완료 시 TextureId만 교체한다.

## 코딩 에이전트 작업 명세

- Texture와 TextureId를 만들고 업로드 길이/크기 제한, checkerboard fallback texture를 구현한다.
- JS file input과 createImageBitmap 기반 디코더를 별도 모듈로 만들고 object URL이나 bitmap 자원을 정리한다.
- 업로드 성공/실패를 UI와 FrameStats 또는 asset status에 표시한다.
- 2x2 RGBA fixture로 채널 순서와 v 방향을 픽셀 정확 테스트한다.

## 검증 기준

- 2x2 texture의 네 모서리를 서로 다른 색으로 두고 debug 화면의 같은 모서리에 나타나는지 확인한다.
- 길이가 한 바이트 짧거나 큰 입력, 0크기, 제한 초과 크기를 명시적 오류로 거부한다.
- 업로드 후 원본 JS TypedArray를 바꿔도 Rust texture가 변하지 않는 소유권 정책인지 테스트한다.
- 렌더 프레임 중 texture 전체 복사나 Canvas getImageData가 반복되지 않는지 프로파일로 확인한다.

### 자주 생기는 오류

- 이미지의 row 0과 UV v=0 규약을 암묵적으로 두면 texture가 상하 반전된다. 2x2 모서리 fixture로 고정한다.
- FileReader의 data URL처럼 불필요한 base64 경로는 메모리와 복사를 늘린다. Blob/ImageBitmap 경로를 우선한다.
- cross-origin 이미지를 무조건 읽을 수 있다고 가정하지 않는다. 오류는 렌더러 panic이 아니라 asset UI에 표시한다.
