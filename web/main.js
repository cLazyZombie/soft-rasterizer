import init, { Renderer } from "./pkg/renderer_wasm.js";
import { InputCollector } from "./input.js";
import { readObjFileBytes, validateObjFileSize } from "./mesh-upload.js";
import {
  hasGlbMagic,
  prepareDecodeAndCommitGlb,
  readGlbFileBytes,
  validateGlbFileSize,
} from "./glb-upload.js";
import { FramebufferPresenter } from "./present.js";
import { decodeImageFileToRgba, validateDecodedTextureSize } from "./texture-upload.js";
import { FrameRateTracker, FrameTimingRing, summarizeFrameTimings } from "./frame-timing.js";
import { resolveRasterPath } from "./raster-path.js";

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
    alphaDiscardedSamples: renderer.stats_alpha_discarded_samples(),
    depthWrittenSamples: renderer.stats_depth_written_samples(),
    blendedSamples: renderer.stats_blended_samples(),
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
    renderScale: renderer.stats_render_scale(),
    resolvedPixels: renderer.stats_resolved_pixels(),
    mipSamples: renderer.stats_mip_samples(),
    minMipLevel: renderer.stats_min_mip_level(),
    maxMipLevel: renderer.stats_max_mip_level(),
    invalidLodSamples: renderer.stats_invalid_lod_samples(),
    overdrawnPixels: renderer.stats_overdrawn_pixels(),
    maxOverdraw: renderer.stats_max_overdraw(),
    tiledRasterizedTriangles: renderer.stats_tiled_rasterized_triangles(),
    tileVisits: renderer.stats_tile_visits(),
    tileCounterOverflow: renderer.stats_tile_counter_overflow(),
    sceneDrawItems: renderer.stats_scene_draw_items(),
    animatedNodes: renderer.stats_animated_nodes(),
    skinnedVertices: renderer.stats_skinned_vertices(),
    jointMatrices: renderer.stats_joint_matrices(),
    samplerDowngrades: renderer.stats_sampler_downgrades(),
  };
}

function formatMilliseconds(value) {
  return `${value.toFixed(3)} ms`;
}

