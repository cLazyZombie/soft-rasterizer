const MAX_TEXTURE_FILE_BYTES = 32 * 1024 * 1024;
export const MAX_TEXTURE_PIXELS = 16_777_216;

export function validateDecodedTextureSize(width, height) {
  if (!Number.isInteger(width) || !Number.isInteger(height) || width <= 0 || height <= 0) {
    throw new RangeError("디코딩된 texture 크기는 양의 정수여야 합니다.");
  }
  const pixelCount = width * height;
  if (!Number.isSafeInteger(pixelCount) || pixelCount > MAX_TEXTURE_PIXELS) {
    throw new RangeError(
      `디코딩된 texture texel 수 ${pixelCount}이 최대 ${MAX_TEXTURE_PIXELS}을 넘었습니다.`,
    );
  }
  return pixelCount;
}

export async function decodeImageFileToRgba(
  file,
  {
    createBitmap = (source) => createImageBitmap(source),
    createCanvas = () => document.createElement("canvas"),
  } = {},
) {
  if (!(file instanceof Blob)) {
    throw new TypeError("이미지 입력은 File 또는 Blob이어야 합니다.");
  }
  if (file.size > MAX_TEXTURE_FILE_BYTES) {
    throw new RangeError("이미지 파일은 32 MiB 이하여야 합니다.");
  }

  let bitmap;
  try {
    bitmap = await createBitmap(file);
    validateDecodedTextureSize(bitmap.width, bitmap.height);
    const canvas = createCanvas();
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (context === null) {
      throw new Error("texture 디코드용 Canvas 2D context를 만들 수 없습니다.");
    }
    context.drawImage(bitmap, 0, 0);
    const image = context.getImageData(0, 0, bitmap.width, bitmap.height);
    return {
      width: bitmap.width,
      height: bitmap.height,
      pixels: image.data,
    };
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`이미지를 RGBA8로 디코딩하지 못했습니다: ${detail}`, { cause: error });
  } finally {
    bitmap?.close();
  }
}
