//! Browser APIs에 의존하지 않는 소프트웨어 래스터라이저의 순수 Rust 코어.
//!
//! 23장까지 homogeneous clipping 뒤 scalar mesh pipeline, linear texture/mipmap sampling,
//! Blinn-Phong 조명, OBJ, alpha queue와 2x SSAA resolve를 조립한다.

pub mod camera;
pub mod camera_control;
pub mod clip;
pub mod color;
pub mod import;
pub mod math;
pub mod mesh;
pub mod raster;
pub mod texture;
pub mod transform;

use std::error::Error;
use std::fmt::{Display, Formatter};

use camera::{
    NdcPosition, ViewportPosition, look_at_lh, perspective_divide, perspective_lh_zo, viewport,
};
use camera_control::{CameraControlInput, CameraController, CameraMode};
use clip::{ClipPlane, ClipStatus, TriangleClipper};
use color::{srgb_decode_channel, srgb_decode_rgba, srgb_encode_rgba};
use import::{MeshBounds, ObjImportError, import_obj};
use math::{Mat4, Vec2, Vec3, Vec4};
use mesh::{ClipVertex, DrawItem, MaterialId, Mesh, MeshId, unit_cube_mesh};
use raster::{
    AttributeInterpolationMode, CullMode, DepthDebugMode, FaceOrientation, FragmentInput,
    PipelineDebugMode, ScreenVertex, TriangleDisposition, TriangleSetup, TriangleSetupError,
    WindingDebugMode, classify_triangle, normalized_channel_to_u8,
};
use texture::{
    AlphaMode, Material, NormalMode, SamplerState, ShaderMode, Texture, TextureColorSpace,
    TextureError, TextureId, TextureStore,
};
#[cfg(test)]
use transform::CoordinateSpace;
use transform::{
    ClipPlaneDistances, CoordinateDiagnostics, ObjectPosition, Transform, TransformPipeline,
    VertexTrace,
};

/// 4096 × 4096 RGBA8와 깊이 버퍼까지만 허용한다.
pub const MAX_PIXEL_COUNT: usize = 16_777_216;
pub const DEPTH_RANGE_EPSILON: f32 = 1.0e-6;
const MAX_FRAME_DT_SECONDS: f32 = 0.1;
const MODEL_ANGULAR_SPEED_RADIANS: f32 = 0.75;
const CUBE_SELECTED_VERTEX_INDEX: usize = 6;
const CLIP_DEBUG_SELECTED_VERTEX_INDEX: usize = 2;
const COVERAGE_DEBUG_SELECTED_VERTEX_INDEX: usize = 0;
const INTERPOLATION_DEBUG_SELECTED_VERTEX_INDEX: usize = 0;
const PERSPECTIVE_DEBUG_SELECTED_VERTEX_INDEX: usize = 0;
const DEPTH_DEBUG_SELECTED_VERTEX_INDEX: usize = 0;
const DEPTH_DEBUG_BACKGROUND: Color = Color::rgb(12, 18, 28);
const CAMERA_EYE: Vec3 = Vec3::new(0.0, 0.0, -3.0);
const CAMERA_TARGET: Vec3 = Vec3::ZERO;
const CAMERA_WORLD_UP: Vec3 = Vec3::Y;
const CAMERA_FOV_Y_RADIANS: f32 = std::f32::consts::FRAC_PI_3;
const CAMERA_NEAR: f32 = 0.1;
const CAMERA_FAR: f32 = 100.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color {
    red: u8,
    green: u8,
    blue: u8,
}

impl Color {
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    pub const fn rgba(self) -> [u8; 4] {
        [self.red, self.green, self.blue, 255]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderQueue {
    Opaque,
    Cutout,
    Transparent,
}

impl AlphaMode {
    pub const fn render_queue(self) -> RenderQueue {
        match self {
            Self::Opaque => RenderQueue::Opaque,
            Self::Mask => RenderQueue::Cutout,
            Self::Blend => RenderQueue::Transparent,
        }
    }

    pub const fn writes_depth(self) -> bool {
        !matches!(self, Self::Blend)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlendColorSpace {
    Linear,
    EncodedWrongWay,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QualityMode {
    #[default]
    NoAa,
    Ssaa2x,
}

impl QualityMode {
    pub const fn render_scale(self) -> usize {
        match self {
            Self::NoAa => 1,
            Self::Ssaa2x => 2,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::NoAa => "no AA",
            Self::Ssaa2x => "2x SSAA",
        }
    }
}

/// Straight-alpha source-over를 linear RGB에서 계산한다. 교육용 framebuffer는
/// 항상 opaque이므로 반환 alpha도 1이다.
pub fn blend_source_over_linear(source: Vec4, destination: Vec4) -> Option<Vec4> {
    if ![
        source.x,
        source.y,
        source.z,
        source.w,
        destination.x,
        destination.y,
        destination.z,
    ]
    .into_iter()
    .all(f32::is_finite)
    {
        return None;
    }
    let alpha = source.w.clamp(0.0, 1.0);
    let inverse_alpha = 1.0 - alpha;
    Some(Vec4::new(
        source.x * alpha + destination.x * inverse_alpha,
        source.y * alpha + destination.y * inverse_alpha,
        source.z * alpha + destination.z * inverse_alpha,
        1.0,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

impl ScreenPoint {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepthTestResult {
    Passed,
    Failed,
    Invalid,
}

/// 렌더 타깃 생성 또는 크기 변경이 거부된 이유다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderTargetError {
    ZeroDimension,
    DimensionOverflow,
    PixelLimitExceeded { requested: usize, maximum: usize },
}

impl Display for RenderTargetError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroDimension => formatter.write_str("렌더 타깃 크기는 0보다 커야 합니다"),
            Self::DimensionOverflow => {
                formatter.write_str("렌더 타깃 크기 계산에서 정수 overflow가 발생했습니다")
            }
            Self::PixelLimitExceeded { requested, maximum } => write!(
                formatter,
                "렌더 타깃 픽셀 수 {requested}이 최대 허용치 {maximum}을 넘었습니다"
            ),
        }
    }
}

impl Error for RenderTargetError {}

/// 같은 픽셀 인덱스를 공유하는 RGBA8 색 버퍼와 f32 깊이 버퍼다.
#[derive(Debug)]
pub struct RenderTarget {
    width: usize,
    height: usize,
    color: Vec<u8>,
    depth: Vec<f32>,
}

impl RenderTarget {
    pub fn new(width: usize, height: usize) -> Result<Self, RenderTargetError> {
        let (pixel_count, color_byte_count) = checked_buffer_lengths(width, height)?;
        Ok(Self {
            width,
            height,
            color: vec![0; color_byte_count],
            depth: vec![f32::INFINITY; pixel_count],
        })
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub fn color(&self) -> &[u8] {
        &self.color
    }

    pub fn depth(&self) -> &[f32] {
        &self.depth
    }

    pub fn clear_color(&mut self, color: Color) {
        for pixel in self.color.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color.rgba());
        }
        self.depth.fill(f32::INFINITY);
    }

    pub fn resolve_ssaa_2x_from(&mut self, source: &Self) -> bool {
        if source.width != self.width.saturating_mul(2)
            || source.height != self.height.saturating_mul(2)
        {
            return false;
        }
        for y in 0..self.height {
            for x in 0..self.width {
                let mut linear = Vec4::ZERO;
                let mut resolved_depth = f32::INFINITY;
                for offset_y in 0..2 {
                    for offset_x in 0..2 {
                        let source_index = (2 * y + offset_y) * source.width + 2 * x + offset_x;
                        let byte_index = 4 * source_index;
                        linear = linear
                            + srgb_decode_rgba(Vec4::new(
                                f32::from(source.color[byte_index]) / 255.0,
                                f32::from(source.color[byte_index + 1]) / 255.0,
                                f32::from(source.color[byte_index + 2]) / 255.0,
                                1.0,
                            ));
                        resolved_depth = resolved_depth.min(source.depth[source_index]);
                    }
                }
                let encoded = srgb_encode_rgba(linear / 4.0);
                let destination_index = y * self.width + x;
                let byte_index = 4 * destination_index;
                self.color[byte_index..byte_index + 4].copy_from_slice(&[
                    normalized_channel_to_u8(encoded.x),
                    normalized_channel_to_u8(encoded.y),
                    normalized_channel_to_u8(encoded.z),
                    255,
                ]);
                self.depth[destination_index] = resolved_depth;
            }
        }
        true
    }

    fn pixel_index(&self, point: ScreenPoint) -> Option<usize> {
        let (Ok(x), Ok(y)) = (usize::try_from(point.x), usize::try_from(point.y)) else {
            return None;
        };
        (x < self.width && y < self.height).then_some(y * self.width + x)
    }

    fn normalized_depth_candidate(
        &self,
        point: ScreenPoint,
        candidate: f32,
    ) -> Option<(usize, f32)> {
        if !candidate.is_finite()
            || !(-DEPTH_RANGE_EPSILON..=1.0 + DEPTH_RANGE_EPSILON).contains(&candidate)
        {
            return None;
        }
        Some((self.pixel_index(point)?, candidate.clamp(0.0, 1.0)))
    }

    pub fn test_depth(&self, point: ScreenPoint, candidate: f32) -> DepthTestResult {
        let Some((pixel_index, candidate)) = self.normalized_depth_candidate(point, candidate)
        else {
            return DepthTestResult::Invalid;
        };
        if candidate < self.depth[pixel_index] {
            DepthTestResult::Passed
        } else {
            DepthTestResult::Failed
        }
    }

    fn commit_depth_and_color(&mut self, point: ScreenPoint, candidate: f32, color: Color) -> bool {
        let Some((pixel_index, candidate)) = self.normalized_depth_candidate(point, candidate)
        else {
            return false;
        };
        if candidate >= self.depth[pixel_index] {
            return false;
        }
        self.depth[pixel_index] = candidate;
        let byte_index = 4 * pixel_index;
        self.color[byte_index..byte_index + 4].copy_from_slice(&color.rgba());
        true
    }

    fn blend_color_without_depth(
        &mut self,
        point: ScreenPoint,
        source_linear: Vec4,
        color_space: BlendColorSpace,
    ) -> bool {
        let Some(pixel_index) = self.pixel_index(point) else {
            return false;
        };
        let alpha = source_linear.w.clamp(0.0, 1.0);
        if alpha == 0.0 {
            return true;
        }
        let byte_index = 4 * pixel_index;
        let destination_encoded = Vec4::new(
            f32::from(self.color[byte_index]) / 255.0,
            f32::from(self.color[byte_index + 1]) / 255.0,
            f32::from(self.color[byte_index + 2]) / 255.0,
            1.0,
        );
        let encoded = match color_space {
            BlendColorSpace::Linear => {
                let destination_linear = srgb_decode_rgba(destination_encoded);
                srgb_encode_rgba(
                    blend_source_over_linear(source_linear, destination_linear)
                        .expect("검증된 fragment와 RGBA8 destination은 유한해야 한다"),
                )
            }
            BlendColorSpace::EncodedWrongWay => {
                let source_encoded = srgb_encode_rgba(source_linear);
                blend_source_over_linear(source_encoded, destination_encoded)
                    .expect("검증된 encoded source와 destination은 유한해야 한다")
            }
        };
        self.color[byte_index..byte_index + 4].copy_from_slice(&[
            normalized_channel_to_u8(encoded.x),
            normalized_channel_to_u8(encoded.y),
            normalized_channel_to_u8(encoded.z),
            255,
        ]);
        true
    }

    pub fn put_pixel(&mut self, point: ScreenPoint, color: Color) -> bool {
        let Some(pixel_index) = self.pixel_index(point) else {
            return false;
        };
        let byte_index = 4 * pixel_index;
        self.color[byte_index..byte_index + 4].copy_from_slice(&color.rgba());
        true
    }

    pub fn render_gradient_checker(&mut self) {
        let x_denominator = self.width.saturating_sub(1).max(1);
        let y_denominator = self.height.saturating_sub(1).max(1);
        for y in 0..self.height {
            let green = ((255 * y + y_denominator / 2) / y_denominator) as u8;
            for x in 0..self.width {
                let red = ((255 * x + x_denominator / 2) / x_denominator) as u8;
                let blue = if (x / 8 + y / 8).is_multiple_of(2) {
                    220
                } else {
                    40
                };
                let byte_index = 4 * (y * self.width + x);
                self.color[byte_index..byte_index + 4].copy_from_slice(&[red, green, blue, 255]);
            }
        }
        self.depth.fill(f32::INFINITY);
    }

    /// Texture row 0을 화면 row 0에 두고 nearest로 전체 target에 확대한다.
    pub fn render_texture_nearest(&mut self, texture: &Texture) -> u32 {
        for y in 0..self.height {
            let texture_y = nearest_texture_coordinate(y, self.height, texture.height());
            for x in 0..self.width {
                let texture_x = nearest_texture_coordinate(x, self.width, texture.width());
                let source = texture
                    .texel_rgba8(texture_x, texture_y)
                    .expect("검증된 nearest 좌표는 texture 내부여야 한다");
                let byte_index = 4 * (y * self.width + x);
                self.color[byte_index..byte_index + 4]
                    .copy_from_slice(&[source[0], source[1], source[2], 255]);
            }
        }
        self.depth.fill(f32::INFINITY);
        u32::try_from(self.width * self.height)
            .expect("RenderTarget 최대 pixel 수는 u32 안에 들어가야 한다")
    }

    pub fn draw_point(&mut self, point: ScreenPoint, color: Color) -> u32 {
        u32::from(self.put_pixel(point, color))
    }

    pub fn draw_line_bresenham(
        &mut self,
        start: ScreenPoint,
        end: ScreenPoint,
        color: Color,
    ) -> u32 {
        walk_bresenham(start, end, |point| self.put_pixel(point, color))
    }

    pub fn draw_rect_outline(
        &mut self,
        first: ScreenPoint,
        second: ScreenPoint,
        color: Color,
    ) -> u32 {
        let left = first.x.min(second.x);
        let right = first.x.max(second.x);
        let top = first.y.min(second.y);
        let bottom = first.y.max(second.y);
        let corners = [
            ScreenPoint::new(left, top),
            ScreenPoint::new(right, top),
            ScreenPoint::new(right, bottom),
            ScreenPoint::new(left, bottom),
        ];
        let mut written = 0_u32;
        for edge in 0..4 {
            written = written.saturating_add(self.draw_line_bresenham(
                corners[edge],
                corners[(edge + 1) % 4],
                color,
            ));
        }
        written
    }

    pub fn draw_wireframe_triangle(
        &mut self,
        vertices: [ScreenPoint; 3],
        edge_colors: [Color; 3],
    ) -> u32 {
        let mut written = 0_u32;
        for edge in 0..3 {
            written = written.saturating_add(self.draw_line_bresenham(
                vertices[edge],
                vertices[(edge + 1) % 3],
                edge_colors[edge],
            ));
        }
        written
    }
}

fn nearest_texture_coordinate(
    target_coordinate: usize,
    target_extent: usize,
    texture_extent: usize,
) -> usize {
    let scaled = target_coordinate as u64 * texture_extent as u64;
    (scaled / target_extent as u64) as usize
}

fn walk_bresenham(
    start: ScreenPoint,
    end: ScreenPoint,
    mut visit: impl FnMut(ScreenPoint) -> bool,
) -> u32 {
    let (mut x0, mut y0) = (i64::from(start.x), i64::from(start.y));
    let (x1, y1) = (i64::from(end.x), i64::from(end.y));
    let dx = (x1 - x0).abs();
    let step_x = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let step_y = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    let mut written = 0_u32;

    loop {
        if visit(ScreenPoint::new(x0 as i32, y0 as i32)) {
            written = written.saturating_add(1);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled_error = 2 * error;
        if doubled_error >= dy {
            error += dy;
            x0 += step_x;
        }
        if doubled_error <= dx {
            error += dx;
            y0 += step_y;
        }
    }
    written
}

/// 한 프레임의 작은 값만 복사하는 통계 snapshot이다.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    pub frame_index: u32,
    pub dt_seconds: f32,
    pub input_bits: u32,
    pub input_vertices: u32,
    pub input_triangles: u32,
    pub transformed_vertices: u32,
    pub submitted_triangles: u32,
    pub culled_triangles: u32,
    pub degenerate_triangles: u32,
    pub invalid_triangles: u32,
    pub fully_clipped_triangles: u32,
    pub clip_invalid_triangles: u32,
    pub generated_triangles: u32,
    pub max_clip_polygon_vertices: u32,
    pub rasterized_triangles: u32,
    pub covered_samples: u32,
    pub shaded_samples: u32,
    pub depth_passed_samples: u32,
    pub depth_failed_samples: u32,
    pub invalid_depth_samples: u32,
    pub alpha_discarded_samples: u32,
    pub depth_written_samples: u32,
    pub blended_samples: u32,
    pub max_barycentric_sum_error: f32,
    pub interpolated_inv_w_samples: u32,
    pub invalid_interpolation_samples: u32,
    pub min_interpolated_inv_w: f32,
    pub max_interpolated_inv_w: f32,
    /// `u32` sample counter가 표현 범위를 넘었음을 나타낸다. 이 값이 참인
    /// 프레임의 단계별 수치는 잘린 값이므로 관계식 검증에 사용할 수 없다.
    pub sample_counter_overflow: bool,
    pub debug_pixels: u32,
    pub invalid_values: u32,
    pub texture_debug_pixels: u32,
    pub texture_upload_successes: u32,
    pub texture_upload_failures: u32,
    pub active_texture_id: u32,
    pub texture_samples: u32,
    pub lighting_samples: u32,
    pub render_scale: u32,
    pub resolved_pixels: u32,
    pub mip_samples: u32,
    pub min_mip_level: u32,
    pub max_mip_level: u32,
    pub invalid_lod_samples: u32,
}

impl FrameStats {
    /// 한 scalar frame이 15장의 단계별 분류와 제출 순서를 완전히 보존했는지 확인한다.
    pub const fn pipeline_relations_hold(self) -> bool {
        !self.sample_counter_overflow
            && self.generated_triangles
                == self
                    .submitted_triangles
                    .saturating_add(self.culled_triangles)
                    .saturating_add(self.degenerate_triangles)
                    .saturating_add(self.invalid_triangles)
            && self.rasterized_triangles == self.submitted_triangles
            && self.covered_samples
                == self
                    .depth_passed_samples
                    .saturating_add(self.depth_failed_samples)
                    .saturating_add(self.invalid_depth_samples)
                    .saturating_add(self.alpha_discarded_samples)
                    .saturating_add(self.invalid_interpolation_samples)
            && self.shaded_samples == self.depth_passed_samples
            && self.interpolated_inv_w_samples
                == self
                    .shaded_samples
                    .saturating_add(self.alpha_discarded_samples)
            && self.depth_passed_samples
                == self
                    .depth_written_samples
                    .saturating_add(self.blended_samples)
            && self.mip_samples <= self.interpolated_inv_w_samples
            && self.invalid_lod_samples <= self.mip_samples
            && ((self.mip_samples == 0 && self.min_mip_level == 0 && self.max_mip_level == 0)
                || (self.mip_samples > 0 && self.min_mip_level <= self.max_mip_level))
    }
}

const fn pipeline_stats_are_consistent_or_overflowed(stats: FrameStats) -> bool {
    stats.sample_counter_overflow || stats.pipeline_relations_hold()
}

/// 18장까지의 고정 scalar pipeline state다. Material은 각 `DrawItem`이 소유하고,
/// depth는 모든 debug mode에서 같은 strict-less test/write 계약을 사용한다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PipelineState {
    pub cull_mode: CullMode,
    pub attribute_interpolation_mode: AttributeInterpolationMode,
    pub debug_mode: PipelineDebugMode,
}

impl Default for PipelineState {
    fn default() -> Self {
        Self {
            cull_mode: CullMode::Back,
            attribute_interpolation_mode: AttributeInterpolationMode::PerspectiveCorrect,
            debug_mode: PipelineDebugMode::Solid,
        }
    }
}

/// DOM 이벤트 수와 무관하게 프레임당 한 번 전달하는 고정 입력 snapshot이다.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputSnapshot {
    held_bits: u32,
    pressed_bits: u32,
    released_bits: u32,
    pointer_dx: f32,
    pointer_dy: f32,
    wheel_delta: f32,
    pointer_buttons: u32,
    flags: u32,
}

pub const INPUT_FORWARD: u32 = 1 << 0;
pub const INPUT_BACKWARD: u32 = 1 << 1;
pub const INPUT_LEFT: u32 = 1 << 2;
pub const INPUT_RIGHT: u32 = 1 << 3;
pub const INPUT_UP: u32 = 1 << 4;
pub const INPUT_DOWN: u32 = 1 << 5;
pub const INPUT_KEY_MASK: u32 = (1 << 6) - 1;
pub const INPUT_FLAG_DRAGGING: u32 = 1 << 0;
pub const INPUT_MODIFIER_SHIFT: u32 = 1 << 1;
pub const INPUT_MODIFIER_CONTROL: u32 = 1 << 2;
pub const INPUT_MODIFIER_ALT: u32 = 1 << 3;
pub const INPUT_MODIFIER_META: u32 = 1 << 4;
pub const INPUT_FLAG_MASK: u32 = (1 << 5) - 1;
pub const INPUT_POINTER_BUTTON_MASK: u32 = (1 << 5) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputSnapshotError {
    UnsupportedKeyBits,
    InvalidPointerDelta,
    InvalidWheelDelta,
    UnsupportedPointerButtons,
    UnsupportedFlags,
}

impl Display for InputSnapshotError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedKeyBits => {
                formatter.write_str("input key bit에는 정의된 이동 키만 사용할 수 있습니다")
            }
            Self::InvalidPointerDelta => {
                formatter.write_str("pointer delta는 유한한 값이어야 합니다")
            }
            Self::InvalidWheelDelta => formatter.write_str("wheel delta는 유한한 값이어야 합니다"),
            Self::UnsupportedPointerButtons => {
                formatter.write_str("pointer button bit는 0..4만 사용할 수 있습니다")
            }
            Self::UnsupportedFlags => {
                formatter.write_str("input flag에는 dragging/modifier bit만 사용할 수 있습니다")
            }
        }
    }
}

impl Error for InputSnapshotError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterialAlphaError {
    InvalidCutoff,
}

impl Display for MaterialAlphaError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCutoff => {
                formatter.write_str("alpha cutoff은 유한한 0..1 값이어야 합니다")
            }
        }
    }
}

impl Error for MaterialAlphaError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextureAssetStatus {
    pub active_texture_id: TextureId,
    pub active_width: usize,
    pub active_height: usize,
    pub mip_levels: usize,
    pub successful_uploads: u32,
    pub failed_uploads: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshAssetStatus {
    pub active_mesh_id: MeshId,
    pub source_positions: usize,
    pub source_faces: usize,
    pub internal_vertices: usize,
    pub triangles: usize,
    pub successful_uploads: u32,
    pub failed_uploads: u32,
    pub source_bounds: MeshBounds,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalLight {
    pub surface_to_light: Vec3,
    pub color: Vec3,
    pub intensity: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            surface_to_light: Vec3::new(-0.4, 0.8, -0.45)
                .normalized()
                .expect("기본 surface_to_light는 0이 아니어야 한다"),
            color: Vec3::new(1.0, 0.96, 0.88),
            intensity: 0.9,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LightingError {
    InvalidDirection,
    InvalidColor,
    InvalidIntensity,
}

impl Display for LightingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDirection => {
                formatter.write_str("surface_to_light는 유한한 0이 아닌 방향이어야 합니다")
            }
            Self::InvalidColor => {
                formatter.write_str("directional light color는 유한한 음이 아닌 값이어야 합니다")
            }
            Self::InvalidIntensity => formatter
                .write_str("directional light intensity는 유한한 음이 아닌 값이어야 합니다"),
        }
    }
}

impl Error for LightingError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterialError {
    InvalidSpecularColor,
    InvalidShininess,
}

impl Display for MaterialError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSpecularColor => {
                formatter.write_str("specular color는 유한한 0..1 sRGB 값이어야 합니다")
            }
            Self::InvalidShininess => {
                formatter.write_str("shininess는 유한한 0보다 큰 값이어야 합니다")
            }
        }
    }
}

impl Error for MaterialError {}

impl DirectionalLight {
    pub fn new(surface_to_light: Vec3, color: Vec3, intensity: f32) -> Result<Self, LightingError> {
        let surface_to_light = surface_to_light
            .normalized()
            .ok_or(LightingError::InvalidDirection)?;
        if !color.x.is_finite()
            || !color.y.is_finite()
            || !color.z.is_finite()
            || color.x < 0.0
            || color.y < 0.0
            || color.z < 0.0
        {
            return Err(LightingError::InvalidColor);
        }
        if !intensity.is_finite() || intensity < 0.0 {
            return Err(LightingError::InvalidIntensity);
        }
        Ok(Self {
            surface_to_light,
            color,
            intensity,
        })
    }
}

impl InputSnapshot {
    pub fn new(
        key_bits: [u32; 3],
        pointer_delta: Vec2,
        wheel_delta: f32,
        pointer_buttons: u32,
        flags: u32,
    ) -> Result<Self, InputSnapshotError> {
        let [held_bits, pressed_bits, released_bits] = key_bits;
        if (held_bits | pressed_bits | released_bits) & !INPUT_KEY_MASK != 0 {
            return Err(InputSnapshotError::UnsupportedKeyBits);
        }
        if !pointer_delta.x.is_finite() || !pointer_delta.y.is_finite() {
            return Err(InputSnapshotError::InvalidPointerDelta);
        }
        if !wheel_delta.is_finite() {
            return Err(InputSnapshotError::InvalidWheelDelta);
        }
        if pointer_buttons & !INPUT_POINTER_BUTTON_MASK != 0 {
            return Err(InputSnapshotError::UnsupportedPointerButtons);
        }
        if flags & !INPUT_FLAG_MASK != 0 {
            return Err(InputSnapshotError::UnsupportedFlags);
        }
        Ok(Self {
            held_bits,
            pressed_bits,
            released_bits,
            pointer_dx: pointer_delta.x,
            pointer_dy: pointer_delta.y,
            wheel_delta,
            pointer_buttons,
            flags,
        })
    }

    pub const fn packed_bits(self) -> u32 {
        self.held_bits
    }

    pub const fn pressed_bits(self) -> u32 {
        self.pressed_bits
    }

    pub const fn released_bits(self) -> u32 {
        self.released_bits
    }

    pub const fn pointer_delta(self) -> Vec2 {
        Vec2::new(self.pointer_dx, self.pointer_dy)
    }

    pub const fn wheel_delta(self) -> f32 {
        self.wheel_delta
    }

    pub const fn pointer_buttons(self) -> u32 {
        self.pointer_buttons
    }

    pub const fn flags(self) -> u32 {
        self.flags
    }

