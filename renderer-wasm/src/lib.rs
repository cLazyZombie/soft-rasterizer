//! `renderer-core`를 브라우저가 프레임 단위로 호출할 수 있게 하는 얇은 어댑터.

use renderer_core::{
    BlendColorSpace, CoordinateDebugSnapshot, FrameStats, InputSnapshot, QualityMode,
    Renderer as CoreRenderer,
    camera_control::CameraMode,
    math::{Vec2, Vec3, Vec4},
    raster::{
        AttributeInterpolationMode, CullMode, DepthDebugMode, PipelineDebugMode, RasterPath,
        WindingDebugMode,
    },
    texture::{
        AddressMode, AlphaMode, FilterMode, NormalMode, SamplerState, ShaderMode,
        TextureColorSpace, TextureId,
    },
    transform::CoordinateSpace,
};
use wasm_bindgen::prelude::*;

const INPUT_SNAPSHOT_LENGTH: usize = 8;

fn input_u32(value: f64, name: &str) -> Result<u32, String> {
    if !value.is_finite() || value.fract() != 0.0 || !(0.0..=u32::MAX as f64).contains(&value) {
        return Err(format!(
            "input snapshot {name}는 u32 범위의 정수여야 합니다"
        ));
    }
    Ok(value as u32)
}

fn input_f32(value: f64, name: &str) -> Result<f32, String> {
    let value = value as f32;
    if !value.is_finite() {
        return Err(format!(
            "input snapshot {name}는 유한한 f32 값이어야 합니다"
        ));
    }
    Ok(value)
}

