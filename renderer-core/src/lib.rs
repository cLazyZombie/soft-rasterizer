//! Browser APIs에 의존하지 않는 소프트웨어 래스터라이저의 순수 Rust 코어.
//!
//! 14장까지 homogeneous clipping 뒤 fixed-point coverage, strict depth test와
//! perspective-correct fragment 속성 복원으로 mesh를 그린다.

pub mod camera;
pub mod clip;
pub mod math;
pub mod mesh;
pub mod raster;
pub mod transform;

use std::error::Error;
use std::fmt::{Display, Formatter};

use camera::{
    NdcPosition, ViewportPosition, look_at_lh, perspective_divide, perspective_lh_zo, viewport,
};
use clip::{ClipPlane, ClipStatus, TriangleClipper};
use math::{Mat4, Vec2, Vec3, Vec4};
use mesh::{ClipVertex, DrawItem, MaterialId, Mesh, MeshId, unit_cube_mesh};
use raster::{
    AttributeInterpolationMode, CullMode, DepthDebugMode, FaceOrientation, FragmentInput,
    ScreenVertex, TriangleDisposition, TriangleSetup, TriangleSetupError, WindingDebugMode,
    classify_triangle, normalized_channel_to_u8,
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
const CAMERA_EYE: Vec3 = Vec3::new(2.0, 1.5, -4.0);
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
    pub shaded_samples: u32,
    pub depth_passed_samples: u32,
    pub depth_failed_samples: u32,
    pub invalid_depth_samples: u32,
    pub max_barycentric_sum_error: f32,
    pub interpolated_inv_w_samples: u32,
    pub invalid_interpolation_samples: u32,
    pub min_interpolated_inv_w: f32,
    pub max_interpolated_inv_w: f32,
    pub debug_pixels: u32,
    pub invalid_values: u32,
}

/// 아직 의미를 부여하지 않은 장치 입력을 한 프레임 단위로 전달하는 작은 값이다.
///
/// 실제 키/포인터 비트 배치는 입력 카메라를 구현하는 20장에서 정한다.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputSnapshot {
    packed_bits: u32,
}

impl InputSnapshot {
    pub const fn from_packed(packed_bits: u32) -> Self {
        Self { packed_bits }
    }

    pub const fn packed_bits(self) -> u32 {
        self.packed_bits
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
        let aspect = width as f32 / height as f32;
        let view = look_at_lh(CAMERA_EYE, CAMERA_TARGET, CAMERA_WORLD_UP)
            .expect("고정 카메라 view 계약은 항상 유효해야 한다");
        let projection = perspective_lh_zo(CAMERA_FOV_Y_RADIANS, aspect, CAMERA_NEAR, CAMERA_FAR)
            .expect("유효한 렌더 타깃의 고정 projection 계약은 항상 유효해야 한다");
        let pipeline = TransformPipeline::new(model.model_matrix(), view, projection);
        self.rebuild_with_pipeline(mesh, pipeline, width, height);
    }

    fn rebuild_identity_debug(&mut self, mesh: &Mesh, width: usize, height: usize) {
        let identity = Mat4::identity();
        self.rebuild_with_pipeline(
            mesh,
            TransformPipeline::new(identity, identity, identity),
            width,
            height,
        );
    }

