import init, { Renderer } from "./pkg/renderer_wasm.js";
import { FramebufferPresenter } from "./present.js";
import { decodeImageFileToRgba, validateDecodedTextureSize } from "./texture-upload.js";

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
    coveredSamples: renderer.stats_covered_samples(),
    shadedSamples: renderer.stats_shaded_samples(),
    depthPassedSamples: renderer.stats_depth_passed_samples(),
    depthFailedSamples: renderer.stats_depth_failed_samples(),
    invalidDepthSamples: renderer.stats_invalid_depth_samples(),
    maxBarycentricSumError: renderer.stats_max_barycentric_sum_error(),
    interpolatedInvWSamples: renderer.stats_interpolated_inv_w_samples(),
    invalidInterpolationSamples: renderer.stats_invalid_interpolation_samples(),
    minInterpolatedInvW: renderer.stats_min_interpolated_inv_w(),
    maxInterpolatedInvW: renderer.stats_max_interpolated_inv_w(),
    sampleCounterOverflow: renderer.stats_sample_counter_overflow(),
    debugPixels: renderer.stats_debug_pixels(),
    invalidValues: renderer.stats_invalid_values(),
    textureDebugPixels: renderer.stats_texture_debug_pixels(),
    textureUploadSuccesses: renderer.stats_texture_upload_successes(),
    textureUploadFailures: renderer.stats_texture_upload_failures(),
    activeTextureId: renderer.stats_active_texture_id(),
    textureSamples: renderer.stats_texture_samples(),
    lightingSamples: renderer.stats_lighting_samples(),
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
  const pipelineDebugModeSelect = document.querySelector("#pipeline-debug-mode");
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
  const textureFileInput = document.querySelector("#texture-file");
  const textureDebugCheckbox = document.querySelector("#texture-debug");
  const textureStatusOutput = document.querySelector("#texture-status");
  const textureSamplingCheckbox = document.querySelector("#texture-sampling");
  const textureFilterSelect = document.querySelector("#texture-filter");
  const textureAddressUSelect = document.querySelector("#texture-address-u");
  const textureAddressVSelect = document.querySelector("#texture-address-v");
  const lightingEnabledCheckbox = document.querySelector("#lighting-enabled");
  const shaderModeSelect = document.querySelector("#shader-mode");
  const normalModeSelect = document.querySelector("#normal-mode");
  const lightXInput = document.querySelector("#light-x");
  const lightYInput = document.querySelector("#light-y");
  const lightZInput = document.querySelector("#light-z");
  const lightIntensityInput = document.querySelector("#light-intensity");
  const specularColorInput = document.querySelector("#specular-color");
  const shininessInput = document.querySelector("#shininess");
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
  let textureStatusText = "fallback checkerboard · 2 × 2 · texture 0";
  let textureDecodeGeneration = 0;

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
    document.querySelector("#texture-sampler").textContent =
      `${textureFilterSelect.options[textureFilterSelect.selectedIndex].text} · ` +
      `U ${textureAddressUSelect.options[textureAddressUSelect.selectedIndex].text} · ` +
      `V ${textureAddressVSelect.options[textureAddressVSelect.selectedIndex].text} · Rust fragment sampling`;
    document.querySelector("#texture-sample-stats").textContent =
      `${renderer.stats_texture_samples()} sampled after depth`;
    document.querySelector("#lighting-status").textContent =
      `${lightingEnabledCheckbox.checked ? `${shaderModeSelect.options[shaderModeSelect.selectedIndex].text} 켬` : "Unlit"} · ` +
      `${normalModeSelect.options[normalModeSelect.selectedIndex].text} normal · ` +
      `surface→light (${renderer.light_surface_to_light_x().toFixed(3)}, ` +
      `${renderer.light_surface_to_light_y().toFixed(3)}, ` +
      `${renderer.light_surface_to_light_z().toFixed(3)}) · ` +
      `intensity ${renderer.light_intensity().toFixed(2)} · ` +
      `specular ${specularColorInput.value} · shininess ${renderer.material_shininess().toFixed(1)}`;
    document.querySelector("#lighting-sample-stats").textContent =
      `${renderer.stats_lighting_samples()} lighting evaluations after depth`;
    document.querySelector("#depth-algorithm").textContent =
      "affine z_ndc · strict < · +infinity clear (Rust)";
    document.querySelector("#pipeline-algorithm").textContent =
      `${pipelineDebugModeSelect.options[pipelineDebugModeSelect.selectedIndex].text} · 같은 Rust coverage/depth 경로`;
    document.querySelector("#math-convention").textContent = "열벡터 · LH · +Z 전방";
    textureStatusOutput.textContent = textureStatusText;
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

  const setPipelineDebugMode = (mode) => {
    renderer.set_pipeline_debug_mode(mode);
    pipelineDebugModeSelect.value = String(mode);
    windingDebugCheckbox.checked = mode === 6;
    barycentricDebugCheckbox.checked = mode === 3;
    depthDebugModeSelect.value = String(mode === 4 ? 1 : mode === 5 ? 2 : 0);
  };

  const setWindingDebugMode = (mode) => {
    setPipelineDebugMode(mode === 1 ? 6 : mode === 2 ? 3 : 0);
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
    setPipelineDebugMode(mode === 1 ? 4 : mode === 2 ? 5 : 0);
  };

  const setTextureDebugEnabled = (enabled) => {
    renderer.set_texture_debug_enabled(enabled);
    textureDebugCheckbox.checked = enabled;
    if (enabled) {
      renderer.set_texture_sampling_enabled(false);
      textureSamplingCheckbox.checked = false;
    }
  };

  const setTextureSamplingEnabled = (enabled) => {
    renderer.set_texture_sampling_enabled(enabled);
    textureSamplingCheckbox.checked = enabled;
    if (enabled) {
      renderer.set_texture_debug_enabled(false);
      textureDebugCheckbox.checked = false;
    }
  };

  const setSamplerState = (filter, addressU, addressV) => {
    renderer.set_sampler_state(filter, addressU, addressV);
    textureFilterSelect.value = String(filter);
    textureAddressUSelect.value = String(addressU);
    textureAddressVSelect.value = String(addressV);
  };

  const setLightingEnabled = (enabled) => {
    renderer.set_shader_mode(enabled ? Number(shaderModeSelect.value) : 0);
    lightingEnabledCheckbox.checked = enabled;
  };

  const setShaderMode = (mode) => {
    renderer.set_shader_mode(mode);
    if (mode !== 0) {
      shaderModeSelect.value = String(mode);
    }
    lightingEnabledCheckbox.checked = mode !== 0;
  };

  const parseSrgbHex = (hex) => {
    const channels = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
    if (channels === null) {
      throw new Error("specular color는 #RRGGBB sRGB 형식이어야 합니다");
    }
    return channels.slice(1).map((channel) => Number.parseInt(channel, 16) / 255);
  };

  const materialSpecularHex = () => {
    const byteHex = (channel) =>
      Math.round(Math.min(Math.max(channel, 0), 1) * 255)
        .toString(16)
        .padStart(2, "0");
    return `#${byteHex(renderer.material_specular_red())}${byteHex(renderer.material_specular_green())}${byteHex(renderer.material_specular_blue())}`;
  };

  const setMaterialSpecular = (red, green, blue, shininess) => {
    renderer.set_material_specular(red, green, blue, shininess);
    specularColorInput.value = materialSpecularHex();
    shininessInput.value = String(renderer.material_shininess());
  };

  const restoreMaterialSpecularInputs = () => {
    specularColorInput.value = materialSpecularHex();
    shininessInput.value = String(renderer.material_shininess());
  };

  const setNormalMode = (mode) => {
    renderer.set_normal_mode(mode);
    normalModeSelect.value = String(mode);
  };

  const setDirectionalLight = (x, y, z, intensity) => {
    renderer.set_directional_light(x, y, z, intensity);
    lightXInput.value = String(x);
    lightYInput.value = String(y);
    lightZInput.value = String(z);
    lightIntensityInput.value = String(intensity);
  };

  const restoreDirectionalLightInputs = () => {
    const formatControlValue = (value) => {
      const expected = Math.fround(value);
      for (let precision = 1; precision <= 9; precision += 1) {
        const candidate = Number(value.toPrecision(precision));
        if (Object.is(Math.fround(candidate), expected)) {
          return String(candidate);
        }
      }
      return String(value);
    };
    lightXInput.value = formatControlValue(renderer.light_surface_to_light_x());
    lightYInput.value = formatControlValue(renderer.light_surface_to_light_y());
    lightZInput.value = formatControlValue(renderer.light_surface_to_light_z());
    lightIntensityInput.value = formatControlValue(renderer.light_intensity());
  };

  const uploadTextureRgba = (width, height, pixels) => {
    try {
      const id = renderer.upload_texture_rgba(width, height, pixels);
      textureStatusText = `업로드 완료 · ${width} × ${height} · texture ${id} · Rust 소유 복사`;
      errorOutput.textContent = "";
      setTextureDebugEnabled(true);
      renderFrame(0);
      return { id, error: null };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      textureStatusText = `업로드 실패 · 기존 texture ${renderer.active_texture_id()} 유지`;
      errorOutput.textContent = message;
      updateStatus();
      return { id: null, error: message };
    }
  };

  // Reload/history restoration can preserve form values independently of the newly created Wasm state.
  setCullMode(Number(cullModeSelect.value));
  setClipDebugEnabled(clipDebugCheckbox.checked);
  setCoverageDebugEnabled(coverageDebugCheckbox.checked);
  setInterpolationDebugEnabled(interpolationDebugCheckbox.checked);
  setPerspectiveDebugEnabled(perspectiveDebugCheckbox.checked);
  setAttributeInterpolationMode(Number(attributeInterpolationModeSelect.value));
  setDepthDebugEnabled(depthDebugCheckbox.checked);
  setDepthOrderReversed(depthOrderReversedCheckbox.checked);
  setPipelineDebugMode(Number(pipelineDebugModeSelect.value));
  setTextureDebugEnabled(textureDebugCheckbox.checked);
  setSamplerState(
    Number(textureFilterSelect.value),
    Number(textureAddressUSelect.value),
    Number(textureAddressVSelect.value),
  );
  setTextureSamplingEnabled(textureSamplingCheckbox.checked);
  setLightingEnabled(lightingEnabledCheckbox.checked);
  setNormalMode(Number(normalModeSelect.value));
  try {
    setMaterialSpecular(
      ...parseSrgbHex(specularColorInput.value),
      Number(shininessInput.value),
    );
  } catch (error) {
    errorOutput.textContent = error instanceof Error ? error.message : String(error);
    restoreMaterialSpecularInputs();
  }
  try {
    setDirectionalLight(
      Number(lightXInput.value),
      Number(lightYInput.value),
      Number(lightZInput.value),
      Number(lightIntensityInput.value),
    );
  } catch (error) {
    errorOutput.textContent = error instanceof Error ? error.message : String(error);
    restoreDirectionalLightInputs();
  }

  cullModeSelect.addEventListener("change", () => {
    setCullMode(Number(cullModeSelect.value));
    renderFrame(0);
  });
  pipelineDebugModeSelect.addEventListener("change", () => {
    setPipelineDebugMode(Number(pipelineDebugModeSelect.value));
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
  textureDebugCheckbox.addEventListener("change", () => {
    setTextureDebugEnabled(textureDebugCheckbox.checked);
    renderFrame(0);
  });
  textureSamplingCheckbox.addEventListener("change", () => {
    setTextureSamplingEnabled(textureSamplingCheckbox.checked);
    renderFrame(0);
  });
  for (const select of [
    textureFilterSelect,
    textureAddressUSelect,
    textureAddressVSelect,
  ]) {
    select.addEventListener("change", () => {
      setSamplerState(
        Number(textureFilterSelect.value),
        Number(textureAddressUSelect.value),
        Number(textureAddressVSelect.value),
      );
      renderFrame(0);
    });
  }
  lightingEnabledCheckbox.addEventListener("change", () => {
    setLightingEnabled(lightingEnabledCheckbox.checked);
    renderFrame(0);
  });
  shaderModeSelect.addEventListener("change", () => {
    if (lightingEnabledCheckbox.checked) {
      setShaderMode(Number(shaderModeSelect.value));
      renderFrame(0);
    }
  });
  for (const input of [specularColorInput, shininessInput]) {
    input.addEventListener("change", () => {
      try {
        setMaterialSpecular(
          ...parseSrgbHex(specularColorInput.value),
          Number(shininessInput.value),
        );
        errorOutput.textContent = "";
        renderFrame(0);
      } catch (error) {
        errorOutput.textContent = error instanceof Error ? error.message : String(error);
        restoreMaterialSpecularInputs();
        updateStatus();
      }
    });
  }
  normalModeSelect.addEventListener("change", () => {
    setNormalMode(Number(normalModeSelect.value));
    renderFrame(0);
  });
  for (const input of [lightXInput, lightYInput, lightZInput, lightIntensityInput]) {
    input.addEventListener("change", () => {
      try {
        setDirectionalLight(
          Number(lightXInput.value),
          Number(lightYInput.value),
          Number(lightZInput.value),
          Number(lightIntensityInput.value),
        );
        errorOutput.textContent = "";
        renderFrame(0);
      } catch (error) {
        errorOutput.textContent = error instanceof Error ? error.message : String(error);
        restoreDirectionalLightInputs();
        updateStatus();
      }
    });
  }
  const decodeAndUploadTextureFile = async (file, decode = decodeImageFileToRgba) => {
    const generation = ++textureDecodeGeneration;
    textureStatusText = `디코딩 중 · ${file.name}`;
    updateStatus();
    try {
      const decoded = await decode(file);
      if (generation !== textureDecodeGeneration) {
        return false;
      }
      uploadTextureRgba(decoded.width, decoded.height, decoded.pixels);
      return true;
    } catch (error) {
      if (generation !== textureDecodeGeneration) {
        return false;
      }
      const message = error instanceof Error ? error.message : String(error);
      textureStatusText = `디코딩 실패 · 기존 texture ${renderer.active_texture_id()} 유지`;
      errorOutput.textContent = message;
      updateStatus();
      return false;
    } finally {
      if (generation === textureDecodeGeneration) {
        textureFileInput.value = "";
      }
    }
  };
  textureFileInput.addEventListener("change", async () => {
    const [file] = textureFileInput.files;
    if (file !== undefined) {
      await decodeAndUploadTextureFile(file);
    }
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
    pipelineDebugMode: Number(pipelineDebugModeSelect.value),
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
    textureDebugEnabled: textureDebugCheckbox.checked,
    textureSamplingEnabled: textureSamplingCheckbox.checked,
    samplerState: {
      filter: renderer.sampler_filter_mode(),
      addressU: renderer.sampler_address_u(),
      addressV: renderer.sampler_address_v(),
    },
    lightingEnabled: lightingEnabledCheckbox.checked,
    shaderMode: renderer.shader_mode(),
    normalMode: renderer.normal_mode(),
    materialSpecular: {
      color: [
        renderer.material_specular_red(),
        renderer.material_specular_green(),
        renderer.material_specular_blue(),
      ],
      shininess: renderer.material_shininess(),
    },
    directionalLight: {
      surfaceToLight: [
        renderer.light_surface_to_light_x(),
        renderer.light_surface_to_light_y(),
        renderer.light_surface_to_light_z(),
      ],
      intensity: renderer.light_intensity(),
    },
    textureStatus: {
      activeId: renderer.active_texture_id(),
      width: renderer.active_texture_width(),
      height: renderer.active_texture_height(),
      successes: renderer.texture_upload_successes(),
      failures: renderer.texture_upload_failures(),
      text: textureStatusText,
    },
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
        setPipelineDebugMode,
        setWindingDebugMode,
        setClipDebugEnabled,
        setCoverageDebugEnabled,
        setInterpolationDebugEnabled,
        setPerspectiveDebugEnabled,
        setAttributeInterpolationMode,
        setDepthDebugEnabled,
        setDepthOrderReversed,
        setDepthDebugMode,
        setTextureDebugEnabled,
        setTextureSamplingEnabled,
        setSamplerState(filter, addressU, addressV) {
          setSamplerState(filter, addressU, addressV);
          renderFrame(0);
          return snapshot();
        },
        setLightingEnabled,
        setShaderMode,
        setNormalMode,
        setMaterialSpecular(red, green, blue, shininess) {
          setMaterialSpecular(red, green, blue, shininess);
          renderFrame(0);
          return snapshot();
        },
        setDirectionalLight(x, y, z, intensity) {
          setDirectionalLight(x, y, z, intensity);
          renderFrame(0);
          return snapshot();
        },
        uploadTextureRgba(width, height, pixelValues) {
          const pixels = Uint8Array.from(pixelValues);
          const result = uploadTextureRgba(width, height, pixels);
          pixels.fill(0);
          renderFrame(0);
          return { ...result, snapshot: snapshot() };
        },
        validateDecodedTextureSize(width, height) {
          try {
            return { pixelCount: validateDecodedTextureSize(width, height), error: null };
          } catch (error) {
            return {
              pixelCount: null,
              error: error instanceof Error ? error.message : String(error),
            };
          }
        },
        async testOversizedDecodeGuard() {
          let canvasCreated = false;
          let bitmapClosed = false;
          try {
            await decodeImageFileToRgba(new Blob([]), {
              createBitmap: async () => ({
                width: 16_777_217,
                height: 1,
                close() {
                  bitmapClosed = true;
                },
              }),
              createCanvas: () => {
                canvasCreated = true;
                throw new Error("크기 검사 전에 Canvas를 만들면 안 됩니다.");
              },
            });
            return { error: null, canvasCreated, bitmapClosed };
          } catch (error) {
            return {
              error: error instanceof Error ? error.message : String(error),
              canvasCreated,
              bitmapClosed,
            };
          }
        },
        async testLatestTextureSelectionWins() {
          let resolveFirst;
          let resolveSecond;
          const firstDecoded = new Promise((resolve) => {
            resolveFirst = resolve;
          });
          const secondDecoded = new Promise((resolve) => {
            resolveSecond = resolve;
          });
          const first = decodeAndUploadTextureFile(
            new File([], "first.png", { type: "image/png" }),
            () => firstDecoded,
          );
          const second = decodeAndUploadTextureFile(
            new File([], "second.png", { type: "image/png" }),
            () => secondDecoded,
          );
          resolveSecond({ width: 1, height: 1, pixels: new Uint8Array([20, 40, 60, 255]) });
          await second;
          const afterSecond = snapshot();
          resolveFirst({ width: 1, height: 1, pixels: new Uint8Array([200, 10, 10, 255]) });
          await first;
          return { afterSecond, afterFirst: snapshot() };
        },
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
