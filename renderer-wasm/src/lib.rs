//! `renderer-core`를 브라우저가 프레임 단위로 호출할 수 있게 하는 얇은 어댑터.

use renderer_core::{
    CoordinateDebugSnapshot, FrameStats, InputSnapshot, Renderer as CoreRenderer,
    math::Vec4,
    raster::{
        AttributeInterpolationMode, CullMode, DepthDebugMode, PipelineDebugMode, WindingDebugMode,
    },
    transform::CoordinateSpace,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Debug)]
pub struct Renderer {
    core: CoreRenderer,
    last_error: String,
}

#[wasm_bindgen]
impl Renderer {
    #[wasm_bindgen(constructor)]
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let core = CoreRenderer::new(width as usize, height as usize)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            core,
            last_error: String::new(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        match self.core.resize(width as usize, height as usize) {
            Ok(()) => {
                self.last_error.clear();
                true
            }
            Err(error) => {
                self.last_error = error.to_string();
                false
            }
        }
    }

    pub fn update_and_render(&mut self, dt_seconds: f32, packed_input: u32) {
        self.core
            .update_and_render(dt_seconds, InputSnapshot::from_packed(packed_input));
    }

    pub fn set_debug_lines_enabled(&mut self, enabled: bool) {
        self.core.set_debug_lines_enabled(enabled);
    }