    fn rebuild_perspective_debug(&mut self, mesh: &Mesh, width: usize, height: usize) {
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
            let normal = pipeline.transform_model_direction(vertex.normal_object);
            let raw_normal = Vec3::new(normal.x, normal.y, normal.z);
            self.clip_vertices.push(ClipVertex {
                clip_pos: trace.clip_pos,
                world_pos: Vec3::new(
                    trace.world_pos.0.x,
                    trace.world_pos.0.y,
                    trace.world_pos.0.z,
                ),
                normal_world: raw_normal.normalized().unwrap_or(raw_normal),
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
}

/// 렌더 타깃과 3-14장 indexed mesh/clipping/coverage/interpolation/depth debug scene 상태를 소유한다.
#[derive(Debug)]
pub struct Renderer {
    target: RenderTarget,
    stats: FrameStats,
    framebuffer_generation: u32,
    debug_lines_enabled: bool,
    cull_mode: CullMode,
    winding_debug_mode: WindingDebugMode,
    clip_debug_enabled: bool,
    coverage_debug_enabled: bool,
    interpolation_debug_enabled: bool,
    perspective_debug_enabled: bool,
    attribute_interpolation_mode: AttributeInterpolationMode,
    depth_debug_enabled: bool,
    depth_order_reversed: bool,
    depth_debug_mode: DepthDebugMode,
    mesh: Mesh,
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
    clipper: TriangleClipper,
}

impl Renderer {
    const fn active_scene(&self) -> ActiveScene {
        if self.depth_debug_enabled {
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
        let mesh = unit_cube_mesh();
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
        let mut renderer = Self {
            target,
            stats: FrameStats::default(),
            framebuffer_generation: 0,
            debug_lines_enabled: true,
            cull_mode: CullMode::Back,
            winding_debug_mode: WindingDebugMode::VertexColor,
            clip_debug_enabled: false,
            coverage_debug_enabled: false,
            interpolation_debug_enabled: false,
            perspective_debug_enabled: false,
            attribute_interpolation_mode: AttributeInterpolationMode::PerspectiveCorrect,
            depth_debug_enabled: false,
            depth_order_reversed: false,
            depth_debug_mode: DepthDebugMode::Off,
            mesh,
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
            clipper: TriangleClipper::default(),
        };
        let draw_options = FrameDrawOptions::from_renderer(&renderer);
        draw_frame(
            &mut renderer.target,
            draw_options,
            &renderer.mesh,
            &mut renderer.clipper,
            &renderer.mesh_scene.clip_vertices,
            CUBE_SELECTED_VERTEX_INDEX,
        );
        Ok(renderer)
    }

    pub fn resize(&mut self, width: usize, height: usize) -> Result<(), RenderTargetError> {
        if width == self.width() && height == self.height() {
            return Ok(());
        }
        let mut replacement = RenderTarget::new(width, height)?;
        let replacement_scene = MeshScene::new(&self.mesh, self.draw_item.model, width, height);
        let replacement_clip_debug_scene =
            MeshScene::new_identity_debug(&self.clip_debug_mesh, width, height);
        let replacement_coverage_debug_scene =
            MeshScene::new_identity_debug(&self.coverage_debug_mesh, width, height);
        let replacement_interpolation_debug_scene =
            MeshScene::new_identity_debug(&self.interpolation_debug_mesh, width, height);
        let replacement_perspective_debug_scene =
            MeshScene::new_perspective_debug(&self.perspective_debug_mesh, width, height);
        let replacement_depth_debug_scene =
            MeshScene::new_identity_debug(&self.depth_debug_near_first_mesh, width, height);
        let (mesh, clip_vertices, selected_vertex_index) = match self.active_scene() {
            ActiveScene::Cube => (
                &self.mesh,
                replacement_scene.clip_vertices.as_slice(),
                CUBE_SELECTED_VERTEX_INDEX,
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
        };
        let draw_options = FrameDrawOptions::from_renderer(self);
        draw_frame(
            &mut replacement,
            draw_options,
            mesh,
            &mut self.clipper,
            clip_vertices,
            selected_vertex_index,
        );
        self.target = replacement;
        self.mesh_scene = replacement_scene;
        self.clip_debug_scene = replacement_clip_debug_scene;
        self.coverage_debug_scene = replacement_coverage_debug_scene;
        self.interpolation_debug_scene = replacement_interpolation_debug_scene;
        self.perspective_debug_scene = replacement_perspective_debug_scene;
        self.depth_debug_scene = replacement_depth_debug_scene;
        self.framebuffer_generation = self.framebuffer_generation.wrapping_add(1);
        Ok(())
    }

    pub fn update_and_render(&mut self, dt_seconds: f32, input: InputSnapshot) -> FrameStats {
        let (dt_seconds, invalid_dt) = sanitize_dt(dt_seconds);
        let rotation_y = self.draw_item.model.rotation_radians.y;
        self.draw_item.model.rotation_radians.y = if rotation_y.is_finite() {
            (rotation_y + dt_seconds * MODEL_ANGULAR_SPEED_RADIANS)
                .rem_euclid(std::f32::consts::TAU)
        } else {
            rotation_y
        };
        match self.active_scene() {
            ActiveScene::Cube => self.mesh_scene.rebuild_cube(
                &self.mesh,
                self.draw_item.model,
                self.target.width(),
                self.target.height(),
            ),
            ActiveScene::Clipping => self.clip_debug_scene.rebuild_identity_debug(
                &self.clip_debug_mesh,
                self.target.width(),
                self.target.height(),
            ),
            ActiveScene::Coverage => self.coverage_debug_scene.rebuild_identity_debug(
                &self.coverage_debug_mesh,
                self.target.width(),
                self.target.height(),
            ),
            ActiveScene::Interpolation => self.interpolation_debug_scene.rebuild_identity_debug(
                &self.interpolation_debug_mesh,
                self.target.width(),
                self.target.height(),
            ),
            ActiveScene::Perspective => self.perspective_debug_scene.rebuild_perspective_debug(
                &self.perspective_debug_mesh,
                self.target.width(),
                self.target.height(),
            ),
            ActiveScene::Depth => self.depth_debug_scene.rebuild_identity_debug(
                &self.depth_debug_near_first_mesh,
                self.target.width(),
                self.target.height(),
            ),
        }
        let (mesh, scene, selected_vertex_index) = match self.active_scene() {
            ActiveScene::Cube => (&self.mesh, &self.mesh_scene, CUBE_SELECTED_VERTEX_INDEX),
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
        };
        let draw_report = draw_frame(
            &mut self.target,
            FrameDrawOptions {
                debug_lines_enabled: self.debug_lines_enabled,
                cull_mode: self.cull_mode,
                winding_debug_mode: self.winding_debug_mode,
                depth_debug_mode: self.depth_debug_mode,
                attribute_interpolation_mode: self.attribute_interpolation_mode,
                uv_checker_enabled: self.perspective_debug_enabled,
            },
            mesh,
            &mut self.clipper,
            &scene.clip_vertices,
            selected_vertex_index,
        );
        let active_vertex_count = mesh.vertices().len() as u32;
        let transformed_vertex_count = scene.traces.len() as u32;
        let active_triangle_count = mesh.triangle_count() as u32;
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
            shaded_samples: draw_report.shaded_samples,
            depth_passed_samples: draw_report.depth_passed_samples,
            depth_failed_samples: draw_report.depth_failed_samples,
            invalid_depth_samples: draw_report.invalid_depth_samples,
            max_barycentric_sum_error: draw_report.max_barycentric_sum_error,
            interpolated_inv_w_samples: draw_report.interpolated_inv_w_samples,
            invalid_interpolation_samples: draw_report.invalid_interpolation_samples,
            min_interpolated_inv_w: draw_report.min_interpolated_inv_w,
            max_interpolated_inv_w: draw_report.max_interpolated_inv_w,
            debug_pixels: draw_report.debug_pixels,
            invalid_values: scene
                .diagnostics
                .invalid_values
                .saturating_add(draw_report.invalid_values)
                .saturating_add(u32::from(invalid_dt)),
        };
        self.stats
    }

    pub fn clear(&mut self, rgb: [u8; 3]) {
        self.target.clear_color(Color::rgb(rgb[0], rgb[1], rgb[2]));
    }

    pub fn set_debug_lines_enabled(&mut self, enabled: bool) {
        self.debug_lines_enabled = enabled;
    }

    pub fn set_cull_mode(&mut self, mode: CullMode) {
        self.cull_mode = mode;
    }

    pub fn set_winding_debug_mode(&mut self, mode: WindingDebugMode) {
        self.winding_debug_mode = mode;
    }

    pub fn set_clip_debug_enabled(&mut self, enabled: bool) {
        self.clip_debug_enabled = enabled;
        if enabled {
            self.coverage_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.perspective_debug_enabled = false;
            self.depth_debug_enabled = false;
        }
    }

    pub fn set_coverage_debug_enabled(&mut self, enabled: bool) {
        self.coverage_debug_enabled = enabled;
        if enabled {
            self.clip_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.perspective_debug_enabled = false;
            self.depth_debug_enabled = false;
        }
    }

    pub fn set_interpolation_debug_enabled(&mut self, enabled: bool) {
        self.interpolation_debug_enabled = enabled;
        if enabled {
            self.clip_debug_enabled = false;
            self.coverage_debug_enabled = false;
            self.perspective_debug_enabled = false;
            self.depth_debug_enabled = false;
        }
    }

    pub fn set_perspective_debug_enabled(&mut self, enabled: bool) {
        self.perspective_debug_enabled = enabled;
        if enabled {
            self.clip_debug_enabled = false;
            self.coverage_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.depth_debug_enabled = false;
        }
    }

    pub fn set_attribute_interpolation_mode(&mut self, mode: AttributeInterpolationMode) {
        self.attribute_interpolation_mode = mode;
    }

    pub fn set_depth_debug_enabled(&mut self, enabled: bool) {
        self.depth_debug_enabled = enabled;
        if enabled {
            self.clip_debug_enabled = false;
            self.coverage_debug_enabled = false;
            self.interpolation_debug_enabled = false;
            self.perspective_debug_enabled = false;
        }
    }

    pub fn set_depth_order_reversed(&mut self, reversed: bool) {
        self.depth_order_reversed = reversed;
    }

    pub fn set_depth_debug_mode(&mut self, mode: DepthDebugMode) {
        self.depth_debug_mode = mode;
    }

    pub fn set_model_rotation_y(&mut self, rotation_y_radians: f32) {
        self.draw_item.model.rotation_radians.y = rotation_y_radians;
        self.mesh_scene.rebuild_cube(
            &self.mesh,
            self.draw_item.model,
            self.target.width(),
            self.target.height(),
        );
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
                    CUBE_SELECTED_VERTEX_INDEX,
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
            cull_mode: self.cull_mode,
            winding_debug_mode: self.winding_debug_mode,
            clip_debug_enabled: self.clip_debug_enabled,
            coverage_debug_enabled: self.coverage_debug_enabled,
            interpolation_debug_enabled: self.interpolation_debug_enabled,
            perspective_debug_enabled: self.perspective_debug_enabled,
            attribute_interpolation_mode: self.attribute_interpolation_mode,
            depth_debug_enabled: self.depth_debug_enabled,
            depth_order_reversed: self.depth_order_reversed,
            depth_debug_mode: self.depth_debug_mode,
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
            math::Vec2::new(0.0, 0.0),
            Vec4::new(1.0, 0.0, 0.0, 1.0),
        ),
        (
            Vec3::new(0.65, 0.65, 0.5),
            math::Vec2::new(1.0, 0.0),
            Vec4::new(0.0, 1.0, 0.0, 1.0),
        ),
        (
            Vec3::new(0.0, -0.65, 0.5),
            math::Vec2::new(0.5, 1.0),
            Vec4::new(0.0, 0.0, 1.0, 1.0),
        ),
    ]
    .into_iter()
    .map(|(position, uv, color)| mesh::Vertex::new(position, Vec3::Z, uv, color))
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
    shaded_samples: u32,
    depth_passed_samples: u32,
    depth_failed_samples: u32,
    invalid_depth_samples: u32,
    max_barycentric_sum_error: f32,
    interpolated_inv_w_samples: u32,
    invalid_interpolation_samples: u32,
    min_interpolated_inv_w: f32,
    max_interpolated_inv_w: f32,
    debug_pixels: u32,
    invalid_values: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameDrawOptions {
    debug_lines_enabled: bool,
    cull_mode: CullMode,
    winding_debug_mode: WindingDebugMode,
    depth_debug_mode: DepthDebugMode,
    attribute_interpolation_mode: AttributeInterpolationMode,
    uv_checker_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RasterDrawOptions {
    cull_mode: CullMode,
    winding_debug_mode: WindingDebugMode,
    depth_debug_mode: DepthDebugMode,
    attribute_interpolation_mode: AttributeInterpolationMode,
    uv_checker_enabled: bool,
}

impl FrameDrawOptions {
    const fn from_renderer(renderer: &Renderer) -> Self {
        Self {
            debug_lines_enabled: renderer.debug_lines_enabled,
            cull_mode: renderer.cull_mode,
            winding_debug_mode: renderer.winding_debug_mode,
            depth_debug_mode: renderer.depth_debug_mode,
            attribute_interpolation_mode: renderer.attribute_interpolation_mode,
            uv_checker_enabled: renderer.perspective_debug_enabled,
        }
    }

    const fn raster(self) -> RasterDrawOptions {
        RasterDrawOptions {
            cull_mode: self.cull_mode,
            winding_debug_mode: self.winding_debug_mode,
            depth_debug_mode: self.depth_debug_mode,
            attribute_interpolation_mode: self.attribute_interpolation_mode,
            uv_checker_enabled: self.uv_checker_enabled,
        }
    }
}

fn draw_frame(
    target: &mut RenderTarget,
    options: FrameDrawOptions,
    mesh: &Mesh,
    clipper: &mut TriangleClipper,
    clip_vertices: &[ClipVertex],
    selected_vertex_index: usize,
) -> FrameDrawReport {
    match options.depth_debug_mode {
        DepthDebugMode::Off => target.render_gradient_checker(),
        DepthDebugMode::Grayscale | DepthDebugMode::Heatmap => {
            target.clear_color(DEPTH_DEBUG_BACKGROUND);
        }
    }
    draw_debug_scene(
        target,
        options,
        mesh,
        clipper,
        clip_vertices,
        selected_vertex_index,
    )
}

fn draw_debug_scene(
    target: &mut RenderTarget,
    options: FrameDrawOptions,
    mesh: &Mesh,
    clipper: &mut TriangleClipper,
    clip_vertices: &[ClipVertex],
    selected_vertex_index: usize,
) -> FrameDrawReport {
    let width = target.width() as i32;
    let height = target.height() as i32;
    let shortest_side = width.min(height);
    let white = Color::rgb(238, 244, 255);
    if !options.debug_lines_enabled {
        return draw_mesh(
            target,
            false,
            options.raster(),
            mesh,
            clipper,
            clip_vertices,
        );
    }
    if width < 16 || height < 16 {
        let mut report = draw_mesh(
            target,
            false,
            options.raster(),
            mesh,
            clipper,
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

    let mut report = draw_mesh(target, true, options.raster(), mesh, clipper, clip_vertices);
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

fn draw_mesh(
    target: &mut RenderTarget,
    draw_enabled: bool,
    options: RasterDrawOptions,
    mesh: &Mesh,
    clipper: &mut TriangleClipper,
    clip_vertices: &[ClipVertex],
) -> FrameDrawReport {
    let mut report = FrameDrawReport::default();
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
            let positions = generated.map(|vertex| {
                perspective_divide(vertex.clip_pos)
                    .and_then(|position| {
                        viewport(position, target.width() as f32, target.height() as f32)
                    })
                    .ok()
            });
            let [Some(first), Some(second), Some(third)] = positions else {
                report.invalid_triangles = report.invalid_triangles.saturating_add(1);
                continue;
            };
            submit_triangle(
                target,
                draw_enabled,
                options,
                *generated,
                [first, second, third],
                &mut report,
            );
        }
    }
    report
}

fn submit_triangle(
    target: &mut RenderTarget,
    draw_enabled: bool,
    options: RasterDrawOptions,
    generated: [ClipVertex; 3],
    positions: [ViewportPosition; 3],
    report: &mut FrameDrawReport,
) {
    let (source_orientation, order) = match classify_triangle(positions, options.cull_mode) {
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
    let mut shaded_samples = 0_u32;
    let mut depth_passed_samples = 0_u32;
    let mut depth_failed_samples = 0_u32;
    let mut invalid_depth_samples = 0_u32;
    let mut interpolated_inv_w_samples = 0_u32;
    let mut invalid_interpolation_samples = 0_u32;
    let mut min_interpolated_inv_w = 0.0_f32;
    let mut max_interpolated_inv_w = 0.0_f32;
    let mut invalid_values = 0_u32;
    setup.rasterize(|sample| {
        let barycentric = setup.covered_barycentric(sample.edge_values);
        max_barycentric_sum_error = max_barycentric_sum_error.max(barycentric.sum_error());
        let depth = barycentric.interpolate_f32(ordered_depths);
        let point = ScreenPoint::new(sample.x as i32, sample.y as i32);
        match target.test_depth(point, depth) {
            DepthTestResult::Passed => {}
            DepthTestResult::Failed => {
                depth_failed_samples = depth_failed_samples.saturating_add(1);
                return;
            }
            DepthTestResult::Invalid => {
                invalid_depth_samples = invalid_depth_samples.saturating_add(1);
                return;
            }
        }
        let Some(fragment) = FragmentInput::from_screen_vertices(
            barycentric,
            ordered_screen_vertices,
            options.attribute_interpolation_mode,
        ) else {
            invalid_interpolation_samples = invalid_interpolation_samples.saturating_add(1);
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
        interpolated_inv_w_samples = interpolated_inv_w_samples.saturating_add(1);
        let fill_color = match options.depth_debug_mode {
            DepthDebugMode::Off => match options.winding_debug_mode {
                WindingDebugMode::VertexColor if options.uv_checker_enabled => {
                    uv_checker_color(fragment.uv())
                }
                WindingDebugMode::VertexColor => debug_color(fragment.color()),
                WindingDebugMode::Facing => facing_color,
                WindingDebugMode::Barycentric => debug_color(fragment.barycentric().debug_color()),
            },
            DepthDebugMode::Grayscale => depth_grayscale_color(depth),
            DepthDebugMode::Heatmap => depth_heatmap_color(depth),
        };
        let written = target.commit_depth_and_color(point, depth, fill_color);
        debug_assert!(
            written,
            "통과한 depth와 clamp된 coverage sample은 색/깊이를 함께 기록해야 한다"
        );
        depth_passed_samples = depth_passed_samples.saturating_add(u32::from(written));
        shaded_samples = shaded_samples.saturating_add(u32::from(written));
    });
    report.submitted_triangles = report.submitted_triangles.saturating_add(1);
    report.rasterized_triangles = report.rasterized_triangles.saturating_add(1);
    report.shaded_samples = report.shaded_samples.saturating_add(shaded_samples);
    report.depth_passed_samples = report
        .depth_passed_samples
        .saturating_add(depth_passed_samples);
    report.depth_failed_samples = report
        .depth_failed_samples
        .saturating_add(depth_failed_samples);
    report.invalid_depth_samples = report
        .invalid_depth_samples
        .saturating_add(invalid_depth_samples);
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
        report.interpolated_inv_w_samples = report
            .interpolated_inv_w_samples
            .saturating_add(interpolated_inv_w_samples);
    }
    report.invalid_interpolation_samples = report
        .invalid_interpolation_samples
        .saturating_add(invalid_interpolation_samples);
    report.invalid_values = report.invalid_values.saturating_add(invalid_values);
    if !draw_enabled {
        return;
    }
    let screen_positions = positions.map(viewport_screen_point);
    let ordered_positions = order.map(|index| screen_positions[index]);
    let (wireframe_positions, edge_colors) = match options.winding_debug_mode {
        WindingDebugMode::VertexColor => {
            // 제출 geometry는 positive winding이지만, 기존 Bresenham 방향과 edge
            // 덮어쓰기 순서를 보존하기 위해 vertex-color wireframe은 원본 순서로 그린다.
            let colors = generated.map(|vertex| debug_color(vertex.color));
            (screen_positions, colors)
        }
        WindingDebugMode::Facing => (ordered_positions, [facing_color; 3]),
        WindingDebugMode::Barycentric => (
            ordered_positions,
            [
                Color::rgb(255, 0, 0),
                Color::rgb(0, 255, 0),
                Color::rgb(0, 0, 255),
            ],
        ),
    };
    report.debug_pixels = report
        .debug_pixels
        .saturating_add(target.draw_wireframe_triangle(wireframe_positions, edge_colors));
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
                cull_mode: CullMode::Back,
                winding_debug_mode: WindingDebugMode::VertexColor,
                depth_debug_mode: DepthDebugMode::Off,
                attribute_interpolation_mode: mode,
                uv_checker_enabled: true,
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

    const fn raster_options(
        cull_mode: CullMode,
        winding_debug_mode: WindingDebugMode,
    ) -> RasterDrawOptions {
        RasterDrawOptions {
            cull_mode,
            winding_debug_mode,
            depth_debug_mode: DepthDebugMode::Off,
            attribute_interpolation_mode: AttributeInterpolationMode::PerspectiveCorrect,
            uv_checker_enabled: false,
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

        let first = renderer.update_and_render(0.25, InputSnapshot::from_packed(0xa5));
        assert_eq!(first.frame_index, 1);
        assert_eq!(first.dt_seconds, 0.1);
        assert_eq!(first.input_bits, 0xa5);
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
        assert_eq!(first.shaded_samples, 329);
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
        assert_eq!(tiny.color_buffer(), [238, 244, 255, 255]);
        tiny.set_debug_lines_enabled(false);
        assert_eq!(
            tiny.update_and_render(0.0, InputSnapshot::default())
                .debug_pixels,
            0
        );
        assert_eq!(tiny.color_buffer(), [255, 89, 64, 255]);
    }

    #[test]
    fn chapter_thirteen_double_sided_depth_coverage_matches_64_by_64_golden_hash() {
        let mut renderer = Renderer::new(64, 64).expect("golden renderer should be valid");
        renderer.set_cull_mode(CullMode::None);
        renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(fnv1a(renderer.color_buffer()), 0x833d_8997);
    }

    #[test]
    fn chapter_eleven_backface_culled_flat_coverage_matches_64_by_64_golden_hash() {
        let renderer = Renderer::new(64, 64).expect("golden renderer should be valid");
        assert_eq!(fnv1a(renderer.color_buffer()), 0x4235_c453);
    }

    #[test]
    fn chapter_ten_near_corner_fixture_clips_before_divide_and_fans_the_polygon() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
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
        assert_eq!(fnv1a(renderer.color_buffer()), 0xb7bd_5d28);

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

        let orange = [255, 89, 38, 255];
        let cyan = [38, 191, 255, 255];
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
        assert_eq!(fnv1a(renderer.color_buffer()), 0x1d6a_3195);

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
                [234, 7, 14, 255],
                [7, 234, 14, 255],
                [7, 13, 235, 255],
                [81, 87, 88, 255],
            ]
        );
        let affine_hash = fnv1a(renderer.color_buffer());
        assert_eq!(affine_hash, 0xdb7e_9eb4);

        renderer.set_winding_debug_mode(WindingDebugMode::Barycentric);
        let barycentric = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(barycentric.submitted_triangles, affine.submitted_triangles);
        assert_eq!(
            barycentric.rasterized_triangles,
            affine.rasterized_triangles
        );
        assert_eq!(barycentric.shaded_samples, affine.shaded_samples);
        assert_eq!(fnv1a(renderer.color_buffer()), affine_hash);

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
            &mut far_first_report,
        );
        submit_triangle(
            &mut far_first_target,
            false,
            raster_options(CullMode::None, WindingDebugMode::VertexColor),
            invalid_generated,
            invalid_positions,
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
                0x373e_c577,
                0x52eb_59c7,
                0x5cbf_6a73,
                [
                    [255, 51, 38, 255],
                    [38, 89, 255, 255],
                    [255, 51, 38, 255],
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
        for (rotation_y, submitted, culled) in [(0.0, 4, 8), (0.75, 6, 6), (-0.75, 4, 8)] {
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
    fn facing_debug_colors_both_orientations_without_changing_geometry_counts() {
        let mut renderer = Renderer::new(64, 64).expect("renderer should be valid");
        renderer.set_cull_mode(CullMode::None);
        renderer.set_winding_debug_mode(WindingDebugMode::Facing);
        let facing = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(
            (facing.submitted_triangles, facing.culled_triangles),
            (12, 0)
        );
        for expected in [[72, 232, 112, 255], [255, 82, 92, 255]] {
            assert!(
                renderer
                    .color_buffer()
                    .chunks_exact(4)
                    .any(|pixel| pixel == expected),
                "missing winding debug color {expected:?}"
            );
        }

        let snapshot = renderer.coordinate_debug_snapshot();
        assert_eq!(snapshot.cull_mode, CullMode::None);
        assert_eq!(snapshot.winding_debug_mode, WindingDebugMode::Facing);
        assert_eq!(snapshot.frame_stats, facing);

        renderer.set_winding_debug_mode(WindingDebugMode::VertexColor);
        let vertex_colors = renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(
            (
                vertex_colors.submitted_triangles,
                vertex_colors.culled_triangles
            ),
            (facing.submitted_triangles, facing.culled_triangles)
        );
        assert_eq!(fnv1a(renderer.color_buffer()), 0x833d_8997);
    }

    #[test]
    fn chapter_eight_scene_contains_projected_mesh_and_selected_vertex_colors() {
        let renderer = Renderer::new(64, 64).expect("debug renderer should be valid");
        for expected in [[238, 244, 255, 255], [255, 89, 64, 255]] {
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
        let snapshot = InputSnapshot::from_packed(u32::MAX);
        assert_eq!(snapshot.packed_bits(), u32::MAX);
        assert_eq!(InputSnapshot::default().packed_bits(), 0);
    }
}
