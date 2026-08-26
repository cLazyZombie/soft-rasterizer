import init, { Renderer } from "./pkg/renderer_wasm.js";
import { FramebufferPresenter } from "./present.js";

const MAX_FRAME_DT_SECONDS = 0.1;

// clientWidth/clientHeight는 물리 픽셀이 아니라 이미 DPR로 나눈 CSS 논리 픽셀이다.
// 따라서 1920 물리 픽셀 화면이 DPR 2라면 960 논리 픽셀만 렌더링한다.
function logicalRenderSize(canvas) {
  return {
    width: Math.max(1, canvas.clientWidth),
    height: Math.max(1, canvas.clientHeight),
  };
}

function rendererStats(renderer) {
  return {
    frameIndex: renderer.stats_frame_index(),
    dtSeconds: renderer.stats_dt_seconds(),
    inputBits: renderer.stats_input_bits(),
    inputVertices: renderer.stats_input_vertices(),
    inputTriangles: renderer.stats_input_triangles(),
    transformedVertices: renderer.stats_transformed_vertices(),
    submittedTriangles: renderer.stats_submitted_triangles(),
    culledTriangles: renderer.stats_culled_triangles(),
    degenerateTriangles: renderer.stats_degenerate_triangles(),
    invalidTriangles: renderer.stats_invalid_triangles(),
    fullyClippedTriangles: renderer.stats_fully_clipped_triangles(),
    clipInvalidTriangles: renderer.stats_clip_invalid_triangles(),
    generatedTriangles: renderer.stats_generated_triangles(),
    maxClipPolygonVertices: renderer.stats_max_clip_polygon_vertices(),
    rasterizedTriangles: renderer.stats_rasterized_triangles(),
    shadedSamples: renderer.stats_shaded_samples(),
    depthPassedSamples: renderer.stats_depth_passed_samples(),
    depthFailedSamples: renderer.stats_depth_failed_samples(),
    invalidDepthSamples: renderer.stats_invalid_depth_samples(),
    maxBarycentricSumError: renderer.stats_max_barycentric_sum_error(),
    interpolatedInvWSamples: renderer.stats_interpolated_inv_w_samples(),
    invalidInterpolationSamples: renderer.stats_invalid_interpolation_samples(),
    minInterpolatedInvW: renderer.stats_min_interpolated_inv_w(),
    maxInterpolatedInvW: renderer.stats_max_interpolated_inv_w(),
    debugPixels: renderer.stats_debug_pixels(),
    invalidValues: renderer.stats_invalid_values(),
  };
}

function collectInputSnapshot() {
  // 2장은 프레임 입력의 ABI만 고정한다. 키/포인터 의미와 event collector는 20장 범위다.
  return 0;
}

function formatMilliseconds(value) {
  return `${value.toFixed(3)} ms`;
}

function framebufferMiB(size) {
  return (size.width * size.height * 4) / (1024 * 1024);
}

