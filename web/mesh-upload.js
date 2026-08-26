export const MAX_OBJ_FILE_BYTES = 8 * 1024 * 1024;

export function validateObjFileSize(size) {
  if (!Number.isSafeInteger(size) || size < 0) {
    throw new RangeError("OBJ 파일 크기는 0 이상의 안전한 정수여야 합니다.");
  }
  if (size > MAX_OBJ_FILE_BYTES) {
    throw new RangeError("OBJ 파일은 8 MiB 이하여야 합니다.");
  }
  return size;
}

export async function readObjFileBytes(
  file,
  { readBuffer = (source) => source.arrayBuffer() } = {},
) {
  if (!(file instanceof Blob)) {
    throw new TypeError("OBJ 입력은 File 또는 Blob이어야 합니다.");
  }
  validateObjFileSize(file.size);
  const buffer = await readBuffer(file);
  validateObjFileSize(buffer.byteLength);
  return new Uint8Array(buffer);
}
