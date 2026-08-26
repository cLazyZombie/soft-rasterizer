export const RasterPathRequest = Object.freeze({
  SCALAR: 0,
  TILED_16: 1,
  SHARED_THREADS: 2,
});

const PATH_LABELS = Object.freeze({
  [RasterPathRequest.SCALAR]: "Scalar reference",
  [RasterPathRequest.TILED_16]: "Single-thread 16×16 tiled",
  [RasterPathRequest.SHARED_THREADS]: "Shared threads request",
});

export function resolveRasterPath(requestedMode, capabilities) {
  if (![0, 1, 2].includes(requestedMode)) {
    throw new Error(`알 수 없는 raster path 요청입니다: ${requestedMode}`);
  }
  if (
    capabilities === null ||
    typeof capabilities !== "object" ||
    typeof capabilities.crossOriginIsolated !== "boolean" ||
    typeof capabilities.wasmSharedMemory !== "boolean" ||
    typeof capabilities.parallelSchedulerBuilt !== "boolean"
  ) {
    throw new Error("raster path capability는 세 boolean 값을 모두 제공해야 합니다");
  }
  if (requestedMode !== RasterPathRequest.SHARED_THREADS) {
    return Object.freeze({
      requestedMode,
      actualMode: requestedMode,
      requestedLabel: PATH_LABELS[requestedMode],
      actualLabel: PATH_LABELS[requestedMode],
      usedFallback: false,
      reason: null,
    });
  }
  if (
    capabilities.crossOriginIsolated &&
    capabilities.wasmSharedMemory &&
    capabilities.parallelSchedulerBuilt
  ) {
    throw new Error(
      "parallelSchedulerBuilt=true인 shared path는 현재 single-thread capstone resolver가 지원하지 않습니다",
    );
  }

  const reasons = [];
  if (!capabilities.crossOriginIsolated) {
    reasons.push("crossOriginIsolated=false: COOP same-origin과 COEP require-corp가 필요합니다");
  }
  if (!capabilities.wasmSharedMemory) {
    reasons.push("현재 Wasm은 shared-memory build가 아닙니다");
  }
  if (!capabilities.parallelSchedulerBuilt) {
    reasons.push("현재 capstone baseline에는 병렬 tile scheduler가 포함되지 않았습니다");
  }
  return Object.freeze({
    requestedMode,
    actualMode: RasterPathRequest.TILED_16,
    requestedLabel: PATH_LABELS[requestedMode],
    actualLabel: PATH_LABELS[RasterPathRequest.TILED_16],
    usedFallback: true,
    reason: `${reasons.join(" · ")} · single-thread tiled로 안전하게 fallback합니다`,
  });
}
