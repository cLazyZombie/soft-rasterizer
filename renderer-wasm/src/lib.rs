//! `renderer-core`를 브라우저가 프레임 단위로 호출할 수 있게 하는 얇은 어댑터.

use renderer_core::{FrameStats, InputSnapshot, Renderer as CoreRenderer};
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

    pub fn stats_invalid_values(&self) -> u32 {
        self.stats().invalid_values
    }
}

impl Renderer {
    fn stats(&self) -> FrameStats {
        self.core.stats()
    }
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
        assert_eq!(renderer.stats_input_vertices(), 0);
        assert_eq!(renderer.stats_input_triangles(), 0);
        assert_eq!(renderer.stats_clipped_triangles(), 0);
        assert_eq!(renderer.stats_rasterized_triangles(), 0);
        assert_eq!(renderer.stats_shaded_samples(), 0);
        assert_eq!(renderer.stats_invalid_values(), 0);
    }

    #[test]
    fn adapter_resize_reports_error_without_destroying_valid_target() {
        let mut renderer = Renderer::new(3, 2).expect("adapter should be valid");
        assert!(!renderer.resize((MAX_PIXEL_COUNT + 1) as u32, 1));
        assert!(renderer.last_error().contains("최대 허용치"));
        assert_eq!((renderer.width(), renderer.height()), (3, 2));
        assert_eq!(renderer.framebuffer_generation(), 0);

        assert!(renderer.resize(4, 2));
        assert_eq!(renderer.last_error(), "");
        assert_eq!(renderer.framebuffer_len(), 32);
        assert_eq!(renderer.framebuffer_generation(), 1);
    }

    #[test]
    fn adapter_constructor_returns_explicit_error_for_zero_dimensions() {
        let error = Renderer::new(0, 1).unwrap_err();
        assert!(error.contains("0보다"));
    }
}