    fn camera_control_input(self) -> CameraControlInput {
        let axis = |positive, negative| {
            let positive = if self.held_bits & positive != 0 {
                1.0
            } else {
                0.0
            };
            let negative = if self.held_bits & negative != 0 {
                1.0
            } else {
                0.0
            };
            positive - negative
        };
        CameraControlInput {
            move_right: axis(INPUT_RIGHT, INPUT_LEFT),
            move_up: axis(INPUT_UP, INPUT_DOWN),
            move_forward: axis(INPUT_FORWARD, INPUT_BACKWARD),
            pointer_dx: self.pointer_dx,
            pointer_dy: self.pointer_dy,
            wheel_delta: self.wheel_delta,
            dragging: self.flags & INPUT_FLAG_DRAGGING != 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct MeshScene {
    traces: Vec<VertexTrace>,
    clip_vertices: Vec<ClipVertex>,
    diagnostic_ndc_positions: Vec<Option<NdcPosition>>,
    diagnostic_viewport_positions: Vec<Option<ViewportPosition>>,
    diagnostics: CoordinateDiagnostics,
    projection_failures: u32,
    aspect: f32,
    camera_world: Vec3,
}

impl MeshScene {
    fn with_capacity(mesh: &Mesh) -> Self {
        let vertex_count = mesh.vertices().len();
        Self {
            traces: Vec::with_capacity(vertex_count),
            clip_vertices: Vec::with_capacity(vertex_count),
            diagnostic_ndc_positions: Vec::with_capacity(vertex_count),
            diagnostic_viewport_positions: Vec::with_capacity(vertex_count),
            diagnostics: CoordinateDiagnostics::from_traces(&[]),
            projection_failures: 0,
            aspect: 1.0,
            camera_world: Vec3::ZERO,
        }
    }

    fn new(mesh: &Mesh, model: Transform, width: usize, height: usize) -> Self {
        let mut scene = Self::with_capacity(mesh);
        scene.rebuild_cube(mesh, model, width, height);
        scene
    }

    fn new_identity_debug(mesh: &Mesh, width: usize, height: usize) -> Self {
        let mut scene = Self::with_capacity(mesh);
        scene.rebuild_identity_debug(mesh, width, height);
        scene
    }

    fn new_perspective_debug(mesh: &Mesh, width: usize, height: usize) -> Self {
        let mut scene = Self::with_capacity(mesh);
        scene.rebuild_perspective_debug(mesh, width, height);
        scene
    }

    fn rebuild_cube(&mut self, mesh: &Mesh, model: Transform, width: usize, height: usize) {
        self.rebuild_cube_with_camera(mesh, model, CAMERA_EYE, CAMERA_TARGET, width, height);
    }

    fn rebuild_cube_with_camera(
        &mut self,
        mesh: &Mesh,
        model: Transform,
        eye: Vec3,
        target: Vec3,
        width: usize,
        height: usize,
    ) {
        self.camera_world = eye;
        let aspect = width as f32 / height as f32;
        let view = look_at_lh(eye, target, CAMERA_WORLD_UP)
            .expect("cube camera view 계약은 항상 유효해야 한다");
        let projection = perspective_lh_zo(CAMERA_FOV_Y_RADIANS, aspect, CAMERA_NEAR, CAMERA_FAR)
            .expect("유효한 렌더 타깃의 고정 projection 계약은 항상 유효해야 한다");
        let pipeline = TransformPipeline::new(model.model_matrix(), view, projection);
        self.rebuild_with_pipeline(mesh, pipeline, width, height);
    }

    fn rebuild_identity_debug(&mut self, mesh: &Mesh, width: usize, height: usize) {
        self.camera_world = Vec3::ZERO;
        let identity = Mat4::identity();
        self.rebuild_with_pipeline(
            mesh,
            TransformPipeline::new(identity, identity, identity),
            width,
            height,
        );
    }

    fn rebuild_perspective_debug(&mut self, mesh: &Mesh, width: usize, height: usize) {
        self.camera_world = Vec3::ZERO;
        let aspect = width as f32 / height as f32;
        let identity = Mat4::identity();
        let projection = perspective_lh_zo(CAMERA_FOV_Y_RADIANS, aspect, CAMERA_NEAR, CAMERA_FAR)
            .expect("유효한 렌더 타깃의 perspective fixture projection은 항상 유효해야 한다");
        self.rebuild_with_pipeline(
            mesh,
            TransformPipeline::new(identity, identity, projection),
            width,
            height,
        );
    }

    fn rebuild_with_pipeline(
        &mut self,
        mesh: &Mesh,
        pipeline: TransformPipeline,
        width: usize,
        height: usize,
    ) {
        let aspect = width as f32 / height as f32;
        self.traces.clear();
        self.clip_vertices.clear();
        self.diagnostic_ndc_positions.clear();
        self.diagnostic_viewport_positions.clear();
        for vertex in mesh.vertices() {
            let trace = pipeline.trace(ObjectPosition(vertex.position_object));
            let normal_world = pipeline
                .transform_model_normal(vertex.normal_object)
                .unwrap_or(Vec3::ZERO);
            self.clip_vertices.push(ClipVertex {
                clip_pos: trace.clip_pos,
                view_depth: trace.view_pos.0.z,
                world_pos: Vec3::new(
                    trace.world_pos.0.x,
                    trace.world_pos.0.y,
                    trace.world_pos.0.z,
                ),
                normal_world,
                uv: vertex.uv,
                color: vertex.color,
            });
            // 이 값은 overlay 전용 source 진단이다. 실제 geometry는 이 캐시를 쓰지
            // 않고 여섯 평면 clipping을 마친 fan만 divide/viewport로 보낸다.
            let clip_position_is_finite = clip_position_is_finite(trace.clip_pos);
            let inside_clip_volume = clip_position_is_finite
                && ClipPlane::ALL
                    .into_iter()
                    .all(|plane| plane.distance(trace.clip_pos) >= 0.0);
            let ndc = inside_clip_volume
                .then(|| perspective_divide(trace.clip_pos).ok())
                .flatten();
            let projected =
                ndc.and_then(|position| viewport(position, width as f32, height as f32).ok());
            self.diagnostic_viewport_positions.push(projected);
            self.diagnostic_ndc_positions.push(ndc);
            self.traces.push(trace);
        }
        self.diagnostics = CoordinateDiagnostics::from_traces(&self.traces);
        self.projection_failures = self
            .traces
            .iter()
            .zip(&self.diagnostic_ndc_positions)
            .filter(|(trace, ndc)| {
                let finite = clip_position_is_finite(trace.clip_pos);
                let inside = finite
                    && ClipPlane::ALL
                        .into_iter()
                        .all(|plane| plane.distance(trace.clip_pos) >= 0.0);
                !finite || (inside && ndc.is_none())
            })
            .count() as u32;
        self.aspect = aspect;
    }
}

/// 개발 overlay가 한 번에 복사하는 선택 정점과 공간별 수치 snapshot이다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateDebugSnapshot {
    pub rotation_y_radians: f32,
    pub selected_vertex_index: usize,
    pub selected_vertex: VertexTrace,
    pub selected_attributes: ClipVertex,
    pub selected_ndc: Option<NdcPosition>,
    pub selected_viewport: Option<ViewportPosition>,
    pub clip_plane_distances: ClipPlaneDistances,
    pub diagnostics: CoordinateDiagnostics,
    pub projection_failures: u32,
    pub fov_y_radians: f32,
    pub near: f32,
    pub far: f32,
    pub aspect: f32,
    pub mesh_vertices: u32,
    pub mesh_indices: u32,
    pub mesh_triangles: u32,
    pub material_id: u32,
    pub pipeline_state: PipelineState,
    pub debug_lines_enabled: bool,
    pub cull_mode: CullMode,
    pub winding_debug_mode: WindingDebugMode,
    pub clip_debug_enabled: bool,
    pub coverage_debug_enabled: bool,
    pub interpolation_debug_enabled: bool,
    pub perspective_debug_enabled: bool,
    pub attribute_interpolation_mode: AttributeInterpolationMode,
    pub depth_debug_enabled: bool,
    pub depth_order_reversed: bool,
    pub depth_debug_mode: DepthDebugMode,
    pub transparency_debug_enabled: bool,
    pub frame_stats: FrameStats,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveScene {
    Cube,
    Clipping,
    Coverage,
    Interpolation,
    Perspective,
    Depth,
    Transparency,
}

fn material_for_id(materials: &[Material], id: MaterialId) -> Option<&Material> {
    materials.get(id.0 as usize)
}

fn material_for_id_mut(materials: &mut [Material], id: MaterialId) -> Option<&mut Material> {
    materials.get_mut(id.0 as usize)
}

/// 렌더 타깃과 scalar mesh pipeline/texture/lighting/camera 상태를 소유한다.
#[derive(Debug)]
pub struct Renderer {
    target: RenderTarget,
    supersample_target: Option<RenderTarget>,
    quality_mode: QualityMode,
    mipmap_enabled: bool,
    mip_debug_enabled: bool,
    stats: FrameStats,
    framebuffer_generation: u32,
    debug_lines_enabled: bool,
    pipeline_state: PipelineState,
    clip_debug_enabled: bool,
    coverage_debug_enabled: bool,
    interpolation_debug_enabled: bool,
    perspective_debug_enabled: bool,
    depth_debug_enabled: bool,
    depth_order_reversed: bool,
    transparency_debug_enabled: bool,
    transparent_sort_enabled: bool,
    blend_color_space: BlendColorSpace,
    texture_debug_enabled: bool,
    textures: TextureStore,
    active_texture_id: TextureId,
    texture_upload_successes: u32,
    texture_upload_failures: u32,
    materials: Vec<Material>,
    directional_light: DirectionalLight,
    camera_controller: CameraController,
    mesh: Mesh,
    mesh_source_positions: usize,
    mesh_source_faces: usize,
    mesh_source_bounds: MeshBounds,
    mesh_upload_successes: u32,
    mesh_upload_failures: u32,
    next_mesh_id: u32,
    draw_item: DrawItem,
    mesh_scene: MeshScene,
    clip_debug_mesh: Mesh,
    clip_debug_scene: MeshScene,
    coverage_debug_mesh: Mesh,
    coverage_debug_scene: MeshScene,
    interpolation_debug_mesh: Mesh,
    interpolation_debug_scene: MeshScene,
    perspective_debug_mesh: Mesh,
    perspective_debug_scene: MeshScene,
    depth_debug_near_first_mesh: Mesh,
    depth_debug_far_first_mesh: Mesh,
    depth_debug_scene: MeshScene,
    transparency_opaque_mesh: Mesh,
    transparency_opaque_scene: MeshScene,
    transparency_cutout_mesh: Mesh,
    transparency_cutout_scene: MeshScene,
    transparency_blend_mesh: Mesh,
    transparency_blend_scene: MeshScene,
    transparency_cutout_texture: Texture,
    clipper: TriangleClipper,
    transparent_scratch: Vec<PreparedTransparentTriangle>,
}

impl Renderer {
    const fn active_scene(&self) -> ActiveScene {
        if self.transparency_debug_enabled {
            ActiveScene::Transparency
        } else if self.depth_debug_enabled {
            ActiveScene::Depth
        } else if self.perspective_debug_enabled {
            ActiveScene::Perspective
        } else if self.interpolation_debug_enabled {
            ActiveScene::Interpolation
        } else if self.coverage_debug_enabled {
            ActiveScene::Coverage
        } else if self.clip_debug_enabled {
            ActiveScene::Clipping
        } else {
            ActiveScene::Cube
        }
    }

    pub fn new(width: usize, height: usize) -> Result<Self, RenderTargetError> {
        let target = RenderTarget::new(width, height)?;
        let textures = TextureStore::new();
        let active_texture_id = textures.fallback_id();
        let mesh = unit_cube_mesh();
        let mesh_source_bounds = MeshBounds {
            source_min: Vec3::new(-0.5, -0.5, -0.5),
            source_max: Vec3::new(0.5, 0.5, 0.5),
            source_center: Vec3::ZERO,
            source_half_extent: 0.5,
        };
        let draw_item = DrawItem::new(
            MeshId(0),
            MaterialId(0),
            Transform {
                translation: Vec3::ZERO,
                rotation_radians: Vec3::new(0.45, 0.0, 0.0),
                scale: Vec3::new(1.25, 1.25, 1.25),
            },
        );
        let mesh_scene = MeshScene::new(&mesh, draw_item.model, width, height);
        let clip_debug_mesh = clipping_debug_fixture();
        let clip_debug_scene = MeshScene::new_identity_debug(&clip_debug_mesh, width, height);
        let coverage_debug_mesh = coverage_debug_fixture();
        let coverage_debug_scene =
            MeshScene::new_identity_debug(&coverage_debug_mesh, width, height);
        let interpolation_debug_mesh = interpolation_debug_fixture();
        let interpolation_debug_scene =
            MeshScene::new_identity_debug(&interpolation_debug_mesh, width, height);
        let perspective_debug_mesh = perspective_debug_fixture(false);
        let perspective_debug_scene =
            MeshScene::new_perspective_debug(&perspective_debug_mesh, width, height);
        let depth_debug_near_first_mesh = depth_debug_fixture(false);
        let depth_debug_far_first_mesh = depth_debug_fixture(true);
        let depth_debug_scene =
            MeshScene::new_identity_debug(&depth_debug_near_first_mesh, width, height);
        let transparency_opaque_mesh = transparency_quad_fixture(
            -0.92,
            0.92,
            0.82,
            -0.82,
            [0.88; 4],
            Vec4::new(1.0, 1.0, 1.0, 1.0),
        );
        let transparency_opaque_scene =
            MeshScene::new_identity_debug(&transparency_opaque_mesh, width, height);
        let transparency_cutout_mesh = transparency_quad_fixture(
            -0.82,
            -0.04,
            0.70,
            -0.70,
            [0.18; 4],
            Vec4::new(1.0, 1.0, 1.0, 1.0),
        );
        let transparency_cutout_scene =
            MeshScene::new_identity_debug(&transparency_cutout_mesh, width, height);
        let transparency_blend_mesh = transparency_blend_fixture();
        let transparency_blend_scene =
            MeshScene::new_identity_debug(&transparency_blend_mesh, width, height);
        let transparency_cutout_texture = transparency_cutout_texture();
        let mut renderer = Self {
            target,
            supersample_target: None,
            quality_mode: QualityMode::NoAa,
            mipmap_enabled: false,
            mip_debug_enabled: false,
            stats: FrameStats::default(),
            framebuffer_generation: 0,
            debug_lines_enabled: false,
            pipeline_state: PipelineState::default(),
            clip_debug_enabled: false,
            coverage_debug_enabled: false,
            interpolation_debug_enabled: false,
            perspective_debug_enabled: false,
            depth_debug_enabled: false,
            depth_order_reversed: false,
            transparency_debug_enabled: false,
            transparent_sort_enabled: true,
            blend_color_space: BlendColorSpace::Linear,
            texture_debug_enabled: false,
            textures,
            active_texture_id,
            texture_upload_successes: 0,
            texture_upload_failures: 0,
            materials: vec![Material::default()],
            directional_light: DirectionalLight::default(),
            camera_controller: CameraController::default(),
            mesh,
            mesh_source_positions: 24,
            mesh_source_faces: 12,
            mesh_source_bounds,
            mesh_upload_successes: 0,
            mesh_upload_failures: 0,
            next_mesh_id: 1,
            draw_item,
            mesh_scene,
            clip_debug_mesh,
            clip_debug_scene,
            coverage_debug_mesh,
            coverage_debug_scene,
            interpolation_debug_mesh,
            interpolation_debug_scene,
            perspective_debug_mesh,
            perspective_debug_scene,
            depth_debug_near_first_mesh,
            depth_debug_far_first_mesh,
            depth_debug_scene,
            transparency_opaque_mesh,
            transparency_opaque_scene,
            transparency_cutout_mesh,
            transparency_cutout_scene,
            transparency_blend_mesh,
            transparency_blend_scene,
            transparency_cutout_texture,
            clipper: TriangleClipper::default(),
            transparent_scratch: Vec::new(),
        };
        let draw_options = FrameDrawOptions {
            debug_lines_enabled: renderer.debug_lines_enabled,
            pipeline_state: renderer.pipeline_state,
            uv_checker_enabled: renderer.perspective_debug_enabled,
            sampled_texture: None,
            material: Material::default(),
            light: renderer.directional_light,
            camera_world: renderer.mesh_scene.camera_world,
            sort_transparent: renderer.transparent_sort_enabled,
            blend_color_space: renderer.blend_color_space,
            mipmap_enabled: renderer.mipmap_enabled,
            mip_debug_enabled: renderer.mip_debug_enabled,
        };
        draw_frame(
            &mut renderer.target,
            draw_options,
            &renderer.mesh,
            &mut renderer.clipper,
            &mut renderer.transparent_scratch,
            &renderer.mesh_scene.clip_vertices,
            CUBE_SELECTED_VERTEX_INDEX,
        );
        Ok(renderer)
    }

    fn primary_selected_vertex_index(&self) -> usize {
        CUBE_SELECTED_VERTEX_INDEX.min(self.mesh.vertices().len().saturating_sub(1))
    }

    pub fn resize(&mut self, width: usize, height: usize) -> Result<(), RenderTargetError> {
        if width == self.width() && height == self.height() {
            return Ok(());
        }
        let mut replacement = RenderTarget::new(width, height)?;
        let scale = self.quality_mode.render_scale();
        let render_width = width
            .checked_mul(scale)
            .ok_or(RenderTargetError::DimensionOverflow)?;
        let render_height = height
            .checked_mul(scale)
            .ok_or(RenderTargetError::DimensionOverflow)?;
        let mut replacement_supersample = match self.quality_mode {
            QualityMode::NoAa => None,
            QualityMode::Ssaa2x => Some(RenderTarget::new(render_width, render_height)?),
        };
        let camera_pose = self.camera_controller.pose();
        let mut replacement_scene = MeshScene::with_capacity(&self.mesh);
        replacement_scene.rebuild_cube_with_camera(
            &self.mesh,
            self.draw_item.model,
            camera_pose.eye,
            camera_pose.target,
            render_width,
            render_height,
        );
        let replacement_clip_debug_scene =
            MeshScene::new_identity_debug(&self.clip_debug_mesh, render_width, render_height);
        let replacement_coverage_debug_scene =
            MeshScene::new_identity_debug(&self.coverage_debug_mesh, render_width, render_height);
        let replacement_interpolation_debug_scene = MeshScene::new_identity_debug(
            &self.interpolation_debug_mesh,
            render_width,
            render_height,
        );
        let replacement_perspective_debug_scene = MeshScene::new_perspective_debug(
            &self.perspective_debug_mesh,
            render_width,
            render_height,
        );
        let replacement_depth_debug_scene = MeshScene::new_identity_debug(
            &self.depth_debug_near_first_mesh,
            render_width,
            render_height,
        );
        let replacement_transparency_opaque_scene = MeshScene::new_identity_debug(
            &self.transparency_opaque_mesh,
            render_width,
            render_height,
        );
        let replacement_transparency_cutout_scene = MeshScene::new_identity_debug(
            &self.transparency_cutout_mesh,
            render_width,
            render_height,
        );
        let replacement_transparency_blend_scene = MeshScene::new_identity_debug(
            &self.transparency_blend_mesh,
            render_width,
            render_height,
        );
        let (mesh, clip_vertices, selected_vertex_index) = match self.active_scene() {
            ActiveScene::Cube => (
                &self.mesh,
                replacement_scene.clip_vertices.as_slice(),
                self.primary_selected_vertex_index(),
            ),
            ActiveScene::Clipping => (
                &self.clip_debug_mesh,
                replacement_clip_debug_scene.clip_vertices.as_slice(),
                CLIP_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Coverage => (
                &self.coverage_debug_mesh,
                replacement_coverage_debug_scene.clip_vertices.as_slice(),
                COVERAGE_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Interpolation => (
                &self.interpolation_debug_mesh,
                replacement_interpolation_debug_scene
                    .clip_vertices
                    .as_slice(),
                INTERPOLATION_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Perspective => (
                &self.perspective_debug_mesh,
                replacement_perspective_debug_scene.clip_vertices.as_slice(),
                PERSPECTIVE_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Depth => (
                if self.depth_order_reversed {
                    &self.depth_debug_far_first_mesh
                } else {
                    &self.depth_debug_near_first_mesh
                },
                replacement_depth_debug_scene.clip_vertices.as_slice(),
                DEPTH_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Transparency => (
                &self.transparency_blend_mesh,
                replacement_transparency_blend_scene
                    .clip_vertices
                    .as_slice(),
                0,
            ),
        };
        let cube_material = *material_for_id(&self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다");
        let material = if matches!(self.active_scene(), ActiveScene::Cube) {
            cube_material
        } else {
            Material {
                normal_mode: cube_material.normal_mode,
                ..Material::default()
            }
        };
        let sampled_texture = if matches!(self.active_scene(), ActiveScene::Cube) {
            material.base_color_texture.map(|texture_id| {
                (
                    self.textures
                        .get(texture_id)
                        .expect("Material texture ID는 저장소에 존재해야 한다"),
                    material.sampler,
                )
            })
        } else {
            None
        };
        let draw_options = FrameDrawOptions {
            debug_lines_enabled: self.debug_lines_enabled,
            pipeline_state: self.pipeline_state,
            uv_checker_enabled: self.perspective_debug_enabled,
            sampled_texture,
            material,
            light: self.directional_light,
            camera_world: if matches!(self.active_scene(), ActiveScene::Cube) {
                replacement_scene.camera_world
            } else {
                Vec3::ZERO
            },
            sort_transparent: self.transparent_sort_enabled,
            blend_color_space: self.blend_color_space,
            mipmap_enabled: self.mipmap_enabled,
            mip_debug_enabled: self.mip_debug_enabled,
        };
        let render_target = replacement_supersample.as_mut().unwrap_or(&mut replacement);
        if self.texture_debug_enabled {
            let texture = self
                .textures
                .get(self.active_texture_id)
                .expect("active texture ID는 저장소에 존재해야 한다");
            render_target.render_texture_nearest(texture);
        } else if self.transparency_debug_enabled {
            draw_transparency_fixture(
                render_target,
                TransparencyFixture {
                    opaque_mesh: &self.transparency_opaque_mesh,
                    opaque_vertices: &replacement_transparency_opaque_scene.clip_vertices,
                    cutout_mesh: &self.transparency_cutout_mesh,
                    cutout_vertices: &replacement_transparency_cutout_scene.clip_vertices,
                    blend_mesh: &self.transparency_blend_mesh,
                    blend_vertices: &replacement_transparency_blend_scene.clip_vertices,
                    cutout_texture: &self.transparency_cutout_texture,
                },
                &mut self.clipper,
                &mut self.transparent_scratch,
                cube_material.alpha_cutoff,
                self.transparent_sort_enabled,
                self.blend_color_space,
            );
        } else {
            draw_frame(
                render_target,
                draw_options,
                mesh,
                &mut self.clipper,
                &mut self.transparent_scratch,
                clip_vertices,
                selected_vertex_index,
            );
        }
        if let Some(source) = replacement_supersample.as_ref() {
            assert!(replacement.resolve_ssaa_2x_from(source));
        }
        self.target = replacement;
        self.supersample_target = replacement_supersample;
        self.mesh_scene = replacement_scene;
        self.clip_debug_scene = replacement_clip_debug_scene;
        self.coverage_debug_scene = replacement_coverage_debug_scene;
        self.interpolation_debug_scene = replacement_interpolation_debug_scene;
        self.perspective_debug_scene = replacement_perspective_debug_scene;
        self.depth_debug_scene = replacement_depth_debug_scene;
        self.transparency_opaque_scene = replacement_transparency_opaque_scene;
        self.transparency_cutout_scene = replacement_transparency_cutout_scene;
        self.transparency_blend_scene = replacement_transparency_blend_scene;
        self.framebuffer_generation = self.framebuffer_generation.wrapping_add(1);
        Ok(())
    }

    pub fn update_and_render(&mut self, dt_seconds: f32, input: InputSnapshot) -> FrameStats {
        let (dt_seconds, invalid_dt) = sanitize_dt(dt_seconds);
        let invalid_camera_update = self
            .camera_controller
            .update(dt_seconds, input.camera_control_input())
            .is_err();
        let camera_pose = self.camera_controller.pose();
        let rotation_y = self.draw_item.model.rotation_radians.y;
        self.draw_item.model.rotation_radians.y = if rotation_y.is_finite() {
            (rotation_y + dt_seconds * MODEL_ANGULAR_SPEED_RADIANS)
                .rem_euclid(std::f32::consts::TAU)
        } else {
            rotation_y
        };
        let (render_width, render_height) = self
            .supersample_target
            .as_ref()
            .map_or((self.target.width(), self.target.height()), |target| {
                (target.width(), target.height())
            });
        if !self.texture_debug_enabled {
            match self.active_scene() {
                ActiveScene::Cube => self.mesh_scene.rebuild_cube_with_camera(
                    &self.mesh,
                    self.draw_item.model,
                    camera_pose.eye,
                    camera_pose.target,
                    render_width,
                    render_height,
                ),
                ActiveScene::Clipping => self.clip_debug_scene.rebuild_identity_debug(
                    &self.clip_debug_mesh,
                    render_width,
                    render_height,
                ),
                ActiveScene::Coverage => self.coverage_debug_scene.rebuild_identity_debug(
                    &self.coverage_debug_mesh,
                    render_width,
                    render_height,
                ),
                ActiveScene::Interpolation => {
                    self.interpolation_debug_scene.rebuild_identity_debug(
                        &self.interpolation_debug_mesh,
                        render_width,
                        render_height,
                    )
                }
                ActiveScene::Perspective => self.perspective_debug_scene.rebuild_perspective_debug(
                    &self.perspective_debug_mesh,
                    render_width,
                    render_height,
                ),
                ActiveScene::Depth => self.depth_debug_scene.rebuild_identity_debug(
                    &self.depth_debug_near_first_mesh,
                    render_width,
                    render_height,
                ),
                ActiveScene::Transparency => {
                    self.transparency_opaque_scene.rebuild_identity_debug(
                        &self.transparency_opaque_mesh,
                        render_width,
                        render_height,
                    );
                    self.transparency_cutout_scene.rebuild_identity_debug(
                        &self.transparency_cutout_mesh,
                        render_width,
                        render_height,
                    );
                    self.transparency_blend_scene.rebuild_identity_debug(
                        &self.transparency_blend_mesh,
                        render_width,
                        render_height,
                    );
                }
            }
        }
        let (mesh, scene, selected_vertex_index) = match self.active_scene() {
            ActiveScene::Cube => (
                &self.mesh,
                &self.mesh_scene,
                self.primary_selected_vertex_index(),
            ),
            ActiveScene::Clipping => (
                &self.clip_debug_mesh,
                &self.clip_debug_scene,
                CLIP_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Coverage => (
                &self.coverage_debug_mesh,
                &self.coverage_debug_scene,
                COVERAGE_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Interpolation => (
                &self.interpolation_debug_mesh,
                &self.interpolation_debug_scene,
                INTERPOLATION_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Perspective => (
                &self.perspective_debug_mesh,
                &self.perspective_debug_scene,
                PERSPECTIVE_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Depth => (
                if self.depth_order_reversed {
                    &self.depth_debug_far_first_mesh
                } else {
                    &self.depth_debug_near_first_mesh
                },
                &self.depth_debug_scene,
                DEPTH_DEBUG_SELECTED_VERTEX_INDEX,
            ),
            ActiveScene::Transparency => (
                &self.transparency_blend_mesh,
                &self.transparency_blend_scene,
                0,
            ),
        };
        let active_scene = self.active_scene();
        let render_target = self.supersample_target.as_mut().unwrap_or(&mut self.target);
        let (draw_report, texture_debug_pixels) = if self.texture_debug_enabled {
            let texture = self
                .textures
                .get(self.active_texture_id)
                .expect("active texture ID는 저장소에 존재해야 한다");
            (
                FrameDrawReport::default(),
                render_target.render_texture_nearest(texture),
            )
        } else if self.transparency_debug_enabled {
            let material = *material_for_id(&self.materials, self.draw_item.material_id)
                .expect("DrawItem material ID는 저장소에 존재해야 한다");
            (
                draw_transparency_fixture(
                    render_target,
                    TransparencyFixture {
                        opaque_mesh: &self.transparency_opaque_mesh,
                        opaque_vertices: &self.transparency_opaque_scene.clip_vertices,
                        cutout_mesh: &self.transparency_cutout_mesh,
                        cutout_vertices: &self.transparency_cutout_scene.clip_vertices,
                        blend_mesh: &self.transparency_blend_mesh,
                        blend_vertices: &self.transparency_blend_scene.clip_vertices,
                        cutout_texture: &self.transparency_cutout_texture,
                    },
                    &mut self.clipper,
                    &mut self.transparent_scratch,
                    material.alpha_cutoff,
                    self.transparent_sort_enabled,
                    self.blend_color_space,
                ),
                0,
            )
        } else {
            let cube_material = *material_for_id(&self.materials, self.draw_item.material_id)
                .expect("DrawItem material ID는 저장소에 존재해야 한다");
            let material = if matches!(active_scene, ActiveScene::Cube) {
                cube_material
            } else {
                Material {
                    normal_mode: cube_material.normal_mode,
                    ..Material::default()
                }
            };
            let sampled_texture = if matches!(active_scene, ActiveScene::Cube) {
                material.base_color_texture.map(|texture_id| {
                    (
                        self.textures
                            .get(texture_id)
                            .expect("Material texture ID는 저장소에 존재해야 한다"),
                        material.sampler,
                    )
                })
            } else {
                None
            };
            (
                draw_frame(
                    render_target,
                    FrameDrawOptions {
                        debug_lines_enabled: self.debug_lines_enabled,
                        pipeline_state: self.pipeline_state,
                        uv_checker_enabled: self.perspective_debug_enabled,
                        sampled_texture,
                        material,
                        light: self.directional_light,
                        camera_world: scene.camera_world,
                        sort_transparent: self.transparent_sort_enabled,
                        blend_color_space: self.blend_color_space,
                        mipmap_enabled: self.mipmap_enabled,
                        mip_debug_enabled: self.mip_debug_enabled,
                    },
                    mesh,
                    &mut self.clipper,
                    &mut self.transparent_scratch,
                    &scene.clip_vertices,
                    selected_vertex_index,
                ),
                0,
            )
        };
        if let Some(source) = self.supersample_target.as_ref() {
            assert!(self.target.resolve_ssaa_2x_from(source));
        }
        let active_vertex_count = if self.texture_debug_enabled {
            0
        } else if self.transparency_debug_enabled {
            (self.transparency_opaque_mesh.vertices().len()
                + self.transparency_cutout_mesh.vertices().len()
                + self.transparency_blend_mesh.vertices().len()) as u32
        } else {
            mesh.vertices().len() as u32
        };
        let transformed_vertex_count = if self.texture_debug_enabled {
            0
        } else if self.transparency_debug_enabled {
            (self.transparency_opaque_scene.traces.len()
                + self.transparency_cutout_scene.traces.len()
                + self.transparency_blend_scene.traces.len()) as u32
        } else {
            scene.traces.len() as u32
        };
        let active_triangle_count = if self.texture_debug_enabled {
            0
        } else if self.transparency_debug_enabled {
            (self.transparency_opaque_mesh.triangle_count()
                + self.transparency_cutout_mesh.triangle_count()
                + self.transparency_blend_mesh.triangle_count()) as u32
        } else {
            mesh.triangle_count() as u32
        };
        self.stats = FrameStats {
            frame_index: self.stats.frame_index.wrapping_add(1),
            dt_seconds,
            input_bits: input.packed_bits(),
            input_vertices: active_vertex_count,
            input_triangles: active_triangle_count,
            transformed_vertices: transformed_vertex_count,
            submitted_triangles: draw_report.submitted_triangles,
            culled_triangles: draw_report.culled_triangles,
            degenerate_triangles: draw_report.degenerate_triangles,
            invalid_triangles: draw_report.invalid_triangles,
            fully_clipped_triangles: draw_report.fully_clipped_triangles,
            clip_invalid_triangles: draw_report.clip_invalid_triangles,
            generated_triangles: draw_report.generated_triangles,
            max_clip_polygon_vertices: draw_report.max_clip_polygon_vertices,
            rasterized_triangles: draw_report.rasterized_triangles,
            covered_samples: draw_report.covered_samples,
            shaded_samples: draw_report.shaded_samples,
            depth_passed_samples: draw_report.depth_passed_samples,
            depth_failed_samples: draw_report.depth_failed_samples,
            invalid_depth_samples: draw_report.invalid_depth_samples,
            alpha_discarded_samples: draw_report.alpha_discarded_samples,
            depth_written_samples: draw_report.depth_written_samples,
            blended_samples: draw_report.blended_samples,
            max_barycentric_sum_error: draw_report.max_barycentric_sum_error,
            interpolated_inv_w_samples: draw_report.interpolated_inv_w_samples,
            invalid_interpolation_samples: draw_report.invalid_interpolation_samples,
            min_interpolated_inv_w: draw_report.min_interpolated_inv_w,
            max_interpolated_inv_w: draw_report.max_interpolated_inv_w,
            sample_counter_overflow: draw_report.sample_counter_overflow,
            debug_pixels: draw_report.debug_pixels,
            invalid_values: (if self.texture_debug_enabled {
                0
            } else if self.transparency_debug_enabled {
                self.transparency_opaque_scene
                    .diagnostics
                    .invalid_values
                    .saturating_add(self.transparency_cutout_scene.diagnostics.invalid_values)
                    .saturating_add(self.transparency_blend_scene.diagnostics.invalid_values)
            } else {
                scene.diagnostics.invalid_values
            })
            .saturating_add(draw_report.invalid_values)
            .saturating_add(u32::from(invalid_dt))
            .saturating_add(u32::from(invalid_camera_update)),
            texture_debug_pixels,
            texture_upload_successes: self.texture_upload_successes,
            texture_upload_failures: self.texture_upload_failures,
            active_texture_id: self.active_texture_id.0,
            texture_samples: draw_report.texture_samples,
            lighting_samples: draw_report.lighting_samples,
            render_scale: self.quality_mode.render_scale() as u32,
            resolved_pixels: if self.quality_mode == QualityMode::Ssaa2x {
                u32::try_from(self.target.width() * self.target.height())
                    .expect("논리 target pixel 수는 u32 안에 들어가야 한다")
            } else {
                0
            },
            mip_samples: draw_report.mip_samples,
            min_mip_level: draw_report.min_mip_level,
            max_mip_level: draw_report.max_mip_level,
            invalid_lod_samples: draw_report.invalid_lod_samples,
        };
        debug_assert!(
            pipeline_stats_are_consistent_or_overflowed(self.stats),
            "15장 scalar pipeline의 단계별 FrameStats 관계식이 깨졌다: {:?}",
            self.stats
        );
        self.stats
    }

    pub fn clear(&mut self, rgb: [u8; 3]) {
        let color = Color::rgb(rgb[0], rgb[1], rgb[2]);
        self.target.clear_color(color);
        if let Some(target) = self.supersample_target.as_mut() {
            target.clear_color(color);
        }
    }

    pub fn set_debug_lines_enabled(&mut self, enabled: bool) {
        self.debug_lines_enabled = enabled;
    }

    pub fn set_camera_mode(
        &mut self,
        mode: CameraMode,
    ) -> Result<(), camera_control::CameraControlError> {
        self.camera_controller.set_mode(mode)
    }

    pub const fn camera_mode(&self) -> CameraMode {
        self.camera_controller.mode()
    }

    pub fn camera_pose(&self) -> camera_control::CameraPose {
        self.camera_controller.pose()
    }

    pub const fn camera_yaw(&self) -> f32 {
        self.camera_controller.yaw()
    }

    pub const fn camera_pitch(&self) -> f32 {
        self.camera_controller.pitch()
    }

    pub const fn camera_orbit_radius(&self) -> f32 {
        self.camera_controller.orbit_radius()
    }

    pub fn set_cull_mode(&mut self, mode: CullMode) {
        self.pipeline_state.cull_mode = mode;
    }

    pub fn set_winding_debug_mode(&mut self, mode: WindingDebugMode) {
        self.pipeline_state.debug_mode = match mode {
            WindingDebugMode::VertexColor => PipelineDebugMode::Solid,
            WindingDebugMode::Facing => PipelineDebugMode::FrontBack,
            WindingDebugMode::Barycentric => PipelineDebugMode::Barycentric,
        };
    }

    pub fn set_clip_debug_enabled(&mut self, enabled: bool) {
        self.clip_debug_enabled = enabled;
        if enabled {
            self.transparency_debug_enabled = false;
            self.coverage_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.perspective_debug_enabled = false;
            self.depth_debug_enabled = false;
            self.mip_debug_enabled = false;
        }
    }

    pub fn set_coverage_debug_enabled(&mut self, enabled: bool) {
        self.coverage_debug_enabled = enabled;
        if enabled {
            self.transparency_debug_enabled = false;
            self.clip_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.perspective_debug_enabled = false;
            self.depth_debug_enabled = false;
            self.mip_debug_enabled = false;
        }
    }

    pub fn set_interpolation_debug_enabled(&mut self, enabled: bool) {
        self.interpolation_debug_enabled = enabled;
        if enabled {
            self.transparency_debug_enabled = false;
            self.clip_debug_enabled = false;
            self.coverage_debug_enabled = false;
            self.perspective_debug_enabled = false;
            self.depth_debug_enabled = false;
            self.mip_debug_enabled = false;
        }
    }

    pub fn set_perspective_debug_enabled(&mut self, enabled: bool) {
        self.perspective_debug_enabled = enabled;
        if enabled {
            self.transparency_debug_enabled = false;
            self.clip_debug_enabled = false;
            self.coverage_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.depth_debug_enabled = false;
            self.mip_debug_enabled = false;
        }
    }

    pub fn set_attribute_interpolation_mode(&mut self, mode: AttributeInterpolationMode) {
        self.pipeline_state.attribute_interpolation_mode = mode;
    }

    pub fn set_depth_debug_enabled(&mut self, enabled: bool) {
        self.depth_debug_enabled = enabled;
        if enabled {
            self.transparency_debug_enabled = false;
            self.clip_debug_enabled = false;
            self.coverage_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.perspective_debug_enabled = false;
            self.mip_debug_enabled = false;
        }
    }

    pub fn set_depth_order_reversed(&mut self, reversed: bool) {
        self.depth_order_reversed = reversed;
    }

    pub fn set_depth_debug_mode(&mut self, mode: DepthDebugMode) {
        self.pipeline_state.debug_mode = match mode {
            DepthDebugMode::Off => PipelineDebugMode::Solid,
            DepthDebugMode::Grayscale => PipelineDebugMode::Depth,
            DepthDebugMode::Heatmap => PipelineDebugMode::DepthHeatmap,
        };
    }

    pub fn set_pipeline_debug_mode(&mut self, mode: PipelineDebugMode) {
        self.pipeline_state.debug_mode = mode;
    }

    pub fn set_texture_debug_enabled(&mut self, enabled: bool) {
        self.texture_debug_enabled = enabled;
        if enabled {
            self.transparency_debug_enabled = false;
            self.mip_debug_enabled = false;
        }
    }

    pub fn set_transparency_debug_enabled(&mut self, enabled: bool) {
        self.transparency_debug_enabled = enabled;
        if enabled {
            self.clip_debug_enabled = false;
            self.coverage_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.perspective_debug_enabled = false;
            self.depth_debug_enabled = false;
            self.texture_debug_enabled = false;
            self.mip_debug_enabled = false;
            self.pipeline_state.debug_mode = PipelineDebugMode::Solid;
        }
    }

    pub const fn transparency_debug_enabled(&self) -> bool {
        self.transparency_debug_enabled
    }

    pub fn set_transparent_sort_enabled(&mut self, enabled: bool) {
        self.transparent_sort_enabled = enabled;
    }

    pub const fn transparent_sort_enabled(&self) -> bool {
        self.transparent_sort_enabled
    }

    pub fn set_blend_color_space(&mut self, color_space: BlendColorSpace) {
        self.blend_color_space = color_space;
    }

    pub const fn blend_color_space(&self) -> BlendColorSpace {
        self.blend_color_space
    }

    pub fn set_alpha_mode(&mut self, alpha_mode: AlphaMode) {
        material_for_id_mut(&mut self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .alpha_mode = alpha_mode;
    }

    pub fn alpha_mode(&self) -> AlphaMode {
        material_for_id(&self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .alpha_mode
    }

    pub fn set_alpha_cutoff(&mut self, cutoff: f32) -> Result<(), MaterialAlphaError> {
        if !cutoff.is_finite() || !(0.0..=1.0).contains(&cutoff) {
            return Err(MaterialAlphaError::InvalidCutoff);
        }
        material_for_id_mut(&mut self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .alpha_cutoff = cutoff;
        Ok(())
    }

    pub fn alpha_cutoff(&self) -> f32 {
        material_for_id(&self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .alpha_cutoff
    }

    pub fn set_texture_sampling_enabled(&mut self, enabled: bool) {
        let material = material_for_id_mut(&mut self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다");
        material.base_color_texture = enabled.then_some(self.active_texture_id);
        if !enabled {
            self.mip_debug_enabled = false;
        }
    }

    pub fn texture_sampling_enabled(&self) -> bool {
        material_for_id(&self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .base_color_texture
            .is_some()
    }

    pub fn set_sampler_state(&mut self, sampler_state: SamplerState) {
        material_for_id_mut(&mut self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .sampler = sampler_state;
    }

    pub fn sampler_state(&self) -> SamplerState {
        material_for_id(&self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .sampler
    }

    pub fn set_lighting_enabled(&mut self, enabled: bool) {
        let material = material_for_id_mut(&mut self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다");
        material.shader_mode = match (enabled, material.shader_mode) {
            (false, _) => ShaderMode::Unlit,
            (true, ShaderMode::Unlit) => ShaderMode::Lambert,
            (true, mode) => mode,
        };
    }

    pub fn lighting_enabled(&self) -> bool {
        self.shader_mode() != ShaderMode::Unlit
    }

    pub fn set_shader_mode(&mut self, shader_mode: ShaderMode) {
        material_for_id_mut(&mut self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .shader_mode = shader_mode;
    }

    pub fn shader_mode(&self) -> ShaderMode {
        material_for_id(&self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .shader_mode
    }

    pub fn set_material_specular(
        &mut self,
        specular_color: Vec3,
        shininess: f32,
    ) -> Result<(), MaterialError> {
        if ![specular_color.x, specular_color.y, specular_color.z]
            .into_iter()
            .all(|channel| channel.is_finite() && (0.0..=1.0).contains(&channel))
        {
            return Err(MaterialError::InvalidSpecularColor);
        }
        if !shininess.is_finite() || shininess <= 0.0 {
            return Err(MaterialError::InvalidShininess);
        }
        let material = material_for_id_mut(&mut self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다");
        material.specular_color = specular_color;
        material.shininess = shininess;
        Ok(())
    }

    pub fn material_specular(&self) -> (Vec3, f32) {
        let material = material_for_id(&self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다");
        (material.specular_color, material.shininess)
    }

    pub fn set_normal_mode(&mut self, normal_mode: NormalMode) {
        material_for_id_mut(&mut self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .normal_mode = normal_mode;
    }

    pub fn normal_mode(&self) -> NormalMode {
        material_for_id(&self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다")
            .normal_mode
    }

    pub fn set_directional_light(
        &mut self,
        surface_to_light: Vec3,
        intensity: f32,
    ) -> Result<(), LightingError> {
        self.directional_light =
            DirectionalLight::new(surface_to_light, self.directional_light.color, intensity)?;
        Ok(())
    }

    pub const fn directional_light(&self) -> DirectionalLight {
        self.directional_light
    }

    pub const fn texture_debug_enabled(&self) -> bool {
        self.texture_debug_enabled
    }

    pub fn upload_texture_rgba8(
        &mut self,
        width: usize,
        height: usize,
        pixels: &[u8],
        color_space: TextureColorSpace,
    ) -> Result<TextureId, TextureError> {
        match self
            .textures
            .upload_rgba8(width, height, pixels, color_space)
        {
            Ok(id) => {
                self.active_texture_id = id;
                let material = material_for_id_mut(&mut self.materials, self.draw_item.material_id)
                    .expect("DrawItem material ID는 저장소에 존재해야 한다");
                if material.base_color_texture.is_some() {
                    material.base_color_texture = Some(id);
                }
                self.texture_upload_successes = self.texture_upload_successes.saturating_add(1);
                Ok(id)
            }
            Err(error) => {
                self.texture_upload_failures = self.texture_upload_failures.saturating_add(1);
                Err(error)
            }
        }
    }

    pub fn set_active_texture(&mut self, id: TextureId) -> Result<(), TextureError> {
        self.textures.get(id)?;
        self.active_texture_id = id;
        let material = material_for_id_mut(&mut self.materials, self.draw_item.material_id)
            .expect("DrawItem material ID는 저장소에 존재해야 한다");
        if material.base_color_texture.is_some() {
            material.base_color_texture = Some(id);
        }
        Ok(())
    }

    pub fn texture_asset_status(&self) -> TextureAssetStatus {
        let active = self
            .textures
            .get(self.active_texture_id)
            .expect("active texture ID는 저장소에 존재해야 한다");
        TextureAssetStatus {
            active_texture_id: self.active_texture_id,
            active_width: active.width(),
            active_height: active.height(),
            mip_levels: active.mip_level_count(),
            successful_uploads: self.texture_upload_successes,
            failed_uploads: self.texture_upload_failures,
        }
    }

    pub fn load_obj(&mut self, bytes: &[u8]) -> Result<MeshId, ObjImportError> {
        let imported = match import_obj(bytes) {
            Ok(imported) => imported,
            Err(error) => {
                self.mesh_upload_failures = self.mesh_upload_failures.saturating_add(1);
                return Err(error);
            }
        };
        let Some(next_mesh_id) = self.next_mesh_id.checked_add(1) else {
            self.mesh_upload_failures = self.mesh_upload_failures.saturating_add(1);
            return Err(ObjImportError::LimitExceeded {
                kind: "mesh ID",
                max: u32::MAX as usize,
            });
        };
        let mesh_id = MeshId(self.next_mesh_id);
        let bounds = imported.bounds();
        let source_positions = imported.source_position_count();
        let source_faces = imported.source_face_count();
        let mesh = imported.into_mesh();
        let model = Transform {
            translation: Vec3::ZERO,
            rotation_radians: Vec3::new(0.35, 0.0, 0.0),
            scale: Vec3::new(1.0, 1.0, 1.0),
        };
        let camera_controller = CameraController::default();
        let camera_pose = camera_controller.pose();
        let (render_width, render_height) = self.render_dimensions();
        let mut mesh_scene = MeshScene::with_capacity(&mesh);
        mesh_scene.rebuild_cube_with_camera(
            &mesh,
            model,
            camera_pose.eye,
            camera_pose.target,
            render_width,
            render_height,
        );

        self.mesh = mesh;
        self.mesh_source_positions = source_positions;
        self.mesh_source_faces = source_faces;
        self.mesh_source_bounds = bounds;
        self.mesh_upload_successes = self.mesh_upload_successes.saturating_add(1);
        self.next_mesh_id = next_mesh_id;
        self.draw_item.mesh_id = mesh_id;
        self.draw_item.model = model;
        self.mesh_scene = mesh_scene;
        self.camera_controller = camera_controller;
        self.clip_debug_enabled = false;
        self.coverage_debug_enabled = false;
        self.interpolation_debug_enabled = false;
        self.perspective_debug_enabled = false;
        self.depth_debug_enabled = false;
        self.transparency_debug_enabled = false;
        self.texture_debug_enabled = false;
        Ok(mesh_id)
    }

    pub fn mesh_asset_status(&self) -> MeshAssetStatus {
        MeshAssetStatus {
            active_mesh_id: self.draw_item.mesh_id,
            source_positions: self.mesh_source_positions,
            source_faces: self.mesh_source_faces,
            internal_vertices: self.mesh.vertices().len(),
            triangles: self.mesh.triangle_count(),
            successful_uploads: self.mesh_upload_successes,
            failed_uploads: self.mesh_upload_failures,
            source_bounds: self.mesh_source_bounds,
        }
    }

    pub fn set_model_rotation_y(&mut self, rotation_y_radians: f32) {
        self.draw_item.model.rotation_radians.y = rotation_y_radians;
        let (render_width, render_height) = self.render_dimensions();
        self.mesh_scene.rebuild_cube(
            &self.mesh,
            self.draw_item.model,
            render_width,
            render_height,
        );
    }

    fn render_dimensions(&self) -> (usize, usize) {
        self.supersample_target
            .as_ref()
            .map_or((self.target.width(), self.target.height()), |target| {
                (target.width(), target.height())
            })
    }

    pub fn set_quality_mode(&mut self, mode: QualityMode) -> Result<(), RenderTargetError> {
        if mode == self.quality_mode {
            return Ok(());
        }
        let supersample_target = match mode {
            QualityMode::NoAa => None,
            QualityMode::Ssaa2x => {
                let width = self
                    .target
                    .width()
                    .checked_mul(2)
                    .ok_or(RenderTargetError::DimensionOverflow)?;
                let height = self
                    .target
                    .height()
                    .checked_mul(2)
                    .ok_or(RenderTargetError::DimensionOverflow)?;
                Some(RenderTarget::new(width, height)?)
            }
        };
        self.supersample_target = supersample_target;
        self.quality_mode = mode;
        self.update_and_render(0.0, InputSnapshot::default());
        Ok(())
    }

    pub const fn quality_mode(&self) -> QualityMode {
        self.quality_mode
    }

    pub fn render_dimensions_public(&self) -> (usize, usize) {
        self.render_dimensions()
    }

    pub fn set_mipmap_enabled(&mut self, enabled: bool) {
        self.mipmap_enabled = enabled;
        if !enabled {
            self.mip_debug_enabled = false;
        }
    }

    pub const fn mipmap_enabled(&self) -> bool {
        self.mipmap_enabled
    }

    pub fn set_mip_debug_enabled(&mut self, enabled: bool) {
        self.mip_debug_enabled = enabled;
        if enabled {
            self.mipmap_enabled = true;
            self.texture_debug_enabled = false;
            self.transparency_debug_enabled = false;
            self.clip_debug_enabled = false;
            self.coverage_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.perspective_debug_enabled = false;
            self.depth_debug_enabled = false;
            self.set_texture_sampling_enabled(true);
        }
    }

    pub const fn mip_debug_enabled(&self) -> bool {
        self.mip_debug_enabled
    }

    pub const fn width(&self) -> usize {
        self.target.width()
    }

    pub const fn height(&self) -> usize {
        self.target.height()
    }

    pub fn color_buffer(&self) -> &[u8] {
        self.target.color()
    }

    pub fn depth_buffer(&self) -> &[f32] {
        self.target.depth()
    }

    pub const fn stats(&self) -> FrameStats {
        self.stats
    }

    pub const fn framebuffer_generation(&self) -> u32 {
        self.framebuffer_generation
    }

    pub fn coordinate_debug_snapshot(&self) -> CoordinateDebugSnapshot {
        let (mesh, scene, selected_vertex_index, rotation_y_radians, material_id) =
            match self.active_scene() {
                ActiveScene::Cube => (
                    &self.mesh,
                    &self.mesh_scene,
                    self.primary_selected_vertex_index(),
                    self.draw_item.model.rotation_radians.y,
                    self.draw_item.material_id.0,
                ),
                ActiveScene::Clipping => (
                    &self.clip_debug_mesh,
                    &self.clip_debug_scene,
                    CLIP_DEBUG_SELECTED_VERTEX_INDEX,
                    0.0,
                    0,
                ),
                ActiveScene::Coverage => (
                    &self.coverage_debug_mesh,
                    &self.coverage_debug_scene,
                    COVERAGE_DEBUG_SELECTED_VERTEX_INDEX,
                    0.0,
                    0,
                ),
                ActiveScene::Interpolation => (
                    &self.interpolation_debug_mesh,
                    &self.interpolation_debug_scene,
                    INTERPOLATION_DEBUG_SELECTED_VERTEX_INDEX,
                    0.0,
                    0,
                ),
                ActiveScene::Perspective => (
                    &self.perspective_debug_mesh,
                    &self.perspective_debug_scene,
                    PERSPECTIVE_DEBUG_SELECTED_VERTEX_INDEX,
                    0.0,
                    0,
                ),
                ActiveScene::Depth => (
                    if self.depth_order_reversed {
                        &self.depth_debug_far_first_mesh
                    } else {
                        &self.depth_debug_near_first_mesh
                    },
                    &self.depth_debug_scene,
                    DEPTH_DEBUG_SELECTED_VERTEX_INDEX,
                    0.0,
                    0,
                ),
                ActiveScene::Transparency => (
                    &self.transparency_blend_mesh,
                    &self.transparency_blend_scene,
                    0,
                    0.0,
                    0,
                ),
            };
        let selected_vertex = scene.traces[selected_vertex_index];
        CoordinateDebugSnapshot {
            rotation_y_radians,
            selected_vertex_index,
            selected_vertex,
            selected_attributes: scene.clip_vertices[selected_vertex_index],
            selected_ndc: scene.diagnostic_ndc_positions[selected_vertex_index],
            selected_viewport: scene.diagnostic_viewport_positions[selected_vertex_index],
            clip_plane_distances: ClipPlaneDistances::from_position(selected_vertex.clip_pos),
            diagnostics: scene.diagnostics,
            projection_failures: scene.projection_failures,
            fov_y_radians: CAMERA_FOV_Y_RADIANS,
            near: CAMERA_NEAR,
            far: CAMERA_FAR,
            aspect: scene.aspect,
            mesh_vertices: mesh.vertices().len() as u32,
            mesh_indices: mesh.indices().len() as u32,
            mesh_triangles: mesh.triangle_count() as u32,
            material_id,
            pipeline_state: self.pipeline_state,
            debug_lines_enabled: self.debug_lines_enabled,
            cull_mode: self.pipeline_state.cull_mode,
            winding_debug_mode: match self.pipeline_state.debug_mode {
                PipelineDebugMode::FrontBack => WindingDebugMode::Facing,
                PipelineDebugMode::Barycentric => WindingDebugMode::Barycentric,
                PipelineDebugMode::Solid
                | PipelineDebugMode::Wireframe
                | PipelineDebugMode::TriangleId
                | PipelineDebugMode::Depth
                | PipelineDebugMode::DepthHeatmap
                | PipelineDebugMode::Normal
                | PipelineDebugMode::NdotL
                | PipelineDebugMode::Diffuse
                | PipelineDebugMode::Specular
                | PipelineDebugMode::ColorSpaceComparison => WindingDebugMode::VertexColor,
            },
            clip_debug_enabled: self.clip_debug_enabled,
            coverage_debug_enabled: self.coverage_debug_enabled,
            interpolation_debug_enabled: self.interpolation_debug_enabled,
            perspective_debug_enabled: self.perspective_debug_enabled,
            attribute_interpolation_mode: self.pipeline_state.attribute_interpolation_mode,
            depth_debug_enabled: self.depth_debug_enabled,
            depth_order_reversed: self.depth_order_reversed,
            depth_debug_mode: match self.pipeline_state.debug_mode {
                PipelineDebugMode::Depth => DepthDebugMode::Grayscale,
                PipelineDebugMode::DepthHeatmap => DepthDebugMode::Heatmap,
                PipelineDebugMode::Solid
                | PipelineDebugMode::Wireframe
                | PipelineDebugMode::TriangleId
                | PipelineDebugMode::Barycentric
                | PipelineDebugMode::FrontBack
                | PipelineDebugMode::Normal
                | PipelineDebugMode::NdotL
                | PipelineDebugMode::Diffuse
                | PipelineDebugMode::Specular
                | PipelineDebugMode::ColorSpaceComparison => DepthDebugMode::Off,
            },
            transparency_debug_enabled: self.transparency_debug_enabled,
            frame_stats: self.stats,
        }
    }
}

fn clipping_debug_fixture() -> Mesh {
    let positions = [
        Vec3::new(-2.0, 2.0, -0.5),
        Vec3::new(0.75, -0.5, 0.5),
        Vec3::new(-0.25, -0.25, 0.5),
    ];
    let colors = [
        Vec4::new(1.0, 0.25, 0.2, 1.0),
        Vec4::new(0.2, 1.0, 0.35, 1.0),
        Vec4::new(0.25, 0.55, 1.0, 1.0),
    ];
    let vertices = positions
        .into_iter()
        .zip(colors)
        .enumerate()
        .map(|(index, (position, color))| {
            mesh::Vertex::new(
                position,
                Vec3::Z,
                math::Vec2::new(index as f32 * 0.5, index as f32 * 0.25),
                color,
            )
        })
        .collect();
    Mesh::new(vertices, vec![0, 1, 2]).expect("고정 clipping debug mesh 계약은 항상 유효해야 한다")
}

fn coverage_debug_fixture() -> Mesh {
    let orange = Vec4::new(1.0, 0.35, 0.15, 1.0);
    let cyan = Vec4::new(0.15, 0.75, 1.0, 1.0);
    let vertices = [
        (Vec3::new(-0.5, 0.5, 0.5), orange),
        (Vec3::new(0.5, 0.5, 0.5), orange),
        (Vec3::new(0.5, -0.5, 0.5), orange),
        (Vec3::new(-0.5, 0.5, 0.5), cyan),
        (Vec3::new(0.5, -0.5, 0.5), cyan),
        (Vec3::new(-0.5, -0.5, 0.5), cyan),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (position, color))| {
        mesh::Vertex::new(
            position,
            Vec3::Z,
            math::Vec2::new((index % 3) as f32 * 0.5, (index / 3) as f32),
            color,
        )
    })
    .collect();
    Mesh::new(vertices, vec![0, 1, 2, 3, 4, 5])
        .expect("고정 top-left coverage quad 계약은 항상 유효해야 한다")
}

fn interpolation_debug_fixture() -> Mesh {
    let vertices = [
        (
            Vec3::new(-0.65, 0.65, 0.5),
            Vec3::new(-0.6, 0.0, -0.8),
            math::Vec2::new(0.0, 0.0),
            Vec4::new(1.0, 0.0, 0.0, 1.0),
        ),
        (
            Vec3::new(0.65, 0.65, 0.5),
            Vec3::new(0.6, 0.0, -0.8),
            math::Vec2::new(1.0, 0.0),
            Vec4::new(0.0, 1.0, 0.0, 1.0),
        ),
        (
            Vec3::new(0.0, -0.65, 0.5),
            Vec3::new(0.0, 0.6, -0.8),
            math::Vec2::new(0.5, 1.0),
            Vec4::new(0.0, 0.0, 1.0, 1.0),
        ),
    ]
    .into_iter()
    .map(|(position, normal, uv, color)| mesh::Vertex::new(position, normal, uv, color))
    .collect();
    Mesh::new(vertices, vec![0, 1, 2])
        .expect("고정 barycentric RGB triangle 계약은 항상 유효해야 한다")
}

fn perspective_debug_fixture(alternate_diagonal: bool) -> Mesh {
    let normal = Vec3::new(6.0, 0.0, -4.0)
        .normalized()
        .expect("고정 perspective fixture normal은 0이 아니어야 한다");
    let vertices = [
        (Vec3::new(-1.0, 1.0, 2.0), Vec2::new(0.0, 0.0)),
        (Vec3::new(1.0, 1.0, 5.0), Vec2::new(1.0, 0.0)),
        (Vec3::new(1.0, -1.0, 5.0), Vec2::new(1.0, 1.0)),
        (Vec3::new(-1.0, -1.0, 2.0), Vec2::new(0.0, 1.0)),
    ]
    .into_iter()
    .map(|(position, uv)| mesh::Vertex::new(position, normal, uv, Vec4::new(1.0, 1.0, 1.0, 1.0)))
    .collect();
    let indices = if alternate_diagonal {
        vec![0, 1, 3, 1, 2, 3]
    } else {
        vec![0, 1, 2, 0, 2, 3]
    };
    Mesh::new(vertices, indices).expect("고정 perspective UV quad 계약은 항상 유효해야 한다")
}

fn depth_debug_fixture(far_first: bool) -> Mesh {
    let near = Vec4::new(1.0, 0.2, 0.15, 1.0);
    let far = Vec4::new(0.15, 0.35, 1.0, 1.0);
    let vertices = [
        (Vec3::new(-0.75, 0.65, 0.25), near),
        (Vec3::new(0.35, 0.65, 0.25), near),
        (Vec3::new(-0.20, -0.65, 0.25), near),
        (Vec3::new(-0.35, 0.45, 0.75), far),
        (Vec3::new(0.75, 0.45, 0.75), far),
        (Vec3::new(0.25, -0.75, 0.75), far),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (position, color))| {
        mesh::Vertex::new(
            position,
            Vec3::Z,
            math::Vec2::new((index % 3) as f32 * 0.5, (index / 3) as f32),
            color,
        )
    })
    .collect();
    let indices = if far_first {
        vec![3, 4, 5, 0, 1, 2]
    } else {
        vec![0, 1, 2, 3, 4, 5]
    };
    Mesh::new(vertices, indices).expect("고정 near/far depth fixture 계약은 항상 유효해야 한다")
}

fn transparency_quad_fixture(
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    depths: [f32; 4],
    color: Vec4,
) -> Mesh {
    let positions = [
        Vec3::new(left, top, depths[0]),
        Vec3::new(right, top, depths[1]),
        Vec3::new(right, bottom, depths[2]),
        Vec3::new(left, bottom, depths[3]),
    ];
    let uvs = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ];
    let vertices = positions
        .into_iter()
        .zip(uvs)
        .map(|(position, uv)| mesh::Vertex::new(position, Vec3::Z, uv, color))
        .collect();
    Mesh::new(vertices, vec![0, 1, 2, 0, 2, 3])
        .expect("고정 transparency quad 계약은 항상 유효해야 한다")
}

fn transparency_blend_fixture() -> Mesh {
    let red = Vec4::new(1.0, 0.08, 0.04, 0.55);
    let cyan = Vec4::new(0.04, 0.75, 1.0, 0.55);
    let mut vertices = Vec::with_capacity(8);
    let mut indices = Vec::with_capacity(12);
    for (offset, mesh) in [
        transparency_quad_fixture(0.0, 0.88, 0.68, -0.68, [0.68, 0.28, 0.28, 0.68], red),
        transparency_quad_fixture(0.08, 0.96, 0.58, -0.78, [0.30, 0.72, 0.72, 0.30], cyan),
    ]
    .into_iter()
    .enumerate()
    {
        let base = u32::try_from(offset * 4).expect("두 quad의 vertex offset은 u32다");
        vertices.extend_from_slice(mesh.vertices());
        indices.extend(mesh.indices().iter().map(|index| base + index));
    }
    Mesh::new(vertices, indices).expect("교차 transparent quad fixture는 유효해야 한다")
}

fn transparency_cutout_texture() -> Texture {
    Texture::from_rgba8(
        4,
        4,
        &[
            70, 220, 95, 255, 70, 220, 95, 0, 70, 220, 95, 255, 70, 220, 95, 0, 70, 220, 95, 0, 70,
            220, 95, 255, 70, 220, 95, 0, 70, 220, 95, 255, 70, 220, 95, 255, 70, 220, 95, 0, 70,
            220, 95, 255, 70, 220, 95, 0, 70, 220, 95, 0, 70, 220, 95, 255, 70, 220, 95, 0, 70,
            220, 95, 255,
        ],
        TextureColorSpace::Srgb,
    )
    .expect("고정 4x4 cutout texture는 유효해야 한다")
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct FrameDrawReport {
    submitted_triangles: u32,
    culled_triangles: u32,
    degenerate_triangles: u32,
    invalid_triangles: u32,
    fully_clipped_triangles: u32,
    clip_invalid_triangles: u32,
    generated_triangles: u32,
    max_clip_polygon_vertices: u32,
    rasterized_triangles: u32,
    covered_samples: u32,
    shaded_samples: u32,
    depth_passed_samples: u32,
    depth_failed_samples: u32,
    invalid_depth_samples: u32,
    alpha_discarded_samples: u32,
    depth_written_samples: u32,
    blended_samples: u32,
    max_barycentric_sum_error: f32,
    interpolated_inv_w_samples: u32,
    invalid_interpolation_samples: u32,
    min_interpolated_inv_w: f32,
    max_interpolated_inv_w: f32,
    sample_counter_overflow: bool,
    debug_pixels: u32,
    invalid_values: u32,
    texture_samples: u32,
    lighting_samples: u32,
    mip_samples: u32,
    min_mip_level: u32,
    max_mip_level: u32,
    invalid_lod_samples: u32,
}

impl FrameDrawReport {
    fn absorb(&mut self, other: Self) {
        self.submitted_triangles = self
            .submitted_triangles
            .saturating_add(other.submitted_triangles);
        self.culled_triangles = self.culled_triangles.saturating_add(other.culled_triangles);
        self.degenerate_triangles = self
            .degenerate_triangles
            .saturating_add(other.degenerate_triangles);
        self.invalid_triangles = self
            .invalid_triangles
            .saturating_add(other.invalid_triangles);
        self.fully_clipped_triangles = self
            .fully_clipped_triangles
            .saturating_add(other.fully_clipped_triangles);
        self.clip_invalid_triangles = self
            .clip_invalid_triangles
            .saturating_add(other.clip_invalid_triangles);
        self.generated_triangles = self
            .generated_triangles
            .saturating_add(other.generated_triangles);
        self.max_clip_polygon_vertices = self
            .max_clip_polygon_vertices
            .max(other.max_clip_polygon_vertices);
        self.rasterized_triangles = self
            .rasterized_triangles
            .saturating_add(other.rasterized_triangles);
        self.covered_samples = self.covered_samples.saturating_add(other.covered_samples);
        self.shaded_samples = self.shaded_samples.saturating_add(other.shaded_samples);
        self.depth_passed_samples = self
            .depth_passed_samples
            .saturating_add(other.depth_passed_samples);
        self.depth_failed_samples = self
            .depth_failed_samples
            .saturating_add(other.depth_failed_samples);
        self.invalid_depth_samples = self
            .invalid_depth_samples
            .saturating_add(other.invalid_depth_samples);
        self.alpha_discarded_samples = self
            .alpha_discarded_samples
            .saturating_add(other.alpha_discarded_samples);
        self.depth_written_samples = self
            .depth_written_samples
            .saturating_add(other.depth_written_samples);
        self.blended_samples = self.blended_samples.saturating_add(other.blended_samples);
        self.max_barycentric_sum_error = self
            .max_barycentric_sum_error
            .max(other.max_barycentric_sum_error);
        if other.interpolated_inv_w_samples > 0 {
            if self.interpolated_inv_w_samples == 0 {
                self.min_interpolated_inv_w = other.min_interpolated_inv_w;
                self.max_interpolated_inv_w = other.max_interpolated_inv_w;
            } else {
                self.min_interpolated_inv_w = self
                    .min_interpolated_inv_w
                    .min(other.min_interpolated_inv_w);
                self.max_interpolated_inv_w = self
                    .max_interpolated_inv_w
                    .max(other.max_interpolated_inv_w);
            }
        }
        self.interpolated_inv_w_samples = self
            .interpolated_inv_w_samples
            .saturating_add(other.interpolated_inv_w_samples);
        self.invalid_interpolation_samples = self
            .invalid_interpolation_samples
            .saturating_add(other.invalid_interpolation_samples);
        self.sample_counter_overflow |= other.sample_counter_overflow;
        self.debug_pixels = self.debug_pixels.saturating_add(other.debug_pixels);
        self.invalid_values = self.invalid_values.saturating_add(other.invalid_values);
        self.texture_samples = self.texture_samples.saturating_add(other.texture_samples);
        self.lighting_samples = self.lighting_samples.saturating_add(other.lighting_samples);
        if other.mip_samples > 0 {
            if self.mip_samples == 0 {
                self.min_mip_level = other.min_mip_level;
                self.max_mip_level = other.max_mip_level;
            } else {
                self.min_mip_level = self.min_mip_level.min(other.min_mip_level);
                self.max_mip_level = self.max_mip_level.max(other.max_mip_level);
            }
        }
        self.mip_samples = self.mip_samples.saturating_add(other.mip_samples);
        self.invalid_lod_samples = self
            .invalid_lod_samples
            .saturating_add(other.invalid_lod_samples);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FrameDrawOptions<'a> {
    debug_lines_enabled: bool,
    pipeline_state: PipelineState,
    uv_checker_enabled: bool,
    sampled_texture: Option<(&'a Texture, SamplerState)>,
    material: Material,
    light: DirectionalLight,
    camera_world: Vec3,
    sort_transparent: bool,
    blend_color_space: BlendColorSpace,
    mipmap_enabled: bool,
    mip_debug_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LinearMaterial {
    base_color: Vec4,
    specular_color: Vec3,
}

impl LinearMaterial {
    fn from_srgb(material: Material) -> Self {
        Self {
            base_color: srgb_decode_rgba(material.base_color),
            specular_color: Vec3::new(
                srgb_decode_channel(material.specular_color.x),
                srgb_decode_channel(material.specular_color.y),
                srgb_decode_channel(material.specular_color.z),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RasterDrawOptions<'a> {
    pipeline_state: PipelineState,
    uv_checker_enabled: bool,
    sampled_texture: Option<(&'a Texture, SamplerState)>,
    material: Material,
    linear_material: LinearMaterial,
    light: DirectionalLight,
    camera_world: Vec3,
    sort_transparent: bool,
    blend_color_space: BlendColorSpace,
    mipmap_enabled: bool,
    mip_debug_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PreparedTransparentTriangle {
    vertices: [ClipVertex; 3],
    triangle_id: u32,
    view_depth: f32,
}

impl<'a> FrameDrawOptions<'a> {
    fn raster(self) -> RasterDrawOptions<'a> {
        RasterDrawOptions {
            pipeline_state: self.pipeline_state,
            uv_checker_enabled: self.uv_checker_enabled,
            sampled_texture: self.sampled_texture,
            material: self.material,
            linear_material: LinearMaterial::from_srgb(self.material),
            light: self.light,
            camera_world: self.camera_world,
            sort_transparent: self.sort_transparent,
            blend_color_space: self.blend_color_space,
            mipmap_enabled: self.mipmap_enabled,
            mip_debug_enabled: self.mip_debug_enabled,
        }
    }
}

fn draw_frame(
    target: &mut RenderTarget,
    options: FrameDrawOptions<'_>,
    mesh: &Mesh,
    clipper: &mut TriangleClipper,
    transparent_scratch: &mut Vec<PreparedTransparentTriangle>,
    clip_vertices: &[ClipVertex],
    selected_vertex_index: usize,
) -> FrameDrawReport {
    match options.pipeline_state.debug_mode {
        PipelineDebugMode::Depth | PipelineDebugMode::DepthHeatmap => {
            target.clear_color(DEPTH_DEBUG_BACKGROUND);
        }
        PipelineDebugMode::Solid
        | PipelineDebugMode::Wireframe
        | PipelineDebugMode::TriangleId
        | PipelineDebugMode::Barycentric
        | PipelineDebugMode::FrontBack
        | PipelineDebugMode::Normal
        | PipelineDebugMode::NdotL
        | PipelineDebugMode::Diffuse
        | PipelineDebugMode::Specular
        | PipelineDebugMode::ColorSpaceComparison => target.render_gradient_checker(),
    }
    draw_debug_scene(
        target,
        options,
        mesh,
        clipper,
        transparent_scratch,
        clip_vertices,
        selected_vertex_index,
    )
}

fn draw_debug_scene(
    target: &mut RenderTarget,
    options: FrameDrawOptions<'_>,
    mesh: &Mesh,
    clipper: &mut TriangleClipper,
    transparent_scratch: &mut Vec<PreparedTransparentTriangle>,
    clip_vertices: &[ClipVertex],
    selected_vertex_index: usize,
) -> FrameDrawReport {
    let width = target.width() as i32;
    let height = target.height() as i32;
    let shortest_side = width.min(height);
    let white = Color::rgb(238, 244, 255);
    if !options.debug_lines_enabled {
        return draw_mesh_with_scratch(
            target,
            false,
            options.raster(),
            mesh,
            clipper,
            transparent_scratch,
            clip_vertices,
        );
    }
    if width < 16 || height < 16 {
        let mut report = draw_mesh_with_scratch(
            target,
            false,
            options.raster(),
            mesh,
            clipper,
            transparent_scratch,
            clip_vertices,
        );
        report.debug_pixels = target.draw_line_bresenham(
            ScreenPoint::new(0, 0),
            ScreenPoint::new(width - 1, height - 1),
            white,
        );
        return report;
    }

    let mut written = 0_u32;
    let grid_spacing = (shortest_side / 8).max(8) as usize;
    let grid = Color::rgb(42, 61, 84);
    for x in (grid_spacing..width as usize).step_by(grid_spacing) {
        written = written.saturating_add(target.draw_line_bresenham(
            ScreenPoint::new(x as i32, 0),
            ScreenPoint::new(x as i32, height - 1),
            grid,
        ));
    }
    for y in (grid_spacing..height as usize).step_by(grid_spacing) {
        written = written.saturating_add(target.draw_line_bresenham(
            ScreenPoint::new(0, y as i32),
            ScreenPoint::new(width - 1, y as i32),
            grid,
        ));
    }

    let mut report = draw_mesh_with_scratch(
        target,
        true,
        options.raster(),
        mesh,
        clipper,
        transparent_scratch,
        clip_vertices,
    );
    written = written.saturating_add(report.debug_pixels);
    if let Some(selected) = clip_vertices
        .get(selected_vertex_index)
        .copied()
        .and_then(|vertex| project_inside_clip(vertex.clip_pos, target.width(), target.height()))
        .map(|(_, position)| viewport_screen_point(position))
    {
        written = written.saturating_add(target.draw_point(selected, white));
    }

    let inset = (shortest_side / 32).max(2);
    written = written.saturating_add(target.draw_rect_outline(
        ScreenPoint::new(inset, inset),
        ScreenPoint::new(width - 1 - inset, height - 1 - inset),
        white,
    ));
    report.debug_pixels = written;
    report
}

#[derive(Clone, Copy)]
struct TransparencyFixture<'a> {
    opaque_mesh: &'a Mesh,
    opaque_vertices: &'a [ClipVertex],
    cutout_mesh: &'a Mesh,
    cutout_vertices: &'a [ClipVertex],
    blend_mesh: &'a Mesh,
    blend_vertices: &'a [ClipVertex],
    cutout_texture: &'a Texture,
}

fn draw_transparency_fixture(
    target: &mut RenderTarget,
    fixture: TransparencyFixture<'_>,
    clipper: &mut TriangleClipper,
    transparent_scratch: &mut Vec<PreparedTransparentTriangle>,
    alpha_cutoff: f32,
    sort_transparent: bool,
    blend_color_space: BlendColorSpace,
) -> FrameDrawReport {
    target.render_gradient_checker();
    let pipeline_state = PipelineState {
        cull_mode: CullMode::Back,
        attribute_interpolation_mode: AttributeInterpolationMode::PerspectiveCorrect,
        debug_mode: PipelineDebugMode::Solid,
    };
    let sampler = SamplerState {
        address_u: texture::AddressMode::ClampToEdge,
        address_v: texture::AddressMode::ClampToEdge,
        filter: texture::FilterMode::Nearest,
    };
    let mut report = FrameDrawReport::default();
    for (mesh, clip_vertices, sampled_texture, material) in [
        (
            fixture.opaque_mesh,
            fixture.opaque_vertices,
            None,
            Material {
                base_color: Vec4::new(0.08, 0.14, 0.32, 1.0),
                alpha_mode: AlphaMode::Opaque,
                ..Material::default()
            },
        ),
        (
            fixture.cutout_mesh,
            fixture.cutout_vertices,
            Some((fixture.cutout_texture, sampler)),
            Material {
                alpha_mode: AlphaMode::Mask,
                alpha_cutoff,
                ..Material::default()
            },
        ),
        (
            fixture.blend_mesh,
            fixture.blend_vertices,
            None,
            Material {
                alpha_mode: AlphaMode::Blend,
                ..Material::default()
            },
        ),
    ] {
        report.absorb(draw_mesh_with_scratch(
            target,
            false,
            RasterDrawOptions {
                pipeline_state,
                uv_checker_enabled: false,
                sampled_texture,
                material,
                linear_material: LinearMaterial::from_srgb(material),
                light: DirectionalLight::default(),
                camera_world: Vec3::ZERO,
                sort_transparent,
                blend_color_space,
                mipmap_enabled: false,
                mip_debug_enabled: false,
            },
            mesh,
            clipper,
            transparent_scratch,
            clip_vertices,
        ));
    }
    report
}

fn clip_position_is_finite(position: transform::ClipPosition) -> bool {
    let position = position.0;
    [position.x, position.y, position.z, position.w]
        .into_iter()
        .all(f32::is_finite)
}

fn project_inside_clip(
    position: transform::ClipPosition,
    width: usize,
    height: usize,
) -> Option<(NdcPosition, ViewportPosition)> {
    if !clip_position_is_finite(position)
        || !ClipPlane::ALL
            .into_iter()
            .all(|plane| plane.distance(position) >= 0.0)
    {
        return None;
    }
    let ndc = perspective_divide(position).ok()?;
    let viewport = viewport(ndc, width as f32, height as f32).ok()?;
    Some((ndc, viewport))
}

fn draw_mesh_with_scratch(
    target: &mut RenderTarget,
    draw_enabled: bool,
    options: RasterDrawOptions<'_>,
    mesh: &Mesh,
    clipper: &mut TriangleClipper,
    transparent_scratch: &mut Vec<PreparedTransparentTriangle>,
    clip_vertices: &[ClipVertex],
) -> FrameDrawReport {
    let mut report = FrameDrawReport::default();
    if options.material.alpha_mode == AlphaMode::Blend {
        transparent_scratch.clear();
        visit_clipped_triangles(
            mesh,
            clipper,
            clip_vertices,
            &mut report,
            |generated, triangle_id, _| {
                transparent_scratch.push(PreparedTransparentTriangle {
                    vertices: generated,
                    triangle_id,
                    view_depth: generated
                        .into_iter()
                        .map(|vertex| vertex.view_depth)
                        .sum::<f32>()
                        / 3.0,
                });
            },
        );
        if options.sort_transparent {
            transparent_scratch.sort_unstable_by(|first, second| {
                second
                    .view_depth
                    .total_cmp(&first.view_depth)
                    .then_with(|| first.triangle_id.cmp(&second.triangle_id))
            });
        }
        for prepared in transparent_scratch.iter().copied() {
            submit_generated_triangle(
                target,
                draw_enabled,
                options,
                prepared.vertices,
                prepared.triangle_id,
                &mut report,
            );
        }
        return report;
    }
    visit_clipped_triangles(
        mesh,
        clipper,
        clip_vertices,
        &mut report,
        |generated, triangle_id, report| {
            submit_generated_triangle(
                target,
                draw_enabled,
                options,
                generated,
                triangle_id,
                report,
            );
        },
    );
    report
}

#[cfg(test)]
fn draw_mesh(
    target: &mut RenderTarget,
    draw_enabled: bool,
    options: RasterDrawOptions<'_>,
    mesh: &Mesh,
    clipper: &mut TriangleClipper,
    clip_vertices: &[ClipVertex],
) -> FrameDrawReport {
    draw_mesh_with_scratch(
        target,
        draw_enabled,
        options,
        mesh,
        clipper,
        &mut Vec::new(),
        clip_vertices,
    )
}

fn visit_clipped_triangles(
    mesh: &Mesh,
    clipper: &mut TriangleClipper,
    clip_vertices: &[ClipVertex],
    report: &mut FrameDrawReport,
    mut visit: impl FnMut([ClipVertex; 3], u32, &mut FrameDrawReport),
) {
    let mut generated_triangle_id = 0_u32;
    for triangle in mesh.triangles() {
        let vertices = triangle.map(|index| clip_vertices.get(index).copied());
        let [Some(first), Some(second), Some(third)] = vertices else {
            report.clip_invalid_triangles = report.clip_invalid_triangles.saturating_add(1);
            continue;
        };
        let clipped = clipper.clip_triangle([first, second, third]);
        report.max_clip_polygon_vertices = report
            .max_clip_polygon_vertices
            .max(clipped.max_polygon_vertices as u32);
        match clipped.status {
            ClipStatus::FullyClipped => {
                report.fully_clipped_triangles = report.fully_clipped_triangles.saturating_add(1);
                continue;
            }
            ClipStatus::Invalid => {
                report.clip_invalid_triangles = report.clip_invalid_triangles.saturating_add(1);
                continue;
            }
            ClipStatus::Visible => {}
        }
        report.generated_triangles = report
            .generated_triangles
            .saturating_add(clipped.triangles.len() as u32);

        for generated in clipped.triangles {
            let triangle_id = generated_triangle_id;
            generated_triangle_id = generated_triangle_id.wrapping_add(1);
            visit(*generated, triangle_id, report);
        }
    }
}

fn submit_generated_triangle(
    target: &mut RenderTarget,
    draw_enabled: bool,
    options: RasterDrawOptions<'_>,
    generated: [ClipVertex; 3],
    triangle_id: u32,
    report: &mut FrameDrawReport,
) {
    let positions = generated.map(|vertex| {
        perspective_divide(vertex.clip_pos)
            .and_then(|position| viewport(position, target.width() as f32, target.height() as f32))
            .ok()
    });
    let [Some(first), Some(second), Some(third)] = positions else {
        report.invalid_triangles = report.invalid_triangles.saturating_add(1);
        return;
    };
    submit_triangle(
        target,
        draw_enabled,
        options,
        generated,
        [first, second, third],
        triangle_id,
        report,
    );
}

fn submit_triangle(
    target: &mut RenderTarget,
    draw_enabled: bool,
    options: RasterDrawOptions<'_>,
    generated: [ClipVertex; 3],
    positions: [ViewportPosition; 3],
    triangle_id: u32,
    report: &mut FrameDrawReport,
) {
    let (source_orientation, order) =
        match classify_triangle(positions, options.pipeline_state.cull_mode) {
            TriangleDisposition::Submit {
                source_orientation,
                order,
            } => (source_orientation, order),
            TriangleDisposition::Culled(_) => {
                report.culled_triangles = report.culled_triangles.saturating_add(1);
                return;
            }
            TriangleDisposition::Degenerate => {
                report.degenerate_triangles = report.degenerate_triangles.saturating_add(1);
                return;
            }
            TriangleDisposition::Invalid => {
                report.invalid_triangles = report.invalid_triangles.saturating_add(1);
                return;
            }
        };
    let screen_vertices = [
        ScreenVertex::from_clip_vertex(generated[0], positions[0]),
        ScreenVertex::from_clip_vertex(generated[1], positions[1]),
        ScreenVertex::from_clip_vertex(generated[2], positions[2]),
    ];
    let [Some(first), Some(second), Some(third)] = screen_vertices else {
        report.invalid_triangles = report.invalid_triangles.saturating_add(1);
        return;
    };
    let screen_vertices = [first, second, third];
    let flat_normal = if options.material.normal_mode == NormalMode::Flat {
        let first_edge = generated[1].world_pos - generated[0].world_pos;
        let second_edge = generated[2].world_pos - generated[0].world_pos;
        let Some(normal) = first_edge.cross(second_edge).normalized() else {
            report.invalid_triangles = report.invalid_triangles.saturating_add(1);
            return;
        };
        Some(normal)
    } else {
        None
    };
    let ordered_positions = order.map(|index| positions[index]);
    let setup = match TriangleSetup::new(ordered_positions, target.width(), target.height()) {
        Ok(setup) => setup,
        Err(TriangleSetupError::Degenerate) => {
            report.degenerate_triangles = report.degenerate_triangles.saturating_add(1);
            return;
        }
        Err(
            TriangleSetupError::InvalidTarget
            | TriangleSetupError::NonFinitePosition
            | TriangleSetupError::FixedPointOverflow
            | TriangleSetupError::ArithmeticOverflow
            | TriangleSetupError::BackFacing,
        ) => {
            report.invalid_triangles = report.invalid_triangles.saturating_add(1);
            return;
        }
    };
    let ordered_screen_vertices = order.map(|index| screen_vertices[index]);
    let ordered_depths = order.map(|index| positions[index].z_ndc);
    let facing_color = match source_orientation {
        FaceOrientation::Front => Color::rgb(72, 232, 112),
        FaceOrientation::Back => Color::rgb(255, 82, 92),
    };
    let mut max_barycentric_sum_error = 0.0_f32;
    let mut covered_samples = 0_u32;
    let mut shaded_samples = 0_u32;
    let mut depth_passed_samples = 0_u32;
    let mut depth_failed_samples = 0_u32;
    let mut invalid_depth_samples = 0_u32;
    let mut interpolated_inv_w_samples = 0_u32;
    let mut invalid_interpolation_samples = 0_u32;
    let mut min_interpolated_inv_w = 0.0_f32;
    let mut max_interpolated_inv_w = 0.0_f32;
    let mut invalid_values = 0_u32;
    let mut texture_samples = 0_u32;
    let mut lighting_samples = 0_u32;
    let mut alpha_discarded_samples = 0_u32;
    let mut depth_written_samples = 0_u32;
    let mut blended_samples = 0_u32;
    let mut mip_samples = 0_u32;
    let mut min_mip_level = 0_u32;
    let mut max_mip_level = 0_u32;
    let mut invalid_lod_samples = 0_u32;
    let mut sample_counter_overflow = false;
    setup.rasterize(|sample| {
        increment_sample_counter(&mut covered_samples, &mut sample_counter_overflow);
        let barycentric = setup.covered_barycentric(sample.edge_values);
        max_barycentric_sum_error = max_barycentric_sum_error.max(barycentric.sum_error());
        let depth = barycentric.interpolate_f32(ordered_depths);
        let point = ScreenPoint::new(sample.x as i32, sample.y as i32);
        match target.test_depth(point, depth) {
            DepthTestResult::Passed => {}
            DepthTestResult::Failed => {
                increment_sample_counter(&mut depth_failed_samples, &mut sample_counter_overflow);
                return;
            }
            DepthTestResult::Invalid => {
                increment_sample_counter(&mut invalid_depth_samples, &mut sample_counter_overflow);
                return;
            }
        }
        let fragment = match options.material.normal_mode {
            NormalMode::Smooth => FragmentInput::from_screen_vertices(
                barycentric,
                ordered_screen_vertices,
                options.pipeline_state.attribute_interpolation_mode,
            ),
            NormalMode::Flat => FragmentInput::from_screen_vertices_for_flat_normal(
                barycentric,
                ordered_screen_vertices,
                options.pipeline_state.attribute_interpolation_mode,
            ),
        };
        let Some(fragment) = fragment else {
            increment_sample_counter(
                &mut invalid_interpolation_samples,
                &mut sample_counter_overflow,
            );
            invalid_values = invalid_values.saturating_add(1);
            return;
        };
        let interpolated_inv_w = fragment.interpolated_inv_w();
        if interpolated_inv_w_samples == 0 {
            min_interpolated_inv_w = interpolated_inv_w;
            max_interpolated_inv_w = interpolated_inv_w;
        } else {
            min_interpolated_inv_w = min_interpolated_inv_w.min(interpolated_inv_w);
            max_interpolated_inv_w = max_interpolated_inv_w.max(interpolated_inv_w);
        }
        increment_sample_counter(
            &mut interpolated_inv_w_samples,
            &mut sample_counter_overflow,
        );
        let mip_selection = if options.mipmap_enabled {
            options.sampled_texture.map(|(texture, _)| {
                let (d_uv_dx, d_uv_dy) = observe_lod_value(
                    setup.uv_derivatives(
                        sample.edge_values,
                        ordered_screen_vertices,
                        options.pipeline_state.attribute_interpolation_mode,
                    ),
                    &mut invalid_lod_samples,
                    &mut sample_counter_overflow,
                )
                .unwrap_or((Vec2::ZERO, Vec2::ZERO));
                let lod = observe_lod_value(
                    mip_lod_from_uv_derivatives(
                        d_uv_dx,
                        d_uv_dy,
                        texture.width(),
                        texture.height(),
                    ),
                    &mut invalid_lod_samples,
                    &mut sample_counter_overflow,
                )
                .unwrap_or(0.0);
                let level = texture
                    .nearest_mip_level(lod)
                    .expect("검증된 finite LOD는 nearest mip level을 가져야 한다")
                    as u32;
                if mip_samples == 0 {
                    min_mip_level = level;
                    max_mip_level = level;
                } else {
                    min_mip_level = min_mip_level.min(level);
                    max_mip_level = max_mip_level.max(level);
                }
                increment_sample_counter(&mut mip_samples, &mut sample_counter_overflow);
                (lod, level)
            })
        } else {
            None
        };
        let shading_normal = flat_normal.unwrap_or_else(|| fragment.normal());
        let alpha_mode = options.material.alpha_mode;
        let policy_albedo = if alpha_mode == AlphaMode::Opaque {
            None
        } else {
            if options.sampled_texture.is_some() {
                increment_sample_counter(&mut texture_samples, &mut sample_counter_overflow);
            }
            Some(fragment_albedo_linear(
                fragment,
                options,
                mip_selection.map(|selection| selection.0),
            ))
        };
        let mut solid_source_linear = None;
        let mut fill_color = match options.pipeline_state.debug_mode {
            PipelineDebugMode::Solid if options.uv_checker_enabled => {
                uv_checker_color(fragment.uv())
            }
            PipelineDebugMode::Solid => {
                let albedo = if let Some(albedo) = policy_albedo {
                    albedo
                } else {
                    if options.sampled_texture.is_some() {
                        increment_sample_counter(
                            &mut texture_samples,
                            &mut sample_counter_overflow,
                        );
                    }
                    fragment_albedo_linear(
                        fragment,
                        options,
                        mip_selection.map(|selection| selection.0),
                    )
                };
                if options.material.shader_mode != ShaderMode::Unlit {
                    increment_sample_counter(&mut lighting_samples, &mut sample_counter_overflow);
                }
                let shaded = shade_material_linear(
                    albedo,
                    shading_normal,
                    fragment.world_position(),
                    options.material,
                    options.linear_material,
                    options.light,
                    options.camera_world,
                );
                solid_source_linear = Some(shaded);
                linear_display_color(shaded)
            }
            PipelineDebugMode::Diffuse => {
                let albedo = if let Some(albedo) = policy_albedo {
                    albedo
                } else {
                    if options.sampled_texture.is_some() {
                        increment_sample_counter(
                            &mut texture_samples,
                            &mut sample_counter_overflow,
                        );
                    }
                    fragment_albedo_linear(
                        fragment,
                        options,
                        mip_selection.map(|selection| selection.0),
                    )
                };
                increment_sample_counter(&mut lighting_samples, &mut sample_counter_overflow);
                let terms = lighting_terms_linear(LightingInput {
                    albedo,
                    normal_world: shading_normal,
                    fragment_world: fragment.world_position(),
                    material: options.material,
                    linear_material: options.linear_material,
                    light: options.light,
                    camera_world: options.camera_world,
                    compute_specular: false,
                });
                linear_display_color(Vec4::new(
                    terms.diffuse.x,
                    terms.diffuse.y,
                    terms.diffuse.z,
                    albedo.w,
                ))
            }
            PipelineDebugMode::Specular => {
                increment_sample_counter(&mut lighting_samples, &mut sample_counter_overflow);
                let terms = lighting_terms_linear(LightingInput {
                    albedo: Vec4::new(1.0, 1.0, 1.0, 1.0),
                    normal_world: shading_normal,
                    fragment_world: fragment.world_position(),
                    material: options.material,
                    linear_material: options.linear_material,
                    light: options.light,
                    camera_world: options.camera_world,
                    compute_specular: true,
                });
                linear_display_color(Vec4::new(
                    terms.specular.x,
                    terms.specular.y,
                    terms.specular.z,
                    1.0,
                ))
            }
            PipelineDebugMode::ColorSpaceComparison => {
                let correct = policy_albedo.unwrap_or_else(|| {
                    fragment_albedo_linear(
                        fragment,
                        options,
                        mip_selection.map(|selection| selection.0),
                    )
                });
                let wrong = fragment_albedo_encoded_wrong_way(
                    fragment,
                    options,
                    mip_selection.map(|selection| selection.0),
                );
                if policy_albedo.is_none() && options.sampled_texture.is_some() {
                    increment_sample_counter(&mut texture_samples, &mut sample_counter_overflow);
                }
                if point.x < target.width() as i32 / 2 {
                    linear_display_color(correct)
                } else {
                    debug_color(wrong)
                }
            }
            PipelineDebugMode::Wireframe => wireframe_fragment_color(fragment.barycentric()),
            PipelineDebugMode::TriangleId => triangle_id_color(triangle_id),
            PipelineDebugMode::Barycentric => debug_color(fragment.barycentric().debug_color()),
            PipelineDebugMode::Depth => depth_grayscale_color(depth),
            PipelineDebugMode::DepthHeatmap => depth_heatmap_color(depth),
            PipelineDebugMode::FrontBack => facing_color,
            PipelineDebugMode::Normal => normal_debug_color(shading_normal),
            PipelineDebugMode::NdotL => {
                increment_sample_counter(&mut lighting_samples, &mut sample_counter_overflow);
                let ndotl = lambert_ndotl(shading_normal, options.light);
                debug_color(Vec4::new(ndotl, ndotl, ndotl, 1.0))
            }
        };
        let source_alpha = policy_albedo.map_or(1.0, |albedo| albedo.w);
        if options.mip_debug_enabled {
            fill_color = mip_debug_color(mip_selection.map_or(0, |selection| selection.1));
            solid_source_linear = Some(display_color_linear(fill_color, source_alpha));
        }
        if alpha_mode == AlphaMode::Mask && source_alpha < options.material.alpha_cutoff {
            increment_sample_counter(&mut alpha_discarded_samples, &mut sample_counter_overflow);
            return;
        }
        let written = if alpha_mode == AlphaMode::Blend {
            let source = solid_source_linear
                .unwrap_or_else(|| display_color_linear(fill_color, source_alpha));
            target.blend_color_without_depth(point, source, options.blend_color_space)
        } else {
            target.commit_depth_and_color(point, depth, fill_color)
        };
        assert!(
            written,
            "통과한 depth와 clamp된 coverage sample은 alpha policy에 따라 기록되어야 한다"
        );
        increment_sample_counter(&mut depth_passed_samples, &mut sample_counter_overflow);
        increment_sample_counter(&mut shaded_samples, &mut sample_counter_overflow);
        if alpha_mode.writes_depth() {
            increment_sample_counter(&mut depth_written_samples, &mut sample_counter_overflow);
        } else {
            increment_sample_counter(&mut blended_samples, &mut sample_counter_overflow);
        }
    });
    report.submitted_triangles = report.submitted_triangles.saturating_add(1);
    report.rasterized_triangles = report.rasterized_triangles.saturating_add(1);
    report.sample_counter_overflow |= sample_counter_overflow;
    add_sample_counter(
        &mut report.covered_samples,
        covered_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.shaded_samples,
        shaded_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.depth_passed_samples,
        depth_passed_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.depth_failed_samples,
        depth_failed_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.invalid_depth_samples,
        invalid_depth_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.alpha_discarded_samples,
        alpha_discarded_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.depth_written_samples,
        depth_written_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.blended_samples,
        blended_samples,
        &mut report.sample_counter_overflow,
    );
    report.max_barycentric_sum_error = report
        .max_barycentric_sum_error
        .max(max_barycentric_sum_error);
    if interpolated_inv_w_samples > 0 {
        if report.interpolated_inv_w_samples == 0 {
            report.min_interpolated_inv_w = min_interpolated_inv_w;
            report.max_interpolated_inv_w = max_interpolated_inv_w;
        } else {
            report.min_interpolated_inv_w =
                report.min_interpolated_inv_w.min(min_interpolated_inv_w);
            report.max_interpolated_inv_w =
                report.max_interpolated_inv_w.max(max_interpolated_inv_w);
        }
        add_sample_counter(
            &mut report.interpolated_inv_w_samples,
            interpolated_inv_w_samples,
            &mut report.sample_counter_overflow,
        );
    }
    add_sample_counter(
        &mut report.invalid_interpolation_samples,
        invalid_interpolation_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.texture_samples,
        texture_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.lighting_samples,
        lighting_samples,
        &mut report.sample_counter_overflow,
    );
    if mip_samples > 0 {
        if report.mip_samples == 0 {
            report.min_mip_level = min_mip_level;
            report.max_mip_level = max_mip_level;
        } else {
            report.min_mip_level = report.min_mip_level.min(min_mip_level);
            report.max_mip_level = report.max_mip_level.max(max_mip_level);
        }
    }
    add_sample_counter(
        &mut report.mip_samples,
        mip_samples,
        &mut report.sample_counter_overflow,
    );
    add_sample_counter(
        &mut report.invalid_lod_samples,
        invalid_lod_samples,
        &mut report.sample_counter_overflow,
    );
    report.invalid_values = report.invalid_values.saturating_add(invalid_values);
    if !draw_enabled {
        return;
    }
    let screen_positions = positions.map(viewport_screen_point);
    let ordered_positions = order.map(|index| screen_positions[index]);
    let (wireframe_positions, edge_colors) = match options.pipeline_state.debug_mode {
        PipelineDebugMode::FrontBack => (ordered_positions, [facing_color; 3]),
        PipelineDebugMode::Barycentric => (
            ordered_positions,
            [
                Color::rgb(255, 0, 0),
                Color::rgb(0, 255, 0),
                Color::rgb(0, 0, 255),
            ],
        ),
        PipelineDebugMode::Solid
        | PipelineDebugMode::Wireframe
        | PipelineDebugMode::TriangleId
        | PipelineDebugMode::Depth
        | PipelineDebugMode::DepthHeatmap
        | PipelineDebugMode::Normal
        | PipelineDebugMode::NdotL
        | PipelineDebugMode::Diffuse
        | PipelineDebugMode::Specular
        | PipelineDebugMode::ColorSpaceComparison => {
            // 제출 geometry는 positive winding이지만, 기존 Bresenham 방향과 edge
            // 덮어쓰기 순서를 보존하기 위해 vertex-color wireframe은 원본 순서로 그린다.
            let colors = generated.map(|vertex| debug_color(vertex.color));
            (screen_positions, colors)
        }
    };
    report.debug_pixels = report
        .debug_pixels
        .saturating_add(target.draw_wireframe_triangle(wireframe_positions, edge_colors));
}

#[inline]
fn increment_sample_counter(counter: &mut u32, overflow: &mut bool) {
    add_sample_counter(counter, 1, overflow);
}

#[inline]
fn add_sample_counter(counter: &mut u32, amount: u32, overflow: &mut bool) {
    if let Some(sum) = counter.checked_add(amount) {
        *counter = sum;
    } else {
        *counter = u32::MAX;
        *overflow = true;
    }
}

fn observe_lod_value<T>(
    value: Option<T>,
    invalid_samples: &mut u32,
    overflow: &mut bool,
) -> Option<T> {
    if value.is_none() {
        increment_sample_counter(invalid_samples, overflow);
    }
    value
}

fn wireframe_fragment_color(barycentric: raster::BarycentricCoordinates) -> Color {
    let edge_distance = barycentric.components().into_iter().fold(1.0_f32, f32::min);
    if edge_distance <= 0.025 {
        Color::rgb(238, 244, 255)
    } else {
        DEPTH_DEBUG_BACKGROUND
    }
}

fn triangle_id_color(triangle_id: u32) -> Color {
    const PALETTE: [Color; 12] = [
        Color::rgb(239, 83, 80),
        Color::rgb(255, 167, 38),
        Color::rgb(255, 238, 88),
        Color::rgb(102, 187, 106),
        Color::rgb(38, 198, 218),
        Color::rgb(66, 165, 245),
        Color::rgb(126, 87, 194),
        Color::rgb(236, 64, 122),
        Color::rgb(141, 110, 99),
        Color::rgb(120, 144, 156),
        Color::rgb(156, 204, 101),
        Color::rgb(255, 112, 67),
    ];
    PALETTE[triangle_id as usize % PALETTE.len()]
}

fn mip_debug_color(level: u32) -> Color {
    const PALETTE: [Color; 8] = [
        Color::rgb(235, 64, 52),
        Color::rgb(255, 159, 28),
        Color::rgb(255, 214, 10),
        Color::rgb(48, 209, 88),
        Color::rgb(50, 173, 230),
        Color::rgb(10, 132, 255),
        Color::rgb(94, 92, 230),
        Color::rgb(191, 90, 242),
    ];
    PALETTE[level as usize % PALETTE.len()]
}

fn mip_lod_from_uv_derivatives(
    d_uv_dx: Vec2,
    d_uv_dy: Vec2,
    texture_width: usize,
    texture_height: usize,
) -> Option<f32> {
    let rho_x = Vec2::new(
        d_uv_dx.x * texture_width as f32,
        d_uv_dx.y * texture_height as f32,
    )
    .length();
    let rho_y = Vec2::new(
        d_uv_dy.x * texture_width as f32,
        d_uv_dy.y * texture_height as f32,
    )
    .length();
    let lod = rho_x.max(rho_y).max(f32::EPSILON).log2().max(0.0);
    lod.is_finite().then_some(lod)
}

fn uv_checker_color(uv: Vec2) -> Color {
    let cell_x = (uv.x * 8.0).floor() as i32;
    let cell_y = (uv.y * 8.0).floor() as i32;
    if (cell_x + cell_y).rem_euclid(2) == 0 {
        Color::rgb(242, 246, 255)
    } else {
        Color::rgb(34, 75, 132)
    }
}

fn debug_color(color: Vec4) -> Color {
    Color::rgb(
        normalized_channel_to_u8(color.x),
        normalized_channel_to_u8(color.y),
        normalized_channel_to_u8(color.z),
    )
}

fn modulate_color(first: Vec4, second: Vec4) -> Vec4 {
    Vec4::new(
        first.x * second.x,
        first.y * second.y,
        first.z * second.z,
        first.w * second.w,
    )
}

fn fragment_albedo_linear(
    fragment: FragmentInput,
    options: RasterDrawOptions<'_>,
    mip_lod: Option<f32>,
) -> Vec4 {
    let texture = options
        .sampled_texture
        .map(|(texture, sampler)| {
            if let Some(lod) = mip_lod {
                sampler
                    .sample_mip(texture, fragment.uv(), lod)
                    .map(|sample| sample.0)
            } else {
                sampler.sample(texture, fragment.uv())
            }
            .expect("FragmentInput과 계산된 LOD는 유한해야 한다")
        })
        .unwrap_or(Vec4::new(1.0, 1.0, 1.0, 1.0));
    modulate_color(
        modulate_color(texture, fragment.color()),
        options.linear_material.base_color,
    )
}

fn fragment_albedo_encoded_wrong_way(
    fragment: FragmentInput,
    options: RasterDrawOptions<'_>,
    mip_lod: Option<f32>,
) -> Vec4 {
    let texture = options
        .sampled_texture
        .map(|(texture, sampler)| {
            if let Some(lod) = mip_lod {
                sampler
                    .sample_mip_encoded(texture, fragment.uv(), lod)
                    .map(|sample| sample.0)
            } else {
                sampler.sample_encoded(texture, fragment.uv())
            }
            .expect("FragmentInput과 계산된 LOD는 유한해야 한다")
        })
        .unwrap_or(Vec4::new(1.0, 1.0, 1.0, 1.0));
    modulate_color(
        modulate_color(texture, fragment.color()),
        options.material.base_color,
    )
}

fn linear_display_color(linear: Vec4) -> Color {
    debug_color(srgb_encode_rgba(linear))
}

fn display_color_linear(color: Color, alpha: f32) -> Vec4 {
    srgb_decode_rgba(Vec4::new(
        f32::from(color.red) / 255.0,
        f32::from(color.green) / 255.0,
        f32::from(color.blue) / 255.0,
        alpha,
    ))
}

pub fn lambert_ndotl(normal_world: Vec3, light: DirectionalLight) -> f32 {
    normal_world.dot(light.surface_to_light).max(0.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LightingTerms {
    ambient: Vec3,
    diffuse: Vec3,
    specular: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LightingInput {
    albedo: Vec4,
    normal_world: Vec3,
    fragment_world: Vec3,
    material: Material,
    linear_material: LinearMaterial,
    light: DirectionalLight,
    camera_world: Vec3,
    compute_specular: bool,
}

pub fn blinn_phong_specular_factor(
    normal_world: Vec3,
    surface_to_light: Vec3,
    surface_to_camera: Vec3,
    shininess: f32,
) -> f32 {
    let ndotl = normal_world.dot(surface_to_light).max(0.0);
    if ndotl <= 0.0 || !shininess.is_finite() || shininess <= 0.0 {
        return 0.0;
    }
    let Some(half_vector) = (surface_to_light + surface_to_camera).normalized() else {
        return 0.0;
    };
    normal_world.dot(half_vector).max(0.0).powf(shininess)
}

fn lighting_terms_linear(input: LightingInput) -> LightingTerms {
    let ndotl = lambert_ndotl(input.normal_world, input.light);
    let specular_factor = if input.compute_specular {
        (input.camera_world - input.fragment_world)
            .normalized()
            .map_or(0.0, |surface_to_camera| {
                blinn_phong_specular_factor(
                    input.normal_world,
                    input.light.surface_to_light,
                    surface_to_camera,
                    input.material.shininess,
                )
            })
    } else {
        0.0
    };
    LightingTerms {
        ambient: Vec3::new(
            input.albedo.x * input.material.ambient,
            input.albedo.y * input.material.ambient,
            input.albedo.z * input.material.ambient,
        ),
        diffuse: Vec3::new(
            input.albedo.x * input.light.color.x * input.light.intensity * ndotl,
            input.albedo.y * input.light.color.y * input.light.intensity * ndotl,
            input.albedo.z * input.light.color.z * input.light.intensity * ndotl,
        ),
        specular: Vec3::new(
            input.linear_material.specular_color.x
                * input.light.color.x
                * input.light.intensity
                * specular_factor,
            input.linear_material.specular_color.y
                * input.light.color.y
                * input.light.intensity
                * specular_factor,
            input.linear_material.specular_color.z
                * input.light.color.z
                * input.light.intensity
                * specular_factor,
        ),
    }
}

fn shade_material_linear(
    albedo: Vec4,
    normal_world: Vec3,
    fragment_world: Vec3,
    material: Material,
    linear_material: LinearMaterial,
    light: DirectionalLight,
    camera_world: Vec3,
) -> Vec4 {
    if material.shader_mode == ShaderMode::Unlit {
        return albedo;
    }
    let terms = lighting_terms_linear(LightingInput {
        albedo,
        normal_world,
        fragment_world,
        material,
        linear_material,
        light,
        camera_world,
        compute_specular: material.shader_mode == ShaderMode::BlinnPhong,
    });
    let specular = if material.shader_mode == ShaderMode::BlinnPhong {
        terms.specular
    } else {
        Vec3::ZERO
    };
    Vec4::new(
        terms.ambient.x + terms.diffuse.x + specular.x,
        terms.ambient.y + terms.diffuse.y + specular.y,
        terms.ambient.z + terms.diffuse.z + specular.z,
        albedo.w,
    )
}

fn normal_debug_color(normal_world: Vec3) -> Color {
    debug_color(Vec4::new(
        normal_world.x * 0.5 + 0.5,
        normal_world.y * 0.5 + 0.5,
        normal_world.z * 0.5 + 0.5,
        1.0,
    ))
}

fn depth_grayscale_color(depth: f32) -> Color {
    let channel = normalized_channel_to_u8(depth);
    Color::rgb(channel, channel, channel)
}

fn depth_heatmap_color(depth: f32) -> Color {
    let depth = depth.clamp(0.0, 1.0);
    debug_color(Vec4::new(
        (2.0 * depth - 1.0).max(0.0),
        1.0 - (2.0 * depth - 1.0).abs(),
        (1.0 - 2.0 * depth).max(0.0),
        1.0,
    ))
}

fn viewport_screen_point(position: ViewportPosition) -> ScreenPoint {
    ScreenPoint::new(position.x.round() as i32, position.y.round() as i32)
}

fn checked_buffer_lengths(
    width: usize,
    height: usize,
) -> Result<(usize, usize), RenderTargetError> {
    if width == 0 || height == 0 {
        return Err(RenderTargetError::ZeroDimension);
    }
    let pixel_count = width
        .checked_mul(height)
        .ok_or(RenderTargetError::DimensionOverflow)?;
    if pixel_count > MAX_PIXEL_COUNT {
        return Err(RenderTargetError::PixelLimitExceeded {
            requested: pixel_count,
            maximum: MAX_PIXEL_COUNT,
        });
    }
    let color_byte_count = pixel_count
        .checked_mul(4)
        .ok_or(RenderTargetError::DimensionOverflow)?;
    Ok((pixel_count, color_byte_count))
}

fn sanitize_dt(dt_seconds: f32) -> (f32, bool) {
    if dt_seconds.is_finite() {
        (dt_seconds.clamp(0.0, MAX_FRAME_DT_SECONDS), false)
    } else {
        (0.0, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1.0e-5,
            "expected {expected}, got {actual}"
        );
    }

    fn pixel(target: &RenderTarget, x: usize, y: usize) -> [u8; 4] {
        let byte_index = 4 * (y * target.width() + x);
        target.color()[byte_index..byte_index + 4]
            .try_into()
            .expect("pixel slice should have four bytes")
    }

    fn fnv1a(bytes: &[u8]) -> u32 {
        bytes.iter().fold(0x811c_9dc5, |hash, byte| {
            (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
        })
    }

    fn render_perspective_fixture(
        alternate_diagonal: bool,
        mode: AttributeInterpolationMode,
    ) -> (RenderTarget, FrameDrawReport) {
        let mesh = perspective_debug_fixture(alternate_diagonal);
        let scene = MeshScene::new_perspective_debug(&mesh, 64, 64);
        let mut target = RenderTarget::new(64, 64).unwrap();
        target.render_gradient_checker();
        let mut clipper = TriangleClipper::default();
        let report = draw_mesh(
            &mut target,
            false,
            RasterDrawOptions {
                pipeline_state: PipelineState {
                    cull_mode: CullMode::Back,
                    attribute_interpolation_mode: mode,
                    debug_mode: PipelineDebugMode::Solid,
                },
                uv_checker_enabled: true,
                sampled_texture: None,
                material: Material::default(),
                linear_material: LinearMaterial::from_srgb(Material::default()),
                light: DirectionalLight::default(),
                camera_world: Vec3::ZERO,
                sort_transparent: true,
                blend_color_space: BlendColorSpace::Linear,
                mipmap_enabled: false,
                mip_debug_enabled: false,
            },
            &mesh,
            &mut clipper,
            &scene.clip_vertices,
        );
        (target, report)
    }

    fn viewport_point(x: f32, y: f32) -> ViewportPosition {
        ViewportPosition { x, y, z_ndc: 0.5 }
    }

    fn raster_options(
        cull_mode: CullMode,
        winding_debug_mode: WindingDebugMode,
    ) -> RasterDrawOptions<'static> {
        RasterDrawOptions {
            pipeline_state: PipelineState {
                cull_mode,
                attribute_interpolation_mode: AttributeInterpolationMode::PerspectiveCorrect,
                debug_mode: match winding_debug_mode {
                    WindingDebugMode::VertexColor => PipelineDebugMode::Solid,
                    WindingDebugMode::Facing => PipelineDebugMode::FrontBack,
                    WindingDebugMode::Barycentric => PipelineDebugMode::Barycentric,
                },
            },
            uv_checker_enabled: false,
            sampled_texture: None,
            material: Material::default(),
            linear_material: LinearMaterial::from_srgb(Material::default()),
            light: DirectionalLight::default(),
            camera_world: Vec3::ZERO,
            sort_transparent: true,
            blend_color_space: BlendColorSpace::Linear,
            mipmap_enabled: false,
            mip_debug_enabled: false,
        }
    }

    fn submitted_orientation(disposition: TriangleDisposition) -> FaceOrientation {
        match disposition {
            TriangleDisposition::Submit {
                source_orientation, ..
            } => source_orientation,
            rejected => panic!("triangle 제출을 기대했지만 {rejected:?}였다"),
        }
    }

    #[test]
    fn target_has_expected_lengths_opaque_clear_and_depth_values() {
        let mut target = RenderTarget::new(3, 2).expect("3x2 target should be valid");
        assert_eq!((target.width(), target.height()), (3, 2));
        assert_eq!(target.color().len(), 24);
        assert_eq!(target.depth().len(), 6);

        let color = Color::rgb(7, 11, 13);
        assert_eq!(color.rgba(), [7, 11, 13, 255]);
        target.clear_color(color);
        assert!(
            target
                .color()
                .chunks_exact(4)
                .all(|pixel| pixel == [7, 11, 13, 255])
        );
        assert!(target.depth().iter().all(|depth| *depth == f32::INFINITY));
    }

    #[test]
    fn depth_test_uses_finite_zero_to_one_strict_less_and_epsilon_clamp() {
        let mut target = RenderTarget::new(2, 1).unwrap();
        let first = ScreenPoint::new(0, 0);
        assert_eq!(target.test_depth(first, 1.0), DepthTestResult::Passed);
        assert!(target.depth()[0].is_infinite());
        assert!(target.commit_depth_and_color(first, 1.0, Color::rgb(1, 2, 3)));
        assert_eq!(target.depth()[0], 1.0);
        assert_eq!(target.test_depth(first, 1.0), DepthTestResult::Failed);
        assert!(!target.commit_depth_and_color(first, 1.0, Color::rgb(4, 5, 6)));
        assert_eq!(target.test_depth(first, 0.25), DepthTestResult::Passed);
        assert!(target.commit_depth_and_color(first, 0.25, Color::rgb(7, 8, 9)));
        assert_eq!(target.test_depth(first, 0.75), DepthTestResult::Failed);
        assert_eq!(target.depth()[0], 0.25);
        assert_eq!(&target.color()[..4], &[7, 8, 9, 255]);

        let second = ScreenPoint::new(1, 0);
        assert_eq!(
            target.test_depth(second, -DEPTH_RANGE_EPSILON / 2.0),
            DepthTestResult::Passed
        );
        assert!(target.commit_depth_and_color(
            second,
            -DEPTH_RANGE_EPSILON / 2.0,
            Color::rgb(10, 11, 12),
        ));
        assert_eq!(target.depth()[1], 0.0);
        target.clear_color(Color::rgb(1, 2, 3));
        assert!(target.depth().iter().all(|depth| depth.is_infinite()));
        assert_eq!(
            target.test_depth(second, 1.0 + DEPTH_RANGE_EPSILON / 2.0),
            DepthTestResult::Passed
        );
        assert!(target.commit_depth_and_color(
            second,
            1.0 + DEPTH_RANGE_EPSILON / 2.0,
            Color::rgb(13, 14, 15),
        ));
        assert_eq!(target.depth()[1], 1.0);

        let depth_before_invalid = target.depth().to_vec();
        for (point, candidate) in [
            (first, f32::NAN),
            (first, f32::INFINITY),
            (first, -2.0 * DEPTH_RANGE_EPSILON),
            (first, 1.0 + 2.0 * DEPTH_RANGE_EPSILON),
            (ScreenPoint::new(-1, 0), 0.5),
            (ScreenPoint::new(2, 0), 0.5),
        ] {
            assert_eq!(
                target.test_depth(point, candidate),
                DepthTestResult::Invalid
            );
            assert!(!target.commit_depth_and_color(point, candidate, Color::rgb(16, 17, 18)));
        }
        assert_eq!(target.depth(), depth_before_invalid);
    }

    #[test]
    fn dimensions_reject_zero_overflow_and_excessive_targets() {
        assert_eq!(
            RenderTarget::new(0, 1).unwrap_err(),
            RenderTargetError::ZeroDimension
        );
        assert_eq!(
            RenderTarget::new(1, 0).unwrap_err(),
            RenderTargetError::ZeroDimension
        );
        assert_eq!(
            RenderTarget::new(usize::MAX, 2).unwrap_err(),
            RenderTargetError::DimensionOverflow
        );

        let error = RenderTarget::new(MAX_PIXEL_COUNT + 1, 1).unwrap_err();
        assert_eq!(
            error,
            RenderTargetError::PixelLimitExceeded {
                requested: MAX_PIXEL_COUNT + 1,
                maximum: MAX_PIXEL_COUNT,
            }
        );
        assert!(error.to_string().contains("최대 허용치"));
        assert!(
            RenderTargetError::DimensionOverflow
                .to_string()
                .contains("overflow")
        );
        assert!(
            RenderTargetError::ZeroDimension
                .to_string()
                .contains("0보다")
        );
    }

    #[test]
    fn two_by_two_gradient_checker_fixes_rgba_channel_and_row_order() {
        let mut target = RenderTarget::new(2, 2).expect("2x2 target should be valid");
        target.render_gradient_checker();
        assert_eq!(
            target.color(),
            [
                0, 0, 220, 255, 255, 0, 220, 255, 0, 255, 220, 255, 255, 255, 220, 255,
            ]
        );
    }

    #[test]
    fn gradient_checker_handles_one_pixel_odd_and_wide_targets() {
        let mut one = RenderTarget::new(1, 1).expect("1x1 target should be valid");
        one.render_gradient_checker();
        assert_eq!(one.color(), [0, 0, 220, 255]);

        let mut odd = RenderTarget::new(17, 17).expect("odd target should be valid");
        odd.render_gradient_checker();
        assert_eq!(pixel(&odd, 0, 0), [0, 0, 220, 255]);
        assert_eq!(pixel(&odd, 8, 0), [128, 0, 40, 255]);
        assert_eq!(pixel(&odd, 0, 8), [0, 128, 40, 255]);
        assert_eq!(pixel(&odd, 8, 8), [128, 128, 220, 255]);
        assert_eq!(pixel(&odd, 16, 16), [255, 255, 220, 255]);

        let mut wide = RenderTarget::new(257, 1).expect("wide target should be valid");
        wide.render_gradient_checker();
        assert_eq!(pixel(&wide, 0, 0), [0, 0, 220, 255]);
        assert_eq!(pixel(&wide, 256, 0), [255, 0, 220, 255]);
        assert!(wide.depth().iter().all(|depth| *depth == f32::INFINITY));
    }

    #[test]
    fn safe_pixel_write_accepts_inside_and_rejects_every_outside_direction() {
        let mut target = RenderTarget::new(3, 2).expect("target should be valid");
        target.clear_color(Color::rgb(0, 0, 0));
        let white = Color::rgb(255, 255, 255);
        assert!(target.put_pixel(ScreenPoint::new(2, 1), white));
        assert_eq!(pixel(&target, 2, 1), [255, 255, 255, 255]);
        assert!(!target.put_pixel(ScreenPoint::new(-1, 0), white));
        assert!(!target.put_pixel(ScreenPoint::new(0, -1), white));
        assert!(!target.put_pixel(ScreenPoint::new(3, 0), white));
        assert!(!target.put_pixel(ScreenPoint::new(0, 2), white));
    }

    #[test]
    fn bresenham_includes_endpoints_and_connects_all_octants() {
        let start = ScreenPoint::new(10, 10);
        let offsets = [
            (5, 0),
            (5, 2),
            (2, 5),
            (0, 5),
            (-2, 5),
            (-5, 2),
            (-5, 0),
            (-5, -2),
            (-2, -5),
            (0, -5),
            (2, -5),
            (5, -2),
        ];
        for (offset_x, offset_y) in offsets {
            let end = ScreenPoint::new(start.x + offset_x, start.y + offset_y);
            let mut points = Vec::new();
            let count = walk_bresenham(start, end, |point| {
                points.push(point);
                true
            });
            assert_eq!(count as usize, points.len());
            assert_eq!(points.first(), Some(&start));
            assert_eq!(points.last(), Some(&end));
            assert_eq!(
                points.len(),
                offset_x.unsigned_abs().max(offset_y.unsigned_abs()) as usize + 1
            );
            assert!(points.windows(2).all(|pair| {
                let dx = (pair[1].x - pair[0].x).abs();
                let dy = (pair[1].y - pair[0].y).abs();
                dx <= 1 && dy <= 1 && dx + dy >= 1
            }));
        }

        let mut point = Vec::new();
        assert_eq!(
            walk_bresenham(start, start, |value| {
                point.push(value);
                true
            }),
            1
        );
        assert_eq!(point, [start]);
    }

    #[test]
    fn debug_helpers_clip_writes_safely_and_keep_wireframe_vertices_connected() {
        let mut target = RenderTarget::new(7, 7).expect("target should be valid");
        target.clear_color(Color::rgb(0, 0, 0));
        let white = Color::rgb(255, 255, 255);
        assert_eq!(target.draw_point(ScreenPoint::new(-1, -1), white), 0);
        assert_eq!(
            target.draw_line_bresenham(ScreenPoint::new(-2, 2), ScreenPoint::new(2, 2), white,),
            3
        );
        assert_eq!(
            target.draw_line_bresenham(ScreenPoint::new(-3, -3), ScreenPoint::new(-1, -1), white,),
            0
        );
        assert_eq!(
            target.draw_rect_outline(
                ScreenPoint::new(5, 5),
                ScreenPoint::new(1, 1),
                Color::rgb(200, 100, 50),
            ),
            20
        );
        let triangle = [
            ScreenPoint::new(1, 5),
            ScreenPoint::new(3, 1),
            ScreenPoint::new(5, 5),
        ];
        assert_eq!(
            target.draw_wireframe_triangle(
                triangle,
                [
                    Color::rgb(255, 0, 0),
                    Color::rgb(0, 255, 0),
                    Color::rgb(0, 0, 255),
                ],
            ),
            15
        );
        assert_eq!(pixel(&target, 2, 4), [255, 0, 0, 255]);
        assert_eq!(pixel(&target, 4, 2), [0, 255, 0, 255]);
        assert_eq!(pixel(&target, 4, 5), [0, 0, 255, 255]);
        assert_eq!(pixel(&target, 1, 5), [0, 0, 255, 255]);
        assert_eq!(pixel(&target, 3, 1), [0, 255, 0, 255]);
        assert_eq!(pixel(&target, 5, 5), [0, 0, 255, 255]);
    }

    #[test]
    fn resize_is_atomic_and_same_size_preserves_allocations() {
        let mut renderer = Renderer::new(3, 2).expect("renderer should be valid");
        renderer.clear([1, 2, 3]);
        let color_pointer = renderer.color_buffer().as_ptr();
        let depth_pointer = renderer.depth_buffer().as_ptr();

        renderer
            .resize(3, 2)
            .expect("same-size resize should succeed");
        assert_eq!(renderer.color_buffer().as_ptr(), color_pointer);
        assert_eq!(renderer.depth_buffer().as_ptr(), depth_pointer);
        assert_eq!(renderer.color_buffer()[..4], [1, 2, 3, 255]);
        assert_eq!(renderer.framebuffer_generation(), 0);

        let error = renderer.resize(MAX_PIXEL_COUNT + 1, 1).unwrap_err();
        assert!(matches!(
            error,
            RenderTargetError::PixelLimitExceeded { .. }
        ));
        assert_eq!((renderer.width(), renderer.height()), (3, 2));
        assert_eq!(renderer.color_buffer()[..4], [1, 2, 3, 255]);
        assert_eq!(renderer.framebuffer_generation(), 0);

        renderer.resize(2, 4).expect("resize should succeed");
        assert_eq!((renderer.width(), renderer.height()), (2, 4));
        assert_eq!(renderer.color_buffer().len(), 32);
        assert_eq!(renderer.depth_buffer().len(), 8);
        assert!(
            renderer
                .depth_buffer()
                .iter()
                .any(|depth| depth.is_finite())
        );
        assert!(
            renderer
                .depth_buffer()
                .iter()
                .all(|depth| { depth.is_infinite() || (0.0..=1.0).contains(depth) })
        );
        assert_eq!(renderer.framebuffer_generation(), 1);
    }

    #[test]
    fn frame_clamps_dt_preserves_buffers_and_resets_debug_count_when_disabled() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        let color_pointer = renderer.color_buffer().as_ptr();
        let depth_pointer = renderer.depth_buffer().as_ptr();
        let trace_cache_pointer = renderer.mesh_scene.traces.as_ptr();
        let clip_cache_pointer = renderer.mesh_scene.clip_vertices.as_ptr();
        let viewport_cache_pointer = renderer.mesh_scene.diagnostic_viewport_positions.as_ptr();

        renderer.set_debug_lines_enabled(true);
        let first = renderer.update_and_render(
            0.25,
            InputSnapshot::new([0x25, 0, 0], Vec2::ZERO, 0.0, 0, 0).unwrap(),
        );
        assert_eq!(first.frame_index, 1);
        assert_eq!(first.dt_seconds, 0.1);
        assert_eq!(first.input_bits, 0x25);
        assert_eq!(first.input_vertices, 24);
        assert_eq!(first.input_triangles, 12);
        assert_eq!(first.transformed_vertices, 24);
        assert_eq!(first.submitted_triangles, 4);
        assert_eq!(first.culled_triangles, 8);
        assert_eq!(first.degenerate_triangles, 0);
        assert_eq!(first.invalid_triangles, 0);
        assert_eq!(first.fully_clipped_triangles, 0);
        assert_eq!(first.clip_invalid_triangles, 0);
        assert_eq!(first.generated_triangles, 12);
        assert_eq!(first.max_clip_polygon_vertices, 3);
        assert_eq!(first.rasterized_triangles, 4);
        assert_eq!(first.shaded_samples, 875);
        assert!(first.max_barycentric_sum_error <= 2.0 * f32::EPSILON);
        assert!(first.debug_pixels > 0);
        assert_eq!(first.invalid_values, 0);
        assert_eq!(renderer.stats(), first);
        assert_eq!(renderer.color_buffer().as_ptr(), color_pointer);
        assert_eq!(renderer.depth_buffer().as_ptr(), depth_pointer);
        assert_eq!(renderer.mesh_scene.traces.as_ptr(), trace_cache_pointer);
        assert_eq!(
            renderer.mesh_scene.clip_vertices.as_ptr(),
            clip_cache_pointer
        );
        assert_eq!(
            renderer.mesh_scene.diagnostic_viewport_positions.as_ptr(),
            viewport_cache_pointer
        );

        renderer.set_debug_lines_enabled(false);
        let negative = renderer.update_and_render(-1.0, InputSnapshot::default());
        assert_eq!(negative.dt_seconds, 0.0);
        assert_eq!(negative.submitted_triangles, 4);
        assert_eq!(negative.culled_triangles, 8);
        assert_eq!(negative.rasterized_triangles, 4);
        assert_eq!(negative.shaded_samples, first.shaded_samples);
        assert_eq!(negative.debug_pixels, 0);
        assert_eq!(negative.invalid_values, 0);
        assert_eq!(renderer.color_buffer()[..4], [0, 0, 220, 255]);
        for invalid_dt in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let stats = renderer.update_and_render(invalid_dt, InputSnapshot::default());
            assert_eq!(stats.dt_seconds, 0.0);
            assert_eq!(stats.debug_pixels, 0);
            assert_eq!(stats.invalid_values, 1);
        }
        assert_eq!(renderer.color_buffer().as_ptr(), color_pointer);
        assert_eq!(renderer.depth_buffer().as_ptr(), depth_pointer);

        let mut tiny = Renderer::new(1, 1).expect("tiny renderer should be valid");
        assert_eq!(tiny.color_buffer(), [255, 160, 137, 255]);
        tiny.set_debug_lines_enabled(true);
        assert!(
            tiny.update_and_render(0.0, InputSnapshot::default())
                .debug_pixels
                > 0
        );
        assert_eq!(tiny.color_buffer(), [238, 244, 255, 255]);
        tiny.set_debug_lines_enabled(false);
        assert_eq!(
            tiny.update_and_render(0.0, InputSnapshot::default())
                .debug_pixels,
            0
        );
        assert_eq!(tiny.color_buffer(), [255, 160, 137, 255]);
    }

    #[test]
    fn chapter_thirteen_double_sided_depth_coverage_matches_64_by_64_golden_hash() {
        let mut renderer = Renderer::new(64, 64).expect("golden renderer should be valid");
        renderer.set_cull_mode(CullMode::None);
        renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(fnv1a(renderer.color_buffer()), 0xf8d0_50be);
    }

    #[test]
    fn chapter_eleven_backface_culled_flat_coverage_matches_64_by_64_golden_hash() {
        let renderer = Renderer::new(64, 64).expect("golden renderer should be valid");
        assert_eq!(fnv1a(renderer.color_buffer()), 0xf8d0_50be);
    }

    #[test]
    fn chapter_ten_near_corner_fixture_clips_before_divide_and_fans_the_polygon() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        renderer.set_debug_lines_enabled(true);
        renderer.set_cull_mode(CullMode::None);
        renderer.set_clip_debug_enabled(true);
        let stats = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(stats.input_vertices, 3);
        assert_eq!(stats.input_triangles, 1);
        assert_eq!(stats.transformed_vertices, 3);
        assert_eq!(stats.fully_clipped_triangles, 0);
        assert_eq!(stats.clip_invalid_triangles, 0);
        assert_eq!(stats.generated_triangles, 3);
        assert_eq!(stats.max_clip_polygon_vertices, 5);
        assert_eq!(
            stats.generated_triangles,
            stats.submitted_triangles
                + stats.culled_triangles
                + stats.degenerate_triangles
                + stats.invalid_triangles
        );
        assert_eq!(stats.submitted_triangles, stats.generated_triangles);
        assert!(stats.debug_pixels > 0);
        assert_eq!(stats.invalid_values, 0);
        let snapshot = renderer.coordinate_debug_snapshot();
        assert_eq!(snapshot.selected_vertex_index, 2);
        assert_eq!(
            snapshot.selected_vertex.object_pos.0,
            Vec3::new(-0.25, -0.25, 0.5)
        );
        assert_eq!(
            snapshot.selected_vertex.world_pos.0,
            Vec4::new(-0.25, -0.25, 0.5, 1.0)
        );
        assert_eq!(
            snapshot.selected_vertex.view_pos.0,
            snapshot.selected_vertex.world_pos.0
        );
        assert_eq!(
            snapshot.selected_vertex.clip_pos.0,
            snapshot.selected_vertex.world_pos.0
        );
        assert_eq!(
            snapshot.selected_ndc.unwrap().0,
            Vec3::new(-0.25, -0.25, 0.5)
        );
        assert_eq!(
            snapshot.selected_viewport.unwrap(),
            ViewportPosition {
                x: 24.0,
                y: 40.0,
                z_ndc: 0.5,
            }
        );
        assert_eq!(snapshot.projection_failures, 0);
        assert_eq!(snapshot.mesh_vertices, 3);
        assert_eq!(snapshot.mesh_indices, 3);
        assert_eq!(snapshot.mesh_triangles, 1);
        assert_eq!(snapshot.rotation_y_radians, 0.0);
        assert_eq!(fnv1a(renderer.color_buffer()), 0xb646_3359);

        renderer
            .resize(128, 64)
            .expect("wide resize should succeed");
        let resized = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(resized.generated_triangles, stats.generated_triangles);
        assert_eq!(
            resized.max_clip_polygon_vertices,
            stats.max_clip_polygon_vertices
        );
    }

    #[test]
    fn chapter_eleven_quad_uses_one_top_left_owner_per_pixel_center() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        renderer.set_debug_lines_enabled(false);
        renderer.set_coverage_debug_enabled(true);
        let stats = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(stats.input_vertices, 6);
        assert_eq!(stats.input_triangles, 2);
        assert_eq!(stats.transformed_vertices, 6);
        assert_eq!(stats.generated_triangles, 2);
        assert_eq!(stats.submitted_triangles, 2);
        assert_eq!(stats.culled_triangles, 0);
        assert_eq!(stats.degenerate_triangles, 0);
        assert_eq!(stats.invalid_triangles, 0);
        assert_eq!(stats.rasterized_triangles, 2);
        assert_eq!(stats.shaded_samples, 32 * 32);
        assert_eq!(stats.depth_passed_samples, 32 * 32);
        assert_eq!(stats.depth_failed_samples, 0);
        assert_eq!(stats.invalid_depth_samples, 0);
        assert!(stats.max_barycentric_sum_error <= 2.0 * f32::EPSILON);
        assert_eq!(stats.debug_pixels, 0);
        assert_eq!(
            renderer
                .depth_buffer()
                .iter()
                .filter(|depth| **depth == 0.5)
                .count(),
            32 * 32
        );

        let orange = [255, 160, 108, 255];
        let cyan = [108, 225, 255, 255];
        let orange_count = renderer
            .color_buffer()
            .chunks_exact(4)
            .filter(|pixel| *pixel == orange)
            .count();
        let cyan_count = renderer
            .color_buffer()
            .chunks_exact(4)
            .filter(|pixel| *pixel == cyan)
            .count();
        assert_eq!((orange_count, cyan_count), (528, 496));
        assert_eq!(orange_count + cyan_count, stats.shaded_samples as usize);
        assert_eq!(fnv1a(renderer.color_buffer()), 0xf618_1515);

        let snapshot = renderer.coordinate_debug_snapshot();
        assert!(!snapshot.clip_debug_enabled);
        assert!(snapshot.coverage_debug_enabled);
        assert_eq!(snapshot.selected_vertex_index, 0);
        assert_eq!(
            snapshot.selected_viewport.unwrap(),
            ViewportPosition {
                x: 16.0,
                y: 16.0,
                z_ndc: 0.5,
            }
        );

        renderer
            .resize(128, 64)
            .expect("active coverage fixture resize should succeed");
        assert_eq!(renderer.framebuffer_generation(), 1);
        let resized = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(resized.input_triangles, 2);
        assert_eq!(resized.rasterized_triangles, 2);
        assert_eq!(resized.shaded_samples, 64 * 32);
        let resized_snapshot = renderer.coordinate_debug_snapshot();
        assert!(resized_snapshot.coverage_debug_enabled);
        assert_eq!(
            resized_snapshot.selected_viewport.unwrap(),
            ViewportPosition {
                x: 32.0,
                y: 16.0,
                z_ndc: 0.5,
            }
        );

        renderer.set_clip_debug_enabled(true);
        let clipping = renderer.coordinate_debug_snapshot();
        assert!(clipping.clip_debug_enabled);
        assert!(!clipping.coverage_debug_enabled);
    }

    #[test]
    fn chapter_twelve_rgb_triangle_interpolates_affine_color_and_barycentric_debug() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        renderer.set_debug_lines_enabled(false);
        renderer.set_interpolation_debug_enabled(true);
        let affine = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(affine.input_vertices, 3);
        assert_eq!(affine.input_triangles, 1);
        assert_eq!(affine.transformed_vertices, 3);
        assert_eq!(affine.generated_triangles, 1);
        assert_eq!(affine.submitted_triangles, 1);
        assert_eq!(affine.rasterized_triangles, 1);
        assert_eq!(affine.shaded_samples, 882);
        assert_eq!(affine.depth_passed_samples, 882);
        assert_eq!(affine.depth_failed_samples, 0);
        assert_eq!(affine.invalid_depth_samples, 0);
        assert_eq!(affine.max_barycentric_sum_error, f32::EPSILON);
        assert_eq!(affine.debug_pixels, 0);
        assert_eq!(
            renderer
                .depth_buffer()
                .iter()
                .filter(|depth| depth.is_finite())
                .count(),
            affine.shaded_samples as usize
        );
        assert!(
            renderer
                .depth_buffer()
                .iter()
                .filter(|depth| depth.is_finite())
                .all(|depth| (*depth - 0.5).abs() <= f32::EPSILON)
        );

        let near_red = pixel(&renderer.target, 13, 13);
        let near_green = pixel(&renderer.target, 50, 13);
        let near_blue = pixel(&renderer.target, 32, 49);
        let centroid = pixel(&renderer.target, 32, 25);
        assert!(near_red[0] > near_red[1] && near_red[0] > near_red[2]);
        assert!(near_green[1] > near_green[0] && near_green[1] > near_green[2]);
        assert!(near_blue[2] > near_blue[0] && near_blue[2] > near_blue[1]);
        let centroid_min = centroid[..3].iter().copied().min().unwrap();
        let centroid_max = centroid[..3].iter().copied().max().unwrap();
        assert!(centroid_max - centroid_min <= 8);
        assert_eq!(
            [near_red, near_green, near_blue, centroid],
            [
                [245, 46, 67, 255],
                [46, 245, 67, 255],
                [46, 64, 246, 255],
                [152, 158, 158, 255],
            ]
        );
        let affine_hash = fnv1a(renderer.color_buffer());
        assert_eq!(affine_hash, 0x768d_4242);

        renderer.set_winding_debug_mode(WindingDebugMode::Barycentric);
        let barycentric = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(barycentric.submitted_triangles, affine.submitted_triangles);
        assert_eq!(
            barycentric.rasterized_triangles,
            affine.rasterized_triangles
        );
        assert_eq!(barycentric.shaded_samples, affine.shaded_samples);
        let barycentric_hash = fnv1a(renderer.color_buffer());
        assert_eq!(barycentric_hash, 0xdb7e_9eb4);
        assert_ne!(barycentric_hash, affine_hash);

        let snapshot = renderer.coordinate_debug_snapshot();
        assert!(snapshot.interpolation_debug_enabled);
        assert!(!snapshot.clip_debug_enabled);
        assert!(!snapshot.coverage_debug_enabled);
        assert_eq!(snapshot.winding_debug_mode, WindingDebugMode::Barycentric);
        assert_eq!(snapshot.selected_vertex_index, 0);
        let selected = snapshot.selected_viewport.unwrap();
        assert_close(selected.x, 11.2);
        assert_close(selected.y, 11.2);
        assert_close(selected.z_ndc, 0.5);

        renderer
            .resize(128, 64)
            .expect("active interpolation fixture resize should succeed");
        assert_eq!(renderer.framebuffer_generation(), 1);
        let resized = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(resized.input_vertices, 3);
        assert_eq!(resized.rasterized_triangles, 1);
        assert!(resized.shaded_samples > affine.shaded_samples);
        assert!(resized.max_barycentric_sum_error <= 2.0 * f32::EPSILON);
        let resized_snapshot = renderer.coordinate_debug_snapshot();
        assert!(resized_snapshot.interpolation_debug_enabled);
        let resized_selected = resized_snapshot.selected_viewport.unwrap();
        assert_close(resized_selected.x, 22.4);
        assert_close(resized_selected.y, 11.2);

        renderer.set_coverage_debug_enabled(true);
        let coverage = renderer.coordinate_debug_snapshot();
        assert!(coverage.coverage_debug_enabled);
        assert!(!coverage.interpolation_debug_enabled);
    }

    #[test]
    fn chapter_thirteen_invalid_fragment_does_not_commit_an_invisible_occluder() {
        let mesh = interpolation_debug_fixture();
        let scene = MeshScene::new_identity_debug(&mesh, 64, 64);
        let valid_generated: [ClipVertex; 3] = scene.clip_vertices.clone().try_into().unwrap();
        let mut invalid_generated = valid_generated;
        for vertex in &mut invalid_generated {
            vertex.color = Vec4::new(f32::MAX, f32::MAX, f32::MAX, f32::MAX);
        }
        let invalid_positions = [
            ViewportPosition {
                x: 0.0,
                y: 0.0,
                z_ndc: 0.25,
            },
            ViewportPosition {
                x: 0.125,
                y: 0.0,
                z_ndc: 0.25,
            },
            ViewportPosition {
                x: 0.875,
                y: 0.875,
                z_ndc: 0.25,
            },
        ];
        let farther_positions = invalid_positions.map(|position| ViewportPosition {
            z_ndc: 0.75,
            ..position
        });

        let mut near_first_target = RenderTarget::new(1, 1).unwrap();
        near_first_target.render_gradient_checker();
        let color_before = near_first_target.color().to_vec();
        let depth_before = near_first_target.depth().to_vec();
        let mut invalid_near_first = FrameDrawReport::default();
        submit_triangle(
            &mut near_first_target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            invalid_generated,
            invalid_positions,
            0,
            &mut invalid_near_first,
        );
        assert_eq!(invalid_near_first.submitted_triangles, 1);
        assert_eq!(invalid_near_first.rasterized_triangles, 1);
        assert_eq!(invalid_near_first.shaded_samples, 0);
        assert_eq!(invalid_near_first.depth_passed_samples, 0);
        assert_eq!(invalid_near_first.depth_failed_samples, 0);
        assert_eq!(invalid_near_first.invalid_depth_samples, 0);
        assert_eq!(invalid_near_first.invalid_values, 1);
        assert_eq!(near_first_target.color(), color_before);
        assert_eq!(near_first_target.depth(), depth_before);

        let mut farther_after_invalid = FrameDrawReport::default();
        submit_triangle(
            &mut near_first_target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            valid_generated,
            farther_positions,
            0,
            &mut farther_after_invalid,
        );
        assert_eq!(farther_after_invalid.shaded_samples, 1);
        assert_eq!(farther_after_invalid.depth_passed_samples, 1);
        assert_eq!(farther_after_invalid.depth_failed_samples, 0);

        let expected_color = near_first_target.color().to_vec();
        let expected_depth = near_first_target.depth().to_vec();
        let mut far_first_target = RenderTarget::new(1, 1).unwrap();
        far_first_target.render_gradient_checker();
        let mut far_first_report = FrameDrawReport::default();
        submit_triangle(
            &mut far_first_target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            valid_generated,
            farther_positions,
            0,
            &mut far_first_report,
        );
        submit_triangle(
            &mut far_first_target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            invalid_generated,
            invalid_positions,
            1,
            &mut far_first_report,
        );
        assert_eq!(far_first_report.shaded_samples, 1);
        assert_eq!(far_first_report.depth_passed_samples, 1);
        assert_eq!(far_first_report.depth_failed_samples, 0);
        assert_eq!(far_first_report.invalid_values, 1);
        assert_eq!(far_first_target.color(), expected_color);
        assert_eq!(far_first_target.depth(), expected_depth);
    }

    #[test]
    fn chapter_thirteen_depth_is_order_independent_and_debug_views_are_deterministic() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_debug_lines_enabled(false);
        renderer.set_depth_debug_enabled(true);
        let near_first = renderer.update_and_render(0.0, InputSnapshot::default());
        let near_first_color = renderer.color_buffer().to_vec();
        let near_first_depth = renderer.depth_buffer().to_vec();
        let near_first_hash = fnv1a(&near_first_color);

        renderer.set_depth_order_reversed(true);
        let far_first = renderer.update_and_render(0.0, InputSnapshot::default());
        let far_first_hash = fnv1a(renderer.color_buffer());
        assert_eq!(renderer.color_buffer(), near_first_color);
        assert_eq!(renderer.depth_buffer(), near_first_depth);
        assert_eq!(far_first_hash, near_first_hash);
        assert_eq!(near_first.input_vertices, 6);
        assert_eq!(near_first.input_triangles, 2);
        assert_eq!(near_first.submitted_triangles, 2);
        assert_eq!(near_first.rasterized_triangles, 2);
        assert_eq!(near_first.invalid_depth_samples, 0);
        assert_eq!(far_first.invalid_depth_samples, 0);
        assert_eq!(near_first.depth_passed_samples, near_first.shaded_samples);
        assert_eq!(far_first.depth_passed_samples, far_first.shaded_samples);
        assert!(near_first.depth_failed_samples > 0);
        assert_eq!(far_first.depth_failed_samples, 0);
        assert!(far_first.depth_passed_samples > near_first.depth_passed_samples);

        let base_samples = [
            pixel(&renderer.target, 30, 25),
            pixel(&renderer.target, 48, 25),
            pixel(&renderer.target, 15, 20),
            pixel(&renderer.target, 0, 0),
        ];
        renderer.set_depth_debug_mode(DepthDebugMode::Grayscale);
        let grayscale = renderer.update_and_render(0.0, InputSnapshot::default());
        let grayscale_hash = fnv1a(renderer.color_buffer());
        let grayscale_samples = [
            pixel(&renderer.target, 30, 25),
            pixel(&renderer.target, 48, 25),
            pixel(&renderer.target, 15, 20),
            pixel(&renderer.target, 0, 0),
        ];
        renderer.set_depth_debug_mode(DepthDebugMode::Heatmap);
        let heatmap = renderer.update_and_render(0.0, InputSnapshot::default());
        let heatmap_hash = fnv1a(renderer.color_buffer());
        let heatmap_samples = [
            pixel(&renderer.target, 30, 25),
            pixel(&renderer.target, 48, 25),
            pixel(&renderer.target, 15, 20),
            pixel(&renderer.target, 0, 0),
        ];
        assert_eq!(
            grayscale.depth_passed_samples,
            far_first.depth_passed_samples
        );
        assert_eq!(
            grayscale.depth_failed_samples,
            far_first.depth_failed_samples
        );
        assert_eq!(heatmap.depth_passed_samples, far_first.depth_passed_samples);
        assert_eq!(heatmap.depth_failed_samples, far_first.depth_failed_samples);

        assert_eq!(
            (
                near_first.depth_passed_samples,
                near_first.depth_failed_samples,
                far_first.depth_passed_samples,
                near_first_hash,
                grayscale_hash,
                heatmap_hash,
                base_samples,
                grayscale_samples,
                heatmap_samples,
            ),
            (
                1_199,
                202,
                1_401,
                0x98e6_cc1c,
                0x52eb_59c7,
                0x5cbf_6a73,
                [
                    [255, 124, 108, 255],
                    [108, 160, 255, 255],
                    [255, 124, 108, 255],
                    [0, 0, 220, 255],
                ],
                [
                    [64, 64, 64, 255],
                    [191, 191, 191, 255],
                    [64, 64, 64, 255],
                    [12, 18, 28, 255],
                ],
                [
                    [0, 128, 128, 255],
                    [128, 128, 0, 255],
                    [0, 128, 128, 255],
                    [12, 18, 28, 255],
                ],
            )
        );

        let snapshot = renderer.coordinate_debug_snapshot();
        assert!(snapshot.depth_debug_enabled);
        assert!(snapshot.depth_order_reversed);
        assert_eq!(snapshot.depth_debug_mode, DepthDebugMode::Heatmap);
        assert!(!snapshot.clip_debug_enabled);
        assert!(!snapshot.coverage_debug_enabled);
        assert!(!snapshot.interpolation_debug_enabled);
        assert_eq!(snapshot.mesh_vertices, 6);
        assert_eq!(snapshot.mesh_triangles, 2);
        assert_eq!(snapshot.frame_stats, heatmap);

        renderer.set_depth_debug_mode(DepthDebugMode::Grayscale);
        renderer.resize(128, 64).unwrap();
        assert_eq!(renderer.framebuffer_generation(), 1);
        let resized = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(resized.input_triangles, 2);
        assert_eq!(resized.invalid_depth_samples, 0);
        let resized_snapshot = renderer.coordinate_debug_snapshot();
        assert!(resized_snapshot.depth_debug_enabled);
        let selected = resized_snapshot.selected_viewport.unwrap();
        assert_close(selected.x, 16.0);
        assert_close(selected.y, 11.2);
        assert_close(selected.z_ndc, 0.25);

        renderer.set_interpolation_debug_enabled(true);
        let interpolation = renderer.coordinate_debug_snapshot();
        assert!(interpolation.interpolation_debug_enabled);
        assert!(!interpolation.depth_debug_enabled);

        renderer.set_depth_debug_enabled(true);
        renderer.set_depth_order_reversed(false);
        renderer.resize(64, 64).unwrap();
        assert_eq!(renderer.framebuffer_generation(), 2);
        let resized_near_first = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(resized_near_first.depth_passed_samples, 1_199);
        assert_eq!(resized_near_first.depth_failed_samples, 202);
        assert_eq!(fnv1a(renderer.color_buffer()), 0x52eb_59c7);
    }

    #[test]
    fn chapter_thirteen_invalid_interpolated_depth_is_counted_before_shading() {
        let mesh = interpolation_debug_fixture();
        let scene = MeshScene::new_identity_debug(&mesh, 64, 64);
        let generated: [ClipVertex; 3] = scene.clip_vertices.clone().try_into().unwrap();
        let mut positions = [
            scene.diagnostic_viewport_positions[0].unwrap(),
            scene.diagnostic_viewport_positions[1].unwrap(),
            scene.diagnostic_viewport_positions[2].unwrap(),
        ];
        for position in &mut positions {
            position.z_ndc = 2.0;
        }
        let mut target = RenderTarget::new(64, 64).unwrap();
        target.render_gradient_checker();
        let color_before = target.color().to_vec();
        let mut report = FrameDrawReport::default();
        submit_triangle(
            &mut target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            generated,
            positions,
            0,
            &mut report,
        );

        assert_eq!(report.submitted_triangles, 1);
        assert_eq!(report.rasterized_triangles, 1);
        assert_eq!(report.depth_passed_samples, 0);
        assert_eq!(report.depth_failed_samples, 0);
        assert!(report.invalid_depth_samples > 0);
        assert_eq!(report.shaded_samples, 0);
        assert_eq!(target.color(), color_before);
        assert!(target.depth().iter().all(|depth| depth.is_infinite()));
    }

    #[test]
    fn chapter_fourteen_perspective_uv_differs_from_affine_without_changing_depth() {
        let (affine_target, affine_report) =
            render_perspective_fixture(false, AttributeInterpolationMode::Affine);
        let (perspective_target, perspective_report) =
            render_perspective_fixture(false, AttributeInterpolationMode::PerspectiveCorrect);

        assert_ne!(affine_target.color(), perspective_target.color());
        assert_eq!(fnv1a(affine_target.color()), 0xe8d9_2635);
        assert_eq!(fnv1a(perspective_target.color()), 0xf152_6d3d);
        assert_eq!(affine_target.depth(), perspective_target.depth());
        assert_eq!(affine_report.submitted_triangles, 2);
        assert_eq!(perspective_report.submitted_triangles, 2);
        assert_eq!(
            affine_report.shaded_samples,
            perspective_report.shaded_samples
        );
        assert_eq!(
            affine_report.depth_passed_samples,
            perspective_report.depth_passed_samples
        );
        assert_eq!(perspective_report.invalid_depth_samples, 0);
        assert_eq!(perspective_report.invalid_interpolation_samples, 0);
        assert_eq!(
            perspective_report.interpolated_inv_w_samples,
            perspective_report.shaded_samples
        );
        assert!(perspective_report.min_interpolated_inv_w > 0.2);
        assert!(perspective_report.max_interpolated_inv_w < 0.5);
        assert!(
            perspective_report.min_interpolated_inv_w < perspective_report.max_interpolated_inv_w
        );
    }

    #[test]
    fn chapter_fourteen_invalid_q_never_commits_an_invisible_occluder() {
        let mesh = interpolation_debug_fixture();
        let scene = MeshScene::new_identity_debug(&mesh, 64, 64);
        let valid_generated: [ClipVertex; 3] = scene.clip_vertices.clone().try_into().unwrap();
        let mut tiny_inv_w_generated = valid_generated;
        for vertex in &mut tiny_inv_w_generated {
            vertex.clip_pos.0 = Vec4::new(0.0, 0.0, 5.0e8, 1.0e9);
        }
        let near_positions = [
            ViewportPosition {
                x: 0.0,
                y: 0.0,
                z_ndc: 0.25,
            },
            ViewportPosition {
                x: 0.125,
                y: 0.0,
                z_ndc: 0.25,
            },
            ViewportPosition {
                x: 0.875,
                y: 0.875,
                z_ndc: 0.25,
            },
        ];
        let farther_positions = near_positions.map(|position| ViewportPosition {
            z_ndc: 0.75,
            ..position
        });
        let mut target = RenderTarget::new(1, 1).unwrap();
        target.render_gradient_checker();
        let color_before = target.color().to_vec();
        let depth_before = target.depth().to_vec();
        let mut invalid_report = FrameDrawReport::default();
        submit_triangle(
            &mut target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            tiny_inv_w_generated,
            near_positions,
            0,
            &mut invalid_report,
        );
        assert_eq!(invalid_report.submitted_triangles, 1);
        assert_eq!(invalid_report.rasterized_triangles, 1);
        assert_eq!(invalid_report.interpolated_inv_w_samples, 0);
        assert_eq!(invalid_report.invalid_interpolation_samples, 1);
        assert_eq!(invalid_report.invalid_values, 1);
        assert_eq!(invalid_report.depth_passed_samples, 0);
        assert_eq!(invalid_report.shaded_samples, 0);
        assert_eq!(target.color(), color_before);
        assert_eq!(target.depth(), depth_before);

        let mut valid_report = FrameDrawReport::default();
        submit_triangle(
            &mut target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            valid_generated,
            farther_positions,
            0,
            &mut valid_report,
        );
        assert_eq!(valid_report.depth_passed_samples, 1);
        assert_eq!(valid_report.shaded_samples, 1);
        assert_ne!(target.color(), color_before);
        assert_ne!(target.depth(), depth_before);
    }

    #[test]
    fn chapter_fourteen_perspective_quad_is_diagonal_independent() {
        let (primary, primary_report) =
            render_perspective_fixture(false, AttributeInterpolationMode::PerspectiveCorrect);
        let (alternate, alternate_report) =
            render_perspective_fixture(true, AttributeInterpolationMode::PerspectiveCorrect);
        assert_eq!(primary.color(), alternate.color());
        let max_depth_difference = primary
            .depth()
            .iter()
            .zip(alternate.depth().iter())
            .filter_map(|(primary_depth, alternate_depth)| {
                if primary_depth.is_infinite() && alternate_depth.is_infinite() {
                    None
                } else {
                    Some((primary_depth - alternate_depth).abs())
                }
            })
            .fold(0.0_f32, f32::max);
        assert!(max_depth_difference <= DEPTH_RANGE_EPSILON);
        assert_eq!(
            primary_report.shaded_samples,
            alternate_report.shaded_samples
        );
        assert_eq!(
            primary_report.depth_passed_samples,
            alternate_report.depth_passed_samples
        );
        assert_eq!(primary_report.invalid_interpolation_samples, 0);
        assert_eq!(alternate_report.invalid_interpolation_samples, 0);
    }

    #[test]
    fn chapter_fourteen_scene_mode_and_resize_preserve_public_debug_contract() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_debug_lines_enabled(false);
        renderer.set_perspective_debug_enabled(true);
        renderer.set_attribute_interpolation_mode(AttributeInterpolationMode::Affine);
        let affine = renderer.update_and_render(0.0, InputSnapshot::default());
        let affine_hash = fnv1a(renderer.color_buffer());
        let snapshot = renderer.coordinate_debug_snapshot();
        assert!(snapshot.perspective_debug_enabled);
        assert!(!snapshot.interpolation_debug_enabled);
        assert_eq!(
            snapshot.attribute_interpolation_mode,
            AttributeInterpolationMode::Affine
        );
        assert_eq!(
            (
                snapshot.mesh_vertices,
                snapshot.mesh_indices,
                snapshot.mesh_triangles
            ),
            (4, 6, 2)
        );
        assert_eq!(affine.input_vertices, 4);
        assert_eq!(affine.input_triangles, 2);
        assert_eq!(affine.interpolated_inv_w_samples, affine.shaded_samples);

        renderer.set_attribute_interpolation_mode(AttributeInterpolationMode::PerspectiveCorrect);
        let perspective = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_ne!(fnv1a(renderer.color_buffer()), affine_hash);
        assert_eq!(perspective.invalid_interpolation_samples, 0);

        renderer.resize(96, 48).unwrap();
        let resized = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(resized.input_vertices, 4);
        assert_eq!(resized.input_triangles, 2);
        assert!(
            renderer
                .coordinate_debug_snapshot()
                .perspective_debug_enabled
        );

        renderer.set_depth_debug_enabled(true);
        assert!(
            !renderer
                .coordinate_debug_snapshot()
                .perspective_debug_enabled
        );
        renderer.set_perspective_debug_enabled(true);
        assert!(!renderer.coordinate_debug_snapshot().depth_debug_enabled);
    }

    #[test]
    fn clip_debug_transforms_and_reports_only_its_active_scene() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        renderer.set_model_rotation_y(f32::NAN);
        let hidden_cube_clip_bits: Vec<_> = renderer
            .mesh_scene
            .traces
            .iter()
            .map(|trace| {
                let clip = trace.clip_pos.0;
                [
                    clip.x.to_bits(),
                    clip.y.to_bits(),
                    clip.z.to_bits(),
                    clip.w.to_bits(),
                ]
            })
            .collect();
        let clip_trace_pointer = renderer.clip_debug_scene.traces.as_ptr();
        renderer.set_clip_debug_enabled(true);

        let stats = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(stats.input_vertices, 3);
        assert_eq!(stats.transformed_vertices, 3);
        assert_eq!(stats.invalid_values, 0);
        assert_eq!(stats.clip_invalid_triangles, 0);
        assert_eq!(
            renderer
                .mesh_scene
                .traces
                .iter()
                .map(|trace| {
                    let clip = trace.clip_pos.0;
                    [
                        clip.x.to_bits(),
                        clip.y.to_bits(),
                        clip.z.to_bits(),
                        clip.w.to_bits(),
                    ]
                })
                .collect::<Vec<_>>(),
            hidden_cube_clip_bits
        );
        assert_eq!(
            renderer.clip_debug_scene.traces.as_ptr(),
            clip_trace_pointer
        );
        assert_eq!(
            renderer.coordinate_debug_snapshot().selected_vertex_index,
            2
        );

        renderer.set_clip_debug_enabled(false);
        let cube = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(cube.transformed_vertices, 24);
        assert_eq!(cube.invalid_values, 72);
        assert_eq!(cube.clip_invalid_triangles, 12);
    }

    #[test]
    fn clipped_fan_preserves_winding_for_front_and_back_culling() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        renderer.set_clip_debug_enabled(true);

        renderer.set_cull_mode(CullMode::Back);
        let back = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!((back.submitted_triangles, back.culled_triangles), (3, 0));
        assert_eq!(back.degenerate_triangles, 0);

        renderer.set_cull_mode(CullMode::Front);
        let front = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!((front.submitted_triangles, front.culled_triangles), (0, 3));
        assert_eq!(front.degenerate_triangles, 0);

        let source = clipping_debug_fixture();
        let reversed = Mesh::new(source.vertices().to_vec(), vec![0, 2, 1])
            .expect("reversed clipping fixture should remain a valid mesh");
        let reversed_scene = MeshScene::new_identity_debug(&reversed, 64, 64);
        let mut target = RenderTarget::new(64, 64).expect("target should be valid");
        let mut clipper = TriangleClipper::default();
        let reversed_back = draw_mesh(
            &mut target,
            false,
            raster_options(CullMode::Back, WindingDebugMode::VertexColor),
            &reversed,
            &mut clipper,
            &reversed_scene.clip_vertices,
        );
        assert_eq!(reversed_back.generated_triangles, 3);
        assert_eq!(
            (
                reversed_back.submitted_triangles,
                reversed_back.culled_triangles,
            ),
            (0, 3)
        );
        assert_eq!(reversed_back.degenerate_triangles, 0);

        let reversed_front = draw_mesh(
            &mut target,
            false,
            raster_options(CullMode::Front, WindingDebugMode::VertexColor),
            &reversed,
            &mut clipper,
            &reversed_scene.clip_vertices,
        );
        assert_eq!(reversed_front.generated_triangles, 3);
        assert_eq!(
            (
                reversed_front.submitted_triangles,
                reversed_front.culled_triangles,
            ),
            (3, 0)
        );
        assert_eq!(reversed_front.degenerate_triangles, 0);
    }

    #[test]
    fn cull_modes_report_fixed_cube_counts_and_normalize_double_sided_faces() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        for (rotation_y, submitted, culled) in [(0.0, 4, 8), (0.75, 6, 6), (-0.75, 6, 6)] {
            renderer.set_model_rotation_y(rotation_y);
            let stats = renderer.update_and_render(0.0, InputSnapshot::default());
            assert_eq!(stats.submitted_triangles, submitted);
            assert_eq!(stats.culled_triangles, culled);
            assert_eq!(stats.degenerate_triangles, 0);
            assert_eq!(stats.invalid_triangles, 0);
            assert_eq!(stats.submitted_triangles + stats.culled_triangles, 12);
        }

        renderer.set_model_rotation_y(0.0);
        renderer.set_cull_mode(CullMode::None);
        let double_sided = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(double_sided.submitted_triangles, 12);
        assert_eq!(double_sided.culled_triangles, 0);

        renderer.set_cull_mode(CullMode::Front);
        let front_culled = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(front_culled.submitted_triangles, 8);
        assert_eq!(front_culled.culled_triangles, 4);
    }

    #[test]
    fn double_sided_depth_matches_backface_culled_visible_cube_exactly() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        renderer.set_debug_lines_enabled(false);
        renderer.set_cull_mode(CullMode::Back);
        let back_culled = renderer.update_and_render(0.0, InputSnapshot::default());
        let visible_color = renderer.color_buffer().to_vec();
        let visible_depth = renderer.depth_buffer().to_vec();

        renderer.set_cull_mode(CullMode::None);
        let double_sided = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(renderer.color_buffer(), visible_color);
        assert_eq!(renderer.depth_buffer(), visible_depth);
        assert_eq!(back_culled.depth_failed_samples, 0);
        assert!(double_sided.depth_failed_samples > 0);
        assert_eq!(
            double_sided.depth_passed_samples,
            double_sided.shaded_samples
        );
        assert_eq!(double_sided.invalid_depth_samples, 0);
    }

    #[test]
    fn facing_debug_colors_each_visible_orientation_without_xray_overlay() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        renderer.set_cull_mode(CullMode::None);
        renderer.set_winding_debug_mode(WindingDebugMode::Facing);
        let facing = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(
            (facing.submitted_triangles, facing.culled_triangles),
            (12, 0)
        );
        assert!(
            renderer
                .color_buffer()
                .chunks_exact(4)
                .any(|pixel| pixel == [72, 232, 112, 255])
        );
        assert!(
            !renderer
                .color_buffer()
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 82, 92, 255])
        );
        assert!(facing.depth_failed_samples > 0);

        let snapshot = renderer.coordinate_debug_snapshot();
        assert_eq!(snapshot.cull_mode, CullMode::None);
        assert_eq!(snapshot.winding_debug_mode, WindingDebugMode::Facing);
        assert_eq!(snapshot.frame_stats, facing);

        renderer.set_cull_mode(CullMode::Front);
        let back_faces = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(back_faces.submitted_triangles, 8);
        assert!(
            renderer
                .color_buffer()
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 82, 92, 255])
        );

        renderer.set_cull_mode(CullMode::None);
        renderer.set_winding_debug_mode(WindingDebugMode::VertexColor);
        let vertex_colors = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(
            (
                vertex_colors.submitted_triangles,
                vertex_colors.culled_triangles
            ),
            (facing.submitted_triangles, facing.culled_triangles)
        );
        assert_eq!(fnv1a(renderer.color_buffer()), 0xf8d0_50be);
    }

    #[test]
    fn chapter_eight_scene_contains_projected_mesh_and_selected_vertex_colors() {
        let mut renderer = Renderer::new(64, 64).expect("debug renderer should be valid");
        renderer.set_debug_lines_enabled(true);
        renderer.update_and_render(0.0, InputSnapshot::default());
        for expected in [[238, 244, 255, 255], [255, 160, 137, 255]] {
            assert!(
                renderer
                    .color_buffer()
                    .chunks_exact(4)
                    .any(|pixel| pixel == expected),
                "missing stage color {expected:?}"
            );
        }
    }

    #[test]
    fn empty_and_degenerate_meshes_draw_as_safe_no_op_or_point_wireframe() {
        let empty = Mesh::new(vec![], vec![]).unwrap();
        let empty_scene = MeshScene::new(&empty, Transform::IDENTITY, 64, 64);
        let mut target = RenderTarget::new(64, 64).unwrap();
        let mut clipper = TriangleClipper::default();
        assert_eq!(
            draw_mesh(
                &mut target,
                true,
                raster_options(CullMode::Back, WindingDebugMode::VertexColor),
                &empty,
                &mut clipper,
                &empty_scene.clip_vertices,
            ),
            FrameDrawReport::default()
        );

        let vertex = mesh::Vertex::new(
            Vec3::ZERO,
            Vec3::Z,
            math::Vec2::ZERO,
            Vec4::new(1.0, 1.0, 1.0, 1.0),
        );
        let degenerate = Mesh::new(vec![vertex], vec![0, 0, 0]).unwrap();
        let degenerate_scene = MeshScene::new(&degenerate, Transform::IDENTITY, 64, 64);
        let color_before_degenerate = target.color().to_vec();
        let depth_before_degenerate = target.depth().to_vec();
        assert_eq!(
            draw_mesh(
                &mut target,
                true,
                raster_options(CullMode::Back, WindingDebugMode::VertexColor),
                &degenerate,
                &mut clipper,
                &degenerate_scene.clip_vertices,
            ),
            FrameDrawReport {
                degenerate_triangles: 1,
                generated_triangles: 1,
                max_clip_polygon_vertices: 3,
                ..FrameDrawReport::default()
            }
        );
        assert_eq!(target.color(), color_before_degenerate);
        assert_eq!(target.depth(), depth_before_degenerate);

        let mut invalid_vertices = degenerate_scene.clip_vertices.clone();
        invalid_vertices[0].uv.x = f32::NAN;
        assert_eq!(
            draw_mesh(
                &mut target,
                true,
                raster_options(CullMode::None, WindingDebugMode::VertexColor),
                &degenerate,
                &mut clipper,
                invalid_vertices.as_slice(),
            ),
            FrameDrawReport {
                clip_invalid_triangles: 1,
                ..FrameDrawReport::default()
            }
        );

        let fixture_mesh = clipping_debug_fixture();
        let fixture_scene = MeshScene::new_identity_debug(&fixture_mesh, 64, 64);
        let mut fixture_vertices = fixture_scene.clip_vertices;
        assert_eq!(
            draw_mesh(
                &mut target,
                true,
                raster_options(CullMode::None, WindingDebugMode::VertexColor),
                &fixture_mesh,
                &mut clipper,
                &fixture_vertices[..2],
            ),
            FrameDrawReport {
                clip_invalid_triangles: 1,
                ..FrameDrawReport::default()
            }
        );

        let mut invalid_submission = FrameDrawReport::default();
        submit_triangle(
            &mut target,
            true,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            [
                fixture_vertices[0],
                fixture_vertices[1],
                fixture_vertices[2],
            ],
            [
                ViewportPosition {
                    x: f32::NAN,
                    y: 0.0,
                    z_ndc: 0.5,
                },
                ViewportPosition {
                    x: 1.0,
                    y: 0.0,
                    z_ndc: 0.5,
                },
                ViewportPosition {
                    x: 0.0,
                    y: 1.0,
                    z_ndc: 0.5,
                },
            ],
            0,
            &mut invalid_submission,
        );
        assert_eq!(
            invalid_submission,
            FrameDrawReport {
                invalid_triangles: 1,
                ..FrameDrawReport::default()
            }
        );

        let generated = [
            fixture_vertices[0],
            fixture_vertices[1],
            fixture_vertices[2],
        ];
        let mut quantized_degenerate = FrameDrawReport::default();
        submit_triangle(
            &mut target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            generated,
            [
                viewport_point(0.0, 0.0),
                viewport_point(1.0, 0.0),
                viewport_point(0.0, 0.0015),
            ],
            0,
            &mut quantized_degenerate,
        );
        assert_eq!(
            quantized_degenerate,
            FrameDrawReport {
                degenerate_triangles: 1,
                ..FrameDrawReport::default()
            }
        );

        for invalid_positions in [
            [
                ViewportPosition {
                    x: 0.0,
                    y: 0.0,
                    z_ndc: f32::NAN,
                },
                viewport_point(1.0, 0.0),
                viewport_point(0.0, 1.0),
            ],
            [
                viewport_point(0.0, 0.0),
                viewport_point(f32::MAX, 0.0),
                viewport_point(0.0, 1.0),
            ],
            [
                viewport_point(0.0, 0.0),
                viewport_point(20_000_000.0, 0.0),
                viewport_point(0.0, 20_000_000.0),
            ],
        ] {
            let mut report = FrameDrawReport::default();
            submit_triangle(
                &mut target,
                false,
                raster_options(CullMode::None, WindingDebugMode::VertexColor),
                generated,
                invalid_positions,
                0,
                &mut report,
            );
            assert_eq!(
                report,
                FrameDrawReport {
                    invalid_triangles: 1,
                    ..FrameDrawReport::default()
                }
            );
        }

        for vertex in &mut fixture_vertices {
            vertex.clip_pos.0 = Vec4::new(0.0, 0.0, -0.5, 1.0);
        }
        assert_eq!(
            draw_mesh(
                &mut target,
                true,
                raster_options(CullMode::None, WindingDebugMode::VertexColor),
                &fixture_mesh,
                &mut clipper,
                &fixture_vertices,
            ),
            FrameDrawReport {
                fully_clipped_triangles: 1,
                max_clip_polygon_vertices: 3,
                ..FrameDrawReport::default()
            }
        );

        for vertex in &mut fixture_vertices {
            vertex.clip_pos.0 = Vec4::ZERO;
        }
        assert_eq!(
            draw_mesh(
                &mut target,
                true,
                raster_options(CullMode::None, WindingDebugMode::VertexColor),
                &fixture_mesh,
                &mut clipper,
                &fixture_vertices,
            ),
            FrameDrawReport {
                invalid_triangles: 1,
                generated_triangles: 1,
                max_clip_polygon_vertices: 3,
                ..FrameDrawReport::default()
            }
        );
    }

    #[test]
    fn coordinate_scene_rotates_reports_clip_distances_and_observes_invalid_values() {
        let mut renderer = Renderer::new(64, 64).expect("debug renderer should be valid");
        let initial = renderer.coordinate_debug_snapshot();
        assert_eq!(initial.selected_vertex_index, 6);
        assert_eq!(initial.diagnostics.invalid_values, 0);
        assert_eq!(initial.projection_failures, 0);
        assert_close(initial.fov_y_radians, std::f32::consts::FRAC_PI_3);
        assert_eq!(
            (initial.near, initial.far, initial.aspect),
            (0.1, 100.0, 1.0)
        );
        let initial_ndc = initial
            .selected_ndc
            .expect("selected vertex should project");
        let initial_viewport = initial
            .selected_viewport
            .expect("selected vertex should reach the viewport");
        assert!((-1.0..=1.0).contains(&initial_ndc.0.x));
        assert!((-1.0..=1.0).contains(&initial_ndc.0.y));
        assert!((0.0..=1.0).contains(&initial_ndc.0.z));
        assert!((0.0..=64.0).contains(&initial_viewport.x));
        assert!((0.0..=64.0).contains(&initial_viewport.y));
        assert!(
            initial
                .clip_plane_distances
                .0
                .into_iter()
                .all(|distance| distance >= 0.0)
        );

        let stats = renderer.update_and_render(0.1, InputSnapshot::default());
        let rotated = renderer.coordinate_debug_snapshot();
        assert_eq!(stats.input_vertices, 24);
        assert_eq!(stats.input_triangles, 12);
        assert_eq!(stats.transformed_vertices, 24);
        assert_eq!(stats.submitted_triangles, 4);
        assert_eq!(stats.culled_triangles, 8);
        assert_eq!(stats.degenerate_triangles, 0);
        assert_eq!(stats.invalid_triangles, 0);
        assert_eq!(stats.invalid_values, 0);
        assert_eq!(
            rotated.selected_vertex.object_pos,
            initial.selected_vertex.object_pos
        );
        assert_ne!(
            rotated.selected_vertex.world_pos,
            initial.selected_vertex.world_pos
        );
        assert_ne!(
            rotated.selected_vertex.view_pos.0,
            rotated.selected_vertex.world_pos.0
        );
        assert_close(
            rotated.selected_vertex.clip_pos.0.w,
            rotated.selected_vertex.view_pos.0.z,
        );

        renderer
            .resize(128, 64)
            .expect("wide resize should succeed");
        let wide = renderer.coordinate_debug_snapshot();
        assert_eq!(wide.aspect, 2.0);
        assert_close(
            wide.selected_ndc.unwrap().0.x,
            rotated.selected_ndc.unwrap().0.x * 0.5,
        );

        renderer.set_model_rotation_y(f32::NAN);
        let invalid = renderer.update_and_render(0.0, InputSnapshot::default());
        let invalid_snapshot = renderer.coordinate_debug_snapshot();
        assert_eq!(invalid.invalid_values, 72);
        assert_eq!(invalid.invalid_triangles, 0);
        assert_eq!(invalid.clip_invalid_triangles, 12);
        assert_eq!(invalid.submitted_triangles, 0);
        assert_eq!(invalid_snapshot.projection_failures, 24);
        assert_eq!(invalid_snapshot.selected_ndc, None);
        assert_eq!(invalid_snapshot.selected_viewport, None);
        assert_eq!(
            invalid_snapshot.diagnostics.first_invalid_space,
            Some(CoordinateSpace::World)
        );
        assert!(invalid_snapshot.rotation_y_radians.is_nan());

        renderer.set_model_rotation_y(0.0);
        renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(renderer.stats().invalid_values, 0);
    }

    #[test]
    fn input_snapshot_round_trips_all_packed_bits() {
        let snapshot = InputSnapshot::new([INPUT_KEY_MASK, 0, 0], Vec2::ZERO, 0.0, 0, 0).unwrap();
        assert_eq!(snapshot.packed_bits(), INPUT_KEY_MASK);
        assert_eq!(InputSnapshot::default().packed_bits(), 0);
    }

    #[test]
    fn chapter_twenty_input_snapshot_validates_layout_and_maps_camera_axes() {
        let snapshot = InputSnapshot::new(
            [INPUT_FORWARD | INPUT_RIGHT, INPUT_FORWARD, INPUT_LEFT],
            Vec2::new(12.5, -4.0),
            120.0,
            1,
            INPUT_FLAG_DRAGGING | INPUT_MODIFIER_SHIFT,
        )
        .unwrap();
        assert_eq!(snapshot.packed_bits(), INPUT_FORWARD | INPUT_RIGHT);
        assert_eq!(snapshot.pressed_bits(), INPUT_FORWARD);
        assert_eq!(snapshot.released_bits(), INPUT_LEFT);
        assert_eq!(snapshot.pointer_delta(), Vec2::new(12.5, -4.0));
        assert_eq!(snapshot.wheel_delta(), 120.0);
        assert_eq!(snapshot.pointer_buttons(), 1);
        assert_eq!(snapshot.flags(), INPUT_FLAG_DRAGGING | INPUT_MODIFIER_SHIFT);
        assert_eq!(
            snapshot.camera_control_input(),
            CameraControlInput {
                move_right: 1.0,
                move_forward: 1.0,
                pointer_dx: 12.5,
                pointer_dy: -4.0,
                wheel_delta: 120.0,
                dragging: true,
                ..CameraControlInput::default()
            }
        );

        for (result, expected) in [
            (
                InputSnapshot::new([1 << 8, 0, 0], Vec2::ZERO, 0.0, 0, 0),
                InputSnapshotError::UnsupportedKeyBits,
            ),
            (
                InputSnapshot::new([0, 0, 0], Vec2::new(f32::NAN, 0.0), 0.0, 0, 0),
                InputSnapshotError::InvalidPointerDelta,
            ),
            (
                InputSnapshot::new([0, 0, 0], Vec2::ZERO, f32::INFINITY, 0, 0),
                InputSnapshotError::InvalidWheelDelta,
            ),
            (
                InputSnapshot::new([0, 0, 0], Vec2::ZERO, 0.0, 1 << 8, 0),
                InputSnapshotError::UnsupportedPointerButtons,
            ),
            (
                InputSnapshot::new([0, 0, 0], Vec2::ZERO, 0.0, 0, 1 << 8),
                InputSnapshotError::UnsupportedFlags,
            ),
        ] {
            assert_eq!(result.unwrap_err(), expected);
            assert!(!expected.to_string().is_empty());
        }
    }

    #[test]
    fn chapter_twenty_renderer_applies_orbit_and_dt_scaled_fly_before_projection() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        assert_eq!(renderer.camera_mode(), CameraMode::Orbit);
        assert_eq!(renderer.camera_pose().eye, CAMERA_EYE);
        renderer.update_and_render(
            0.0,
            InputSnapshot::new([0, 0, 0], Vec2::new(20.0, 0.0), 0.0, 1, INPUT_FLAG_DRAGGING)
                .unwrap(),
        );
        assert!(renderer.camera_pose().forward.x > 0.0);
        assert_eq!(renderer.mesh_scene.camera_world, renderer.camera_pose().eye);

        renderer.set_camera_mode(CameraMode::Fly).unwrap();
        let before = renderer.camera_pose();
        let stats = renderer.update_and_render(
            0.1,
            InputSnapshot::new([INPUT_FORWARD, 0, 0], Vec2::ZERO, 0.0, 0, 0).unwrap(),
        );
        let after = renderer.camera_pose();
        assert_eq!(stats.input_bits, INPUT_FORWARD);
        assert!((after.eye - before.eye).length() > 0.29);
        assert!((after.eye - before.eye).length() < 0.31);
        assert_eq!(renderer.mesh_scene.camera_world, after.eye);

        renderer.resize(80, 40).unwrap();
        assert_eq!(renderer.camera_pose(), after);
        assert_eq!(renderer.mesh_scene.camera_world, after.eye);
    }

    #[test]
    fn chapter_twenty_one_obj_upload_is_owned_framed_and_failure_atomic() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        assert_eq!(
            renderer.mesh_asset_status(),
            MeshAssetStatus {
                active_mesh_id: MeshId(0),
                source_positions: 24,
                source_faces: 12,
                internal_vertices: 24,
                triangles: 12,
                successful_uploads: 0,
                failed_uploads: 0,
                source_bounds: MeshBounds {
                    source_min: Vec3::new(-0.5, -0.5, -0.5),
                    source_max: Vec3::new(0.5, 0.5, 0.5),
                    source_center: Vec3::ZERO,
                    source_half_extent: 0.5,
                },
            }
        );
        renderer.set_clip_debug_enabled(true);
        renderer.set_texture_debug_enabled(true);
        renderer.set_transparency_debug_enabled(true);
        renderer.set_camera_mode(CameraMode::Fly).unwrap();
        renderer.update_and_render(
            0.1,
            InputSnapshot::new([INPUT_FORWARD, 0, 0], Vec2::ZERO, 0.0, 0, 0).unwrap(),
        );

        let mut source =
            b"v 10 20 30\nv 12 20 30\nv 10 24 30\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 3/3 2/2\n".to_vec();
        assert_eq!(renderer.load_obj(&source).unwrap(), MeshId(1));
        source.fill(0);
        assert_eq!(renderer.camera_mode(), CameraMode::Orbit);
        assert_eq!(renderer.camera_pose().eye, CAMERA_EYE);
        assert!(!renderer.clip_debug_enabled);
        assert!(!renderer.texture_debug_enabled());
        assert!(!renderer.transparency_debug_enabled());
        assert_eq!(renderer.mesh.vertices().len(), 3);
        assert_eq!(renderer.primary_selected_vertex_index(), 2);
        assert_eq!(renderer.mesh.vertices()[0].uv, Vec2::new(0.0, 1.0));
        assert_eq!(
            renderer.mesh_source_bounds.source_center,
            Vec3::new(11.0, 22.0, 30.0)
        );

        let rendered = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(rendered.input_vertices, 3);
        assert_eq!(rendered.input_triangles, 1);
        assert_eq!(rendered.invalid_values, 0);
        assert!(rendered.covered_samples > 0);
        let color = renderer.color_buffer().to_vec();
        let mesh = renderer.mesh.clone();
        let scene = renderer.mesh_scene.clone();
        let camera = renderer.camera_pose();

        let error = renderer.load_obj(b"v 0 0 0\nf 1 2 3\n").unwrap_err();
        assert!(error.to_string().contains("범위"));
        assert_eq!(renderer.mesh, mesh);
        assert_eq!(renderer.mesh_scene, scene);
        assert_eq!(renderer.camera_pose(), camera);
        assert_eq!(renderer.color_buffer(), color);
        assert_eq!(
            renderer.mesh_asset_status(),
            MeshAssetStatus {
                active_mesh_id: MeshId(1),
                source_positions: 3,
                source_faces: 1,
                internal_vertices: 3,
                triangles: 1,
                successful_uploads: 1,
                failed_uploads: 1,
                source_bounds: MeshBounds {
                    source_min: Vec3::new(10.0, 20.0, 30.0),
                    source_max: Vec3::new(12.0, 24.0, 30.0),
                    source_center: Vec3::new(11.0, 22.0, 30.0),
                    source_half_extent: 2.0,
                },
            }
        );
    }

    #[test]
    fn chapter_twenty_one_mesh_id_exhaustion_keeps_the_active_mesh() {
        let mut renderer = Renderer::new(32, 32).unwrap();
        renderer.next_mesh_id = u32::MAX;
        let initial_mesh = renderer.mesh.clone();
        let error = renderer
            .load_obj(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")
            .unwrap_err();
        assert!(error.to_string().contains("mesh ID"));
        assert_eq!(renderer.mesh, initial_mesh);
        assert_eq!(renderer.mesh_upload_successes, 0);
        assert_eq!(renderer.mesh_upload_failures, 1);
    }

    #[test]
    fn chapter_fifteen_frame_stats_relations_detect_each_broken_stage_boundary() {
        let valid = FrameStats {
            generated_triangles: 12,
            submitted_triangles: 4,
            culled_triangles: 8,
            rasterized_triangles: 4,
            covered_samples: 120,
            shaded_samples: 75,
            depth_passed_samples: 75,
            depth_written_samples: 75,
            depth_failed_samples: 40,
            invalid_depth_samples: 3,
            interpolated_inv_w_samples: 75,
            invalid_interpolation_samples: 2,
            mip_samples: 50,
            min_mip_level: 1,
            max_mip_level: 4,
            invalid_lod_samples: 2,
            ..FrameStats::default()
        };
        assert!(valid.pipeline_relations_hold());
        for invalid in [
            FrameStats {
                generated_triangles: 13,
                ..valid
            },
            FrameStats {
                rasterized_triangles: 3,
                ..valid
            },
            FrameStats {
                covered_samples: 119,
                ..valid
            },
            FrameStats {
                shaded_samples: 74,
                ..valid
            },
            FrameStats {
                interpolated_inv_w_samples: 74,
                ..valid
            },
            FrameStats {
                depth_written_samples: 74,
                ..valid
            },
            FrameStats {
                blended_samples: 1,
                ..valid
            },
            FrameStats {
                mip_samples: 76,
                ..valid
            },
            FrameStats {
                invalid_lod_samples: 51,
                ..valid
            },
            FrameStats {
                mip_samples: 0,
                ..valid
            },
            FrameStats {
                min_mip_level: 5,
                max_mip_level: 4,
                ..valid
            },
            FrameStats {
                sample_counter_overflow: true,
                ..valid
            },
        ] {
            assert!(!invalid.pipeline_relations_hold(), "{invalid:?}");
        }
        let overflowed = FrameStats {
            sample_counter_overflow: true,
            covered_samples: 1,
            ..valid
        };
        assert!(!overflowed.pipeline_relations_hold());
        assert!(pipeline_stats_are_consistent_or_overflowed(overflowed));
    }

    #[test]
    fn chapter_fifteen_sample_counter_overflow_is_saturated_and_observable() {
        let mut counter = u32::MAX - 1;
        let mut overflow = false;
        increment_sample_counter(&mut counter, &mut overflow);
        assert_eq!(counter, u32::MAX);
        assert!(!overflow);

        increment_sample_counter(&mut counter, &mut overflow);
        assert_eq!(counter, u32::MAX);
        assert!(overflow);

        let mut aggregate = u32::MAX - 2;
        let mut aggregate_overflow = false;
        add_sample_counter(&mut aggregate, 3, &mut aggregate_overflow);
        assert_eq!(aggregate, u32::MAX);
        assert!(aggregate_overflow);
    }

    #[test]
    fn chapter_fifteen_debug_modes_share_geometry_coverage_and_depth() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_debug_lines_enabled(false);
        renderer.set_model_rotation_y(0.65);
        let mut reference_counts = None;
        let mut reference_depth = None;
        let mut hashes = Vec::new();
        for mode in [
            PipelineDebugMode::Solid,
            PipelineDebugMode::Wireframe,
            PipelineDebugMode::TriangleId,
            PipelineDebugMode::Barycentric,
            PipelineDebugMode::Depth,
            PipelineDebugMode::DepthHeatmap,
            PipelineDebugMode::FrontBack,
            PipelineDebugMode::Normal,
            PipelineDebugMode::NdotL,
        ] {
            renderer.set_pipeline_debug_mode(mode);
            let stats = renderer.update_and_render(0.0, InputSnapshot::default());
            assert!(stats.pipeline_relations_hold(), "{mode:?}: {stats:?}");
            let counts = (
                stats.submitted_triangles,
                stats.culled_triangles,
                stats.rasterized_triangles,
                stats.covered_samples,
                stats.depth_passed_samples,
                stats.depth_failed_samples,
                stats.shaded_samples,
            );
            assert_eq!(*reference_counts.get_or_insert(counts), counts, "{mode:?}");
            assert_eq!(
                reference_depth.get_or_insert_with(|| renderer.depth_buffer().to_vec()),
                renderer.depth_buffer(),
                "{mode:?}"
            );
            hashes.push(fnv1a(renderer.color_buffer()));
            assert_eq!(
                renderer
                    .coordinate_debug_snapshot()
                    .pipeline_state
                    .debug_mode,
                mode
            );
        }
        hashes.sort_unstable();
        hashes.dedup();
        assert_eq!(hashes.len(), 9);
        assert_eq!(
            raster_options(CullMode::None, WindingDebugMode::Facing)
                .pipeline_state
                .debug_mode,
            PipelineDebugMode::FrontBack
        );
        assert_eq!(
            raster_options(CullMode::None, WindingDebugMode::Barycentric)
                .pipeline_state
                .debug_mode,
            PipelineDebugMode::Barycentric
        );
    }

    #[test]
    fn chapter_fifteen_cube_submission_order_keeps_visible_surface_exact() {
        let mesh = unit_cube_mesh();
        let mut reversed_indices = Vec::with_capacity(mesh.indices().len());
        for triangle in mesh.indices().chunks_exact(3).rev() {
            reversed_indices.extend_from_slice(triangle);
        }
        let reversed = Mesh::new(mesh.vertices().to_vec(), reversed_indices).unwrap();
        let model = Transform {
            translation: Vec3::ZERO,
            rotation_radians: Vec3::new(0.45, 0.65, 0.0),
            scale: Vec3::new(1.25, 1.25, 1.25),
        };
        let original_scene = MeshScene::new(&mesh, model, 64, 64);
        let reversed_scene = MeshScene::new(&reversed, model, 64, 64);
        let mut original_target = RenderTarget::new(64, 64).unwrap();
        original_target.render_gradient_checker();
        let mut reversed_target = RenderTarget::new(64, 64).unwrap();
        reversed_target.render_gradient_checker();
        let options = raster_options(CullMode::None, WindingDebugMode::VertexColor);
        let original_report = draw_mesh(
            &mut original_target,
            false,
            options,
            &mesh,
            &mut TriangleClipper::default(),
            &original_scene.clip_vertices,
        );
        let reversed_report = draw_mesh(
            &mut reversed_target,
            false,
            options,
            &reversed,
            &mut TriangleClipper::default(),
            &reversed_scene.clip_vertices,
        );
        assert_eq!(original_target.color(), reversed_target.color());
        assert_eq!(original_target.depth(), reversed_target.depth());
        assert_eq!(
            original_report.covered_samples,
            reversed_report.covered_samples
        );
        assert_ne!(
            original_report.depth_failed_samples,
            reversed_report.depth_failed_samples
        );
    }

    #[test]
    fn chapter_fifteen_default_pose_normals_match_screen_front_faces_and_culling() {
        let mesh = unit_cube_mesh();
        let model = Transform {
            translation: Vec3::ZERO,
            rotation_radians: Vec3::new(0.45, 0.65, 0.0),
            scale: Vec3::new(1.25, 1.25, 1.25),
        };
        let scene = MeshScene::new(&mesh, model, 64, 64);
        for triangle in mesh.triangles() {
            let screen = triangle.map(|index| scene.diagnostic_viewport_positions[index].unwrap());
            let center = triangle
                .map(|index| scene.clip_vertices[index].world_pos)
                .into_iter()
                .fold(Vec3::ZERO, |sum, position| sum + position)
                / 3.0;
            let outward = scene.clip_vertices[triangle[0]].normal_world;
            let normal_faces_camera = outward.dot(CAMERA_EYE - center) > 0.0;
            let orientation = submitted_orientation(classify_triangle(screen, CullMode::None));
            assert_eq!(
                orientation == FaceOrientation::Front,
                normal_faces_camera,
                "triangle {triangle:?}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "triangle 제출을 기대했지만 Degenerate였다")]
    fn submitted_orientation_rejects_non_submitted_triangles() {
        submitted_orientation(TriangleDisposition::Degenerate);
    }

    #[test]
    fn chapter_fifteen_selected_vertex_projection_rejects_invalid_or_outside_clip_positions() {
        assert_eq!(
            project_inside_clip(
                transform::ClipPosition(Vec4::new(f32::NAN, 0.0, 0.5, 1.0)),
                64,
                64,
            ),
            None
        );
        assert_eq!(
            project_inside_clip(
                transform::ClipPosition(Vec4::new(2.0, 0.0, 0.5, 1.0)),
                64,
                64,
            ),
            None
        );
    }

    #[test]
    fn chapter_fifteen_near_plane_cube_clips_without_invalid_or_screen_explosion() {
        let mesh = unit_cube_mesh();
        let mut scene = MeshScene::with_capacity(&mesh);
        let eye = Vec3::new(0.0, 0.0, -0.55);
        let view = look_at_lh(eye, CAMERA_TARGET, CAMERA_WORLD_UP).unwrap();
        let projection =
            perspective_lh_zo(CAMERA_FOV_Y_RADIANS, 1.0, CAMERA_NEAR, CAMERA_FAR).unwrap();
        let model = Transform {
            translation: Vec3::ZERO,
            rotation_radians: Vec3::new(0.0, 0.35, 0.0),
            scale: Vec3::new(1.0, 1.0, 1.0),
        };
        scene.rebuild_with_pipeline(
            &mesh,
            TransformPipeline::new(model.model_matrix(), view, projection),
            64,
            64,
        );
        let mut target = RenderTarget::new(64, 64).unwrap();
        target.render_gradient_checker();
        let report = draw_mesh(
            &mut target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            &mesh,
            &mut TriangleClipper::default(),
            &scene.clip_vertices,
        );
        assert_eq!(report.clip_invalid_triangles, 0);
        assert_eq!(report.invalid_triangles, 0);
        assert_eq!(report.invalid_depth_samples, 0);
        assert_eq!(report.invalid_interpolation_samples, 0);
        assert!(report.generated_triangles > 0);
        assert!(report.rasterized_triangles > 0);
        assert!((1..=64 * 64 * 12).contains(&(report.covered_samples as usize)));
        assert!(target.depth().iter().all(|depth| {
            depth.is_infinite() || (depth.is_finite() && (0.0..=1.0).contains(depth))
        }));
    }

    #[test]
    fn chapter_fifteen_vertex_stage_reuses_frame_capacity() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        let pointers = (
            renderer.mesh_scene.traces.as_ptr(),
            renderer.mesh_scene.clip_vertices.as_ptr(),
            renderer.mesh_scene.diagnostic_ndc_positions.as_ptr(),
            renderer.mesh_scene.diagnostic_viewport_positions.as_ptr(),
        );
        for _ in 0..16 {
            renderer.update_and_render(0.1, InputSnapshot::default());
        }
        assert_eq!(
            (
                renderer.mesh_scene.traces.as_ptr(),
                renderer.mesh_scene.clip_vertices.as_ptr(),
                renderer.mesh_scene.diagnostic_ndc_positions.as_ptr(),
                renderer.mesh_scene.diagnostic_viewport_positions.as_ptr(),
            ),
            pointers
        );
    }

    #[test]
    fn chapter_fifteen_camera_pose_golden_change_is_quantified() {
        fn render_from_eye(eye: Vec3) -> RenderTarget {
            let mesh = unit_cube_mesh();
            let model = Transform {
                translation: Vec3::ZERO,
                rotation_radians: Vec3::new(0.45, 0.0, 0.0),
                scale: Vec3::new(1.25, 1.25, 1.25),
            };
            let view = look_at_lh(eye, CAMERA_TARGET, CAMERA_WORLD_UP).unwrap();
            let projection =
                perspective_lh_zo(CAMERA_FOV_Y_RADIANS, 1.0, CAMERA_NEAR, CAMERA_FAR).unwrap();
            let mut scene = MeshScene::with_capacity(&mesh);
            scene.rebuild_with_pipeline(
                &mesh,
                TransformPipeline::new(model.model_matrix(), view, projection),
                64,
                64,
            );
            let mut target = RenderTarget::new(64, 64).unwrap();
            target.render_gradient_checker();
            draw_mesh(
                &mut target,
                false,
                raster_options(CullMode::Back, WindingDebugMode::VertexColor),
                &mesh,
                &mut TriangleClipper::default(),
                &scene.clip_vertices,
            );
            target
        }

        let previous = render_from_eye(Vec3::new(2.0, 1.5, -4.0));
        let chapter_fifteen = render_from_eye(CAMERA_EYE);
        let mut differing_pixels = 0_u32;
        let mut max_channel_difference = 0_u8;
        let mut bounds = None;
        for (index, (old, new)) in previous
            .color()
            .chunks_exact(4)
            .zip(chapter_fifteen.color().chunks_exact(4))
            .enumerate()
        {
            let pixel_difference = old.iter().zip(new).any(|(old, new)| old != new);
            if !pixel_difference {
                continue;
            }
            differing_pixels += 1;
            for (&old, &new) in old.iter().zip(new) {
                max_channel_difference = max_channel_difference.max(old.abs_diff(new));
            }
            let point = (index % 64, index / 64);
            let (min_x, min_y, max_x, max_y) =
                bounds.get_or_insert((point.0, point.1, point.0, point.1));
            *min_x = (*min_x).min(point.0);
            *min_y = (*min_y).min(point.1);
            *max_x = (*max_x).max(point.0);
            *max_y = (*max_y).max(point.1);
        }
        assert_eq!(
            (
                differing_pixels,
                max_channel_difference,
                bounds,
                fnv1a(previous.color()),
                fnv1a(chapter_fifteen.color()),
            ),
            (640, 215, Some((16, 15, 47, 45)), 0xe59c_1789, 0xf8d0_50be,)
        );
    }

    #[test]
    fn chapter_sixteen_texture_debug_preserves_rgba_channel_and_row_direction() {
        let texture = Texture::from_rgba8(
            2,
            2,
            &[255, 0, 0, 1, 0, 255, 0, 2, 0, 0, 255, 3, 255, 255, 255, 4],
            TextureColorSpace::Srgb,
        )
        .unwrap();
        let mut target = RenderTarget::new(4, 4).unwrap();
        assert_eq!(target.render_texture_nearest(&texture), 16);

        let pixel = |x: usize, y: usize| {
            let byte = 4 * (y * target.width() + x);
            &target.color()[byte..byte + 4]
        };
        assert_eq!(pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(3, 0), [0, 255, 0, 255]);
        assert_eq!(pixel(0, 3), [0, 0, 255, 255]);
        assert_eq!(pixel(3, 3), [255, 255, 255, 255]);
        assert!(target.depth().iter().all(|value| value.is_infinite()));
    }

    #[test]
    fn chapter_sixteen_nearest_scaling_cannot_overflow_wasm32_coordinates() {
        let texture_extent = texture::MAX_TEXTURE_PIXEL_COUNT;
        let target_extent = 960;
        let coordinates: Vec<_> = (0..target_extent)
            .map(|position| nearest_texture_coordinate(position, target_extent, texture_extent))
            .collect();
        assert_eq!(coordinates[0], 0);
        assert!(coordinates.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(coordinates[target_extent - 1] < texture_extent);
        assert_eq!(
            coordinates[target_extent - 1],
            ((target_extent - 1) as u64 * texture_extent as u64 / target_extent as u64) as usize
        );
    }

    #[test]
    fn chapter_sixteen_renderer_upload_status_and_failure_keep_active_texture() {
        let mut renderer = Renderer::new(4, 4).unwrap();
        assert_eq!(
            renderer.texture_asset_status(),
            TextureAssetStatus {
                active_texture_id: TextureId(0),
                active_width: 2,
                active_height: 2,
                mip_levels: 2,
                successful_uploads: 0,
                failed_uploads: 0,
            }
        );
        let uploaded = renderer
            .upload_texture_rgba8(
                2,
                2,
                &[
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
                TextureColorSpace::Srgb,
            )
            .unwrap();
        assert_eq!(uploaded, TextureId(1));
        let failure = renderer
            .upload_texture_rgba8(2, 2, &[0; 15], TextureColorSpace::Srgb)
            .unwrap_err();
        assert_eq!(
            failure,
            TextureError::ByteLengthMismatch {
                expected: 16,
                actual: 15,
            }
        );
        assert_eq!(renderer.texture_asset_status().active_texture_id, uploaded);
        assert_eq!(renderer.texture_asset_status().successful_uploads, 1);
        assert_eq!(renderer.texture_asset_status().failed_uploads, 1);
        assert_eq!(
            renderer.set_active_texture(TextureId(99)),
            Err(TextureError::InvalidTextureId(TextureId(99)))
        );

        renderer.set_texture_debug_enabled(true);
        assert!(renderer.texture_debug_enabled());
        renderer.set_model_rotation_y(f32::NAN);
        let stats = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(stats.texture_debug_pixels, 16);
        assert_eq!(stats.texture_upload_successes, 1);
        assert_eq!(stats.texture_upload_failures, 1);
        assert_eq!(stats.active_texture_id, uploaded.0);
        assert_eq!(stats.input_triangles, 0);
        assert_eq!(stats.invalid_values, 0);
        assert!(stats.pipeline_relations_hold());
        assert_eq!(&renderer.color_buffer()[0..4], &[255, 0, 0, 255]);

        renderer.resize(2, 2).unwrap();
        assert_eq!(
            renderer.color_buffer(),
            [
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ]
        );
        renderer.set_texture_debug_enabled(false);
        assert!(!renderer.texture_debug_enabled());
        assert!(
            renderer
                .update_and_render(0.0, InputSnapshot::default())
                .input_triangles
                > 0
        );
    }

    #[test]
    fn chapter_seventeen_textured_cube_samples_after_depth_and_changes_filter_output() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer
            .upload_texture_rgba8(
                2,
                2,
                &[
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
                TextureColorSpace::Srgb,
            )
            .unwrap();
        renderer.set_texture_sampling_enabled(true);
        assert!(renderer.texture_sampling_enabled());
        renderer.set_sampler_state(texture::SamplerState {
            address_u: texture::AddressMode::Repeat,
            address_v: texture::AddressMode::Repeat,
            filter: texture::FilterMode::Nearest,
        });
        let nearest = renderer.update_and_render(0.0, InputSnapshot::default());
        let nearest_hash = fnv1a(renderer.color_buffer());
        assert_eq!(nearest.texture_samples, nearest.shaded_samples);
        assert!(nearest.texture_samples > 0);
        assert!(nearest.pipeline_relations_hold());
        assert_eq!(
            renderer.sampler_state().filter,
            texture::FilterMode::Nearest
        );

        renderer.set_sampler_state(texture::SamplerState {
            filter: texture::FilterMode::Bilinear,
            ..renderer.sampler_state()
        });
        let bilinear = renderer.update_and_render(0.0, InputSnapshot::default());
        let bilinear_hash = fnv1a(renderer.color_buffer());
        assert_eq!(bilinear.texture_samples, bilinear.shaded_samples);
        assert_ne!(bilinear_hash, nearest_hash);
        renderer.resize(65, 65).unwrap();
        let resized = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(resized.texture_samples, resized.shaded_samples);
        assert!(resized.texture_samples > 0);
        renderer.set_texture_sampling_enabled(false);
        assert!(!renderer.texture_sampling_enabled());
        assert_eq!(
            renderer
                .update_and_render(0.0, InputSnapshot::default())
                .texture_samples,
            0
        );
    }

    #[test]
    fn chapter_seventeen_draw_item_resolves_its_own_material_sampler_and_texture() {
        let mut renderer = Renderer::new(32, 32).unwrap();
        renderer.materials.push(Material {
            base_color_texture: Some(TextureId(0)),
            sampler: texture::SamplerState {
                address_u: texture::AddressMode::ClampToEdge,
                address_v: texture::AddressMode::Repeat,
                filter: texture::FilterMode::Bilinear,
            },
            ..Material::default()
        });
        renderer.draw_item.material_id = MaterialId(1);

        assert!(renderer.texture_sampling_enabled());
        assert_eq!(renderer.sampler_state(), renderer.materials[1].sampler);
        assert_eq!(renderer.materials[0], Material::default());
        let stats = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(stats.texture_samples, stats.shaded_samples);
        assert!(stats.texture_samples > 0);

        let uploaded = renderer
            .upload_texture_rgba8(1, 1, &[12, 34, 56, 255], TextureColorSpace::Linear)
            .unwrap();
        assert_eq!(renderer.materials[1].base_color_texture, Some(uploaded));
        renderer.set_active_texture(TextureId(0)).unwrap();
        assert_eq!(renderer.materials[1].base_color_texture, Some(TextureId(0)));
        assert_eq!(material_for_id(&renderer.materials, MaterialId(99)), None);
    }

    #[test]
    fn chapter_seventeen_perspective_texture_is_seam_free_across_quad_diagonals() {
        let texture = Texture::from_rgba8(
            2,
            2,
            &[
                255, 32, 16, 255, 16, 255, 32, 255, 32, 16, 255, 255, 240, 240, 240, 255,
            ],
            TextureColorSpace::Srgb,
        )
        .unwrap();
        let sampler = texture::SamplerState {
            address_u: texture::AddressMode::ClampToEdge,
            address_v: texture::AddressMode::ClampToEdge,
            filter: texture::FilterMode::Bilinear,
        };
        let render = |alternate_diagonal| {
            let mesh = perspective_debug_fixture(alternate_diagonal);
            let scene = MeshScene::new_perspective_debug(&mesh, 64, 64);
            let mut target = RenderTarget::new(64, 64).unwrap();
            target.clear_color(Color::rgb(0, 0, 0));
            let report = draw_mesh(
                &mut target,
                false,
                RasterDrawOptions {
                    pipeline_state: PipelineState::default(),
                    uv_checker_enabled: false,
                    sampled_texture: Some((&texture, sampler)),
                    material: Material::default(),
                    linear_material: LinearMaterial::from_srgb(Material::default()),
                    light: DirectionalLight::default(),
                    camera_world: Vec3::ZERO,
                    sort_transparent: true,
                    blend_color_space: BlendColorSpace::Linear,
                    mipmap_enabled: false,
                    mip_debug_enabled: false,
                },
                &mesh,
                &mut TriangleClipper::default(),
                &scene.clip_vertices,
            );
            (target, report)
        };
        let (first, first_report) = render(false);
        let (second, second_report) = render(true);
        let mut differing_pixels = 0_u32;
        let mut max_channel_difference = 0_u8;
        for (first, second) in first
            .color()
            .chunks_exact(4)
            .zip(second.color().chunks_exact(4))
        {
            if first != second {
                differing_pixels += 1;
            }
            for (&first, &second) in first.iter().zip(second) {
                max_channel_difference = max_channel_difference.max(first.abs_diff(second));
            }
        }
        let max_depth_difference = first
            .depth()
            .iter()
            .zip(second.depth())
            .filter_map(|(first, second)| {
                if first.is_infinite() && second.is_infinite() {
                    None
                } else {
                    Some((first - second).abs())
                }
            })
            .fold(0.0_f32, f32::max);
        assert!(
            differing_pixels <= 64,
            "differing pixels: {differing_pixels}"
        );
        assert!(
            max_channel_difference <= 1,
            "max diff: {max_channel_difference}"
        );
        assert!(max_depth_difference <= DEPTH_RANGE_EPSILON);
        assert_eq!(first_report.texture_samples, first_report.shaded_samples);
        assert_eq!(second_report.texture_samples, second_report.shaded_samples);
    }

    #[test]
    fn chapter_eighteen_lambert_endpoints_and_light_validation_are_explicit() {
        let light = DirectionalLight::new(Vec3::Z, Vec3::new(1.0, 0.5, 0.25), 2.0).unwrap();
        assert_eq!(lambert_ndotl(Vec3::Z, light), 1.0);
        assert_eq!(lambert_ndotl(Vec3::X, light), 0.0);
        assert_eq!(lambert_ndotl(Vec3::new(0.0, 0.0, -1.0), light), 0.0);
        let material = Material {
            ambient: 0.1,
            shader_mode: ShaderMode::Lambert,
            ..Material::default()
        };
        let lit = shade_material_linear(
            Vec4::new(0.5, 0.5, 0.5, 0.75),
            Vec3::Z,
            Vec3::ZERO,
            material,
            LinearMaterial::from_srgb(material),
            light,
            Vec3::new(0.0, 0.0, 1.0),
        );
        assert_eq!(lit, Vec4::new(1.05, 0.55, 0.3, 0.75));
        assert_eq!(
            DirectionalLight::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0), 1.0),
            Err(LightingError::InvalidDirection)
        );
        assert_eq!(
            DirectionalLight::new(Vec3::Z, Vec3::new(-1.0, 1.0, 1.0), 1.0),
            Err(LightingError::InvalidColor)
        );
        assert_eq!(
            DirectionalLight::new(Vec3::Z, Vec3::new(1.0, 1.0, 1.0), f32::NAN),
            Err(LightingError::InvalidIntensity)
        );
        for error in [
            LightingError::InvalidDirection,
            LightingError::InvalidColor,
            LightingError::InvalidIntensity,
        ] {
            assert!(!error.to_string().is_empty());
        }

        let mesh = unit_cube_mesh();
        let model = Transform {
            rotation_radians: Vec3::new(0.3, 0.7, 0.0),
            scale: Vec3::new(2.0, 0.5, 1.5),
            ..Transform::IDENTITY
        };
        let first = MeshScene::new(&mesh, model, 64, 64);
        let mut moved_camera = MeshScene::with_capacity(&mesh);
        moved_camera.rebuild_cube_with_camera(
            &mesh,
            model,
            Vec3::new(2.0, 1.0, -4.0),
            Vec3::ZERO,
            64,
            64,
        );
        for (first, moved) in first.clip_vertices.iter().zip(&moved_camera.clip_vertices) {
            assert_eq!(first.normal_world, moved.normal_world);
            assert_eq!(
                lambert_ndotl(first.normal_world, light),
                lambert_ndotl(moved.normal_world, light)
            );
        }
    }

    #[test]
    fn chapter_eighteen_textured_lambert_and_debug_views_share_depth_coverage() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_texture_sampling_enabled(true);
        renderer.set_lighting_enabled(true);
        assert!(renderer.lighting_enabled());
        renderer
            .set_directional_light(Vec3::new(-0.4, 0.8, -0.45), 0.9)
            .unwrap();
        let lit = renderer.update_and_render(0.0, InputSnapshot::default());
        let lit_hash = fnv1a(renderer.color_buffer());
        assert_eq!(lit.lighting_samples, lit.shaded_samples);
        assert_eq!(lit.texture_samples, lit.shaded_samples);
        assert_eq!(renderer.directional_light().intensity, 0.9);

        renderer.set_pipeline_debug_mode(PipelineDebugMode::Normal);
        let normal = renderer.update_and_render(0.0, InputSnapshot::default());
        let normal_hash = fnv1a(renderer.color_buffer());
        assert_eq!(normal.lighting_samples, 0);
        assert_eq!(normal.covered_samples, lit.covered_samples);
        assert_ne!(normal_hash, lit_hash);

        renderer.set_pipeline_debug_mode(PipelineDebugMode::NdotL);
        let ndotl = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(ndotl.lighting_samples, ndotl.shaded_samples);
        assert_eq!(ndotl.covered_samples, lit.covered_samples);
        assert_ne!(fnv1a(renderer.color_buffer()), normal_hash);

        renderer.set_pipeline_debug_mode(PipelineDebugMode::Solid);
        renderer.set_normal_mode(NormalMode::Flat);
        assert_eq!(renderer.normal_mode(), NormalMode::Flat);
        let flat = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(flat.lighting_samples, flat.shaded_samples);
        assert_eq!(fnv1a(renderer.color_buffer()), lit_hash);

        renderer.set_model_rotation_y(0.75);
        renderer.update_and_render(0.0, InputSnapshot::default());
        assert_ne!(fnv1a(renderer.color_buffer()), lit_hash);
        assert!(renderer.set_directional_light(Vec3::ZERO, 1.0).is_err());
        assert_eq!(renderer.directional_light().intensity, 0.9);
        renderer.set_lighting_enabled(false);
        assert!(!renderer.lighting_enabled());
    }

    #[test]
    fn chapter_eighteen_flat_normal_ignores_vertex_normal_and_rejects_degenerate_geometry() {
        let mesh = interpolation_debug_fixture();
        let scene = MeshScene::new_identity_debug(&mesh, 1, 1);
        let mut zero_vertex_normals: [ClipVertex; 3] =
            scene.clip_vertices.clone().try_into().unwrap();
        for vertex in &mut zero_vertex_normals {
            vertex.normal_world = Vec3::ZERO;
        }
        let covered_positions = [
            ViewportPosition {
                x: -1.0,
                y: -1.0,
                z_ndc: 0.5,
            },
            ViewportPosition {
                x: 3.0,
                y: -1.0,
                z_ndc: 0.5,
            },
            ViewportPosition {
                x: -1.0,
                y: 3.0,
                z_ndc: 0.5,
            },
        ];
        let mut options = raster_options(CullMode::None, WindingDebugMode::VertexColor);
        options.material.normal_mode = NormalMode::Flat;
        let mut target = RenderTarget::new(1, 1).unwrap();
        let mut fallback_report = FrameDrawReport::default();
        submit_triangle(
            &mut target,
            false,
            options,
            zero_vertex_normals,
            covered_positions,
            0,
            &mut fallback_report,
        );
        assert_eq!(fallback_report.submitted_triangles, 1);
        assert_eq!(fallback_report.invalid_interpolation_samples, 0);
        assert_eq!(fallback_report.shaded_samples, 1);

        let mut generated: [ClipVertex; 3] = scene.clip_vertices.try_into().unwrap();
        for vertex in &mut generated {
            vertex.world_pos = Vec3::ZERO;
        }
        let positions = [
            ViewportPosition {
                x: 0.0,
                y: 0.0,
                z_ndc: 0.5,
            },
            ViewportPosition {
                x: 1.0,
                y: 0.0,
                z_ndc: 0.5,
            },
            ViewportPosition {
                x: 0.0,
                y: 1.0,
                z_ndc: 0.5,
            },
        ];
        let mut target = RenderTarget::new(1, 1).unwrap();
        let mut report = FrameDrawReport::default();
        submit_triangle(
            &mut target,
            false,
            options,
            generated,
            positions,
            0,
            &mut report,
        );
        assert_eq!(report.invalid_triangles, 1);
        assert_eq!(report.submitted_triangles, 0);
        assert_eq!(report.shaded_samples, 0);
    }

    #[test]
    fn chapter_nineteen_blinn_phong_endpoints_camera_dependence_and_material_validation() {
        assert_eq!(
            linear_display_color(Vec4::new(0.5, 0.5, 0.5, 1.0)).rgba(),
            [188, 188, 188, 255],
            "vertex color varying은 linear에서 보간한 뒤 한 번만 encode해야 한다"
        );
        let authored = Material {
            base_color: Vec4::new(0.5, 0.5, 0.5, 1.0),
            specular_color: Vec3::new(0.5, 0.5, 0.5),
            ..Material::default()
        };
        let decoded = LinearMaterial::from_srgb(authored);
        assert!((decoded.base_color.x - 0.214_041_14).abs() <= 1.0e-7);
        assert!((decoded.specular_color.x - 0.214_041_14).abs() <= 1.0e-7);

        assert_eq!(
            blinn_phong_specular_factor(Vec3::Z, Vec3::Z, Vec3::Z, 32.0),
            1.0
        );
        assert_eq!(
            blinn_phong_specular_factor(Vec3::Z, Vec3::new(0.0, 0.0, -1.0), Vec3::Z, 32.0,),
            0.0
        );
        assert_eq!(
            blinn_phong_specular_factor(Vec3::Z, Vec3::Z, Vec3::new(0.0, 0.0, -1.0), 32.0,),
            0.0
        );
        assert_eq!(
            blinn_phong_specular_factor(Vec3::Z, Vec3::Z, Vec3::Z, f32::NAN),
            0.0
        );

        let light = DirectionalLight::new(Vec3::Z, Vec3::new(1.0, 1.0, 1.0), 1.0).unwrap();
        let albedo = Vec4::new(0.25, 0.25, 0.25, 1.0);
        let lambert = Material {
            ambient: 0.0,
            shader_mode: ShaderMode::Lambert,
            ..Material::default()
        };
        let blinn_phong = Material {
            shader_mode: ShaderMode::BlinnPhong,
            ..lambert
        };
        let camera_center = Vec3::new(0.0, 0.0, 2.0);
        let camera_side = Vec3::new(2.0, 0.0, 2.0);
        assert_eq!(
            shade_material_linear(
                albedo,
                Vec3::Z,
                Vec3::ZERO,
                lambert,
                LinearMaterial::from_srgb(lambert),
                light,
                camera_center,
            ),
            shade_material_linear(
                albedo,
                Vec3::Z,
                Vec3::ZERO,
                lambert,
                LinearMaterial::from_srgb(lambert),
                light,
                camera_side,
            )
        );
        let centered = shade_material_linear(
            albedo,
            Vec3::Z,
            Vec3::ZERO,
            blinn_phong,
            LinearMaterial::from_srgb(blinn_phong),
            light,
            camera_center,
        );
        let side = shade_material_linear(
            albedo,
            Vec3::Z,
            Vec3::ZERO,
            blinn_phong,
            LinearMaterial::from_srgb(blinn_phong),
            light,
            camera_side,
        );
        assert!(centered.x > side.x);

        let cube = unit_cube_mesh();
        let cube_scene = MeshScene::new(&cube, Transform::IDENTITY, 32, 32);
        assert_eq!(cube_scene.camera_world, CAMERA_EYE);
        let debug_mesh = interpolation_debug_fixture();
        let identity_scene = MeshScene::new_identity_debug(&debug_mesh, 32, 32);
        let perspective_scene = MeshScene::new_perspective_debug(&debug_mesh, 32, 32);
        assert_eq!(identity_scene.camera_world, Vec3::ZERO);
        assert_eq!(perspective_scene.camera_world, Vec3::ZERO);

        let mut renderer = Renderer::new(8, 8).unwrap();
        renderer.set_shader_mode(ShaderMode::BlinnPhong);
        assert_eq!(renderer.shader_mode(), ShaderMode::BlinnPhong);
        renderer.set_lighting_enabled(true);
        assert_eq!(renderer.shader_mode(), ShaderMode::BlinnPhong);
        renderer
            .set_material_specular(Vec3::new(0.25, 0.5, 1.0), 64.0)
            .unwrap();
        let approved = renderer.material_specular();
        assert_eq!(approved, (Vec3::new(0.25, 0.5, 1.0), 64.0));
        assert_eq!(
            renderer.set_material_specular(Vec3::new(-1.0, 0.5, 1.0), 8.0),
            Err(MaterialError::InvalidSpecularColor)
        );
        assert_eq!(renderer.material_specular(), approved);
        assert_eq!(
            renderer.set_material_specular(Vec3::new(1.0, 1.0, 1.0), 0.0),
            Err(MaterialError::InvalidShininess)
        );
        assert_eq!(renderer.material_specular(), approved);
        for error in [
            MaterialError::InvalidSpecularColor,
            MaterialError::InvalidShininess,
        ] {
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn chapter_nineteen_shader_and_color_space_debug_views_share_geometry_and_depth() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer
            .upload_texture_rgba8(
                2,
                1,
                &[0, 0, 0, 255, 255, 255, 255, 255],
                TextureColorSpace::Srgb,
            )
            .unwrap();
        renderer.set_texture_sampling_enabled(true);
        renderer.set_sampler_state(SamplerState {
            address_u: texture::AddressMode::ClampToEdge,
            address_v: texture::AddressMode::ClampToEdge,
            filter: texture::FilterMode::Bilinear,
        });
        renderer.set_shader_mode(ShaderMode::Lambert);
        let lambert = renderer.update_and_render(0.0, InputSnapshot::default());
        let lambert_hash = fnv1a(renderer.color_buffer());
        assert_eq!(lambert.texture_samples, lambert.shaded_samples);
        assert_eq!(lambert.lighting_samples, lambert.shaded_samples);

        renderer.set_shader_mode(ShaderMode::BlinnPhong);
        let blinn = renderer.update_and_render(0.0, InputSnapshot::default());
        let blinn_hash = fnv1a(renderer.color_buffer());
        assert_ne!(blinn_hash, lambert_hash);
        assert_eq!(blinn.covered_samples, lambert.covered_samples);

        let mut hashes = Vec::new();
        for mode in [
            PipelineDebugMode::Diffuse,
            PipelineDebugMode::Specular,
            PipelineDebugMode::ColorSpaceComparison,
        ] {
            renderer.set_pipeline_debug_mode(mode);
            let stats = renderer.update_and_render(0.0, InputSnapshot::default());
            assert_eq!(stats.covered_samples, lambert.covered_samples);
            assert_eq!(stats.depth_passed_samples, lambert.depth_passed_samples);
            hashes.push(fnv1a(renderer.color_buffer()));
        }
        assert_ne!(hashes[0], hashes[1]);
        assert_ne!(hashes[1], hashes[2]);
        assert_ne!(hashes[0], hashes[2]);
    }

    #[test]
    fn chapter_twenty_two_alpha_modes_split_queues_and_validate_cutoff_atomically() {
        let mut empty_report = FrameDrawReport::default();
        empty_report.absorb(FrameDrawReport::default());
        assert_eq!(empty_report, FrameDrawReport::default());
        assert_eq!(AlphaMode::Opaque.render_queue(), RenderQueue::Opaque);
        assert_eq!(AlphaMode::Mask.render_queue(), RenderQueue::Cutout);
        assert_eq!(AlphaMode::Blend.render_queue(), RenderQueue::Transparent);
        assert!(AlphaMode::Opaque.writes_depth());
        assert!(AlphaMode::Mask.writes_depth());
        assert!(!AlphaMode::Blend.writes_depth());

        let mut renderer = Renderer::new(8, 8).unwrap();
        renderer.set_alpha_mode(AlphaMode::Mask);
        assert_eq!(renderer.alpha_mode(), AlphaMode::Mask);
        renderer.set_alpha_cutoff(0.25).unwrap();
        assert_eq!(renderer.alpha_cutoff(), 0.25);
        for invalid in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert_eq!(
                renderer.set_alpha_cutoff(invalid),
                Err(MaterialAlphaError::InvalidCutoff)
            );
            assert_eq!(renderer.alpha_cutoff(), 0.25);
        }
        assert!(
            MaterialAlphaError::InvalidCutoff
                .to_string()
                .contains("0..1")
        );
    }

    #[test]
    fn chapter_twenty_two_source_over_is_linear_and_preserves_endpoint_identity() {
        let black = Vec4::new(0.0, 0.0, 0.0, 1.0);
        let white = Vec4::new(1.0, 1.0, 1.0, 1.0);
        assert_eq!(
            blend_source_over_linear(Vec4::new(1.0, 0.0, 0.0, 0.0), black),
            Some(black)
        );
        assert_eq!(blend_source_over_linear(white, black), Some(white));
        assert_eq!(
            blend_source_over_linear(Vec4::new(1.0, 1.0, 1.0, 0.5), black),
            Some(Vec4::new(0.5, 0.5, 0.5, 1.0))
        );
        assert_eq!(
            blend_source_over_linear(Vec4::new(2.0, 0.0, 0.0, 0.5), black),
            Some(Vec4::new(1.0, 0.0, 0.0, 1.0))
        );
        assert_eq!(
            blend_source_over_linear(Vec4::new(f32::NAN, 0.0, 0.0, 1.0), black),
            None
        );

        let mut target = RenderTarget::new(1, 1).unwrap();
        target.clear_color(Color::rgb(12, 34, 56));
        assert!(target.blend_color_without_depth(
            ScreenPoint::new(0, 0),
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            BlendColorSpace::Linear,
        ));
        assert_eq!(pixel(&target, 0, 0), [12, 34, 56, 255]);
        target.clear_color(Color::rgb(0, 0, 0));
        assert!(target.blend_color_without_depth(
            ScreenPoint::new(0, 0),
            Vec4::new(1.0, 1.0, 1.0, 0.5),
            BlendColorSpace::Linear,
        ));
        assert_eq!(pixel(&target, 0, 0), [188, 188, 188, 255]);
        target.clear_color(Color::rgb(0, 0, 0));
        assert!(target.blend_color_without_depth(
            ScreenPoint::new(0, 0),
            Vec4::new(2.0, 0.0, 0.0, 0.5),
            BlendColorSpace::Linear,
        ));
        assert_eq!(pixel(&target, 0, 0), [255, 0, 0, 255]);
        target.clear_color(Color::rgb(0, 0, 0));
        assert!(target.blend_color_without_depth(
            ScreenPoint::new(0, 0),
            Vec4::new(1.0, 1.0, 1.0, 0.5),
            BlendColorSpace::EncodedWrongWay,
        ));
        assert_eq!(pixel(&target, 0, 0), [127, 127, 127, 255]);
        assert!(!target.blend_color_without_depth(
            ScreenPoint::new(-1, 0),
            white,
            BlendColorSpace::Linear,
        ));
    }

    #[test]
    fn chapter_twenty_two_cutout_writes_only_kept_depth_and_blend_never_writes_depth() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_transparency_debug_enabled(true);
        assert!(renderer.transparency_debug_enabled());
        let stats = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!((stats.input_vertices, stats.input_triangles), (16, 8));
        assert!(stats.alpha_discarded_samples > 0);
        assert!(stats.blended_samples > 0);
        assert!(stats.depth_written_samples > 0);
        assert!(stats.depth_written_samples < stats.depth_passed_samples);
        assert!(stats.pipeline_relations_hold());
        let finite_depths: Vec<_> = renderer
            .depth_buffer()
            .iter()
            .copied()
            .filter(|depth| depth.is_finite())
            .collect();
        assert!(
            finite_depths
                .iter()
                .any(|depth| (*depth - 0.18).abs() < 1.0e-5)
        );
        assert!(
            finite_depths
                .iter()
                .any(|depth| (*depth - 0.88).abs() < 1.0e-5)
        );
        assert!(
            finite_depths
                .iter()
                .all(|depth| { (*depth - 0.18).abs() < 1.0e-5 || (*depth - 0.88).abs() < 1.0e-5 })
        );
        let debug = renderer.coordinate_debug_snapshot();
        assert!(debug.transparency_debug_enabled);
        assert_eq!(debug.mesh_vertices, 8);
        renderer.resize(32, 48).unwrap();
        assert_eq!((renderer.width(), renderer.height()), (32, 48));
        let resized = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!((resized.input_vertices, resized.input_triangles), (16, 8));
    }

    #[test]
    fn chapter_twenty_two_sort_and_color_space_debugs_are_deterministic_and_distinct() {
        let mut renderer = Renderer::new(64, 64).unwrap();
        renderer.set_transparency_debug_enabled(true);
        let sorted = renderer.update_and_render(0.0, InputSnapshot::default());
        let sorted_hash = fnv1a(renderer.color_buffer());
        assert!(renderer.transparent_sort_enabled());
        assert_eq!(renderer.blend_color_space(), BlendColorSpace::Linear);

        renderer.set_transparent_sort_enabled(false);
        let unsorted = renderer.update_and_render(0.0, InputSnapshot::default());
        let unsorted_hash = fnv1a(renderer.color_buffer());
        assert_ne!(sorted_hash, unsorted_hash);
        assert_eq!(sorted.covered_samples, unsorted.covered_samples);
        assert_eq!(sorted.depth_written_samples, unsorted.depth_written_samples);

        renderer.set_transparent_sort_enabled(true);
        renderer.set_blend_color_space(BlendColorSpace::EncodedWrongWay);
        let wrong = renderer.update_and_render(0.0, InputSnapshot::default());
        let wrong_hash = fnv1a(renderer.color_buffer());
        assert_ne!(sorted_hash, wrong_hash);
        assert_eq!(wrong.covered_samples, sorted.covered_samples);
        renderer.set_transparency_debug_enabled(false);
        assert!(!renderer.transparency_debug_enabled());
    }

    #[test]
    fn chapter_twenty_two_debug_colors_preserve_mask_and_blend_depth_policy() {
        let mask_mesh = transparency_quad_fixture(
            -0.8,
            0.8,
            0.8,
            -0.8,
            [0.4; 4],
            Vec4::new(1.0, 1.0, 1.0, 1.0),
        );
        let mask_scene = MeshScene::new_identity_debug(&mask_mesh, 32, 32);
        let cutout = transparency_cutout_texture();
        let sampler = SamplerState {
            address_u: texture::AddressMode::ClampToEdge,
            address_v: texture::AddressMode::ClampToEdge,
            filter: texture::FilterMode::Nearest,
        };
        let blend_mesh = transparency_quad_fixture(
            -0.7,
            0.7,
            0.7,
            -0.7,
            [0.3; 4],
            Vec4::new(1.0, 0.2, 0.1, 0.5),
        );
        let blend_scene = MeshScene::new_identity_debug(&blend_mesh, 32, 32);
        let mut mask_coverage = None;
        let mut blend_coverage = None;
        for debug_mode in [
            PipelineDebugMode::Solid,
            PipelineDebugMode::Wireframe,
            PipelineDebugMode::Normal,
            PipelineDebugMode::Diffuse,
        ] {
            let pipeline_state = PipelineState {
                debug_mode,
                ..PipelineState::default()
            };
            let mask_material = Material {
                alpha_mode: AlphaMode::Mask,
                alpha_cutoff: 0.5,
                ..Material::default()
            };
            let mut target = RenderTarget::new(32, 32).unwrap();
            target.clear_color(Color::rgb(8, 12, 20));
            let mask = draw_mesh(
                &mut target,
                false,
                RasterDrawOptions {
                    pipeline_state,
                    uv_checker_enabled: false,
                    sampled_texture: Some((&cutout, sampler)),
                    material: mask_material,
                    linear_material: LinearMaterial::from_srgb(mask_material),
                    light: DirectionalLight::default(),
                    camera_world: Vec3::ZERO,
                    sort_transparent: true,
                    blend_color_space: BlendColorSpace::Linear,
                    mipmap_enabled: false,
                    mip_debug_enabled: false,
                },
                &mask_mesh,
                &mut TriangleClipper::default(),
                &mask_scene.clip_vertices,
            );
            assert!(mask.alpha_discarded_samples > 0);
            assert!(mask.depth_written_samples > 0);
            assert_eq!(mask.blended_samples, 0);
            assert_eq!(mask.texture_samples, mask.interpolated_inv_w_samples);
            assert_eq!(
                *mask_coverage.get_or_insert(mask.covered_samples),
                mask.covered_samples
            );

            let blend_material = Material {
                alpha_mode: AlphaMode::Blend,
                ..Material::default()
            };
            target.clear_color(Color::rgb(8, 12, 20));
            let blend = draw_mesh(
                &mut target,
                false,
                RasterDrawOptions {
                    pipeline_state,
                    uv_checker_enabled: false,
                    sampled_texture: None,
                    material: blend_material,
                    linear_material: LinearMaterial::from_srgb(blend_material),
                    light: DirectionalLight::default(),
                    camera_world: Vec3::ZERO,
                    sort_transparent: true,
                    blend_color_space: BlendColorSpace::Linear,
                    mipmap_enabled: false,
                    mip_debug_enabled: false,
                },
                &blend_mesh,
                &mut TriangleClipper::default(),
                &blend_scene.clip_vertices,
            );
            assert_eq!(blend.alpha_discarded_samples, 0);
            assert_eq!(blend.depth_written_samples, 0);
            assert!(blend.blended_samples > 0);
            assert!(target.depth().iter().all(|depth| depth.is_infinite()));
            assert_eq!(
                *blend_coverage.get_or_insert(blend.covered_samples),
                blend.covered_samples
            );
        }
    }

    #[test]
    fn chapter_twenty_three_ssaa_resolve_averages_linear_color_and_preserves_solid_regions() {
        let mut source = RenderTarget::new(4, 2).unwrap();
        source.clear_color(Color::rgb(37, 149, 221));
        let mut destination = RenderTarget::new(2, 1).unwrap();
        assert!(destination.resolve_ssaa_2x_from(&source));
        assert_eq!(destination.color(), &[37, 149, 221, 255, 37, 149, 221, 255]);

        source.put_pixel(ScreenPoint::new(0, 0), Color::rgb(0, 0, 0));
        source.put_pixel(ScreenPoint::new(1, 0), Color::rgb(255, 255, 255));
        source.put_pixel(ScreenPoint::new(0, 1), Color::rgb(0, 0, 0));
        source.put_pixel(ScreenPoint::new(1, 1), Color::rgb(255, 255, 255));
        source.depth[0] = 0.8;
        source.depth[1] = 0.2;
        assert!(destination.resolve_ssaa_2x_from(&source));
        assert_eq!(&destination.color()[..4], &[188, 188, 188, 255]);
        assert_eq!(destination.depth()[0], 0.2);
        assert!(!destination.resolve_ssaa_2x_from(&RenderTarget::new(3, 2).unwrap()));
    }

    #[test]
    fn chapter_twenty_three_quality_mode_keeps_public_framebuffer_logical_and_costs_four_samples() {
        let mut renderer = Renderer::new(48, 32).unwrap();
        let no_aa = renderer.update_and_render(0.0, InputSnapshot::default());
        let public_len = renderer.color_buffer().len();
        renderer.set_quality_mode(QualityMode::Ssaa2x).unwrap();
        renderer.set_quality_mode(QualityMode::Ssaa2x).unwrap();
        renderer.clear([7, 11, 13]);
        let ssaa = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(renderer.quality_mode(), QualityMode::Ssaa2x);
        assert_eq!(renderer.render_dimensions_public(), (96, 64));
        assert_eq!(renderer.color_buffer().len(), public_len);
        assert_eq!(ssaa.render_scale, 2);
        assert_eq!(ssaa.resolved_pixels, 48 * 32);
        assert!(ssaa.shaded_samples >= no_aa.shaded_samples.saturating_mul(3));
        renderer.set_quality_mode(QualityMode::NoAa).unwrap();
        assert_eq!(renderer.render_dimensions_public(), (48, 32));
        assert_eq!(renderer.stats().render_scale, 1);
        assert_eq!(QualityMode::NoAa.label(), "no AA");
        assert_eq!(QualityMode::Ssaa2x.label(), "2x SSAA");

        let logical_pixels = MAX_PIXEL_COUNT / 4 + 1;
        let mut maximum = Renderer::new(logical_pixels, 1).unwrap();
        assert_eq!(
            maximum.set_quality_mode(QualityMode::Ssaa2x),
            Err(RenderTargetError::PixelLimitExceeded {
                requested: logical_pixels * 4,
                maximum: MAX_PIXEL_COUNT,
            })
        );
        assert_eq!(maximum.quality_mode(), QualityMode::NoAa);
    }

    #[test]
    fn chapter_twenty_three_perspective_lod_increases_as_the_rendered_quad_shrinks() {
        let texture =
            Texture::from_rgba8(256, 256, &vec![180; 256 * 256 * 4], TextureColorSpace::Srgb)
                .unwrap();
        let sampler = SamplerState::default();
        let render = |extent| {
            let mesh = perspective_debug_fixture(false);
            let scene = MeshScene::new_perspective_debug(&mesh, extent, extent);
            let mut target = RenderTarget::new(extent, extent).unwrap();
            draw_mesh(
                &mut target,
                false,
                RasterDrawOptions {
                    pipeline_state: PipelineState {
                        debug_mode: PipelineDebugMode::ColorSpaceComparison,
                        ..PipelineState::default()
                    },
                    uv_checker_enabled: false,
                    sampled_texture: Some((&texture, sampler)),
                    material: Material::default(),
                    linear_material: LinearMaterial::from_srgb(Material::default()),
                    light: DirectionalLight::default(),
                    camera_world: Vec3::ZERO,
                    sort_transparent: true,
                    blend_color_space: BlendColorSpace::Linear,
                    mipmap_enabled: true,
                    mip_debug_enabled: true,
                },
                &mesh,
                &mut TriangleClipper::default(),
                &scene.clip_vertices,
            )
        };
        let near = render(128);
        let far = render(32);
        assert!(near.mip_samples > 0);
        assert_eq!(near.invalid_lod_samples, 0);
        assert!(far.min_mip_level >= near.min_mip_level);
        assert!(far.max_mip_level > near.max_mip_level);

        assert_eq!(
            mip_lod_from_uv_derivatives(Vec2::new(f32::MAX, 0.0), Vec2::ZERO, usize::MAX, 1,),
            None
        );
    }

    #[test]
    fn chapter_twenty_three_frame_report_absorbs_mip_ranges() {
        let mut combined = FrameDrawReport::default();
        combined.absorb(FrameDrawReport {
            mip_samples: 3,
            min_mip_level: 2,
            max_mip_level: 4,
            invalid_lod_samples: 1,
            ..FrameDrawReport::default()
        });
        assert_eq!((combined.min_mip_level, combined.max_mip_level), (2, 4));
        combined.absorb(FrameDrawReport {
            mip_samples: 5,
            min_mip_level: 1,
            max_mip_level: 6,
            invalid_lod_samples: 2,
            ..FrameDrawReport::default()
        });
        assert_eq!(combined.mip_samples, 8);
        assert_eq!((combined.min_mip_level, combined.max_mip_level), (1, 6));
        assert_eq!(combined.invalid_lod_samples, 3);
        let mut invalid_samples = 0;
        let mut overflow = false;
        assert_eq!(
            observe_lod_value(Some(7_u32), &mut invalid_samples, &mut overflow),
            Some(7)
        );
        assert_eq!(
            observe_lod_value::<u32>(None, &mut invalid_samples, &mut overflow),
            None
        );
        assert_eq!(invalid_samples, 1);
        assert!(!overflow);
    }

    #[test]
    fn chapter_twenty_three_mip_debug_overrides_all_shader_views_and_blend_source() {
        let texture =
            Texture::from_rgba8(256, 256, &vec![190; 256 * 256 * 4], TextureColorSpace::Srgb)
                .unwrap();
        let mesh = perspective_debug_fixture(false);
        let scene = MeshScene::new_perspective_debug(&mesh, 32, 32);
        let render = |debug_mode, alpha_mode, mip_debug_enabled| {
            let material = Material {
                base_color: Vec4::new(1.0, 1.0, 1.0, 0.5),
                alpha_mode,
                ..Material::default()
            };
            let mut target = RenderTarget::new(32, 32).unwrap();
            target.clear_color(Color::rgb(0, 0, 0));
            let report = draw_mesh(
                &mut target,
                false,
                RasterDrawOptions {
                    pipeline_state: PipelineState {
                        debug_mode,
                        ..PipelineState::default()
                    },
                    uv_checker_enabled: false,
                    sampled_texture: Some((&texture, SamplerState::default())),
                    material,
                    linear_material: LinearMaterial::from_srgb(material),
                    light: DirectionalLight::default(),
                    camera_world: Vec3::ZERO,
                    sort_transparent: true,
                    blend_color_space: BlendColorSpace::Linear,
                    mipmap_enabled: true,
                    mip_debug_enabled,
                },
                &mesh,
                &mut TriangleClipper::default(),
                &scene.clip_vertices,
            );
            (target.color().to_vec(), report)
        };
        let (solid, solid_report) = render(PipelineDebugMode::Solid, AlphaMode::Opaque, true);
        let (normal, normal_report) = render(PipelineDebugMode::Normal, AlphaMode::Opaque, true);
        assert_eq!(solid, normal);
        assert_eq!(solid_report.covered_samples, normal_report.covered_samples);
        let (blend_debug, blend_report) =
            render(PipelineDebugMode::Diffuse, AlphaMode::Blend, true);
        let (blend_plain, _) = render(PipelineDebugMode::Diffuse, AlphaMode::Blend, false);
        assert_ne!(blend_debug, blend_plain);
        assert!(blend_report.blended_samples > 0);
        assert!(blend_report.mip_samples > 0);
    }

    #[test]
    fn chapter_twenty_three_mip_debug_exclusivity_is_symmetric() {
        let mut renderer = Renderer::new(16, 16).unwrap();
        renderer.set_mip_debug_enabled(true);
        assert!(renderer.texture_sampling_enabled());
        renderer.set_clip_debug_enabled(true);
        assert!(!renderer.mip_debug_enabled());
        renderer.set_mip_debug_enabled(true);
        renderer.set_coverage_debug_enabled(true);
        assert!(!renderer.mip_debug_enabled());
        renderer.set_mip_debug_enabled(true);
        renderer.set_interpolation_debug_enabled(true);
        assert!(!renderer.mip_debug_enabled());
        renderer.set_mip_debug_enabled(true);
        renderer.set_perspective_debug_enabled(true);
        assert!(!renderer.mip_debug_enabled());
        renderer.set_mip_debug_enabled(true);
        renderer.set_depth_debug_enabled(true);
        assert!(!renderer.mip_debug_enabled());
        renderer.set_mip_debug_enabled(true);
        renderer.set_transparency_debug_enabled(true);
        assert!(!renderer.mip_debug_enabled());
        renderer.set_mip_debug_enabled(true);
        renderer.set_texture_debug_enabled(true);
        assert!(!renderer.mip_debug_enabled());
        renderer.set_mip_debug_enabled(true);
        renderer.set_texture_sampling_enabled(false);
        assert!(!renderer.mip_debug_enabled());
    }
}