    pub fn set_cull_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => CullMode::None,
            1 => CullMode::Back,
            2 => CullMode::Front,
            _ => return Err(format!("알 수 없는 cull mode입니다: {mode}")),
        };
        self.core.set_cull_mode(mode);
        Ok(())
    }

    pub fn set_winding_debug_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => WindingDebugMode::VertexColor,
            1 => WindingDebugMode::Facing,
            2 => WindingDebugMode::Barycentric,
            _ => return Err(format!("알 수 없는 winding debug mode입니다: {mode}")),
        };
        self.core.set_winding_debug_mode(mode);
        Ok(())
    }

    pub fn set_clip_debug_enabled(&mut self, enabled: bool) {
        self.core.set_clip_debug_enabled(enabled);
    }

    pub fn set_coverage_debug_enabled(&mut self, enabled: bool) {
        self.core.set_coverage_debug_enabled(enabled);
    }

    pub fn set_interpolation_debug_enabled(&mut self, enabled: bool) {
        self.core.set_interpolation_debug_enabled(enabled);
    }

    pub fn set_perspective_debug_enabled(&mut self, enabled: bool) {
        self.core.set_perspective_debug_enabled(enabled);
    }

    pub fn set_attribute_interpolation_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => AttributeInterpolationMode::Affine,
            1 => AttributeInterpolationMode::PerspectiveCorrect,
            _ => {
                return Err(format!(
                    "알 수 없는 attribute interpolation mode입니다: {mode}"
                ));
            }
        };
        self.core.set_attribute_interpolation_mode(mode);
        Ok(())
    }

    pub fn set_depth_debug_enabled(&mut self, enabled: bool) {
        self.core.set_depth_debug_enabled(enabled);
    }

    pub fn set_depth_order_reversed(&mut self, reversed: bool) {
        self.core.set_depth_order_reversed(reversed);
    }

    pub fn set_depth_debug_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => DepthDebugMode::Off,
            1 => DepthDebugMode::Grayscale,
            2 => DepthDebugMode::Heatmap,
            _ => return Err(format!("알 수 없는 depth debug mode입니다: {mode}")),
        };
        self.core.set_depth_debug_mode(mode);
        Ok(())
    }

    pub fn set_pipeline_debug_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => PipelineDebugMode::Solid,
            1 => PipelineDebugMode::Wireframe,
            2 => PipelineDebugMode::TriangleId,
            3 => PipelineDebugMode::Barycentric,
            4 => PipelineDebugMode::Depth,
            5 => PipelineDebugMode::DepthHeatmap,
            6 => PipelineDebugMode::FrontBack,
            _ => return Err(format!("알 수 없는 pipeline debug mode입니다: {mode}")),
        };
        self.core.set_pipeline_debug_mode(mode);
        Ok(())
    }

    pub fn set_model_rotation_y(&mut self, rotation_y_radians: f32) {
        self.core.set_model_rotation_y(rotation_y_radians);
    }

    pub fn coordinate_debug_text(&self) -> String {
        format_coordinate_debug(self.core.coordinate_debug_snapshot())
    }

    pub fn width(&self) -> u32 {
        self.core.width() as u32
    }

    pub fn height(&self) -> u32 {
        self.core.height() as u32
    }

    pub fn framebuffer_ptr(&self) -> *const u8 {
        self.core.color_buffer().as_ptr()
    }

    pub fn framebuffer_len(&self) -> usize {
        self.core.color_buffer().len()
    }

    pub fn framebuffer_generation(&self) -> u32 {
        self.core.framebuffer_generation()
    }

    pub fn last_error(&self) -> String {
        self.last_error.clone()
    }

    pub fn stats_frame_index(&self) -> u32 {
        self.stats().frame_index
    }

    pub fn stats_dt_seconds(&self) -> f32 {
        self.stats().dt_seconds
    }

    pub fn stats_input_bits(&self) -> u32 {
        self.stats().input_bits
    }

    pub fn stats_input_vertices(&self) -> u32 {
        self.stats().input_vertices
    }

    pub fn stats_input_triangles(&self) -> u32 {
        self.stats().input_triangles
    }

    pub fn stats_transformed_vertices(&self) -> u32 {
        self.stats().transformed_vertices
    }

    pub fn stats_submitted_triangles(&self) -> u32 {
        self.stats().submitted_triangles
    }

    pub fn stats_culled_triangles(&self) -> u32 {
        self.stats().culled_triangles
    }

    pub fn stats_degenerate_triangles(&self) -> u32 {
        self.stats().degenerate_triangles
    }

    pub fn stats_invalid_triangles(&self) -> u32 {
        self.stats().invalid_triangles
    }

    pub fn stats_fully_clipped_triangles(&self) -> u32 {
        self.stats().fully_clipped_triangles
    }

    pub fn stats_clip_invalid_triangles(&self) -> u32 {
        self.stats().clip_invalid_triangles
    }

    pub fn stats_generated_triangles(&self) -> u32 {
        self.stats().generated_triangles
    }

    pub fn stats_max_clip_polygon_vertices(&self) -> u32 {
        self.stats().max_clip_polygon_vertices
    }

    pub fn stats_rasterized_triangles(&self) -> u32 {
        self.stats().rasterized_triangles
    }

    pub fn stats_covered_samples(&self) -> u32 {
        self.stats().covered_samples
    }

    pub fn stats_shaded_samples(&self) -> u32 {
        self.stats().shaded_samples
    }

    pub fn stats_depth_passed_samples(&self) -> u32 {
        self.stats().depth_passed_samples
    }

    pub fn stats_depth_failed_samples(&self) -> u32 {
        self.stats().depth_failed_samples
    }

    pub fn stats_invalid_depth_samples(&self) -> u32 {
        self.stats().invalid_depth_samples
    }

    pub fn stats_max_barycentric_sum_error(&self) -> f32 {
        self.stats().max_barycentric_sum_error
    }

    pub fn stats_interpolated_inv_w_samples(&self) -> u32 {
        self.stats().interpolated_inv_w_samples
    }

    pub fn stats_invalid_interpolation_samples(&self) -> u32 {
        invalid_interpolation_samples(self.stats())
    }

    pub fn stats_min_interpolated_inv_w(&self) -> f32 {
        self.stats().min_interpolated_inv_w
    }

    pub fn stats_max_interpolated_inv_w(&self) -> f32 {
        self.stats().max_interpolated_inv_w
    }

    pub fn stats_sample_counter_overflow(&self) -> bool {
        self.stats().sample_counter_overflow
    }

    pub fn stats_debug_pixels(&self) -> u32 {
        self.stats().debug_pixels
    }

    pub fn stats_invalid_values(&self) -> u32 {
        self.stats().invalid_values
    }
}

const fn invalid_interpolation_samples(stats: FrameStats) -> u32 {
    stats.invalid_interpolation_samples
}

impl Renderer {
    fn stats(&self) -> FrameStats {
        self.core.stats()
    }
}

fn format_vec4(value: Vec4) -> String {
    format!(
        "({:.3}, {:.3}, {:.3}, {:.3})",
        value.x, value.y, value.z, value.w
    )
}

