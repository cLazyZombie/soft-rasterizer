import { decodeImageFileToRgba } from "./texture-upload.js";

export const MAX_GLB_FILE_BYTES = 32 * 1024 * 1024;

export function validateGlbFileSize(size) {
  if (!Number.isSafeInteger(size) || size < 0) {
    throw new RangeError("GLB 파일 크기는 0 이상의 안전한 정수여야 합니다.");
  }
  if (size > MAX_GLB_FILE_BYTES) {
    throw new RangeError("GLB 파일은 32 MiB 이하여야 합니다.");
  }
  return size;
}

export function hasGlbMagic(bytes) {
  return (
    bytes instanceof Uint8Array &&
    bytes.length >= 4 &&
    bytes[0] === 0x67 &&
    bytes[1] === 0x6c &&
    bytes[2] === 0x54 &&
    bytes[3] === 0x46
  );
}

export async function readGlbFileBytes(
  file,
  { readBuffer = (source) => source.arrayBuffer() } = {},
) {
  if (!(file instanceof Blob)) {
    throw new TypeError("GLB 입력은 File 또는 Blob이어야 합니다.");
  }
  validateGlbFileSize(file.size);
  const buffer = await readBuffer(file);
  validateGlbFileSize(buffer.byteLength);
  const bytes = new Uint8Array(buffer);
  if (!hasGlbMagic(bytes)) {
    throw new TypeError("GLB magic이 없습니다. JSON .gltf는 지원하지 않습니다.");
  }
  return bytes;
}

export async function prepareDecodeAndCommitGlb(
  renderer,
  bytes,
  { decodeImage = decodeImageFileToRgba } = {},
) {
  if (!(bytes instanceof Uint8Array)) {
    throw new TypeError("GLB byte 입력은 Uint8Array여야 합니다.");
  }
  validateGlbFileSize(bytes.byteLength);
  const pendingId = renderer.prepare_glb(bytes);
  try {
    const imageCount = renderer.pending_glb_image_count(pendingId);
    for (let imageIndex = 0; imageIndex < imageCount; imageIndex += 1) {
      const mimeType = renderer.pending_glb_image_mime(pendingId, imageIndex);
      const encoded = renderer.pending_glb_image_bytes(pendingId, imageIndex);
      const decoded = await decodeImage(new Blob([encoded], { type: mimeType }));
      encoded.fill(0);
      renderer.supply_glb_image_rgba(
        pendingId,
        imageIndex,
        decoded.width,
        decoded.height,
        decoded.pixels,
      );
    }
    renderer.commit_glb(pendingId);
    return pendingId;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    try {
      renderer.fail_glb(pendingId, message);
    } catch {
      // A failed commit may already consume the pending generation. The active
      // scene is still unchanged, which is the transaction boundary we need.
    }
    throw error;
  }
}