fn decode_input_snapshot(values: &[f64]) -> Result<InputSnapshot, String> {
    if values.len() != INPUT_SNAPSHOT_LENGTH {
        return Err(format!(
            "input snapshot 길이는 {INPUT_SNAPSHOT_LENGTH}이어야 하지만 {}입니다",
            values.len()
        ));
    }
    InputSnapshot::new(
        [
            input_u32(values[0], "held_bits")?,
            input_u32(values[1], "pressed_bits")?,
            input_u32(values[2], "released_bits")?,
        ],
        Vec2::new(
            input_f32(values[3], "pointer_dx")?,
            input_f32(values[4], "pointer_dy")?,
        ),
        input_f32(values[5], "wheel_delta")?,
        input_u32(values[6], "pointer_buttons")?,
        input_u32(values[7], "flags")?,
    )
    .map_err(|error| error.to_string())
}

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

    pub fn update_and_render(&mut self, dt_seconds: f32, packed_input: u32) -> bool {
        let input = InputSnapshot::new(
            [packed_input, 0, 0],
            renderer_core::math::Vec2::ZERO,
            0.0,
            0,
            0,
        );
        match input {
            Ok(input) => {
                self.core.update_and_render(dt_seconds, input);
                self.last_error.clear();
                true
            }
            Err(error) => {
                self.last_error = error.to_string();
                false
            }
        }
    }

    /// Layout: held, pressed, released, pointer dx, pointer dy, wheel, buttons, flags.
    pub fn update_and_render_input(
        &mut self,
        dt_seconds: f32,
        values: &[f64],
    ) -> Result<(), String> {
        let input = decode_input_snapshot(values)?;
        self.core.update_and_render(dt_seconds, input);
        Ok(())
    }

    pub fn set_camera_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => CameraMode::Orbit,
            1 => CameraMode::Fly,
            _ => return Err(format!("알 수 없는 camera mode입니다: {mode}")),
        };
        self.core
            .set_camera_mode(mode)
            .map_err(|error| error.to_string())
    }

    pub fn camera_mode(&self) -> u32 {
        match self.core.camera_mode() {
            CameraMode::Orbit => 0,
            CameraMode::Fly => 1,
        }
    }

    pub fn camera_eye_x(&self) -> f32 {
        self.core.camera_pose().eye.x
    }

    pub fn camera_eye_y(&self) -> f32 {
        self.core.camera_pose().eye.y
    }

    pub fn camera_eye_z(&self) -> f32 {
        self.core.camera_pose().eye.z
    }

    pub fn camera_forward_x(&self) -> f32 {
        self.core.camera_pose().forward.x
    }

    pub fn camera_forward_y(&self) -> f32 {
        self.core.camera_pose().forward.y
    }

    pub fn camera_forward_z(&self) -> f32 {
        self.core.camera_pose().forward.z
    }

    pub fn camera_yaw(&self) -> f32 {
        self.core.camera_yaw()
    }

    pub fn camera_pitch(&self) -> f32 {
        self.core.camera_pitch()
    }

    pub fn camera_orbit_radius(&self) -> f32 {
        self.core.camera_orbit_radius()
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
            7 => PipelineDebugMode::Normal,
            8 => PipelineDebugMode::NdotL,
            9 => PipelineDebugMode::Diffuse,
            10 => PipelineDebugMode::Specular,
            11 => PipelineDebugMode::ColorSpaceComparison,
            12 => PipelineDebugMode::Uv,
            13 => PipelineDebugMode::Overdraw,
            _ => return Err(format!("알 수 없는 pipeline debug mode입니다: {mode}")),
        };
        self.core.set_pipeline_debug_mode(mode);
        Ok(())
    }

    pub fn set_model_rotation_y(&mut self, rotation_y_radians: f32) {
        self.core.set_model_rotation_y(rotation_y_radians);
    }

    pub fn load_obj(&mut self, bytes: &[u8]) -> Result<u32, String> {
        match self.core.load_obj(bytes) {
            Ok(id) => {
                self.last_error.clear();
                Ok(id.0)
            }
            Err(error) => {
                self.last_error = error.to_string();
                Err(self.last_error.clone())
            }
        }
    }

    pub fn active_mesh_id(&self) -> u32 {
        self.core.mesh_asset_status().active_mesh_id.0
    }

    pub fn mesh_source_positions(&self) -> u32 {
        self.core.mesh_asset_status().source_positions as u32
    }

    pub fn mesh_source_faces(&self) -> u32 {
        self.core.mesh_asset_status().source_faces as u32
    }

    pub fn mesh_internal_vertices(&self) -> u32 {
        self.core.mesh_asset_status().internal_vertices as u32
    }

    pub fn mesh_triangles(&self) -> u32 {
        self.core.mesh_asset_status().triangles as u32
    }

    pub fn mesh_upload_successes(&self) -> u32 {
        self.core.mesh_asset_status().successful_uploads
    }

    pub fn mesh_upload_failures(&self) -> u32 {
        self.core.mesh_asset_status().failed_uploads
    }

    pub fn mesh_source_min_x(&self) -> f32 {
        self.core.mesh_asset_status().source_bounds.source_min.x
    }

    pub fn mesh_source_min_y(&self) -> f32 {
        self.core.mesh_asset_status().source_bounds.source_min.y
    }

    pub fn mesh_source_min_z(&self) -> f32 {
        self.core.mesh_asset_status().source_bounds.source_min.z
    }

    pub fn mesh_source_max_x(&self) -> f32 {
        self.core.mesh_asset_status().source_bounds.source_max.x
    }

    pub fn mesh_source_max_y(&self) -> f32 {
        self.core.mesh_asset_status().source_bounds.source_max.y
    }

    pub fn mesh_source_max_z(&self) -> f32 {
        self.core.mesh_asset_status().source_bounds.source_max.z
    }

    pub fn upload_texture_rgba(
        &mut self,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<u32, String> {
        match self.core.upload_texture_rgba8(
            width as usize,
            height as usize,
            pixels,
            TextureColorSpace::Srgb,
        ) {
            Ok(id) => {
                self.last_error.clear();
                Ok(id.0)
            }
            Err(error) => {
                self.last_error = error.to_string();
                Err(self.last_error.clone())
            }
        }
    }

    pub fn prepare_glb(&mut self, bytes: &[u8]) -> Result<u32, String> {
        match self.core.prepare_glb(bytes) {
            Ok(id) => {
                self.last_error.clear();
                Ok(id)
            }
            Err(error) => {
                self.last_error = error.to_string();
                Err(self.last_error.clone())
            }
        }
    }

    pub fn pending_glb_image_count(&self, id: u32) -> Result<u32, String> {
        self.core
            .pending_glb_image_count(id)
            .map(|count| count as u32)
            .map_err(|error| error.to_string())
    }

    pub fn pending_glb_image_mime(&self, id: u32, image_index: u32) -> Result<String, String> {
        self.core
            .pending_glb_image_mime(id, image_index as usize)
            .map(str::to_owned)
            .map_err(|error| error.to_string())
    }

    pub fn pending_glb_image_bytes(&self, id: u32, image_index: u32) -> Result<Vec<u8>, String> {
        self.core
            .pending_glb_image_bytes(id, image_index as usize)
            .map(<[u8]>::to_vec)
            .map_err(|error| error.to_string())
    }

    pub fn supply_glb_image_rgba(
        &mut self,
        id: u32,
        image_index: u32,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<(), String> {
        self.core
            .supply_glb_image_rgba8(
                id,
                image_index as usize,
                width as usize,
                height as usize,
                pixels,
            )
            .map_err(|error| error.to_string())
    }

    pub fn commit_glb(&mut self, id: u32) -> Result<(), String> {
        match self.core.commit_glb(id) {
            Ok(()) => {
                self.last_error.clear();
                Ok(())
            }
            Err(error) => {
                self.last_error = error.to_string();
                Err(self.last_error.clone())
            }
        }
    }

    pub fn cancel_glb(&mut self, id: u32) -> Result<(), String> {
        self.core.cancel_glb(id).map_err(|error| error.to_string())
    }

    pub fn fail_glb(&mut self, id: u32, reason: &str) -> Result<(), String> {
        self.core
            .fail_glb(id, reason)
            .map_err(|error| error.to_string())
    }

    pub fn glb_active(&self) -> bool {
        self.core.glb_asset_status().active
    }
    pub fn glb_pending_id(&self) -> u32 {
        self.core.glb_asset_status().pending_id.unwrap_or(0)
    }
    pub fn glb_upload_successes(&self) -> u32 {
        self.core.glb_asset_status().successful_uploads
    }
    pub fn glb_upload_failures(&self) -> u32 {
        self.core.glb_asset_status().failed_uploads
    }
    pub fn glb_last_failure(&self) -> String {
        self.core
            .glb_asset_status()
            .last_failure
            .unwrap_or_default()
            .to_owned()
    }
    pub fn glb_runtime_error(&self) -> String {
        self.core
            .glb_asset_status()
            .runtime_error
            .unwrap_or_default()
            .to_owned()
    }
    pub fn glb_draw_items(&self) -> u32 {
        self.core
            .glb_asset_status()
            .scene
            .map_or(0, |stats| stats.draw_items as u32)
    }
    pub fn glb_nodes(&self) -> u32 {
        self.core
            .glb_asset_status()
            .scene
            .map_or(0, |stats| stats.nodes as u32)
    }
    pub fn glb_skins(&self) -> u32 {
        self.core
            .glb_asset_status()
            .scene
            .map_or(0, |stats| stats.skins as u32)
    }
    pub fn glb_joints(&self) -> u32 {
        self.core
            .glb_asset_status()
            .scene
            .map_or(0, |stats| stats.joints as u32)
    }
    pub fn glb_vertices(&self) -> u32 {
        self.core
            .glb_asset_status()
            .scene
            .map_or(0, |stats| stats.vertices as u32)
    }
    pub fn glb_triangles(&self) -> u32 {
        self.core
            .glb_asset_status()
            .scene
            .map_or(0, |stats| stats.triangles as u32)
    }
    pub fn glb_sampler_downgrades(&self) -> u32 {
        self.core
            .glb_asset_status()
            .scene
            .map_or(0, |stats| stats.sampler_downgrades as u32)
    }

    pub fn glb_clip_count(&self) -> u32 {
        self.core.glb_clip_count() as u32
    }
    pub fn glb_clip_name(&self, index: u32) -> String {
        self.core
            .glb_clip_name(index as usize)
            .unwrap_or("")
            .to_owned()
    }
    pub fn glb_selected_clip(&self) -> u32 {
        self.core
            .glb_selected_clip()
            .map_or(u32::MAX, |index| index as u32)
    }
    pub fn set_glb_clip(&mut self, index: u32) -> Result<(), String> {
        self.core
            .set_glb_clip(index as usize)
            .map_err(|error| error.to_string())
    }
    pub fn glb_animation_time(&self) -> f32 {
        self.core.glb_animation_time()
    }
    pub fn glb_animation_duration(&self) -> f32 {
        self.core.glb_animation_duration()
    }
    pub fn glb_animation_playing(&self) -> bool {
        self.core.glb_animation_playing()
    }
    pub fn glb_animation_looping(&self) -> bool {
        self.core.glb_animation_looping()
    }
    pub fn set_glb_animation_playing(&mut self, playing: bool) -> Result<(), String> {
        self.core
            .set_glb_animation_playing(playing)
            .map_err(|error| error.to_string())
    }
    pub fn set_glb_animation_looping(&mut self, looping: bool) -> Result<(), String> {
        self.core
            .set_glb_animation_looping(looping)
            .map_err(|error| error.to_string())
    }
    pub fn seek_glb_animation(&mut self, time_seconds: f32) -> Result<(), String> {
        self.core
            .seek_glb_animation(time_seconds)
            .map_err(|error| error.to_string())
    }

    pub fn set_active_texture(&mut self, id: u32) -> Result<(), String> {
        self.core
            .set_active_texture(TextureId(id))
            .map_err(|error| error.to_string())
    }

    pub fn set_texture_debug_enabled(&mut self, enabled: bool) {
        self.core.set_texture_debug_enabled(enabled);
    }

    pub fn texture_debug_enabled(&self) -> bool {
        self.core.texture_debug_enabled()
    }

    pub fn texture_mip_levels(&self) -> u32 {
        self.core.texture_asset_status().mip_levels as u32
    }

    pub fn set_mipmap_enabled(&mut self, enabled: bool) {
        self.core.set_mipmap_enabled(enabled);
    }

    pub fn mipmap_enabled(&self) -> bool {
        self.core.mipmap_enabled()
    }

    pub fn set_mip_debug_enabled(&mut self, enabled: bool) {
        self.core.set_mip_debug_enabled(enabled);
    }

    pub fn mip_debug_enabled(&self) -> bool {
        self.core.mip_debug_enabled()
    }

    pub fn set_texture_sampling_enabled(&mut self, enabled: bool) {
        self.core.set_texture_sampling_enabled(enabled);
    }

    pub fn texture_sampling_enabled(&self) -> bool {
        self.core.texture_sampling_enabled()
    }

    pub fn set_sampler_state(
        &mut self,
        filter: u32,
        address_u: u32,
        address_v: u32,
    ) -> Result<(), String> {
        let filter = match filter {
            0 => FilterMode::Nearest,
            1 => FilterMode::Bilinear,
            _ => return Err(format!("알 수 없는 texture filter mode입니다: {filter}")),
        };
        let address = |value| match value {
            0 => Ok(AddressMode::Repeat),
            1 => Ok(AddressMode::ClampToEdge),
            2 => Ok(AddressMode::MirroredRepeat),
            _ => Err(format!("알 수 없는 texture address mode입니다: {value}")),
        };
        self.core.set_sampler_state(SamplerState {
            filter,
            address_u: address(address_u)?,
            address_v: address(address_v)?,
        });
        Ok(())
    }

    pub fn sampler_filter_mode(&self) -> u32 {
        match self.core.sampler_state().filter {
            FilterMode::Nearest => 0,
            FilterMode::Bilinear => 1,
        }
    }

    pub fn sampler_address_u(&self) -> u32 {
        match self.core.sampler_state().address_u {
            AddressMode::Repeat => 0,
            AddressMode::ClampToEdge => 1,
            AddressMode::MirroredRepeat => 2,
        }
    }

    pub fn sampler_address_v(&self) -> u32 {
        match self.core.sampler_state().address_v {
            AddressMode::Repeat => 0,
            AddressMode::ClampToEdge => 1,
            AddressMode::MirroredRepeat => 2,
        }
    }

    pub fn set_lighting_enabled(&mut self, enabled: bool) {
        self.core.set_lighting_enabled(enabled);
    }

    pub fn lighting_enabled(&self) -> bool {
        self.core.lighting_enabled()
    }

    pub fn set_shader_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => ShaderMode::Unlit,
            1 => ShaderMode::Lambert,
            2 => ShaderMode::BlinnPhong,
            _ => return Err(format!("알 수 없는 shader mode입니다: {mode}")),
        };
        self.core.set_shader_mode(mode);
        Ok(())
    }

    pub fn shader_mode(&self) -> u32 {
        match self.core.shader_mode() {
            ShaderMode::Unlit => 0,
            ShaderMode::Lambert => 1,
            ShaderMode::BlinnPhong => 2,
        }
    }

    pub fn set_material_specular(
        &mut self,
        red: f32,
        green: f32,
        blue: f32,
        shininess: f32,
    ) -> Result<(), String> {
        self.core
            .set_material_specular(Vec3::new(red, green, blue), shininess)
            .map_err(|error| error.to_string())
    }

    pub fn material_specular_red(&self) -> f32 {
        self.core.material_specular().0.x
    }

    pub fn material_specular_green(&self) -> f32 {
        self.core.material_specular().0.y
    }

    pub fn material_specular_blue(&self) -> f32 {
        self.core.material_specular().0.z
    }

    pub fn material_shininess(&self) -> f32 {
        self.core.material_specular().1
    }

    pub fn set_normal_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => NormalMode::Smooth,
            1 => NormalMode::Flat,
            _ => return Err(format!("알 수 없는 normal mode입니다: {mode}")),
        };
        self.core.set_normal_mode(mode);
        Ok(())
    }

    pub fn normal_mode(&self) -> u32 {
        match self.core.normal_mode() {
            NormalMode::Smooth => 0,
            NormalMode::Flat => 1,
        }
    }

    pub fn set_alpha_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => AlphaMode::Opaque,
            1 => AlphaMode::Mask,
            2 => AlphaMode::Blend,
            _ => return Err(format!("알 수 없는 alpha mode입니다: {mode}")),
        };
        self.core.set_alpha_mode(mode);
        Ok(())
    }

    pub fn alpha_mode(&self) -> u32 {
        match self.core.alpha_mode() {
            AlphaMode::Opaque => 0,
            AlphaMode::Mask => 1,
            AlphaMode::Blend => 2,
        }
    }

    pub fn set_alpha_cutoff(&mut self, cutoff: f32) -> Result<(), String> {
        self.core
            .set_alpha_cutoff(cutoff)
            .map_err(|error| error.to_string())
    }

    pub fn alpha_cutoff(&self) -> f32 {
        self.core.alpha_cutoff()
    }

    pub fn set_transparency_debug_enabled(&mut self, enabled: bool) {
        self.core.set_transparency_debug_enabled(enabled);
    }

    pub fn transparency_debug_enabled(&self) -> bool {
        self.core.transparency_debug_enabled()
    }

    pub fn set_transparent_sort_enabled(&mut self, enabled: bool) {
        self.core.set_transparent_sort_enabled(enabled);
    }

    pub fn transparent_sort_enabled(&self) -> bool {
        self.core.transparent_sort_enabled()
    }

    pub fn set_blend_color_space(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => BlendColorSpace::Linear,
            1 => BlendColorSpace::EncodedWrongWay,
            _ => return Err(format!("알 수 없는 blend color space입니다: {mode}")),
        };
        self.core.set_blend_color_space(mode);
        Ok(())
    }

    pub fn blend_color_space(&self) -> u32 {
        match self.core.blend_color_space() {
            BlendColorSpace::Linear => 0,
            BlendColorSpace::EncodedWrongWay => 1,
        }
    }

    pub fn set_directional_light(
        &mut self,
        surface_to_light_x: f32,
        surface_to_light_y: f32,
        surface_to_light_z: f32,
        intensity: f32,
    ) -> Result<(), String> {
        self.core
            .set_directional_light(
                renderer_core::math::Vec3::new(
                    surface_to_light_x,
                    surface_to_light_y,
                    surface_to_light_z,
                ),
                intensity,
            )
            .map_err(|error| error.to_string())
    }

    pub fn light_surface_to_light_x(&self) -> f32 {
        self.core.directional_light().surface_to_light.x
    }

    pub fn light_surface_to_light_y(&self) -> f32 {
        self.core.directional_light().surface_to_light.y
    }

    pub fn light_surface_to_light_z(&self) -> f32 {
        self.core.directional_light().surface_to_light.z
    }

    pub fn light_intensity(&self) -> f32 {
        self.core.directional_light().intensity
    }

    pub fn active_texture_id(&self) -> u32 {
        self.core.texture_asset_status().active_texture_id.0
    }

    pub fn active_texture_width(&self) -> u32 {
        self.core.texture_asset_status().active_width as u32
    }

    pub fn active_texture_height(&self) -> u32 {
        self.core.texture_asset_status().active_height as u32
    }

    pub fn texture_upload_successes(&self) -> u32 {
        self.core.texture_asset_status().successful_uploads
    }

    pub fn texture_upload_failures(&self) -> u32 {
        self.core.texture_asset_status().failed_uploads
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

    pub fn set_quality_mode(&mut self, mode: u32) -> Result<(), String> {
        let mode = match mode {
            0 => QualityMode::NoAa,
            1 => QualityMode::Ssaa2x,
            _ => return Err(format!("알 수 없는 quality mode입니다: {mode}")),
        };
        self.core
            .set_quality_mode(mode)
            .map_err(|error| error.to_string())
    }

    pub fn quality_mode(&self) -> u32 {
        match self.core.quality_mode() {
            QualityMode::NoAa => 0,
            QualityMode::Ssaa2x => 1,
        }
    }

    pub fn set_raster_path(&mut self, path: u32) -> Result<(), String> {
        let path = match path {
            0 => RasterPath::Scalar,
            1 => RasterPath::Tiled16,
            _ => return Err(format!("알 수 없는 raster path입니다: {path}")),
        };
        self.core.set_raster_path(path);
        Ok(())
    }

    pub fn raster_path(&self) -> u32 {
        match self.core.raster_path() {
            RasterPath::Scalar => 0,
            RasterPath::Tiled16 => 1,
        }
    }

    pub fn render_width(&self) -> u32 {
        self.core.render_dimensions_public().0 as u32
    }

    pub fn render_height(&self) -> u32 {
        self.core.render_dimensions_public().1 as u32
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

    pub fn stats_alpha_discarded_samples(&self) -> u32 {
        self.stats().alpha_discarded_samples
    }

    pub fn stats_depth_written_samples(&self) -> u32 {
        self.stats().depth_written_samples
    }

    pub fn stats_blended_samples(&self) -> u32 {
        self.stats().blended_samples
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

    pub fn stats_texture_debug_pixels(&self) -> u32 {
        self.stats().texture_debug_pixels
    }

    pub fn stats_texture_upload_successes(&self) -> u32 {
        self.stats().texture_upload_successes
    }

    pub fn stats_texture_upload_failures(&self) -> u32 {
        self.stats().texture_upload_failures
    }

    pub fn stats_active_texture_id(&self) -> u32 {
        self.stats().active_texture_id
    }

    pub fn stats_texture_samples(&self) -> u32 {
        self.stats().texture_samples
    }

    pub fn stats_lighting_samples(&self) -> u32 {
        self.stats().lighting_samples
    }

    pub fn stats_render_scale(&self) -> u32 {
        self.stats().render_scale
    }

    pub fn stats_resolved_pixels(&self) -> u32 {
        self.stats().resolved_pixels
    }

    pub fn stats_mip_samples(&self) -> u32 {
        self.stats().mip_samples
    }

    pub fn stats_min_mip_level(&self) -> u32 {
        self.stats().min_mip_level
    }

    pub fn stats_max_mip_level(&self) -> u32 {
        self.stats().max_mip_level
    }

    pub fn stats_invalid_lod_samples(&self) -> u32 {
        self.stats().invalid_lod_samples
    }

    pub fn stats_overdrawn_pixels(&self) -> u32 {
        self.stats().overdrawn_pixels
    }

    pub fn stats_max_overdraw(&self) -> u32 {
        self.stats().max_overdraw
    }

    pub fn stats_tiled_rasterized_triangles(&self) -> u32 {
        self.stats().tiled_rasterized_triangles
    }

    pub fn stats_tile_visits(&self) -> u32 {
        self.stats().tile_visits
    }

    pub fn stats_tile_counter_overflow(&self) -> bool {
        self.stats().tile_counter_overflow
    }

    pub fn stats_scene_draw_items(&self) -> u32 {
        self.stats().scene_draw_items
    }
    pub fn stats_animated_nodes(&self) -> u32 {
        self.stats().animated_nodes
    }
    pub fn stats_skinned_vertices(&self) -> u32 {
        self.stats().skinned_vertices
    }
    pub fn stats_joint_matrices(&self) -> u32 {
        self.stats().joint_matrices
    }
    pub fn stats_sampler_downgrades(&self) -> u32 {
        self.stats().sampler_downgrades
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
    let pipeline = if snapshot.glb_active {
        format!(
            "glTF RH 입력 → LH/+Z 변환 · GLB scene camera · fov {:.1}° · near {:.3} · far {:.1} · aspect {:.3}",
            snapshot.fov_y_radians.to_degrees(),
            snapshot.near,
            snapshot.far,
            snapshot.aspect
        )
    } else if snapshot.transparency_debug_enabled {
        format!(
            "opaque → cutout → transparent queue · identity M/V/P · viewport aspect {:.3}",
            snapshot.aspect
        )
    } else if snapshot.depth_debug_enabled {
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
    let scene_name = if snapshot.glb_active {
        "GLB first draw primitive"
    } else if snapshot.transparency_debug_enabled {
        "intersecting transparent quad mesh"
    } else if snapshot.depth_debug_enabled {
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
        "indexed mesh"
    };
    let scene_suffix = if snapshot.transparency_debug_enabled {
        " · primitive view +Z sort limitation fixture"
    } else if snapshot.depth_debug_enabled {
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
         pipeline state debug {} · strict depth test/write · material {} · alpha policy별 depth write\n\
         triangle stats input {} · submitted {} · culled {} · degenerate {} · invalid {}\n\
         clip stats fully clipped {} · clip invalid {} · generated {} · max polygon vertices {}\n\
         coverage stats rasterized {} · covered {} · shaded samples {} · counter overflow {} · S=256 pixel center/top-left\n\
         raster path {} · tiled triangles {} · tile visits {} · overflow {} · disjoint 16x16 writes\n\
         quality stats render scale {} · resolved {} · mip samples {} · level {}..{} · invalid LOD {}\n\
         diagnostic stats overdrawn pixels {} · max overdraw {}\n\
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
        snapshot.raster_path.label(),
        snapshot.frame_stats.tiled_rasterized_triangles,
        snapshot.frame_stats.tile_visits,
        snapshot.frame_stats.tile_counter_overflow,
        snapshot.frame_stats.render_scale,
        snapshot.frame_stats.resolved_pixels,
        snapshot.frame_stats.mip_samples,
        snapshot.frame_stats.min_mip_level,
        snapshot.frame_stats.max_mip_level,
        snapshot.frame_stats.invalid_lod_samples,
        snapshot.frame_stats.overdrawn_pixels,
        snapshot.frame_stats.max_overdraw,
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

    fn adapter_glb_fixture() -> Vec<u8> {
        let mut binary = Vec::new();
        for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            binary.extend_from_slice(&value.to_le_bytes());
        }
        let indices_offset = binary.len();
        for value in [0u16, 1, 2] {
            binary.extend_from_slice(&value.to_le_bytes());
        }
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        let image_offset = binary.len();
        binary.extend_from_slice(&[0x89, b'P', b'N', b'G']);
        let times_offset = binary.len();
        for value in [0.0f32, 1.0] {
            binary.extend_from_slice(&value.to_le_bytes());
        }
        let translations_offset = binary.len();
        for value in [0.0f32, 0.0, 0.0, 0.0, 0.25, 0.0] {
            binary.extend_from_slice(&value.to_le_bytes());
        }
        binary.push(0);
        let byte_length = binary.len();
        let json = format!(
            r#"{{
          "asset":{{"version":"2.0"}},"scene":0,
          "scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],
          "buffers":[{{"byteLength":{byte_length}}}],
          "bufferViews":[
            {{"buffer":0,"byteOffset":0,"byteLength":36}},
            {{"buffer":0,"byteOffset":{indices_offset},"byteLength":6}},
            {{"buffer":0,"byteOffset":{image_offset},"byteLength":4}},
            {{"buffer":0,"byteOffset":{times_offset},"byteLength":8}},
            {{"buffer":0,"byteOffset":{translations_offset},"byteLength":24}}
          ],
          "accessors":[
            {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
            {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}},
            {{"bufferView":3,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]}},
            {{"bufferView":4,"componentType":5126,"count":2,"type":"VEC3"}}
          ],
          "images":[{{"bufferView":2,"mimeType":"image/png"}}],
          "textures":[{{"source":0}}],
          "materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],
          "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1,"material":0}}]}}],
          "animations":[{{"name":"Move","samplers":[{{"input":2,"output":3}}],
            "channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}]}}]
        }}"#
        );
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + binary.len();
        let mut output = Vec::with_capacity(total);
        output.extend_from_slice(b"glTF");
        output.extend_from_slice(&2u32.to_le_bytes());
        output.extend_from_slice(&(total as u32).to_le_bytes());
        output.extend_from_slice(&(json.len() as u32).to_le_bytes());
        output.extend_from_slice(&0x4e4f534au32.to_le_bytes());
        output.extend_from_slice(&json);
        output.extend_from_slice(&(binary.len() as u32).to_le_bytes());
        output.extend_from_slice(&0x004e4942u32.to_le_bytes());
        output.extend_from_slice(&binary);
        output
    }

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

        assert!(renderer.update_and_render(0.016, 0x25));
        assert_eq!(renderer.stats_frame_index(), 1);
        assert_eq!(renderer.stats_dt_seconds(), 0.016);
        assert_eq!(renderer.stats_input_bits(), 0x25);
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
    fn adapter_maps_all_pipeline_debug_modes_through_chapter_twenty_four() {
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
            "world normal RGB",
            "Lambert N dot L",
            "linear diffuse only",
            "Blinn-Phong specular only",
            "linear correct / encoded wrong-way",
            "perspective UV",
            "covered sample overdraw",
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
                .set_pipeline_debug_mode(14)
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
        assert!(text.contains("indexed mesh · vertices 24 · indices 36 · triangles 12"));
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

    #[test]
    fn adapter_uploads_owned_texture_and_exposes_chapter_sixteen_status() {
        let mut renderer = Renderer::new(4, 4).unwrap();
        assert_eq!(renderer.active_texture_id(), 0);
        assert_eq!(
            (
                renderer.active_texture_width(),
                renderer.active_texture_height()
            ),
            (2, 2)
        );
        assert_eq!(
            (
                renderer.texture_upload_successes(),
                renderer.texture_upload_failures()
            ),
            (0, 0)
        );

        let mut source = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        assert_eq!(renderer.upload_texture_rgba(2, 2, &source).unwrap(), 1);
        source.fill(0);
        renderer.set_texture_debug_enabled(true);
        assert!(renderer.texture_debug_enabled());
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_texture_debug_pixels(), 16);
        assert_eq!(renderer.stats_texture_upload_successes(), 1);
        assert_eq!(renderer.stats_texture_upload_failures(), 0);
        assert_eq!(renderer.stats_active_texture_id(), 1);
        assert_eq!(&renderer.core.color_buffer()[0..4], &[255, 0, 0, 255]);

        let error = renderer.upload_texture_rgba(2, 2, &[0; 15]).unwrap_err();
        assert!(error.contains("16이어야"));
        assert_eq!(renderer.last_error(), error);
        assert_eq!(renderer.active_texture_id(), 1);
        assert_eq!(renderer.texture_upload_failures(), 1);
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_texture_upload_failures(), 1);

        assert!(renderer.set_active_texture(99).unwrap_err().contains("99"));
        renderer.set_active_texture(0).unwrap();
        assert_eq!(renderer.active_texture_id(), 0);
        renderer.set_texture_debug_enabled(false);
        assert!(!renderer.texture_debug_enabled());
    }

    #[test]
    fn adapter_maps_chapter_seventeen_sampler_state_and_stats() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer
            .upload_texture_rgba(
                2,
                2,
                &[
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
            )
            .unwrap();
        assert!(!renderer.texture_sampling_enabled());
        renderer.set_texture_sampling_enabled(true);
        assert!(renderer.texture_sampling_enabled());
        renderer.set_sampler_state(1, 1, 0).unwrap();
        assert_eq!(renderer.sampler_filter_mode(), 1);
        assert_eq!(renderer.sampler_address_u(), 1);
        assert_eq!(renderer.sampler_address_v(), 0);
        renderer.update_and_render(0.0, 0);
        assert!(renderer.stats_texture_samples() > 0);
        assert_eq!(
            renderer.stats_texture_samples(),
            renderer.stats_shaded_samples()
        );

        assert!(
            renderer
                .set_sampler_state(2, 0, 0)
                .unwrap_err()
                .contains("filter")
        );
        assert!(
            renderer
                .set_sampler_state(0, 3, 0)
                .unwrap_err()
                .contains("address")
        );
        assert!(
            renderer
                .set_sampler_state(0, 0, 3)
                .unwrap_err()
                .contains("address")
        );
        renderer.set_sampler_state(0, 2, 2).unwrap();
        assert_eq!(renderer.sampler_address_u(), 2);
        assert_eq!(renderer.sampler_address_v(), 2);
        renderer.set_sampler_state(0, 0, 1).unwrap();
        assert_eq!(renderer.sampler_filter_mode(), 0);
        assert_eq!(renderer.sampler_address_u(), 0);
        assert_eq!(renderer.sampler_address_v(), 1);
    }

    #[test]
    fn adapter_maps_chapter_eighteen_light_normal_modes_and_stats() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        assert!(!renderer.lighting_enabled());
        assert_eq!(renderer.normal_mode(), 0);
        renderer.set_normal_mode(0).unwrap();
        renderer.set_lighting_enabled(true);
        assert!(renderer.lighting_enabled());
        renderer.set_normal_mode(1).unwrap();
        assert_eq!(renderer.normal_mode(), 1);
        assert!(
            renderer
                .set_normal_mode(2)
                .unwrap_err()
                .contains("normal mode")
        );
        renderer
            .set_directional_light(0.0, 0.0, -2.0, 1.25)
            .unwrap();
        assert_eq!(renderer.light_surface_to_light_x(), 0.0);
        assert_eq!(renderer.light_surface_to_light_y(), 0.0);
        assert_eq!(renderer.light_surface_to_light_z(), -1.0);
        assert_eq!(renderer.light_intensity(), 1.25);
        renderer.update_and_render(0.0, 0);
        assert_eq!(
            renderer.stats_lighting_samples(),
            renderer.stats_shaded_samples()
        );
        assert!(
            renderer
                .set_directional_light(0.0, 0.0, 0.0, 1.0)
                .unwrap_err()
                .contains("surface_to_light")
        );
        assert!(
            renderer
                .set_directional_light(0.0, 0.0, -1.0, -1.0)
                .unwrap_err()
                .contains("intensity")
        );
    }

    #[test]
    fn adapter_maps_chapter_nineteen_shader_and_specular_material_atomically() {
        let mut renderer = Renderer::new(32, 32).unwrap();
        assert_eq!(renderer.shader_mode(), 0);
        renderer.set_shader_mode(1).unwrap();
        assert_eq!(renderer.shader_mode(), 1);
        renderer.set_shader_mode(2).unwrap();
        assert_eq!(renderer.shader_mode(), 2);
        renderer.set_shader_mode(0).unwrap();
        assert_eq!(renderer.shader_mode(), 0);
        assert!(
            renderer
                .set_shader_mode(3)
                .unwrap_err()
                .contains("shader mode")
        );

        renderer
            .set_material_specular(0.25, 0.5, 0.75, 48.0)
            .unwrap();
        assert_eq!(renderer.material_specular_red(), 0.25);
        assert_eq!(renderer.material_specular_green(), 0.5);
        assert_eq!(renderer.material_specular_blue(), 0.75);
        assert_eq!(renderer.material_shininess(), 48.0);
        assert!(
            renderer
                .set_material_specular(1.1, 0.5, 0.75, 48.0)
                .unwrap_err()
                .contains("specular color")
        );
        assert_eq!(renderer.material_specular_red(), 0.25);
        assert!(
            renderer
                .set_material_specular(0.25, 0.5, 0.75, f32::NAN)
                .unwrap_err()
                .contains("shininess")
        );
        assert_eq!(renderer.material_shininess(), 48.0);
    }

    #[test]
    fn adapter_validates_chapter_twenty_input_layout_and_maps_camera_state() {
        let valid = [1.0, 1.0, 0.0, 10.0, -2.0, 0.0, 1.0, 1.0];
        let decoded = decode_input_snapshot(&valid).unwrap();
        assert_eq!(decoded.packed_bits(), 1);
        assert_eq!(decoded.pressed_bits(), 1);

        for (values, message) in [
            (vec![0.0; 7], "길이"),
            (vec![f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], "정수"),
            (vec![0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], "정수"),
            (
                vec![u32::MAX as f64 + 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                "정수",
            ),
            (vec![0.0, 0.0, 0.0, f64::MAX, 0.0, 0.0, 0.0, 0.0], "f32"),
            (vec![64.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], "이동 키"),
        ] {
            assert!(
                decode_input_snapshot(&values)
                    .unwrap_err()
                    .contains(message)
            );
        }

        let mut renderer = Renderer::new(64, 64).unwrap();
        assert_eq!(renderer.camera_mode(), 0);
        assert_eq!(renderer.camera_eye_x(), 0.0);
        assert_eq!(renderer.camera_eye_y(), 0.0);
        assert_eq!(renderer.camera_eye_z(), -3.0);
        assert_eq!(renderer.camera_forward_x(), 0.0);
        assert_eq!(renderer.camera_forward_y(), 0.0);
        assert_eq!(renderer.camera_forward_z(), 1.0);
        assert_eq!(renderer.camera_yaw(), 0.0);
        assert_eq!(renderer.camera_pitch(), 0.0);
        assert_eq!(renderer.camera_orbit_radius(), 3.0);
        renderer.set_camera_mode(0).unwrap();
        renderer.set_camera_mode(1).unwrap();
        renderer
            .update_and_render_input(0.1, &[1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
            .unwrap();
        assert_eq!(renderer.camera_mode(), 1);
        assert!(renderer.camera_eye_z() > -3.0);
        assert!(
            renderer
                .set_camera_mode(2)
                .unwrap_err()
                .contains("camera mode")
        );
        assert!(
            renderer
                .update_and_render_input(0.0, &[0.0; 7])
                .unwrap_err()
                .contains("길이")
        );
        let frame_index = renderer.stats_frame_index();
        assert!(!renderer.update_and_render(0.0, 1 << 31));
        assert!(renderer.last_error().contains("이동 키"));
        assert_eq!(renderer.stats_frame_index(), frame_index);
        assert!(renderer.update_and_render(0.0, 0));
        assert_eq!(renderer.last_error(), "");
    }

    #[test]
    fn adapter_loads_chapter_twenty_one_obj_and_preserves_it_on_failure() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        assert_eq!(renderer.active_mesh_id(), 0);
        assert_eq!(renderer.mesh_source_positions(), 24);
        assert_eq!(renderer.mesh_source_faces(), 12);
        assert_eq!(renderer.mesh_internal_vertices(), 24);
        assert_eq!(renderer.mesh_triangles(), 12);
        assert_eq!(renderer.mesh_upload_successes(), 0);
        assert_eq!(renderer.mesh_upload_failures(), 0);

        let id = renderer
            .load_obj(b"v -2 -1 4\nv 2 -1 4\nv 0 3 4\nf 1 3 2\n")
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(renderer.active_mesh_id(), 1);
        assert_eq!(renderer.mesh_source_positions(), 3);
        assert_eq!(renderer.mesh_source_faces(), 1);
        assert_eq!(renderer.mesh_internal_vertices(), 3);
        assert_eq!(renderer.mesh_triangles(), 1);
        assert_eq!(renderer.mesh_upload_successes(), 1);
        assert_eq!(renderer.mesh_upload_failures(), 0);
        assert_eq!(renderer.mesh_source_min_x(), -2.0);
        assert_eq!(renderer.mesh_source_min_y(), -1.0);
        assert_eq!(renderer.mesh_source_min_z(), 4.0);
        assert_eq!(renderer.mesh_source_max_x(), 2.0);
        assert_eq!(renderer.mesh_source_max_y(), 3.0);
        assert_eq!(renderer.mesh_source_max_z(), 4.0);
        assert_eq!(renderer.last_error(), "");

        let error = renderer.load_obj(b"v 0 0 0\nf 1 2 3\n").unwrap_err();
        assert!(error.contains("범위"));
        assert_eq!(renderer.last_error(), error);
        assert_eq!(renderer.active_mesh_id(), 1);
        assert_eq!(renderer.mesh_upload_successes(), 1);
        assert_eq!(renderer.mesh_upload_failures(), 1);
    }

    #[test]
    fn adapter_maps_chapter_twenty_two_alpha_queue_and_blend_stats() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        assert_eq!(renderer.alpha_mode(), 0);
        renderer.set_alpha_mode(1).unwrap();
        assert_eq!(renderer.alpha_mode(), 1);
        renderer.set_alpha_mode(2).unwrap();
        assert_eq!(renderer.alpha_mode(), 2);
        renderer.set_alpha_mode(0).unwrap();
        assert_eq!(renderer.alpha_mode(), 0);
        assert!(
            renderer
                .set_alpha_mode(3)
                .unwrap_err()
                .contains("alpha mode")
        );

        renderer.set_alpha_cutoff(0.25).unwrap();
        assert_eq!(renderer.alpha_cutoff(), 0.25);
        assert!(
            renderer
                .set_alpha_cutoff(f32::NAN)
                .unwrap_err()
                .contains("0..1")
        );
        assert_eq!(renderer.alpha_cutoff(), 0.25);

        renderer.set_transparency_debug_enabled(true);
        assert!(renderer.transparency_debug_enabled());
        assert!(renderer.transparent_sort_enabled());
        assert_eq!(renderer.blend_color_space(), 0);
        renderer.update_and_render(0.0, 0);
        assert!(renderer.stats_alpha_discarded_samples() > 0);
        assert!(renderer.stats_depth_written_samples() > 0);
        assert!(renderer.stats_blended_samples() > 0);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("intersecting transparent quad mesh")
        );
        assert!(renderer.resize(32, 48));

        renderer.set_transparent_sort_enabled(false);
        assert!(!renderer.transparent_sort_enabled());
        renderer.set_blend_color_space(1).unwrap();
        assert_eq!(renderer.blend_color_space(), 1);
        renderer.set_blend_color_space(0).unwrap();
        assert_eq!(renderer.blend_color_space(), 0);
        assert!(
            renderer
                .set_blend_color_space(2)
                .unwrap_err()
                .contains("blend color space")
        );
        renderer.set_transparency_debug_enabled(false);
        assert!(!renderer.transparency_debug_enabled());
    }

    #[test]
    fn adapter_maps_chapter_twenty_three_quality_mipmap_and_stats() {
        let mut renderer = Renderer::new(32, 24).unwrap();
        assert_eq!(renderer.quality_mode(), 0);
        assert_eq!(
            (renderer.render_width(), renderer.render_height()),
            (32, 24)
        );
        renderer.set_quality_mode(1).unwrap();
        assert_eq!(renderer.quality_mode(), 1);
        assert_eq!(
            (renderer.render_width(), renderer.render_height()),
            (64, 48)
        );
        assert_eq!(renderer.stats_render_scale(), 2);
        assert_eq!(renderer.stats_resolved_pixels(), 32 * 24);
        assert_eq!(renderer.framebuffer_len(), 32 * 24 * 4);
        assert!(
            renderer
                .set_quality_mode(2)
                .unwrap_err()
                .contains("quality mode")
        );
        assert_eq!(renderer.quality_mode(), 1);
        assert!(renderer.resize(16, 20));
        assert_eq!(
            (renderer.render_width(), renderer.render_height()),
            (32, 40)
        );

        renderer
            .upload_texture_rgba(4, 4, &[128; 4 * 4 * 4])
            .unwrap();
        assert_eq!(renderer.texture_mip_levels(), 3);
        renderer.set_texture_sampling_enabled(true);
        renderer.set_mipmap_enabled(true);
        assert!(renderer.mipmap_enabled());
        renderer.set_mip_debug_enabled(true);
        assert!(renderer.mip_debug_enabled());
        assert!(renderer.mipmap_enabled());
        renderer.update_and_render(0.0, 0);
        assert!(renderer.stats_mip_samples() > 0);
        assert!(renderer.stats_min_mip_level() <= renderer.stats_max_mip_level());
        assert_eq!(renderer.stats_invalid_lod_samples(), 0);
        renderer.set_mipmap_enabled(false);
        assert!(!renderer.mipmap_enabled());
        assert!(!renderer.mip_debug_enabled());
        renderer.set_quality_mode(0).unwrap();
        assert_eq!(
            (renderer.render_width(), renderer.render_height()),
            (16, 20)
        );
    }

    #[test]
    fn adapter_maps_chapter_twenty_four_uv_overdraw_and_stats() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_pipeline_debug_mode(12).unwrap();
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_overdrawn_pixels(), 0);
        assert_eq!(renderer.stats_max_overdraw(), 0);

        renderer.set_pipeline_debug_mode(13).unwrap();
        renderer.update_and_render(0.0, 0);
        assert!(renderer.stats_overdrawn_pixels() > 0);
        assert!(renderer.stats_max_overdraw() >= 1);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("diagnostic stats overdrawn pixels")
        );
        assert!(
            renderer
                .set_pipeline_debug_mode(14)
                .unwrap_err()
                .contains("pipeline debug mode")
        );
    }

    #[test]
    fn adapter_maps_chapter_twenty_five_scalar_and_tiled_paths() {
        let mut renderer = Renderer::new(64, 48).unwrap();
        assert_eq!(renderer.raster_path(), 0);
        assert_eq!(renderer.stats_tiled_rasterized_triangles(), 0);
        assert_eq!(renderer.stats_tile_visits(), 0);
        renderer.set_raster_path(1).unwrap();
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.raster_path(), 1);
        assert_eq!(
            renderer.stats_tiled_rasterized_triangles(),
            renderer.stats_rasterized_triangles()
        );
        assert!(renderer.stats_tile_visits() > 0);
        assert!(!renderer.stats_tile_counter_overflow());
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("single-thread 16x16 tiled")
        );
        assert!(
            renderer
                .set_raster_path(2)
                .unwrap_err()
                .contains("raster path")
        );
        assert_eq!(renderer.raster_path(), 1);
        renderer.set_raster_path(0).unwrap();
        assert_eq!(renderer.raster_path(), 0);
    }

    #[test]
    fn adapter_stages_chapter_twenty_six_glb_images_scene_and_animation() {
        let mut renderer = Renderer::new(64, 48).unwrap();
        assert!(!renderer.glb_active());
        assert_eq!(renderer.glb_pending_id(), 0);
        assert!(
            renderer
                .prepare_glb(b"broken")
                .unwrap_err()
                .contains("header")
        );
        assert_eq!(renderer.glb_upload_failures(), 1);

        let pending = renderer.prepare_glb(&adapter_glb_fixture()).unwrap();
        assert_eq!(renderer.glb_pending_id(), pending);
        assert_eq!(renderer.pending_glb_image_count(pending), Ok(1));
        assert_eq!(
            renderer.pending_glb_image_mime(pending, 0),
            Ok("image/png".into())
        );
        assert_eq!(
            renderer.pending_glb_image_bytes(pending, 0),
            Ok(vec![0x89, b'P', b'N', b'G'])
        );
        assert!(
            renderer
                .pending_glb_image_count(pending + 1)
                .unwrap_err()
                .contains("stale")
        );
        assert!(renderer.commit_glb(pending).unwrap_err().contains("decode"));
        renderer
            .supply_glb_image_rgba(pending, 0, 1, 1, &[220, 140, 70, 255])
            .unwrap();
        renderer.commit_glb(pending).unwrap();
        assert!(renderer.glb_active());
        assert_eq!(renderer.glb_pending_id(), 0);
        assert_eq!(renderer.glb_upload_successes(), 1);
        assert_eq!(renderer.glb_last_failure(), "");
        assert_eq!(renderer.glb_runtime_error(), "");
        assert_eq!(renderer.glb_draw_items(), 1);
        assert_eq!(renderer.glb_nodes(), 1);
        assert_eq!(renderer.glb_skins(), 0);
        assert_eq!(renderer.glb_joints(), 0);
        assert_eq!(renderer.glb_vertices(), 3);
        assert_eq!(renderer.glb_triangles(), 1);
        assert_eq!(renderer.glb_sampler_downgrades(), 0);
        assert_eq!(renderer.glb_clip_count(), 1);
        assert_eq!(renderer.glb_clip_name(0), "Move");
        assert_eq!(renderer.glb_clip_name(9), "");
        assert_eq!(renderer.glb_selected_clip(), 0);
        renderer.set_glb_clip(0).unwrap();
        renderer.set_glb_animation_looping(false).unwrap();
        renderer.seek_glb_animation(0.75).unwrap();
        assert_eq!(renderer.glb_animation_time(), 0.75);
        assert_eq!(renderer.glb_animation_duration(), 1.0);
        assert!(renderer.glb_animation_playing());
        assert!(!renderer.glb_animation_looping());
        renderer.set_glb_animation_playing(false).unwrap();
        assert!(!renderer.glb_animation_playing());
        renderer.update_and_render(0.0, 0);
        assert_eq!(renderer.stats_scene_draw_items(), 1);
        assert_eq!(renderer.stats_animated_nodes(), 1);
        assert_eq!(renderer.stats_skinned_vertices(), 0);
        assert_eq!(renderer.stats_joint_matrices(), 0);
        assert_eq!(renderer.stats_sampler_downgrades(), 0);
        assert!(
            renderer
                .coordinate_debug_text()
                .contains("GLB first draw primitive · vertices 3 · indices 3 · triangles 1")
        );

        let cancelled = renderer.prepare_glb(&adapter_glb_fixture()).unwrap();
        renderer.cancel_glb(cancelled).unwrap();
        assert!(
            renderer
                .cancel_glb(cancelled)
                .unwrap_err()
                .contains("stale")
        );
        let failed = renderer.prepare_glb(&adapter_glb_fixture()).unwrap();
        renderer.fail_glb(failed, "image decode failed").unwrap();
        assert_eq!(renderer.glb_upload_failures(), 2);
        assert_eq!(renderer.glb_last_failure(), "image decode failed");
        assert!(renderer.fail_glb(failed, "stale").is_err());
    }
}