function formatFramesPerSecond(value) {
  return value === null ? "FPS 측정 중" : `${value.toFixed(1)} FPS`;
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
  const cameraModeSelect = document.querySelector("#camera-mode");
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
  const meshFileInput = document.querySelector("#mesh-file");
  const meshStatusOutput = document.querySelector("#mesh-status");
  const glbStatusOutput = document.querySelector("#glb-status");
  const animationStatusOutput = document.querySelector("#animation-status");
  const animationClipSelect = document.querySelector("#animation-clip");
  const animationPlayButton = document.querySelector("#animation-play");
  const animationLoopCheckbox = document.querySelector("#animation-loop");
  const animationTimeInput = document.querySelector("#animation-time");
  const animationTimeLabel = document.querySelector("#animation-time-label");
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
  const alphaModeSelect = document.querySelector("#alpha-mode");
  const alphaCutoffInput = document.querySelector("#alpha-cutoff");
  const transparencyDebugCheckbox = document.querySelector("#transparency-debug");
  const transparentSortCheckbox = document.querySelector("#transparent-sort");
  const blendColorSpaceSelect = document.querySelector("#blend-color-space");
  const qualityModeSelect = document.querySelector("#quality-mode");
  const rasterPathSelect = document.querySelector("#raster-path");
  const mipmapEnabledCheckbox = document.querySelector("#mipmap-enabled");
  const mipDebugCheckbox = document.querySelector("#mip-debug");
  const initialMipmapEnabled = mipmapEnabledCheckbox.checked;
  const initialMipDebugEnabled = mipDebugCheckbox.checked;
  const context = canvas.getContext("2d", { alpha: false });
  if (context === null) {
    throw new Error("Canvas 2D context를 만들 수 없습니다.");
  }

  const wasm = await init();
  const initialSize = logicalRenderSize(canvas);
  canvas.width = initialSize.width;
  canvas.height = initialSize.height;
  const renderer = new Renderer(initialSize.width, initialSize.height);
  let seekGlbAnimation = (timeSeconds) => renderer.seek_glb_animation(timeSeconds);
  let animationTimeEditing = false;
  const inputCollector = new InputCollector(canvas);
  const presenter = new FramebufferPresenter(context, wasm.memory, renderer);
  let previousTimestamp = null;
  let updateCalls = 0;
  let resizeEvents = 0;
  let resizeScheduled = false;
  let currentSize = initialSize;
  let lastFrameMetrics = null;
  const frameTimingWindow = new FrameTimingRing();
  const frameRateTracker = new FrameRateTracker();
  let lastInputSnapshot = new Float64Array(8);
  let coordinateDebugText = "좌표 계산 대기 중";
  let textureStatusText = "fallback checkerboard · 2 × 2 · 2 mip levels · texture 0";
  let textureDecodeGeneration = 0;
  let meshStatusText = "내장 cube · mesh 0 · 24 vertices · 12 triangles";
  let glbStatusText = "내장 Fox GLB를 준비하는 중";
  let activeAssetKind = "cube";
  let meshLoadGeneration = 0;
  const rasterCapabilities = Object.freeze({
    crossOriginIsolated: globalThis.crossOriginIsolated === true,
    wasmSharedMemory: false,
    parallelSchedulerBuilt: false,
  });
  const parallelCapability = resolveRasterPath(2, rasterCapabilities);
  let rasterResolution = resolveRasterPath(Number(rasterPathSelect.value), rasterCapabilities);

  const updateStatus = () => {
    document.querySelector("#internal-size").textContent = `${currentSize.width} × ${currentSize.height} px`;
    document.querySelector("#css-size").textContent = `${canvas.clientWidth} × ${canvas.clientHeight} CSS px`;
    document.querySelector("#display-scale").textContent = `${window.devicePixelRatio || 1}× (논리 해상도 사용)`;
    document.querySelector("#present-path").textContent = "Rust/Wasm RGBA8 → Canvas 2D";
    document.querySelector("#framebuffer-mib").textContent = `${framebufferMiB(currentSize).toFixed(2)} MiB`;
    document.querySelector("#line-algorithm").textContent = "All-octants Bresenham (Rust)";
    document.querySelector("#coverage-algorithm").textContent =
      "S=256 incremental edge · pixel center · top-left (Rust)";
    document.querySelector("#raster-status").textContent =
      `${rasterResolution.actualLabel} · ${renderer.stats_tiled_rasterized_triangles()} tiled triangles · ` +
      `${renderer.stats_tile_visits()} tile visits · overflow ${renderer.stats_tile_counter_overflow()}`;
    document.querySelector("#parallel-capability").textContent =
      rasterResolution.usedFallback
        ? rasterResolution.reason
        : `Shared threads 미사용 · 요청 시 ${parallelCapability.reason}`;
    document.querySelector("#interpolation-algorithm").textContent =
      attributeInterpolationModeSelect.value === "1"
        ? "Σ(λ · attribute/w) ÷ Σ(λ/w) · normal 재정규화 (Rust)"
        : "affine attribute 비교 경로 (Rust)";
    document.querySelector("#texture-sampler").textContent =
      activeAssetKind === "glb"
        ? `GLB material별 imported sampler · ${renderer.glb_sampler_downgrades()} downgrade · Rust fragment sampling`
        : `${textureFilterSelect.options[textureFilterSelect.selectedIndex].text} · ` +
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
    document.querySelector("#transparency-status").textContent =
      `${alphaModeSelect.options[alphaModeSelect.selectedIndex].text} · cutoff ${renderer.alpha_cutoff().toFixed(2)} · ` +
      `${transparentSortCheckbox.checked ? "view +Z descending" : "source order debug"} · ` +
      `${blendColorSpaceSelect.options[blendColorSpaceSelect.selectedIndex].text} · ` +
      `${renderer.stats_alpha_discarded_samples()} discarded · ${renderer.stats_depth_written_samples()} depth writes · ` +
      `${renderer.stats_blended_samples()} blended`;
    const mipStatus = activeAssetKind === "glb"
      ? `GLB texture별 mip chain · ${mipmapEnabledCheckbox.checked ? `${renderer.stats_min_mip_level()}..${renderer.stats_max_mip_level()} selected` : "base level only"}`
      : `${renderer.texture_mip_levels()} mip levels · ${mipmapEnabledCheckbox.checked ? `${renderer.stats_min_mip_level()}..${renderer.stats_max_mip_level()} selected` : "base level only"}`;
    document.querySelector("#quality-status").textContent =
      `${qualityModeSelect.options[qualityModeSelect.selectedIndex].text} · ` +
      `render ${renderer.render_width()} × ${renderer.render_height()} · ` +
      `${renderer.stats_shaded_samples()} shaded · ${renderer.stats_resolved_pixels()} resolved · ` +
      `${mipStatus} · ${renderer.stats_invalid_lod_samples()} invalid LOD`;
    document.querySelector("#camera-status").textContent =
      `${cameraModeSelect.options[cameraModeSelect.selectedIndex].text} · ` +
      `eye (${renderer.camera_eye_x().toFixed(3)}, ${renderer.camera_eye_y().toFixed(3)}, ${renderer.camera_eye_z().toFixed(3)}) · ` +
      `forward (${renderer.camera_forward_x().toFixed(3)}, ${renderer.camera_forward_y().toFixed(3)}, ${renderer.camera_forward_z().toFixed(3)}) · ` +
      `yaw ${renderer.camera_yaw().toFixed(3)} · pitch ${renderer.camera_pitch().toFixed(3)} · radius ${renderer.camera_orbit_radius().toFixed(3)}`;
    document.querySelector("#depth-algorithm").textContent =
      "affine z_ndc · strict < · +infinity clear (Rust)";
    document.querySelector("#pipeline-algorithm").textContent =
      `${pipelineDebugModeSelect.options[pipelineDebugModeSelect.selectedIndex].text} · 같은 Rust coverage/depth 경로`;
    document.querySelector("#math-convention").textContent = "열벡터 · LH · +Z 전방";
    textureStatusOutput.textContent = textureStatusText;
    meshStatusOutput.textContent = meshStatusText;
    const glbRuntimeError = renderer.glb_runtime_error();
    glbStatusOutput.textContent = glbRuntimeError
      ? `${glbStatusText} · animation paused: ${glbRuntimeError}`
      : glbStatusText;
    const animationTime = renderer.glb_animation_time();
    const animationDuration = renderer.glb_animation_duration();
    animationTimeInput.max = String(animationDuration);
    if (!animationTimeEditing) {
      animationTimeInput.value = String(animationTime);
    }
    animationPlayButton.textContent = renderer.glb_animation_playing() ? "일시정지" : "재생";
    animationTimeLabel.textContent = `${animationTime.toFixed(3)} / ${animationDuration.toFixed(3)} s`;
    animationStatusOutput.textContent = renderer.glb_active()
      ? `${renderer.glb_clip_count()} clips · ${renderer.stats_animated_nodes()} animated nodes · ` +
        `${renderer.stats_skinned_vertices()} skinned vertices · ${renderer.stats_joint_matrices()} joint matrices`
      : "clip 없음";
    document.querySelector("#coordinate-debug").textContent = coordinateDebugText;
    document.querySelector("#frame-index").textContent = String(updateCalls);
    document.querySelector("#current-fps").textContent = formatFramesPerSecond(
      frameRateTracker.summary().fps,
    );
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
    const timingSummary = frameTimingWindow.summary();
    document.querySelector("#timing-window").textContent =
      timingSummary.count === 0
        ? "warm-up 전"
        : `${timingSummary.count} frames · ` +
          `update p50 ${formatMilliseconds(timingSummary.updateMs.p50)} / p95 ${formatMilliseconds(timingSummary.updateMs.p95)} · ` +
          `present p50 ${formatMilliseconds(timingSummary.presentMs.p50)} / p95 ${formatMilliseconds(timingSummary.presentMs.p95)} · ` +
          `total p50 ${formatMilliseconds(timingSummary.totalMs.p50)} / p95 ${formatMilliseconds(timingSummary.totalMs.p95)}`;
    document.querySelector("#overdraw-stats").textContent =
      `${renderer.stats_overdrawn_pixels()} pixels · max ${renderer.stats_max_overdraw()} layers`;
  };

  const renderFrame = (dtSeconds) => {
    const frameStart = performance.now();
    const inputStart = performance.now();
    const inputSnapshot = inputCollector.snapshot();
    lastInputSnapshot = inputSnapshot;
    const inputEnd = performance.now();
    const updateStart = performance.now();
    renderer.update_and_render_input(dtSeconds, inputSnapshot);
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
    frameTimingWindow.push(lastFrameMetrics);
    updateStatus();
    return lastFrameMetrics;
  };

  const setCullMode = (mode) => {
    renderer.set_cull_mode(mode);
    cullModeSelect.value = String(mode);
  };

  const setCameraMode = (mode) => {
    renderer.set_camera_mode(mode);
    cameraModeSelect.value = String(mode);
  };

  const setRasterPath = (requestedMode) => {
    const resolution = resolveRasterPath(requestedMode, rasterCapabilities);
    renderer.set_raster_path(resolution.actualMode);
    rasterResolution = resolution;
    rasterPathSelect.value = String(requestedMode);
    return resolution;
  };

  const syncPipelineViewControls = (mode) => {
    pipelineDebugModeSelect.value = String(mode);
    windingDebugCheckbox.checked = mode === 6;
    barycentricDebugCheckbox.checked = mode === 3;
    depthDebugModeSelect.value = String(mode === 4 ? 1 : mode === 5 ? 2 : 0);
  };

  const setPipelineDebugMode = (mode) => {
    renderer.set_pipeline_debug_mode(mode);
    syncPipelineViewControls(mode);
    textureDebugCheckbox.checked = false;
    transparencyDebugCheckbox.checked = false;
    syncMipControls();
  };

  const setWindingDebugMode = (mode) => {
    setPipelineDebugMode(mode === 1 ? 6 : mode === 2 ? 3 : 0);
  };

  const syncMipControls = () => {
    mipmapEnabledCheckbox.checked = renderer.mipmap_enabled();
    mipDebugCheckbox.checked = renderer.mip_debug_enabled();
    textureSamplingCheckbox.checked = renderer.texture_sampling_enabled();
  };

  const setClipDebugEnabled = (enabled) => {
    renderer.set_clip_debug_enabled(enabled);
    syncMipControls();
    clipDebugCheckbox.checked = enabled;
    if (enabled) {
      transparencyDebugCheckbox.checked = false;
      coverageDebugCheckbox.checked = false;
      interpolationDebugCheckbox.checked = false;
      perspectiveDebugCheckbox.checked = false;
      depthDebugCheckbox.checked = false;
    }
  };

  const setCoverageDebugEnabled = (enabled) => {
    renderer.set_coverage_debug_enabled(enabled);
    syncMipControls();
    coverageDebugCheckbox.checked = enabled;
    if (enabled) {
      transparencyDebugCheckbox.checked = false;
      clipDebugCheckbox.checked = false;
      interpolationDebugCheckbox.checked = false;
      perspectiveDebugCheckbox.checked = false;
      depthDebugCheckbox.checked = false;
    }
  };

  const setInterpolationDebugEnabled = (enabled) => {
    renderer.set_interpolation_debug_enabled(enabled);
    syncMipControls();
    interpolationDebugCheckbox.checked = enabled;
    if (enabled) {
      transparencyDebugCheckbox.checked = false;
      clipDebugCheckbox.checked = false;
      coverageDebugCheckbox.checked = false;
      perspectiveDebugCheckbox.checked = false;
      depthDebugCheckbox.checked = false;
    }
  };

  const setPerspectiveDebugEnabled = (enabled) => {
    renderer.set_perspective_debug_enabled(enabled);
    syncMipControls();
    perspectiveDebugCheckbox.checked = enabled;
    if (enabled) {
      transparencyDebugCheckbox.checked = false;
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
    syncMipControls();
    depthDebugCheckbox.checked = enabled;
    if (enabled) {
      transparencyDebugCheckbox.checked = false;
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
      syncPipelineViewControls(0);
      transparencyDebugCheckbox.checked = false;
      renderer.set_texture_sampling_enabled(false);
      textureSamplingCheckbox.checked = false;
    }
    syncMipControls();
  };

  const setTextureSamplingEnabled = (enabled) => {
    renderer.set_texture_sampling_enabled(enabled);
    textureSamplingCheckbox.checked = enabled;
    if (enabled) {
      renderer.set_texture_debug_enabled(false);
      textureDebugCheckbox.checked = false;
    }
    syncMipControls();
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

  const setAlphaMode = (mode) => {
    renderer.set_alpha_mode(mode);
    alphaModeSelect.value = String(mode);
  };

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

  const setAlphaCutoff = (cutoff) => {
    renderer.set_alpha_cutoff(cutoff);
    alphaCutoffInput.value = formatControlValue(renderer.alpha_cutoff());
  };

  const setTransparencyDebugEnabled = (enabled) => {
    renderer.set_transparency_debug_enabled(enabled);
    transparencyDebugCheckbox.checked = enabled;
    if (enabled) {
      clipDebugCheckbox.checked = false;
      coverageDebugCheckbox.checked = false;
      interpolationDebugCheckbox.checked = false;
      perspectiveDebugCheckbox.checked = false;
      depthDebugCheckbox.checked = false;
      textureDebugCheckbox.checked = false;
      syncPipelineViewControls(0);
    }
    syncMipControls();
  };

  const setTransparentSortEnabled = (enabled) => {
    renderer.set_transparent_sort_enabled(enabled);
    transparentSortCheckbox.checked = enabled;
  };

  const setBlendColorSpace = (mode) => {
    renderer.set_blend_color_space(mode);
    blendColorSpaceSelect.value = String(mode);
  };

  const setQualityMode = (mode) => {
    renderer.set_quality_mode(mode);
    qualityModeSelect.value = String(renderer.quality_mode());
  };

  const setMipmapEnabled = (enabled) => {
    renderer.set_mipmap_enabled(enabled);
    syncMipControls();
  };

  const setMipDebugEnabled = (enabled) => {
    renderer.set_mip_debug_enabled(enabled);
    syncMipControls();
    if (enabled) {
      syncPipelineViewControls(0);
      textureDebugCheckbox.checked = false;
      transparencyDebugCheckbox.checked = false;
      clipDebugCheckbox.checked = false;
      coverageDebugCheckbox.checked = false;
      interpolationDebugCheckbox.checked = false;
      perspectiveDebugCheckbox.checked = false;
      depthDebugCheckbox.checked = false;
    }
  };

  const setDirectionalLight = (x, y, z, intensity) => {
    renderer.set_directional_light(x, y, z, intensity);
    lightXInput.value = String(x);
    lightYInput.value = String(y);
    lightZInput.value = String(z);
    lightIntensityInput.value = String(intensity);
  };

  const restoreDirectionalLightInputs = () => {
    lightXInput.value = formatControlValue(renderer.light_surface_to_light_x());
    lightYInput.value = formatControlValue(renderer.light_surface_to_light_y());
    lightZInput.value = formatControlValue(renderer.light_surface_to_light_z());
    lightIntensityInput.value = formatControlValue(renderer.light_intensity());
  };

  const uploadTextureRgba = (width, height, pixels) => {
    try {
      const id = renderer.upload_texture_rgba(width, height, pixels);
      textureStatusText = `업로드 완료 · ${width} × ${height} · ${renderer.texture_mip_levels()} mip levels · texture ${id} · Rust 소유 복사`;
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

  const syncPrimarySceneControls = () => {
    cameraModeSelect.value = "0";
    clipDebugCheckbox.checked = false;
    coverageDebugCheckbox.checked = false;
    interpolationDebugCheckbox.checked = false;
    perspectiveDebugCheckbox.checked = false;
    depthDebugCheckbox.checked = false;
    textureDebugCheckbox.checked = false;
    transparencyDebugCheckbox.checked = false;
  };

  const syncImportedMaterialControls = () => {
    const imported = activeAssetKind === "glb";
    if (imported) {
      textureFilterSelect.value = "2";
      textureAddressUSelect.value = "3";
      textureAddressVSelect.value = "3";
    } else {
      textureFilterSelect.value = String(renderer.sampler_filter_mode());
      textureAddressUSelect.value = String(renderer.sampler_address_u());
      textureAddressVSelect.value = String(renderer.sampler_address_v());
    }
    textureFilterSelect.disabled = imported;
    textureAddressUSelect.disabled = imported;
    textureAddressVSelect.disabled = imported;
    alphaModeSelect.disabled = imported;
    alphaCutoffInput.disabled = imported;
  };

  const syncAnimationControls = (preferredClipName = null) => {
    animationClipSelect.replaceChildren();
    const clipCount = renderer.glb_clip_count();
    for (let index = 0; index < clipCount; index += 1) {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = renderer.glb_clip_name(index) || `Animation ${index}`;
      animationClipSelect.append(option);
    }
    const enabled = clipCount > 0;
    animationClipSelect.disabled = !enabled;
    animationPlayButton.disabled = !enabled;
    animationLoopCheckbox.disabled = !enabled;
    animationTimeInput.disabled = !enabled;
    if (!enabled) {
      const option = document.createElement("option");
      option.value = "";
      option.textContent = "clip 없음";
      animationClipSelect.append(option);
      return;
    }
    let selected = renderer.glb_selected_clip();
    if (preferredClipName !== null) {
      for (let index = 0; index < clipCount; index += 1) {
        if (renderer.glb_clip_name(index) === preferredClipName) {
          selected = index;
          renderer.set_glb_clip(index);
          break;
        }
      }
    }
    animationClipSelect.value = String(selected);
    animationLoopCheckbox.checked = renderer.glb_animation_looping();
  };

  const uploadObjBytes = (bytes, label = "OBJ") => {
    try {
      const id = renderer.load_obj(bytes);
      activeAssetKind = "obj";
      syncPrimarySceneControls();
      syncImportedMaterialControls();
      syncAnimationControls();
      glbStatusText = "GLB 비활성 · OBJ 단일 mesh";
      meshStatusText =
        `${label} 로드 완료 · mesh ${id} · ` +
        `${renderer.mesh_internal_vertices()} vertices · ${renderer.mesh_triangles()} triangles · ` +
        "LH +X/+Y/+Z profile · Rust 소유";
      errorOutput.textContent = "";
      renderFrame(0);
      return { id, error: null };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      meshStatusText = `OBJ 로드 실패 · 기존 mesh ${renderer.active_mesh_id()} 유지`;
      errorOutput.textContent = message;
      updateStatus();
      return { id: null, error: message };
    }
  };

  const uploadGlbBytes = async (
    bytes,
    label = "GLB",
    { renderNow = true, isCurrent = () => true, decodeImage } = {},
  ) => {
    if (!isCurrent()) {
      return { id: null, error: null, stale: true };
    }
    try {
      const pendingId = await prepareDecodeAndCommitGlb(
        renderer,
        bytes,
        decodeImage === undefined ? {} : { decodeImage },
      );
      if (!isCurrent()) {
        return { id: null, error: null, stale: true };
      }
      activeAssetKind = "glb";
      syncPrimarySceneControls();
      if (label === "내장 Fox") {
        textureSamplingCheckbox.checked = true;
      }
      renderer.set_texture_sampling_enabled(textureSamplingCheckbox.checked);
      setLightingEnabled(label === "내장 Fox" || lightingEnabledCheckbox.checked);
      renderer.set_glb_animation_looping(true);
      syncImportedMaterialControls();
      syncAnimationControls(label === "내장 Fox" ? "Walk" : null);
      meshStatusText =
        `${label} 로드 완료 · ${renderer.glb_draw_items()} draw items · ` +
        `${renderer.glb_vertices()} vertices · ${renderer.glb_triangles()} triangles`;
      glbStatusText =
        `GLB ${pendingId} commit · ${label} · ${renderer.glb_nodes()} nodes · ${renderer.glb_skins()} skins · ` +
        `${renderer.glb_joints()} joints · ${renderer.glb_clip_count()} clips · ` +
        `${renderer.glb_sampler_downgrades()} sampler downgrade`;
      errorOutput.textContent = "";
      if (renderNow) {
        renderFrame(0);
      } else {
        updateStatus();
      }
      return { id: pendingId, error: null, stale: false };
    } catch (error) {
      if (!isCurrent()) {
        return { id: null, error: null, stale: true };
      }
      const message = error instanceof Error ? error.message : String(error);
      glbStatusText = `GLB 로드 실패 · 기존 ${activeAssetKind} scene 유지`;
      errorOutput.textContent = message;
      updateStatus();
      return { id: null, error: message, stale: false };
    }
  };

  const loadBundledFox = async (
    { renderNow = true, fetchAsset = fetch, isCurrent = () => true, decodeImage } = {},
  ) => {
    const response = await fetchAsset(new URL("./assets/Fox.glb", import.meta.url));
    if (!isCurrent()) {
      return { id: null, error: null, stale: true };
    }
    if (!response.ok) {
      throw new Error(`내장 Fox.glb를 가져오지 못했습니다: HTTP ${response.status}`);
    }
    const buffer = await response.arrayBuffer();
    validateGlbFileSize(buffer.byteLength);
    const bytes = new Uint8Array(buffer);
    try {
      return await uploadGlbBytes(bytes, "내장 Fox", {
        renderNow,
        isCurrent,
        decodeImage,
      });
    } finally {
      bytes.fill(0);
    }
  };

  // Reload/history restoration can preserve form values independently of the newly created Wasm state.
  setCameraMode(Number(cameraModeSelect.value));
  setRasterPath(Number(rasterPathSelect.value));
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
  setAlphaMode(Number(alphaModeSelect.value));
  try {
    setAlphaCutoff(Number(alphaCutoffInput.value));
  } catch (error) {
    errorOutput.textContent = error instanceof Error ? error.message : String(error);
    alphaCutoffInput.value = formatControlValue(renderer.alpha_cutoff());
  }
  setTransparentSortEnabled(transparentSortCheckbox.checked);
  setBlendColorSpace(Number(blendColorSpaceSelect.value));
  setTransparencyDebugEnabled(transparencyDebugCheckbox.checked);
  try {
    setQualityMode(Number(qualityModeSelect.value));
  } catch (error) {
    errorOutput.textContent = error instanceof Error ? error.message : String(error);
    qualityModeSelect.value = String(renderer.quality_mode());
  }
  setMipmapEnabled(initialMipmapEnabled);
  setMipDebugEnabled(initialMipDebugEnabled);
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

  cameraModeSelect.addEventListener("change", () => {
    setCameraMode(Number(cameraModeSelect.value));
    renderFrame(0);
  });
  rasterPathSelect.addEventListener("change", () => {
    setRasterPath(Number(rasterPathSelect.value));
    renderFrame(0);
  });
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
  alphaModeSelect.addEventListener("change", () => {
    setAlphaMode(Number(alphaModeSelect.value));
    renderFrame(0);
  });
  alphaCutoffInput.addEventListener("change", () => {
    try {
      setAlphaCutoff(Number(alphaCutoffInput.value));
      errorOutput.textContent = "";
    } catch (error) {
      errorOutput.textContent = error instanceof Error ? error.message : String(error);
      alphaCutoffInput.value = formatControlValue(renderer.alpha_cutoff());
    }
    renderFrame(0);
  });
  transparencyDebugCheckbox.addEventListener("change", () => {
    setTransparencyDebugEnabled(transparencyDebugCheckbox.checked);
    renderFrame(0);
  });
  transparentSortCheckbox.addEventListener("change", () => {
    setTransparentSortEnabled(transparentSortCheckbox.checked);
    renderFrame(0);
  });
  blendColorSpaceSelect.addEventListener("change", () => {
    setBlendColorSpace(Number(blendColorSpaceSelect.value));
    renderFrame(0);
  });
  qualityModeSelect.addEventListener("change", () => {
    try {
      setQualityMode(Number(qualityModeSelect.value));
      errorOutput.textContent = "";
    } catch (error) {
      errorOutput.textContent = error instanceof Error ? error.message : String(error);
      qualityModeSelect.value = String(renderer.quality_mode());
    }
    renderFrame(0);
  });
  mipmapEnabledCheckbox.addEventListener("change", () => {
    setMipmapEnabled(mipmapEnabledCheckbox.checked);
    renderFrame(0);
  });
  mipDebugCheckbox.addEventListener("change", () => {
    setMipDebugEnabled(mipDebugCheckbox.checked);
    renderFrame(0);
  });
  normalModeSelect.addEventListener("change", () => {
    setNormalMode(Number(normalModeSelect.value));
    renderFrame(0);
  });
  animationClipSelect.addEventListener("change", () => {
    try {
      renderer.set_glb_clip(Number(animationClipSelect.value));
      animationLoopCheckbox.checked = renderer.glb_animation_looping();
      errorOutput.textContent = "";
    } catch (error) {
      errorOutput.textContent = error instanceof Error ? error.message : String(error);
      syncAnimationControls();
    }
    renderFrame(0);
  });
  animationPlayButton.addEventListener("click", () => {
    renderer.set_glb_animation_playing(!renderer.glb_animation_playing());
    renderFrame(0);
  });
  animationLoopCheckbox.addEventListener("change", () => {
    renderer.set_glb_animation_looping(animationLoopCheckbox.checked);
    renderFrame(0);
  });
  animationTimeInput.addEventListener("pointerdown", () => {
    animationTimeEditing = true;
  });
  animationTimeInput.addEventListener("input", () => {
    animationTimeEditing = true;
    try {
      seekGlbAnimation(Number(animationTimeInput.value));
      errorOutput.textContent = "";
    } catch (error) {
      errorOutput.textContent = error instanceof Error ? error.message : String(error);
      animationTimeInput.value = String(renderer.glb_animation_time());
    }
    renderFrame(0);
  });
  const finishAnimationTimeEditing = () => {
    animationTimeEditing = false;
    animationTimeInput.value = String(renderer.glb_animation_time());
    updateStatus();
  };
  animationTimeInput.addEventListener("change", finishAnimationTimeEditing);
  animationTimeInput.addEventListener("pointercancel", finishAnimationTimeEditing);
  animationTimeInput.addEventListener("blur", finishAnimationTimeEditing);
  window.addEventListener("pointerup", () => {
    if (animationTimeEditing) {
      finishAnimationTimeEditing();
    }
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

  const readAndUploadAssetFile = async (
    file,
    { readObj = readObjFileBytes, readGlb = readGlbFileBytes, decodeGlb } = {},
  ) => {
    const generation = ++meshLoadGeneration;
    const pendingId = renderer.glb_pending_id();
    if (pendingId !== 0) {
      renderer.cancel_glb(pendingId);
    }
    const glbByName = file.name.toLowerCase().endsWith(".glb");
    meshStatusText = `${glbByName ? "GLB" : "OBJ"} 읽는 중 · ${file.name}`;
    updateStatus();
    try {
      const bytes = await (glbByName ? readGlb(file) : readObj(file));
      if (generation !== meshLoadGeneration) {
        bytes.fill(0);
        return false;
      }
      const result = glbByName || hasGlbMagic(bytes)
          ? await uploadGlbBytes(bytes, file.name, {
              isCurrent: () => generation === meshLoadGeneration,
              decodeImage: decodeGlb,
            })
        : uploadObjBytes(bytes, file.name);
      if (result.id === null && !result.stale) {
        meshStatusText = `${glbByName ? "GLB" : "asset"} 로드 실패 · 기존 ${activeAssetKind} scene 유지`;
        updateStatus();
      }
      bytes.fill(0);
      return result.id !== null;
    } catch (error) {
      if (generation !== meshLoadGeneration) {
        return false;
      }
      const message = error instanceof Error ? error.message : String(error);
      meshStatusText = `asset 로드 실패 · 기존 ${activeAssetKind} scene 유지`;
      errorOutput.textContent = message;
      updateStatus();
      return false;
    } finally {
      if (generation === meshLoadGeneration) {
        meshFileInput.value = "";
      }
    }
  };
  meshFileInput.addEventListener("change", async () => {
    const [file] = meshFileInput.files;
    if (file !== undefined) {
      await readAndUploadAssetFile(file);
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
  window.addEventListener("beforeunload", () => inputCollector.dispose(), { once: true });

  const snapshot = () => ({
    internalSize: [renderer.width(), renderer.height()],
    cssSize: [canvas.clientWidth, canvas.clientHeight],
    deviceScaleFactor: window.devicePixelRatio || 1,
    framebufferLength: renderer.framebuffer_len(),
    framebufferMiB: framebufferMiB(currentSize),
    framebufferGeneration: renderer.framebuffer_generation(),
    renderSize: [renderer.render_width(), renderer.render_height()],
    typedArrayViewRebuilds: presenter.viewRebuilds,
    updateAndRenderCalls: updateCalls,
    lastFrameMetrics,
    timingWindow: frameTimingWindow.summary(),
    frameRate: frameRateTracker.summary(),
    resizeEvents,
    contextKind: "2d",
    camera: {
      mode: renderer.camera_mode(),
      eye: [renderer.camera_eye_x(), renderer.camera_eye_y(), renderer.camera_eye_z()],
      forward: [
        renderer.camera_forward_x(),
        renderer.camera_forward_y(),
        renderer.camera_forward_z(),
      ],
      yaw: renderer.camera_yaw(),
      pitch: renderer.camera_pitch(),
      orbitRadius: renderer.camera_orbit_radius(),
      input: inputCollector.debugState(),
    },
    inputSnapshot: {
      heldBits: lastInputSnapshot[0],
      pressedBits: lastInputSnapshot[1],
      releasedBits: lastInputSnapshot[2],
      pointerDx: lastInputSnapshot[3],
      pointerDy: lastInputSnapshot[4],
      wheelDelta: lastInputSnapshot[5],
      pointerButtons: lastInputSnapshot[6],
      flags: lastInputSnapshot[7],
    },
    cullMode: Number(cullModeSelect.value),
    rasterPath: {
      requestedMode: rasterResolution.requestedMode,
      actualMode: renderer.raster_path(),
      actualLabel: rasterResolution.actualLabel,
      usedFallback: rasterResolution.usedFallback,
      reason: rasterResolution.reason,
      capabilities: rasterCapabilities,
    },
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
    transparency: {
      debugEnabled: renderer.transparency_debug_enabled(),
      alphaMode: renderer.alpha_mode(),
      alphaCutoff: renderer.alpha_cutoff(),
      sortEnabled: renderer.transparent_sort_enabled(),
      blendColorSpace: renderer.blend_color_space(),
    },
    quality: {
      mode: renderer.quality_mode(),
      mipmapEnabled: renderer.mipmap_enabled(),
      mipDebugEnabled: renderer.mip_debug_enabled(),
      mipLevels: renderer.texture_mip_levels(),
    },
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
      mipLevels: renderer.texture_mip_levels(),
      successes: renderer.texture_upload_successes(),
      failures: renderer.texture_upload_failures(),
      text: textureStatusText,
    },
    meshStatus: {
      activeId: renderer.active_mesh_id(),
      sourcePositions: renderer.mesh_source_positions(),
      sourceFaces: renderer.mesh_source_faces(),
      internalVertices: renderer.mesh_internal_vertices(),
      triangles: renderer.mesh_triangles(),
      successes: renderer.mesh_upload_successes(),
      failures: renderer.mesh_upload_failures(),
      sourceMin: [
        renderer.mesh_source_min_x(),
        renderer.mesh_source_min_y(),
        renderer.mesh_source_min_z(),
      ],
      sourceMax: [
        renderer.mesh_source_max_x(),
        renderer.mesh_source_max_y(),
        renderer.mesh_source_max_z(),
      ],
      text: meshStatusText,
    },
    glbStatus: {
      active: renderer.glb_active(),
      pendingId: renderer.glb_pending_id(),
      drawItems: renderer.glb_draw_items(),
      nodes: renderer.glb_nodes(),
      skins: renderer.glb_skins(),
      joints: renderer.glb_joints(),
      vertices: renderer.glb_vertices(),
      triangles: renderer.glb_triangles(),
      clips: renderer.glb_clip_count(),
      samplerDowngrades: renderer.glb_sampler_downgrades(),
      successes: renderer.glb_upload_successes(),
      failures: renderer.glb_upload_failures(),
      lastFailure: renderer.glb_last_failure(),
      runtimeError: renderer.glb_runtime_error(),
      text: glbStatusText,
    },
    animation: {
      selectedClip: renderer.glb_selected_clip(),
      selectedName: renderer.glb_clip_name(renderer.glb_selected_clip()),
      timeSeconds: renderer.glb_animation_time(),
      durationSeconds: renderer.glb_animation_duration(),
      playing: renderer.glb_animation_playing(),
      looping: renderer.glb_animation_looping(),
    },
    pixelHash: canvasPixelHash(context, renderer.width(), renderer.height()),
    stats: rendererStats(renderer),
  });

  const runBenchmark = (warmupFrames, sampleFrames, fixedDtSeconds) => {
    if (!Number.isInteger(warmupFrames) || warmupFrames < 0 || warmupFrames > 120) {
      throw new Error("benchmark warm-up frame 수는 0..120 정수여야 합니다");
    }
    if (!Number.isInteger(sampleFrames) || sampleFrames < 1 || sampleFrames > 240) {
      throw new Error("benchmark sample frame 수는 1..240 정수여야 합니다");
    }
    if (
      !Number.isFinite(fixedDtSeconds) ||
      fixedDtSeconds < 0 ||
      fixedDtSeconds > MAX_FRAME_DT_SECONDS
    ) {
      throw new Error(`benchmark fixed dt는 0..${MAX_FRAME_DT_SECONDS}초여야 합니다`);
    }
    for (let frame = 0; frame < warmupFrames; frame += 1) {
      renderFrame(fixedDtSeconds);
    }
    const samples = [];
    for (let frame = 0; frame < sampleFrames; frame += 1) {
      samples.push(renderFrame(fixedDtSeconds));
    }
    const stats = rendererStats(renderer);
    const logicalPixels = renderer.width() * renderer.height();
    const supersamplePixels =
      renderer.quality_mode() === 1 ? renderer.render_width() * renderer.render_height() : 0;
    const bytesPerColorDepthPixel = 8;
    return {
      buildMode: "release Wasm · test automation web",
      browser: navigator.userAgent,
      device: {
        hardwareConcurrency: navigator.hardwareConcurrency ?? null,
        deviceMemoryGiB: navigator.deviceMemory ?? null,
        deviceScaleFactor: window.devicePixelRatio || 1,
      },
      resolution: [renderer.render_width(), renderer.render_height()],
      logicalResolution: [renderer.width(), renderer.height()],
      memory: {
        logicalColorDepthMiB:
          (logicalPixels * bytesPerColorDepthPixel) / (1024 * 1024),
        supersampleColorDepthMiB:
          (supersamplePixels * bytesPerColorDepthPixel) / (1024 * 1024),
        estimatedRendererTargetsMiB:
          ((logicalPixels + supersamplePixels) * bytesPerColorDepthPixel) / (1024 * 1024),
      },
      warmupFrames,
      sampleFrames,
      fixedDtSeconds,
      rasterPath: {
        requestedMode: rasterResolution.requestedMode,
        actualMode: renderer.raster_path(),
        actualLabel: rasterResolution.actualLabel,
      },
      triangles: stats.inputTriangles,
      coveredSamples: stats.coveredSamples,
      shadedSamples: stats.shadedSamples,
      timings: summarizeFrameTimings(samples),
    };
  };

  const onAnimationFrame = (timestamp) => {
    frameRateTracker.pushTimestamp(timestamp);
    const dtSeconds =
      previousTimestamp === null
        ? 0
        : Math.min(Math.max((timestamp - previousTimestamp) / 1000, 0), MAX_FRAME_DT_SECONDS);
    previousTimestamp = timestamp;
    renderFrame(dtSeconds);

    if (__AUTOMATION__) {
      window.__softRasterizer = Object.freeze({
        ready: true,
        runBenchmark,
        testFrameRateTimestamps(timestamps) {
          frameRateTracker.reset();
          for (const timestamp of timestamps) {
            frameRateTracker.pushTimestamp(timestamp);
          }
          updateStatus();
          return {
            summary: frameRateTracker.summary(),
            text: document.querySelector("#current-fps").textContent,
          };
        },
        advanceFrame(requestedDtSeconds) {
          renderFrame(requestedDtSeconds);
          return snapshot();
        },
        applyDisplayResize,
        setCameraMode(mode) {
          setCameraMode(mode);
          renderFrame(0);
          return snapshot();
        },
        inputState() {
          return inputCollector.debugState();
        },
        testInputSnapshot(values) {
          try {
            renderer.update_and_render_input(0, Float64Array.from(values));
            return { error: null, snapshot: snapshot() };
          } catch (error) {
            return {
              error: error instanceof Error ? error.message : String(error),
              snapshot: snapshot(),
            };
          }
        },
        setDebugLinesEnabled(enabled) {
          renderer.set_debug_lines_enabled(enabled);
        },
        setCullMode,
        setRasterPath(mode) {
          const resolution = setRasterPath(mode);
          renderFrame(0);
          return { resolution, snapshot: snapshot() };
        },
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
        setAlphaMode(mode) {
          setAlphaMode(mode);
          renderFrame(0);
          return snapshot();
        },
        setAlphaCutoff(cutoff) {
          try {
            setAlphaCutoff(cutoff);
            renderFrame(0);
            return { error: null, snapshot: snapshot() };
          } catch (error) {
            return {
              error: error instanceof Error ? error.message : String(error),
              snapshot: snapshot(),
            };
          }
        },
        setTransparencyDebugEnabled(enabled) {
          setTransparencyDebugEnabled(enabled);
          renderFrame(0);
          return snapshot();
        },
        setTransparentSortEnabled(enabled) {
          setTransparentSortEnabled(enabled);
          renderFrame(0);
          return snapshot();
        },
        setBlendColorSpace(mode) {
          setBlendColorSpace(mode);
          renderFrame(0);
          return snapshot();
        },
        setQualityMode(mode) {
          try {
            setQualityMode(mode);
            renderFrame(0);
            return { error: null, snapshot: snapshot() };
          } catch (error) {
            return {
              error: error instanceof Error ? error.message : String(error),
              snapshot: snapshot(),
            };
          }
        },
        setMipmapEnabled(enabled) {
          setMipmapEnabled(enabled);
          renderFrame(0);
          return snapshot();
        },
        setMipDebugEnabled(enabled) {
          setMipDebugEnabled(enabled);
          renderFrame(0);
          return snapshot();
        },
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
        uploadObjText(source, label = "automation.obj") {
          const bytes = new TextEncoder().encode(source);
          const result = uploadObjBytes(bytes, label);
          bytes.fill(0);
          return { ...result, snapshot: snapshot() };
        },
        async loadBundledFox() {
          try {
            const result = await loadBundledFox();
            return { ...result, snapshot: snapshot() };
          } catch (error) {
            return {
              id: null,
              error: error instanceof Error ? error.message : String(error),
              snapshot: snapshot(),
            };
          }
        },
        async testBundledFoxFetchFailure() {
          try {
            await loadBundledFox({
              fetchAsset: async () => new Response(null, { status: 503 }),
            });
            return { error: null, snapshot: snapshot() };
          } catch (error) {
            return {
              error: error instanceof Error ? error.message : String(error),
              snapshot: snapshot(),
            };
          }
        },
        async testBundledFoxDecodeFailure() {
          const result = await loadBundledFox({
            decodeImage: async () => {
              throw new Error("injected Fox image decode failure");
            },
          });
          return { ...result, snapshot: snapshot() };
        },
        async testLatestGlbSelectionWins() {
          const response = await fetch(new URL("./assets/Fox.glb", import.meta.url));
          if (!response.ok) {
            throw new Error(`race fixture Fox.glb HTTP ${response.status}`);
          }
          const sourceBytes = new Uint8Array(await response.arrayBuffer());
          const successesBeforeRace = renderer.glb_upload_successes();
          let signalDecodeStarted;
          const decodeStarted = new Promise((resolve) => {
            signalDecodeStarted = resolve;
          });
          let releaseDecode;
          const decodeGate = new Promise((resolve) => {
            releaseDecode = resolve;
          });
          const first = readAndUploadAssetFile(new File([], "stale.glb"), {
            readGlb: async () => sourceBytes.slice(),
            decodeGlb: async (blob) => {
              signalDecodeStarted();
              await decodeGate;
              return decodeImageFileToRgba(blob);
            },
          });
          await decodeStarted;
          const failuresBeforeCancel = renderer.glb_upload_failures();
          const second = readAndUploadAssetFile(new File([], "latest.glb"), {
            readGlb: async () => sourceBytes.slice(),
          });
          await second;
          const afterSecond = snapshot();
          releaseDecode();
          await first;
          sourceBytes.fill(0);
          return {
            successesBeforeRace,
            failuresBeforeCancel,
            afterSecond,
            afterStale: snapshot(),
          };
        },
        async uploadGlbBytes(byteValues, label = "automation.glb") {
          const bytes = Uint8Array.from(byteValues);
          try {
            const result = await uploadGlbBytes(bytes, label);
            return { ...result, snapshot: snapshot() };
          } finally {
            bytes.fill(0);
          }
        },
        setGlbClip(index) {
          renderer.set_glb_clip(index);
          animationClipSelect.value = String(index);
          renderFrame(0);
          return snapshot();
        },
        setGlbAnimationPlaying(playing) {
          renderer.set_glb_animation_playing(playing);
          renderFrame(0);
          return snapshot();
        },
        setGlbAnimationLooping(looping) {
          renderer.set_glb_animation_looping(looping);
          animationLoopCheckbox.checked = looping;
          renderFrame(0);
          return snapshot();
        },
        seekGlbAnimation(timeSeconds) {
          renderer.seek_glb_animation(timeSeconds);
          renderFrame(0);
          return snapshot();
        },
        testGlbSeekControlFailure() {
          const previousSeek = seekGlbAnimation;
          animationTimeInput.focus();
          animationTimeInput.value = String(renderer.glb_animation_duration());
          seekGlbAnimation = () => {
            throw new Error("injected animation seek failure");
          };
          try {
            animationTimeInput.dispatchEvent(new Event("input"));
            animationTimeInput.dispatchEvent(new Event("change"));
          } finally {
            seekGlbAnimation = previousSeek;
          }
          return {
            inputValue: Number(animationTimeInput.value),
            errorText: errorOutput.textContent,
            snapshot: snapshot(),
          };
        },
        validateGlbFileSize(size) {
          try {
            return { bytes: validateGlbFileSize(size), error: null };
          } catch (error) {
            return {
              bytes: null,
              error: error instanceof Error ? error.message : String(error),
            };
          }
        },
        validateObjFileSize(size) {
          try {
            return { bytes: validateObjFileSize(size), error: null };
          } catch (error) {
            return {
              bytes: null,
              error: error instanceof Error ? error.message : String(error),
            };
          }
        },
        async testOversizedObjGuard() {
          let bufferRead = false;
          const file = new Blob([]);
          Object.defineProperty(file, "size", { value: 8 * 1024 * 1024 + 1 });
          try {
            await readObjFileBytes(file, {
              readBuffer: async () => {
                bufferRead = true;
                return new ArrayBuffer(0);
              },
            });
            return { error: null, bufferRead };
          } catch (error) {
            return {
              error: error instanceof Error ? error.message : String(error),
              bufferRead,
            };
          }
        },
        async testLatestObjSelectionWins() {
          let resolveFirst;
          let resolveSecond;
          const firstBytes = new Promise((resolve) => {
            resolveFirst = resolve;
          });
          const secondBytes = new Promise((resolve) => {
            resolveSecond = resolve;
          });
          const first = readAndUploadAssetFile(
            new File([], "first.obj", { type: "text/plain" }),
            { readObj: () => firstBytes },
          );
          const second = readAndUploadAssetFile(
            new File([], "second.obj", { type: "text/plain" }),
            { readObj: () => secondBytes },
          );
          resolveSecond(
            new TextEncoder().encode("v 0 0 0\nv 2 0 0\nv 0 2 0\nf 1 3 2\n"),
          );
          await second;
          const afterSecond = snapshot();
          resolveFirst(
            new TextEncoder().encode("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 3 2\n"),
          );
          await first;
          return { afterSecond, afterFirst: snapshot() };
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
      document.documentElement.dataset.ready = "true";
      requestAnimationFrame(onAnimationFrame);
    }
  };

  syncImportedMaterialControls();
  syncAnimationControls();
  if (__AUTOMATION__) {
    glbStatusText = "test automation 시작 override · 내장 cube 유지";
  } else {
    const startupGeneration = ++meshLoadGeneration;
    try {
      const result = await loadBundledFox({
        renderNow: false,
        isCurrent: () => startupGeneration === meshLoadGeneration,
      });
      if (result.id === null && !result.stale) {
        meshStatusText = "내장 Fox 실패 · 내장 cube fallback";
        activeAssetKind = "cube";
      }
    } catch (error) {
      if (startupGeneration === meshLoadGeneration) {
        const message = error instanceof Error ? error.message : String(error);
        activeAssetKind = "cube";
        meshStatusText = "내장 Fox 실패 · 내장 cube fallback";
        glbStatusText = "GLB startup 실패 · cube를 계속 렌더링합니다";
        errorOutput.textContent = `${message} · 파일 선택에서 다른 .glb를 시도할 수 있습니다.`;
        syncImportedMaterialControls();
      }
    }
  }
  requestAnimationFrame(onAnimationFrame);
}

bootstrap().catch((error) => {
  const message = error instanceof Error ? error.message : String(error);
  document.querySelector("#error").textContent = message;
  document.documentElement.dataset.ready = "error";
  throw error;
});
