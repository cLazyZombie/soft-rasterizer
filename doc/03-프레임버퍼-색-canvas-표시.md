# 3장. 프레임버퍼, 색, Canvas 표시

> _프레임버퍼는 이후 모든 알고리즘의 종착지다. 주소 계산, 크기 검증, 화면 해상도 규약을 여기서 완전히 고정한다._

> **이번 장의 눈에 보이는 결과**  Rust가 생성한 그라데이션과 8x8 체크무늬가 Canvas에 정확히 보이고, resize와 고해상도 화면에서도 버퍼가 깨지지 않는다.

## 왜 필요한가

삼각형 알고리즘이 맞아도 byte index가 한 칸 틀리면 화면은 줄무늬나 뒤틀린 색으로 보인다. 이 장에서는 3D와 무관한 패턴만 그려 색 버퍼와 표시 경로를 독립 검증한다.

Canvas에는 CSS 크기와 실제 픽셀 크기가 있다. devicePixelRatio를 무조건 곱하면 선명해지지만 CPU 렌더링 비용은 픽셀 수에 비례해 급격히 커진다. 교육용 렌더러는 내부 해상도를 명시적으로 선택하고 CSS로 확대하는 경로를 기본으로 삼는 편이 좋다.

## 배경지식

- <strong>RGBA8</strong>은 픽셀당 R, G, B, A 각 1바이트다. 메모리는 행 우선이며 왼쪽에서 오른쪽, 위에서 아래로 저장한다.
- <strong>Uint8ClampedArray</strong>는 0보다 작은 값은 0, 255보다 큰 값은 255로 제한하는 8비트 뷰다. ImageData 생성자가 받을 수 있다.
- <strong>알파</strong>는 초기 과정에서 항상 255로 둔다. 투명도는 22장에서 premultiplied alpha와 blending을 함께 배운다.
- <strong>색 공간</strong>은 초기에는 단순 0-255 값으로 취급한다. 조명 계산을 시작하는 19장에서 sRGB 입력을 linear로 바꾸고 다시 encode하는 이유를 다룬다.

## 핵심 식과 불변조건

```text
len_color = checked(width * height * 4),  len_depth = checked(width * height)
byte = 4 * (y * width + x),  rgba = buffer[byte .. byte + 4]
픽셀 비용 비율: (2W * 2H) / (W * H) = 4배
```

## 2×2 이미지와 필요한 메모리 계산

폭과 높이가 모두 2이면 분모 `max(size-1,1)`은 1이다. 2×2에서는 네 픽셀이 모두 첫 8×8 체크 셀에 속하므로 blue=220이다. 그라데이션과 체크무늬를 함께 계산한 결과는 다음과 같다.

```text
(0,0): [  0,   0, 220, 255]
(1,0): [255,   0, 220, 255]
(0,1): [  0, 255, 220, 255]
(1,1): [255, 255, 220, 255]
```

1×1에서는 두 분모가 1이고 x=y=0이므로 `[0,0,220,255]`다. 8×8 체크무늬의 셀 선택은 `((floor(x/8)+floor(y/8)) mod 2)==0`이다. 이 값으로 두 색 중 하나를 고른다.

RGBA8는 픽셀당 4byte, f32 depth도 4byte다. 두 버퍼만 계산하면 `W*H*(4+4)`byte다. 800×600에서는 각각 1,920,000byte, 합계 3,840,000byte, 약 3.66MiB다. mesh·texture·임시 배열을 포함한 전체 앱 메모리는 이보다 크다.

## 알고리즘과 구현 순서

1. width와 height를 usize로 바꾸기 전에 0, 최대 크기, 정수 범위를 검사한다. 곱셈은 checked_mul을 사용한다.
1. resize가 실제 크기 변경일 때만 Vec을 재할당한다. 같은 크기에서는 기존 capacity를 재사용한다.
1. put_pixel(x, y, rgba)는 디버그/선 그리기용 안전 버전으로 만든다. 이 장에서는 safe indexing을 유지한다. unsafe 최적화는 범위 증명과 기준 이미지 비교를 갖춘 뒤 25장에서 별도로 검토한다.
1. x/max(width-1,1)과 y/max(height-1,1)로 그라데이션을 만들고, x/8과 y/8의 짝홀로 체크무늬를 만든다. 두 패턴은 row stride와 채널 순서를 쉽게 드러낸다.
1. JS는 내부 렌더 해상도와 CSS 표시 크기를 분리한다. 예: 내부 800x600을 CSS로 컨테이너에 맞추고 CSS image-rendering으로 확대 표시 방식을 선택한다. Canvas의 imageSmoothingEnabled는 putImageData나 CSS 확대를 제어하지 않는다.

```text
for y in 0 .. height:
  for x in 0 .. width:
    i = 4 * (y * width + x)
    color[i + 0] = round(255 * x / max(width - 1, 1))
    color[i + 1] = round(255 * y / max(height - 1, 1))
    color[i + 2] = checker(x, y) ? 220 : 40
    color[i + 3] = 255
```

## JS-Wasm 경계

Wasm은 색 버퍼의 연속 RGBA8와 내부 width/height를 제공한다. JS는 같은 길이의 Uint8ClampedArray 뷰로 ImageData를 만들고 putImageData를 호출한다. Canvas CSS 확대가 필요하면 style 크기만 바꾸고, 내부 해상도 변경은 명시적인 renderer.resize와 함께 수행한다.

## 코딩 에이전트 작업 명세

- checked resize, clear_color, safe put_pixel, gradient/checker debug pass를 구현한다.
- JS present 모듈이 memory.buffer와 pointer/length를 캐시하되 변경을 감지해 뷰를 재생성하게 한다.
- Canvas의 CSS 크기, 내부 크기, DPR, 프레임버퍼 MiB를 overlay에 표시한다.
- 프레임 hot path에서 색 Vec 재할당이 일어나지 않는지 계측 또는 테스트로 확인한다.

## 검증 기준

- 2x2 버퍼의 정확한 16바이트 배열을 기대값과 비교해 채널 순서와 행 순서를 검사한다.
- 가로 그라데이션은 왼쪽 0, 오른쪽 255이며 세로 그라데이션은 위 0, 아래 255여야 한다.
- 1x1, 홀수 크기, 매우 넓고 낮은 크기, resize 연속 호출에서 panic이나 경계 쓰기가 없어야 한다.
- resize 전후에 JS view.buffer identity 또는 byteLength가 달라지는 경우 새 뷰로 표시되는지 확인한다.

### 자주 생기는 오류

- width \* height \* 4를 검증 없이 계산하면 overflow 뒤 작은 버퍼를 만들고 큰 인덱스로 쓸 수 있다.
- canvas.style.width와 canvas.width를 혼동하면 브라우저가 이미지를 확대/축소해 흐릿하거나 비용이 예상과 달라진다.
- ImageData의 width \* height \* 4와 view 길이가 다르면 예외가 발생한다. pointer와 len을 한 세트로 읽는다.