fn format_coordinate_debug(snapshot: CoordinateDebugSnapshot) -> String {
    let trace = snapshot.selected_vertex;
    let bounds = snapshot.diagnostics.bounds;
    let distances = snapshot.clip_plane_distances.0;
    let first_invalid = snapshot
        .diagnostics
        .first_invalid_space
        .map_or("없음", CoordinateSpace::label);
    let ndc = snapshot.selected_ndc.map_or_else(
        || "invalid".to_string(),
        |position| {
            format!(
                "({:.3}, {:.3}, {:.3})",
                position.0.x, position.0.y, position.0.z
            )
        },
    );
    let screen = snapshot.selected_viewport.map_or_else(
        || "invalid".to_string(),
        |position| {
            format!(
                "({:.1}, {:.1}, z={:.3})",
                position.x, position.y, position.z_ndc
            )
        },
    );
    let attributes = snapshot.selected_attributes;
    let pipeline = if snapshot.depth_debug_enabled {
        format!(
            "depth overlap fixture · identity M/V/P vertex stage · viewport aspect {:.3}",
            snapshot.aspect
        )
    } else if snapshot.perspective_debug_enabled {
        format!(
            "perspective UV fixture · identity M/V · LH zero-to-one P · viewport aspect {:.3}",
            snapshot.aspect
        )
    } else if snapshot.interpolation_debug_enabled {
        format!(
            "affine RGB fixture · identity M/V/P vertex stage · viewport aspect {:.3}",
            snapshot.aspect
        )
    } else if snapshot.coverage_debug_enabled {
        format!(
            "top-left coverage fixture · identity M/V/P vertex stage · viewport aspect {:.3}",
            snapshot.aspect
        )
    } else if snapshot.clip_debug_enabled {
        format!(
            "동차 clip fixture · identity M/V/P vertex stage · viewport aspect {:.3}",
            snapshot.aspect
        )
    } else {
        format!(
            "LH/+Z 카메라 · fov {:.1}° · near {:.3} · far {:.1} · aspect {:.3}",
            snapshot.fov_y_radians.to_degrees(),
            snapshot.near,
            snapshot.far,
            snapshot.aspect
        )
    };
    let scene_name = if snapshot.depth_debug_enabled {
        "near/far overlap triangle mesh"
    } else if snapshot.perspective_debug_enabled {
        "tilted procedural checker quad mesh"
    } else if snapshot.interpolation_debug_enabled {
        "barycentric RGB triangle mesh"
    } else if snapshot.coverage_debug_enabled {
        "coverage quad mesh"
    } else if snapshot.clip_debug_enabled {
        "clip debug mesh"
    } else {
        "indexed cube mesh"
    };
    let scene_suffix = if snapshot.depth_debug_enabled {
        if snapshot.depth_order_reversed {
            " · far-first submission"
        } else {
            " · near-first submission"
        }
    } else if snapshot.perspective_debug_enabled {
        match snapshot.attribute_interpolation_mode {
            AttributeInterpolationMode::Affine => " · affine comparison",
            AttributeInterpolationMode::PerspectiveCorrect => " · perspective-correct UV",
        }
    } else if snapshot.interpolation_debug_enabled {
        " · vertex colors R/G/B"
    } else if snapshot.coverage_debug_enabled {
        " · 두 삼각형/공유 대각선"
    } else if snapshot.clip_debug_enabled {
        " · near/left/top 교차"
    } else {
        ""
    };
    let scene = format!(
        "{} · vertices {} · indices {} · triangles {} · material {}{}",
        scene_name,
        snapshot.mesh_vertices,
        snapshot.mesh_indices,
        snapshot.mesh_triangles,
        snapshot.material_id,
        scene_suffix
    );
    format!(
        "{}\n\
         {}\n\
         winding screen y-down orient2d > 0 front · cull {} · debug {}\n\
         pipeline state debug {} · strict depth test/write · material {}\n\
         triangle stats input {} · submitted {} · culled {} · degenerate {} · invalid {}\n\
         clip stats fully clipped {} · clip invalid {} · generated {} · max polygon vertices {}\n\
         coverage stats rasterized {} · covered {} · shaded samples {} · counter overflow {} · S=256 pixel center/top-left\n\
         interpolation stats max |lambda sum - 1| {:.9} · mode {}\n\
         inv_w stats samples {} · invalid {} · q range [{:.6}, {:.6}]\n\
         depth stats passed {} · failed {} · invalid {} · strict < · clear +infinity · debug {}\n\
         선택 정점 v{} (X-ray overlay {} · culling/depth 무관) · model Y {:.3} rad\n\
         normal ({:.3}, {:.3}, {:.3}) · UV ({:.3}, {:.3}) · color ({:.3}, {:.3}, {:.3}, {:.3})\n\
         Object {}\nWorld  {}\nView   {}\nClip   {}\n\
         w_clip {:.3} · NDC {} · Screen {}\n\
         clip 거리 [x+w, w-x, y+w, w-y, z, w-z]\n\
         [{:.3}, {:.3}, {:.3}, {:.3}, {:.3}, {:.3}]\n\
         Object 범위 {} .. {}\nWorld  범위 {} .. {}\n\
         View   범위 {} .. {}\nClip   범위 {} .. {}\n\
         invalid values: {} · projection failures: {} · 첫 공간: {}",
        pipeline,
        scene,
        snapshot.cull_mode.label(),
        snapshot.winding_debug_mode.label(),
        snapshot.pipeline_state.debug_mode.label(),
        snapshot.material_id,
        snapshot.frame_stats.input_triangles,
        snapshot.frame_stats.submitted_triangles,
        snapshot.frame_stats.culled_triangles,
        snapshot.frame_stats.degenerate_triangles,
        snapshot.frame_stats.invalid_triangles,
        snapshot.frame_stats.fully_clipped_triangles,
        snapshot.frame_stats.clip_invalid_triangles,
        snapshot.frame_stats.generated_triangles,
        snapshot.frame_stats.max_clip_polygon_vertices,
        snapshot.frame_stats.rasterized_triangles,
        snapshot.frame_stats.covered_samples,
        snapshot.frame_stats.shaded_samples,
        snapshot.frame_stats.sample_counter_overflow,
        snapshot.frame_stats.max_barycentric_sum_error,
        snapshot.attribute_interpolation_mode.label(),
        snapshot.frame_stats.interpolated_inv_w_samples,
        snapshot.frame_stats.invalid_interpolation_samples,
        snapshot.frame_stats.min_interpolated_inv_w,
        snapshot.frame_stats.max_interpolated_inv_w,
        snapshot.frame_stats.depth_passed_samples,
        snapshot.frame_stats.depth_failed_samples,
        snapshot.frame_stats.invalid_depth_samples,
        snapshot.depth_debug_mode.label(),
        snapshot.selected_vertex_index,
        if snapshot.debug_lines_enabled {
            "on"
        } else {
            "off"
        },
        snapshot.rotation_y_radians,
        attributes.normal_world.x,
        attributes.normal_world.y,
        attributes.normal_world.z,
        attributes.uv.x,
        attributes.uv.y,
        attributes.color.x,
        attributes.color.y,
        attributes.color.z,
        attributes.color.w,
        format_vec4(trace.value(CoordinateSpace::Object)),
        format_vec4(trace.value(CoordinateSpace::World)),
        format_vec4(trace.value(CoordinateSpace::View)),
        format_vec4(trace.value(CoordinateSpace::Clip)),
        trace.clip_pos.0.w,
        ndc,
        screen,
        distances[0],
        distances[1],
        distances[2],
        distances[3],
        distances[4],
        distances[5],
        format_vec4(bounds[0].min),
        format_vec4(bounds[0].max),
        format_vec4(bounds[1].min),
        format_vec4(bounds[1].max),
        format_vec4(bounds[2].min),
        format_vec4(bounds[2].max),
        format_vec4(bounds[3].min),
        format_vec4(bounds[3].max),
        snapshot.diagnostics.invalid_values,
        snapshot.projection_failures,
        first_invalid,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderer_core::MAX_PIXEL_COUNT;

    #[test]
    fn invalid_interpolation_counter_mapping_preserves_nonzero_values() {
        assert_eq!(
            invalid_interpolation_samples(FrameStats {
                invalid_interpolation_samples: 7,
                ..FrameStats::default()
            }),
            7
        );
    }

    #[test]
    fn adapter_exposes_framebuffer_input_and_mesh_pipeline_stats() {
        let mut renderer = Renderer::new(3, 2).expect("adapter should be valid");
        assert_eq!((renderer.width(), renderer.height()), (3, 2));
        assert!(!renderer.framebuffer_ptr().is_null());
        assert_eq!(renderer.framebuffer_len(), 24);
        assert_eq!(renderer.framebuffer_generation(), 0);
        assert_eq!(renderer.last_error(), "");

        renderer.update_and_render(0.016, 0xa5);
        assert_eq!(renderer.stats_frame_index(), 1);
        assert_eq!(renderer.stats_dt_seconds(), 0.016);
        assert_eq!(renderer.stats_input_bits(), 0xa5);
        assert_eq!(renderer.stats_input_vertices(), 24);
        assert_eq!(renderer.stats_input_triangles(), 12);
        assert_eq!(renderer.stats_transformed_vertices(), 24);
        assert_eq!(renderer.stats_submitted_triangles(), 4);
        assert_eq!(renderer.stats_culled_triangles(), 8);
        assert_eq!(renderer.stats_degenerate_triangles(), 0);
        assert_eq!(renderer.stats_invalid_triangles(), 0);
        assert_eq!(renderer.stats_fully_clipped_triangles(), 0);
        assert_eq!(renderer.stats_clip_invalid_triangles(), 0);
        assert_eq!(renderer.stats_generated_triangles(), 12);
        assert_eq!(renderer.stats_max_clip_polygon_vertices(), 3);
        assert_eq!(renderer.stats_rasterized_triangles(), 4);
        assert_eq!(renderer.stats_covered_samples(), 1);
        assert_eq!(renderer.stats_shaded_samples(), 1);
        assert_eq!(renderer.stats_depth_passed_samples(), 1);
        assert_eq!(renderer.stats_depth_failed_samples(), 0);
        assert_eq!(renderer.stats_invalid_depth_samples(), 0);
        assert_eq!(renderer.stats_max_barycentric_sum_error(), 0.0);
        assert_eq!(renderer.stats_interpolated_inv_w_samples(), 1);
        assert_eq!(renderer.stats_invalid_interpolation_samples(), 0);
        assert!(renderer.stats_min_interpolated_inv_w() > 0.0);
        assert_eq!(
            renderer.stats_min_interpolated_inv_w(),
            renderer.stats_max_interpolated_inv_w()
        );
        assert!(!renderer.stats_sample_counter_overflow());
        assert_eq!(renderer.stats_debug_pixels(), 0);
        assert_eq!(renderer.stats_invalid_values(), 0);
    }

    #[test]
    fn adapter_resize_reports_error_without_destroying_valid_target() {
        let mut renderer = Renderer::new(3, 2).expect("adapter should be valid");
        assert!(!renderer.resize((MAX_PIXEL_COUNT + 1) as u32, 1));
        assert!(renderer.last_error().contains("최대 허용치"));
        assert_eq!((renderer.width(), renderer.height()), (3, 2));
        assert_eq!(renderer.framebuffer_generation(), 0);

        let pointer = renderer.framebuffer_ptr();
        assert!(renderer.resize(3, 2));
        assert_eq!(renderer.framebuffer_ptr(), pointer);
        assert_eq!(renderer.framebuffer_generation(), 0);

        assert!(renderer.resize(4, 2));
        assert_eq!(renderer.last_error(), "");
        assert_eq!(renderer.framebuffer_len(), 32);
        assert_eq!(renderer.framebuffer_generation(), 1);
    }

    #[test]
    fn adapter_toggles_bresenham_debug_pass() {
        let mut renderer = Renderer::new(64, 64).expect("adapter should be valid");
        renderer.set_debug_lines_enabled(false);
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_debug_pixels(), 0);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("X-ray overlay off · culling/depth 무관")
        );
        renderer.set_debug_lines_enabled(true);
        renderer.update_and_render(0.0, 0);
        assert!(renderer.stats_debug_pixels() > 0);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("X-ray overlay on · culling/depth 무관")
        );
    }

    #[test]
    fn adapter_exposes_near_corner_clipping_fixture_and_stats() {
        let mut renderer = Renderer::new(64, 64).expect("adapter should be valid");
        renderer.set_cull_mode(0).unwrap();
        renderer.set_clip_debug_enabled(true);
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_input_vertices(), 3);
        assert_eq!(renderer.stats_input_triangles(), 1);
        assert_eq!(renderer.stats_generated_triangles(), 3);
        assert_eq!(renderer.stats_submitted_triangles(), 3);
        assert_eq!(renderer.stats_max_clip_polygon_vertices(), 5);
        assert!(
            renderer.coordinate_debug_text().contains(
                "동차 clip fixture · identity M/V/P vertex stage · viewport aspect 1.000"
            )
        );
        assert!(renderer.coordinate_debug_text().contains(
            "clip debug mesh · vertices 3 · indices 3 · triangles 1 · material 0 · near/left/top 교차"
        ));
        assert!(renderer.coordinate_debug_text().contains("선택 정점 v2"));
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("Object (-0.250, -0.250, 0.500, 1.000)")
        );
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("Clip   (-0.250, -0.250, 0.500, 1.000)")
        );
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("Screen (24.0, 40.0, z=0.500)")
        );
        assert!(renderer.coordinate_debug_text().contains(
            "clip stats fully clipped 0 · clip invalid 0 · generated 3 · max polygon vertices 5"
        ));
    }

    #[test]
    fn adapter_exposes_fixed_point_top_left_coverage_fixture_and_stats() {
        let mut renderer = Renderer::new(64, 64).expect("adapter should be valid");
        renderer.set_debug_lines_enabled(false);
        renderer.set_coverage_debug_enabled(true);
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_input_vertices(), 6);
        assert_eq!(renderer.stats_input_triangles(), 2);
        assert_eq!(renderer.stats_generated_triangles(), 2);
        assert_eq!(renderer.stats_submitted_triangles(), 2);
        assert_eq!(renderer.stats_rasterized_triangles(), 2);
        assert_eq!(renderer.stats_covered_samples(), 1_024);
        assert_eq!(renderer.stats_shaded_samples(), 1_024);
        assert!(renderer.stats_max_barycentric_sum_error() <= 2.0 * f32::EPSILON);
        assert_eq!(renderer.stats_debug_pixels(), 0);
        let text = renderer.coordinate_debug_text();
        assert!(text.contains(
            "top-left coverage fixture · identity M/V/P vertex stage · viewport aspect 1.000"
        ));
        assert!(text.contains(
            "coverage quad mesh · vertices 6 · indices 6 · triangles 2 · material 0 · 두 삼각형/공유 대각선"
        ));
        assert!(text.contains(
            "coverage stats rasterized 2 · covered 1024 · shaded samples 1024 · counter overflow false · S=256 pixel center/top-left"
        ));

        renderer.set_clip_debug_enabled(true);
        renderer.update_and_render(0.0, 0);
        assert!(renderer.coordinate_debug_text().contains("clip debug mesh"));
    }

    #[test]
    fn adapter_exposes_affine_rgb_interpolation_fixture_and_barycentric_stats() {
        let mut renderer = Renderer::new(64, 64).expect("adapter should be valid");
        renderer.set_debug_lines_enabled(false);
        renderer.set_interpolation_debug_enabled(true);
        renderer.set_winding_debug_mode(2).unwrap();
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_input_vertices(), 3);
        assert_eq!(renderer.stats_input_triangles(), 1);
        assert_eq!(renderer.stats_generated_triangles(), 1);
        assert_eq!(renderer.stats_submitted_triangles(), 1);
        assert_eq!(renderer.stats_rasterized_triangles(), 1);
        assert_eq!(renderer.stats_shaded_samples(), 882);
        assert_eq!(renderer.stats_max_barycentric_sum_error(), f32::EPSILON);
        assert_eq!(renderer.stats_debug_pixels(), 0);
        let text = renderer.coordinate_debug_text();
        assert!(
            text.contains(
                "affine RGB fixture · identity M/V/P vertex stage · viewport aspect 1.000"
            )
        );
        assert!(text.contains(
            "barycentric RGB triangle mesh · vertices 3 · indices 3 · triangles 1 · material 0 · vertex colors R/G/B"
        ));
        assert!(text.contains("cull back · debug barycentric RGB"));
        assert!(text.contains("interpolation stats max |lambda sum - 1| 0.000000119"));

        renderer.set_clip_debug_enabled(true);
        renderer.update_and_render(0.0, 0);
        assert!(renderer.coordinate_debug_text().contains("clip debug mesh"));
    }

    #[test]
    fn adapter_exposes_depth_order_stats_and_debug_modes() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_debug_lines_enabled(false);
        renderer.set_depth_debug_enabled(true);
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_input_vertices(), 6);
        assert_eq!(renderer.stats_input_triangles(), 2);
        assert_eq!(renderer.stats_submitted_triangles(), 2);
        assert_eq!(renderer.stats_rasterized_triangles(), 2);
        assert_eq!(renderer.stats_shaded_samples(), 1_199);
        assert_eq!(renderer.stats_depth_passed_samples(), 1_199);
        assert_eq!(renderer.stats_depth_failed_samples(), 202);
        assert_eq!(renderer.stats_invalid_depth_samples(), 0);
        let text = renderer.coordinate_debug_text();
        assert!(text.contains(
            "depth overlap fixture · identity M/V/P vertex stage · viewport aspect 1.000"
        ));
        assert!(text.contains(
            "near/far overlap triangle mesh · vertices 6 · indices 6 · triangles 2 · material 0 · near-first submission"
        ));
        assert!(text.contains(
            "depth stats passed 1199 · failed 202 · invalid 0 · strict < · clear +infinity · debug off"
        ));

        renderer.set_depth_order_reversed(true);
        renderer.set_depth_debug_mode(1).unwrap();
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_shaded_samples(), 1_401);
        assert_eq!(renderer.stats_depth_passed_samples(), 1_401);
        assert_eq!(renderer.stats_depth_failed_samples(), 0);
        let text = renderer.coordinate_debug_text();
        assert!(text.contains("far-first submission"));
        assert!(text.contains("debug grayscale"));

        renderer.set_depth_debug_mode(2).unwrap();
        renderer.update_and_render(0.0, 0);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("debug range heatmap")
        );
        assert!(
            renderer
                .set_depth_debug_mode(3)
                .unwrap_err()
                .contains("depth debug mode")
        );
        renderer.set_depth_debug_mode(0).unwrap();
        renderer.update_and_render(0.0, 0);
        assert!(renderer.coordinate_debug_text().contains("debug off"));

        renderer.set_clip_debug_enabled(true);
        renderer.update_and_render(0.0, 0);
        assert!(renderer.coordinate_debug_text().contains("clip debug mesh"));
    }

    #[test]
    fn adapter_exposes_perspective_uv_fixture_modes_and_q_stats() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_debug_lines_enabled(false);
        renderer.set_perspective_debug_enabled(true);
        renderer.set_attribute_interpolation_mode(0).unwrap();
        renderer.update_and_render(0.0, 0);
        let affine_text = renderer.coordinate_debug_text();
        assert_eq!(renderer.stats_input_vertices(), 4);
        assert_eq!(renderer.stats_input_triangles(), 2);
        assert_eq!(renderer.stats_submitted_triangles(), 2);
        assert_eq!(renderer.stats_invalid_interpolation_samples(), 0);
        assert_eq!(
            renderer.stats_interpolated_inv_w_samples(),
            renderer.stats_shaded_samples()
        );
        assert!(renderer.stats_min_interpolated_inv_w() > 0.2);
        assert!(renderer.stats_max_interpolated_inv_w() < 0.5);
        assert!(affine_text.contains(
            "perspective UV fixture · identity M/V · LH zero-to-one P · viewport aspect 1.000"
        ));
        assert!(affine_text.contains(
            "tilted procedural checker quad mesh · vertices 4 · indices 6 · triangles 2 · material 0 · affine comparison"
        ));
        assert!(affine_text.contains("mode affine comparison"));
        assert!(affine_text.contains("inv_w stats samples"));

        renderer.set_attribute_interpolation_mode(1).unwrap();
        renderer.update_and_render(0.0, 0);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("perspective-correct UV")
        );
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("mode perspective-correct")
        );
        assert!(
            renderer
                .set_attribute_interpolation_mode(2)
                .unwrap_err()
                .contains("attribute interpolation mode")
        );

        renderer.set_interpolation_debug_enabled(true);
        renderer.update_and_render(0.0, 0);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("affine RGB fixture")
        );
    }

    #[test]
    fn adapter_maps_culling_and_winding_debug_modes_and_rejects_unknown_values() {
        let mut renderer = Renderer::new(64, 64).expect("adapter should be valid");
        renderer.set_cull_mode(1).unwrap();
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_submitted_triangles(), 4);
        assert_eq!(renderer.stats_culled_triangles(), 8);

        renderer.set_cull_mode(0).unwrap();
        renderer.set_winding_debug_mode(1).unwrap();
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_submitted_triangles(), 12);
        assert_eq!(renderer.stats_culled_triangles(), 0);
        let text = renderer.coordinate_debug_text();
        assert!(text.contains("cull none · debug front green / back red"));

        renderer.set_cull_mode(2).unwrap();
        renderer.set_winding_debug_mode(2).unwrap();
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_submitted_triangles(), 8);
        assert_eq!(renderer.stats_culled_triangles(), 4);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("cull front · debug barycentric RGB")
        );

        renderer.set_winding_debug_mode(0).unwrap();

        assert!(renderer.set_cull_mode(3).unwrap_err().contains("cull mode"));
        assert!(
            renderer
                .set_winding_debug_mode(3)
                .unwrap_err()
                .contains("winding debug mode")
        );
    }

    #[test]
    fn adapter_maps_all_chapter_fifteen_pipeline_debug_modes() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_debug_lines_enabled(true);
        let expected_labels = [
            "solid vertex color",
            "wireframe",
            "triangle ID",
            "barycentric RGB",
            "depth grayscale",
            "depth range heatmap",
            "front green / back red",
        ];
        let mut reference_counts = None;
        for (mode, expected_label) in expected_labels.into_iter().enumerate() {
            renderer.set_pipeline_debug_mode(mode as u32).unwrap();
            renderer.update_and_render(0.0, 0);
            assert!(
                renderer
                    .coordinate_debug_text()
                    .contains(&format!("pipeline state debug {expected_label}"))
            );
            let counts = (
                renderer.stats_generated_triangles(),
                renderer.stats_submitted_triangles(),
                renderer.stats_rasterized_triangles(),
                renderer.stats_covered_samples(),
                renderer.stats_depth_passed_samples(),
                renderer.stats_depth_failed_samples(),
            );
            assert_eq!(*reference_counts.get_or_insert(counts), counts);
            assert!(renderer.stats_debug_pixels() > 0);
        }
        assert!(
            renderer
                .set_pipeline_debug_mode(7)
                .unwrap_err()
                .contains("pipeline debug mode")
        );
    }

    #[test]
    fn adapter_formats_coordinate_overlay_and_reports_invalid_rotation() {
        let mut renderer = Renderer::new(64, 64).expect("adapter should be valid");
        renderer.update_and_render(0.0, 0);
        let text = renderer.coordinate_debug_text();
        assert!(text.contains("선택 정점 v6"));
        assert!(text.contains("X-ray overlay off · culling/depth 무관"));
        assert!(text.contains("indexed cube mesh · vertices 24 · indices 36 · triangles 12"));
        assert!(text.contains("winding screen y-down orient2d > 0 front · cull back"));
        assert!(text.contains(
            "triangle stats input 12 · submitted 4 · culled 8 · degenerate 0 · invalid 0"
        ));
        assert!(text.contains("normal ("));
        assert!(text.contains("UV ("));
        assert!(text.contains("LH/+Z 카메라 · fov 60.0° · near 0.100 · far 100.0"));
        assert!(text.contains("w_clip"));
        assert!(text.contains("NDC ("));
        assert!(text.contains("Screen ("));
        assert!(text.contains("Object"));
        assert!(text.contains("clip 거리"));
        assert!(text.contains("invalid values: 0 · projection failures: 0 · 첫 공간: 없음"));

        renderer.set_model_rotation_y(f32::NAN);
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_invalid_values(), 72);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("NDC invalid · Screen invalid")
        );
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("invalid values: 72 · projection failures: 24 · 첫 공간: World")
        );

        renderer.set_model_rotation_y(0.0);
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_invalid_values(), 0);
    }

    #[test]
    fn adapter_constructor_returns_explicit_error_for_zero_dimensions() {
        let error = Renderer::new(0, 1).unwrap_err();
        assert!(error.contains("0보다"));
    }
}
