//! `renderer-core`를 브라우저가 프레임 단위로 호출할 수 있게 하는 얇은 어댑터.

use renderer_core::{
    CoordinateDebugSnapshot, FrameStats, InputSnapshot, Renderer as CoreRenderer, math::Vec4,
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

    pub fn stats_clipped_triangles(&self) -> u32 {
        self.stats().clipped_triangles
    }

    pub fn stats_rasterized_triangles(&self) -> u32 {
        self.stats().rasterized_triangles
    }

    pub fn stats_shaded_samples(&self) -> u32 {
        self.stats().shaded_samples
    }

    pub fn stats_debug_pixels(&self) -> u32 {
        self.stats().debug_pixels
    }

    pub fn stats_invalid_values(&self) -> u32 {
        self.stats().invalid_values
    }
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
    format!(
        "선택 정점 v{} · model Y {:.3} rad\n\
         Object {}\nWorld  {}\nView   {}\nClip   {}\n\
         clip 거리 [x+w, w-x, y+w, w-y, z, w-z]\n\
         [{:.3}, {:.3}, {:.3}, {:.3}, {:.3}, {:.3}]\n\
         Object 범위 {} .. {}\nWorld  범위 {} .. {}\n\
         View   범위 {} .. {}\nClip   범위 {} .. {}\n\
         invalid values: {} · 첫 공간: {}",
        snapshot.selected_vertex_index,
        snapshot.rotation_y_radians,
        format_vec4(trace.value(CoordinateSpace::Object)),
        format_vec4(trace.value(CoordinateSpace::World)),
        format_vec4(trace.value(CoordinateSpace::View)),
        format_vec4(trace.value(CoordinateSpace::Clip)),
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
        first_invalid,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderer_core::MAX_PIXEL_COUNT;

    #[test]
    fn adapter_exposes_framebuffer_input_and_zeroed_pipeline_stats() {
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
        assert_eq!(renderer.stats_input_vertices(), 8);
        assert_eq!(renderer.stats_input_triangles(), 0);
        assert_eq!(renderer.stats_clipped_triangles(), 0);
        assert_eq!(renderer.stats_rasterized_triangles(), 0);
        assert_eq!(renderer.stats_shaded_samples(), 0);
        assert!(renderer.stats_debug_pixels() > 0);
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
        renderer.set_debug_lines_enabled(true);
        renderer.update_and_render(0.0, 0);
        assert!(renderer.stats_debug_pixels() > 0);
    }

    #[test]
    fn adapter_formats_coordinate_overlay_and_reports_invalid_rotation() {
        let mut renderer = Renderer::new(64, 64).expect("adapter should be valid");
        renderer.update_and_render(0.0, 0);
        let text = renderer.coordinate_debug_text();
        assert!(text.contains("선택 정점 v6"));
        assert!(text.contains("Object"));
        assert!(text.contains("clip 거리"));
        assert!(text.contains("invalid values: 0 · 첫 공간: 없음"));

        renderer.set_model_rotation_y(f32::NAN);
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_invalid_values(), 24);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("invalid values: 24 · 첫 공간: World")
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