function canvasPixelHash(context, width, height) {
  const bytes = context.getImageData(0, 0, width, height).data;
  let hash = 0x811c9dc5;
  for (const byte of bytes) {
    hash ^= byte;
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

async function bootstrap() {
  const canvas = document.querySelector("#framebuffer");
  const errorOutput = document.querySelector("#error");
  const cullModeSelect = document.querySelector("#cull-mode");
  const windingDebugCheckbox = document.querySelector("#winding-debug");
  const barycentricDebugCheckbox = document.querySelector("#barycentric-debug");
  const clipDebugCheckbox = document.querySelector("#clip-debug");
  const coverageDebugCheckbox = document.querySelector("#coverage-debug");
  const interpolationDebugCheckbox = document.querySelector("#interpolation-debug");
  const perspectiveDebugCheckbox = document.querySelector("#perspective-debug");
  const attributeInterpolationModeSelect = document.querySelector(
    "#attribute-interpolation-mode",
  );
  const depthDebugCheckbox = document.querySelector("#depth-debug");
  const depthOrderReversedCheckbox = document.querySelector("#depth-order-reversed");
  const depthDebugModeSelect = document.querySelector("#depth-debug-mode");
  const context = canvas.getContext("2d", { alpha: false });
  if (context === null) {
    throw new Error("Canvas 2D context를 만들 수 없습니다.");
  }

  const wasm = await init();
  const initialSize = logicalRenderSize(canvas);
  canvas.width = initialSize.width;
  canvas.height = initialSize.height;
  const renderer = new Renderer(initialSize.width, initialSize.height);
  const presenter = new FramebufferPresenter(context, wasm.memory, renderer);
  let previousTimestamp = null;
  let updateCalls = 0;
  let resizeEvents = 0;
  let resizeScheduled = false;
  let currentSize = initialSize;
  let lastFrameMetrics = null;
  let coordinateDebugText = "좌표 계산 대기 중";

  const updateStatus = () => {
    document.querySelector("#internal-size").textContent = `${currentSize.width} × ${currentSize.height} px`;
    document.querySelector("#css-size").textContent = `${canvas.clientWidth} × ${canvas.clientHeight} CSS px`;
    document.querySelector("#display-scale").textContent = `${window.devicePixelRatio || 1}× (논리 해상도 사용)`;
    document.querySelector("#present-path").textContent = "Rust/Wasm RGBA8 → Canvas 2D";
    document.querySelector("#framebuffer-mib").textContent = `${framebufferMiB(currentSize).toFixed(2)} MiB`;
    document.querySelector("#line-algorithm").textContent = "All-octants Bresenham (Rust)";
    document.querySelector("#coverage-algorithm").textContent =
      "S=256 incremental edge · pixel center · top-left (Rust)";
    document.querySelector("#interpolation-algorithm").textContent =
      attributeInterpolationModeSelect.value === "1"
        ? "Σ(λ · attribute/w) ÷ Σ(λ/w) · normal 재정규화 (Rust)"
        : "affine attribute 비교 경로 (Rust)";
    document.querySelector("#depth-algorithm").textContent =
      "affine z_ndc · strict < · +infinity clear (Rust)";
    document.querySelector("#math-convention").textContent = "열벡터 · LH · +Z 전방";
    document.querySelector("#coordinate-debug").textContent = coordinateDebugText;
    document.querySelector("#frame-index").textContent = String(updateCalls);
    if (lastFrameMetrics !== null) {
      document.querySelector("#high-level-calls").textContent = String(
        lastFrameMetrics.highLevelRenderCalls,
      );
      document.querySelector("#wasm-boundary-calls").textContent = String(
        lastFrameMetrics.wasmBoundaryCalls,
      );
      document.querySelector("#input-time").textContent = formatMilliseconds(
        lastFrameMetrics.inputMs,
      );
      document.querySelector("#update-time").textContent = formatMilliseconds(
        lastFrameMetrics.updateMs,
      );
      document.querySelector("#present-time").textContent = formatMilliseconds(
        lastFrameMetrics.presentMs,
      );
      document.querySelector("#frame-time").textContent = formatMilliseconds(
        lastFrameMetrics.totalMs,
      );
    }
  };

  const renderFrame = (dtSeconds) => {
    const frameStart = performance.now();
    const inputStart = performance.now();
    const packedInput = collectInputSnapshot();
    const inputEnd = performance.now();
    const updateStart = performance.now();
    renderer.update_and_render(dtSeconds, packedInput);
    coordinateDebugText = renderer.coordinate_debug_text();
    const updateEnd = performance.now();
    updateCalls += 1;
    const presentStart = performance.now();
    const presentBoundaryCalls = presenter.present();
    const presentEnd = performance.now();
    lastFrameMetrics = {
      highLevelRenderCalls: 1,
      wasmBoundaryCalls: 2 + presentBoundaryCalls,
      inputMs: inputEnd - inputStart,
      updateMs: updateEnd - updateStart,
      presentMs: presentEnd - presentStart,
      totalMs: presentEnd - frameStart,
    };
    updateStatus();
  };

  const setCullMode = (mode) => {
    renderer.set_cull_mode(mode);
    cullModeSelect.value = String(mode);
  };

  const setWindingDebugMode = (mode) => {
    renderer.set_winding_debug_mode(mode);
    windingDebugCheckbox.checked = mode === 1;
    barycentricDebugCheckbox.checked = mode === 2;
  };

  const setClipDebugEnabled = (enabled) => {
    renderer.set_clip_debug_enabled(enabled);
    clipDebugCheckbox.checked = enabled;
    if (enabled) {
      coverageDebugCheckbox.checked = false;
      interpolationDebugCheckbox.checked = false;
      perspectiveDebugCheckbox.checked = false;
      depthDebugCheckbox.checked = false;
    }
  };

  const setCoverageDebugEnabled = (enabled) => {
    renderer.set_coverage_debug_enabled(enabled);
    coverageDebugCheckbox.checked = enabled;
    if (enabled) {
      clipDebugCheckbox.checked = false;
      interpolationDebugCheckbox.checked = false;
      perspectiveDebugCheckbox.checked = false;
      depthDebugCheckbox.checked = false;
    }
  };

  const setInterpolationDebugEnabled = (enabled) => {
    renderer.set_interpolation_debug_enabled(enabled);
    interpolationDebugCheckbox.checked = enabled;
    if (enabled) {
      clipDebugCheckbox.checked = false;
      coverageDebugCheckbox.checked = false;
      perspectiveDebugCheckbox.checked = false;
      depthDebugCheckbox.checked = false;
    }
  };

  const setPerspectiveDebugEnabled = (enabled) => {
    renderer.set_perspective_debug_enabled(enabled);
    perspectiveDebugCheckbox.checked = enabled;
    if (enabled) {
      clipDebugCheckbox.checked = false;
      coverageDebugCheckbox.checked = false;
      interpolationDebugCheckbox.checked = false;
      depthDebugCheckbox.checked = false;
    }
  };

  const setAttributeInterpolationMode = (mode) => {
    renderer.set_attribute_interpolation_mode(mode);
    attributeInterpolationModeSelect.value = String(mode);
  };

  const setDepthDebugEnabled = (enabled) => {
    renderer.set_depth_debug_enabled(enabled);
    depthDebugCheckbox.checked = enabled;
    if (enabled) {
      clipDebugCheckbox.checked = false;
      coverageDebugCheckbox.checked = false;
      interpolationDebugCheckbox.checked = false;
      perspectiveDebugCheckbox.checked = false;
    }
  };

  const setDepthOrderReversed = (reversed) => {
    renderer.set_depth_order_reversed(reversed);
    depthOrderReversedCheckbox.checked = reversed;
  };

  const setDepthDebugMode = (mode) => {
    renderer.set_depth_debug_mode(mode);
    depthDebugModeSelect.value = String(mode);
  };

  // Reload/history restoration can preserve form values independently of the newly created Wasm state.
  setCullMode(Number(cullModeSelect.value));
  setWindingDebugMode(
    barycentricDebugCheckbox.checked ? 2 : windingDebugCheckbox.checked ? 1 : 0,
  );
  setClipDebugEnabled(clipDebugCheckbox.checked);
  setCoverageDebugEnabled(coverageDebugCheckbox.checked);
  setInterpolationDebugEnabled(interpolationDebugCheckbox.checked);
  setPerspectiveDebugEnabled(perspectiveDebugCheckbox.checked);
  setAttributeInterpolationMode(Number(attributeInterpolationModeSelect.value));
  setDepthDebugEnabled(depthDebugCheckbox.checked);
  setDepthOrderReversed(depthOrderReversedCheckbox.checked);
  setDepthDebugMode(Number(depthDebugModeSelect.value));

  cullModeSelect.addEventListener("change", () => {
    setCullMode(Number(cullModeSelect.value));
    renderFrame(0);
  });
  windingDebugCheckbox.addEventListener("change", () => {
    setWindingDebugMode(windingDebugCheckbox.checked ? 1 : 0);
    renderFrame(0);
  });
  barycentricDebugCheckbox.addEventListener("change", () => {
    setWindingDebugMode(barycentricDebugCheckbox.checked ? 2 : 0);
    renderFrame(0);
  });
  clipDebugCheckbox.addEventListener("change", () => {
    setClipDebugEnabled(clipDebugCheckbox.checked);
    renderFrame(0);
  });
  coverageDebugCheckbox.addEventListener("change", () => {
    setCoverageDebugEnabled(coverageDebugCheckbox.checked);
    renderFrame(0);
  });
  interpolationDebugCheckbox.addEventListener("change", () => {
    setInterpolationDebugEnabled(interpolationDebugCheckbox.checked);
    renderFrame(0);
  });
  perspectiveDebugCheckbox.addEventListener("change", () => {
    setPerspectiveDebugEnabled(perspectiveDebugCheckbox.checked);
    renderFrame(0);
  });
  attributeInterpolationModeSelect.addEventListener("change", () => {
    setAttributeInterpolationMode(Number(attributeInterpolationModeSelect.value));
    renderFrame(0);
  });
  depthDebugCheckbox.addEventListener("change", () => {
    setDepthDebugEnabled(depthDebugCheckbox.checked);
    renderFrame(0);
  });
  depthOrderReversedCheckbox.addEventListener("change", () => {
    setDepthOrderReversed(depthOrderReversedCheckbox.checked);
    renderFrame(0);
  });
  depthDebugModeSelect.addEventListener("change", () => {
    setDepthDebugMode(Number(depthDebugModeSelect.value));
    renderFrame(0);
  });

  const applyDisplayResize = () => {
    resizeScheduled = false;
    const size = logicalRenderSize(canvas);
    if (size.width === currentSize.width && size.height === currentSize.height) {
      updateStatus();
      return;
    }
    if (!renderer.resize(size.width, size.height)) {
      errorOutput.textContent = renderer.last_error();
      return;
    }
    canvas.width = size.width;
    canvas.height = size.height;
    currentSize = size;
    resizeEvents += 1;
    coordinateDebugText = renderer.coordinate_debug_text();
    presenter.present();
    updateStatus();
  };

  const scheduleDisplayResize = () => {
    if (!resizeScheduled) {
      resizeScheduled = true;
      requestAnimationFrame(applyDisplayResize);
    }
  };

  const resizeObserver = new ResizeObserver(scheduleDisplayResize);
  resizeObserver.observe(canvas);
  window.addEventListener("resize", scheduleDisplayResize);

  const snapshot = () => ({
    internalSize: [renderer.width(), renderer.height()],
    cssSize: [canvas.clientWidth, canvas.clientHeight],
    deviceScaleFactor: window.devicePixelRatio || 1,
    framebufferLength: renderer.framebuffer_len(),
    framebufferMiB: framebufferMiB(currentSize),
    framebufferGeneration: renderer.framebuffer_generation(),
    typedArrayViewRebuilds: presenter.viewRebuilds,
    updateAndRenderCalls: updateCalls,
    lastFrameMetrics,
    resizeEvents,
    contextKind: "2d",
    cullMode: Number(cullModeSelect.value),
    windingDebugMode: barycentricDebugCheckbox.checked
      ? 2
      : windingDebugCheckbox.checked
        ? 1
        : 0,
    clipDebugEnabled: clipDebugCheckbox.checked,
    coverageDebugEnabled: coverageDebugCheckbox.checked,
    interpolationDebugEnabled: interpolationDebugCheckbox.checked,
    perspectiveDebugEnabled: perspectiveDebugCheckbox.checked,
    attributeInterpolationMode: Number(attributeInterpolationModeSelect.value),
    depthDebugEnabled: depthDebugCheckbox.checked,
    depthOrderReversed: depthOrderReversedCheckbox.checked,
    depthDebugMode: Number(depthDebugModeSelect.value),
    pixelHash: canvasPixelHash(context, renderer.width(), renderer.height()),
    stats: rendererStats(renderer),
  });

  const onAnimationFrame = (timestamp) => {
    const dtSeconds =
      previousTimestamp === null
        ? 0
        : Math.min(Math.max((timestamp - previousTimestamp) / 1000, 0), MAX_FRAME_DT_SECONDS);
    previousTimestamp = timestamp;
    renderFrame(dtSeconds);

    if (__AUTOMATION__) {
      window.__softRasterizer = Object.freeze({
        ready: true,
        advanceFrame(requestedDtSeconds) {
          renderFrame(requestedDtSeconds);
          return snapshot();
        },
        applyDisplayResize,
        setDebugLinesEnabled(enabled) {
          renderer.set_debug_lines_enabled(enabled);
        },
        setCullMode,
        setWindingDebugMode,
        setClipDebugEnabled,
        setCoverageDebugEnabled,
        setInterpolationDebugEnabled,
        setPerspectiveDebugEnabled,
        setAttributeInterpolationMode,
        setDepthDebugEnabled,
        setDepthOrderReversed,
        setDepthDebugMode,
        setModelRotationY(rotationYRadians) {
          renderer.set_model_rotation_y(rotationYRadians);
        },
        growMemory(pages = 1) {
          const previousBuffer = wasm.memory.buffer;
          const previousPages = wasm.memory.grow(pages);
          return {
            previousPages,
            currentPages: wasm.memory.buffer.byteLength / 65_536,
            bufferChanged: wasm.memory.buffer !== previousBuffer,
          };
        },
        invalidConstructorError() {
          try {
            const unexpectedRenderer = new Renderer(0, 1);
            unexpectedRenderer.free();
            return null;
          } catch (error) {
            return String(error);
          }
        },
        snapshot,
      });
      document.documentElement.dataset.ready = "true";
    } else {
      requestAnimationFrame(onAnimationFrame);
    }
  };

  requestAnimationFrame(onAnimationFrame);
  errorOutput.textContent = "";
}

bootstrap().catch((error) => {
  const message = error instanceof Error ? error.message : String(error);
  document.querySelector("#error").textContent = message;
  document.documentElement.dataset.ready = "error";
  throw error;
});
