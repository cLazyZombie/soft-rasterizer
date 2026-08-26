//! Browser APIs에 의존하지 않는 소프트웨어 래스터라이저의 순수 Rust 코어.
//!
//! 9장까지 RGBA8 프레임버퍼에 winding/culling이 적용된 indexed mesh wireframe 큐브를 그린다.

pub mod camera;
pub mod math;
pub mod mesh;
pub mod raster;
pub mod transform;

use std::error::Error;
use std::fmt::{Display, Formatter};

use camera::{
    NdcPosition, ViewportPosition, look_at_lh, perspective_divide, perspective_lh_zo, viewport,
};
use math::{Vec3, Vec4};
use mesh::{ClipVertex, DrawItem, MaterialId, Mesh, MeshId, unit_cube_mesh};
use raster::{CullMode, FaceOrientation, TriangleDisposition, WindingDebugMode, classify_triangle};
#[cfg(test)]
use transform::CoordinateSpace;
use transform::{
    ClipPlaneDistances, CoordinateDiagnostics, ObjectPosition, Transform, TransformPipeline,
    VertexTrace,
};

/// 4096 × 4096 RGBA8와 깊이 버퍼까지만 허용한다.
pub const MAX_PIXEL_COUNT: usize = 16_777_216;
const MAX_FRAME_DT_SECONDS: f32 = 0.1;
const MODEL_ANGULAR_SPEED_RADIANS: f32 = 0.75;
const SELECTED_VERTEX_INDEX: usize = 6;
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

    pub fn put_pixel(&mut self, point: ScreenPoint, color: Color) -> bool {
        let (Ok(x), Ok(y)) = (usize::try_from(point.x), usize::try_from(point.y)) else {
            return false;
        };
        if x >= self.width || y >= self.height {
            return false;
        }
        let byte_index = 4 * (y * self.width + x);
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
    pub clipped_triangles: u32,
    pub rasterized_triangles: u32,
    pub shaded_samples: u32,
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
    ndc_positions: Vec<Option<NdcPosition>>,
    viewport_positions: Vec<Option<ViewportPosition>>,
    diagnostics: CoordinateDiagnostics,
    projection_failures: u32,
    aspect: f32,
}

impl MeshScene {
    fn new(mesh: &Mesh, model: Transform, width: usize, height: usize) -> Self {
        let vertex_count = mesh.vertices().len();
        let mut scene = Self {
            traces: Vec::with_capacity(vertex_count),
            clip_vertices: Vec::with_capacity(vertex_count),
            ndc_positions: Vec::with_capacity(vertex_count),
            viewport_positions: Vec::with_capacity(vertex_count),
            diagnostics: CoordinateDiagnostics::from_traces(&[]),
            projection_failures: 0,
            aspect: 1.0,
        };
        scene.rebuild(mesh, model, width, height);
        scene
    }

    fn rebuild(&mut self, mesh: &Mesh, model: Transform, width: usize, height: usize) {
        let aspect = width as f32 / height as f32;
        let view = look_at_lh(CAMERA_EYE, CAMERA_TARGET, CAMERA_WORLD_UP)
            .expect("고정 카메라 view 계약은 항상 유효해야 한다");
        let projection = perspective_lh_zo(CAMERA_FOV_Y_RADIANS, aspect, CAMERA_NEAR, CAMERA_FAR)
            .expect("유효한 렌더 타깃의 고정 projection 계약은 항상 유효해야 한다");
        let pipeline = TransformPipeline::new(model.model_matrix(), view, projection);
        self.traces.clear();
        self.clip_vertices.clear();
        self.ndc_positions.clear();
        self.viewport_positions.clear();
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
            let ndc = perspective_divide(trace.clip_pos).ok();
            self.viewport_positions.push(
                ndc.and_then(|position| viewport(position, width as f32, height as f32).ok()),
            );
            self.ndc_positions.push(ndc);
            self.traces.push(trace);
        }
        self.diagnostics = CoordinateDiagnostics::from_traces(&self.traces);
        self.projection_failures = self
            .ndc_positions
            .iter()
            .filter(|position| position.is_none())
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
    pub frame_stats: FrameStats,
}

/// 렌더 타깃과 3-9장 indexed mesh debug scene 상태를 소유한다.
#[derive(Debug)]
pub struct Renderer {
    target: RenderTarget,
    stats: FrameStats,
    framebuffer_generation: u32,
    debug_lines_enabled: bool,
    cull_mode: CullMode,
    winding_debug_mode: WindingDebugMode,
    mesh: Mesh,
    draw_item: DrawItem,
    mesh_scene: MeshScene,
}

impl Renderer {
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
        let mut renderer = Self {
            target,
            stats: FrameStats::default(),
            framebuffer_generation: 0,
            debug_lines_enabled: true,
            cull_mode: CullMode::Back,
            winding_debug_mode: WindingDebugMode::VertexColor,
            mesh,
            draw_item,
            mesh_scene,
        };
        draw_frame(
            &mut renderer.target,
            renderer.debug_lines_enabled,
            renderer.cull_mode,
            renderer.winding_debug_mode,
            &renderer.mesh,
            &renderer.mesh_scene.viewport_positions,
            &renderer.mesh_scene.clip_vertices,
        );
        Ok(renderer)
    }

    pub fn resize(&mut self, width: usize, height: usize) -> Result<(), RenderTargetError> {
        if width == self.width() && height == self.height() {
            return Ok(());
        }
        let mut replacement = RenderTarget::new(width, height)?;
        let replacement_scene = MeshScene::new(&self.mesh, self.draw_item.model, width, height);
        draw_frame(
            &mut replacement,
            self.debug_lines_enabled,
            self.cull_mode,
            self.winding_debug_mode,
            &self.mesh,
            &replacement_scene.viewport_positions,
            &replacement_scene.clip_vertices,
        );
        self.target = replacement;
        self.mesh_scene = replacement_scene;
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
        self.mesh_scene.rebuild(
            &self.mesh,
            self.draw_item.model,
            self.target.width(),
            self.target.height(),
        );
        let draw_report = draw_frame(
            &mut self.target,
            self.debug_lines_enabled,
            self.cull_mode,
            self.winding_debug_mode,
            &self.mesh,
            &self.mesh_scene.viewport_positions,
            &self.mesh_scene.clip_vertices,
        );
        self.stats = FrameStats {
            frame_index: self.stats.frame_index.wrapping_add(1),
            dt_seconds,
            input_bits: input.packed_bits(),
            input_vertices: self.mesh.vertices().len() as u32,
            input_triangles: self.mesh.triangle_count() as u32,
            transformed_vertices: self.mesh_scene.traces.len() as u32,
            submitted_triangles: draw_report.submitted_triangles,
            culled_triangles: draw_report.culled_triangles,
            degenerate_triangles: draw_report.degenerate_triangles,
            invalid_triangles: draw_report.invalid_triangles,
            debug_pixels: draw_report.debug_pixels,
            invalid_values: self
                .mesh_scene
                .diagnostics
                .invalid_values
                .saturating_add(self.mesh_scene.projection_failures)
                .saturating_add(u32::from(invalid_dt)),
            ..FrameStats::default()
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

    pub fn set_model_rotation_y(&mut self, rotation_y_radians: f32) {
        self.draw_item.model.rotation_radians.y = rotation_y_radians;
        self.mesh_scene.rebuild(
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
        let selected_vertex = self.mesh_scene.traces[SELECTED_VERTEX_INDEX];
        CoordinateDebugSnapshot {
            rotation_y_radians: self.draw_item.model.rotation_radians.y,
            selected_vertex_index: SELECTED_VERTEX_INDEX,
            selected_vertex,
            selected_attributes: self.mesh_scene.clip_vertices[SELECTED_VERTEX_INDEX],
            selected_ndc: self.mesh_scene.ndc_positions[SELECTED_VERTEX_INDEX],
            selected_viewport: self.mesh_scene.viewport_positions[SELECTED_VERTEX_INDEX],
            clip_plane_distances: ClipPlaneDistances::from_position(selected_vertex.clip_pos),
            diagnostics: self.mesh_scene.diagnostics,
            projection_failures: self.mesh_scene.projection_failures,
            fov_y_radians: CAMERA_FOV_Y_RADIANS,
            near: CAMERA_NEAR,
            far: CAMERA_FAR,
            aspect: self.mesh_scene.aspect,
            mesh_vertices: self.mesh.vertices().len() as u32,
            mesh_indices: self.mesh.indices().len() as u32,
            mesh_triangles: self.mesh.triangle_count() as u32,
            material_id: self.draw_item.material_id.0,
            cull_mode: self.cull_mode,
            winding_debug_mode: self.winding_debug_mode,
            frame_stats: self.stats,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FrameDrawReport {
    submitted_triangles: u32,
    culled_triangles: u32,
    degenerate_triangles: u32,
    invalid_triangles: u32,
    debug_pixels: u32,
}

fn draw_frame(
    target: &mut RenderTarget,
    debug_lines_enabled: bool,
    cull_mode: CullMode,
    winding_debug_mode: WindingDebugMode,
    mesh: &Mesh,
    viewport_positions: &[Option<ViewportPosition>],
    clip_vertices: &[ClipVertex],
) -> FrameDrawReport {
    target.render_gradient_checker();
    draw_debug_scene(
        target,
        debug_lines_enabled,
        cull_mode,
        winding_debug_mode,
        mesh,
        viewport_positions,
        clip_vertices,
    )
}

fn draw_debug_scene(
    target: &mut RenderTarget,
    debug_lines_enabled: bool,
    cull_mode: CullMode,
    winding_debug_mode: WindingDebugMode,
    mesh: &Mesh,
    viewport_positions: &[Option<ViewportPosition>],
    clip_vertices: &[ClipVertex],
) -> FrameDrawReport {
    let width = target.width() as i32;
    let height = target.height() as i32;
    let shortest_side = width.min(height);
    let white = Color::rgb(238, 244, 255);
    if !debug_lines_enabled {
        return draw_mesh_wireframe(
            target,
            false,
            cull_mode,
            winding_debug_mode,
            mesh,
            viewport_positions,
            clip_vertices,
        );
    }
    if width < 16 || height < 16 {
        let mut report = draw_mesh_wireframe(
            target,
            false,
            cull_mode,
            winding_debug_mode,
            mesh,
            viewport_positions,
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

    let mut report = draw_mesh_wireframe(
        target,
        true,
        cull_mode,
        winding_debug_mode,
        mesh,
        viewport_positions,
        clip_vertices,
    );
    written = written.saturating_add(report.debug_pixels);
    if let Some(selected) = viewport_positions
        .get(SELECTED_VERTEX_INDEX)
        .copied()
        .flatten()
        .map(viewport_screen_point)
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

fn draw_mesh_wireframe(
    target: &mut RenderTarget,
    draw_enabled: bool,
    cull_mode: CullMode,
    winding_debug_mode: WindingDebugMode,
    mesh: &Mesh,
    viewport_positions: &[Option<ViewportPosition>],
    clip_vertices: &[ClipVertex],
) -> FrameDrawReport {
    let mut report = FrameDrawReport::default();
    for triangle in mesh.triangles() {
        let positions = triangle.map(|index| viewport_positions.get(index).copied().flatten());
        let [Some(first), Some(second), Some(third)] = positions else {
            report.invalid_triangles = report.invalid_triangles.saturating_add(1);
            continue;
        };
        let (source_orientation, order) = match classify_triangle([first, second, third], cull_mode)
        {
            TriangleDisposition::Submit {
                source_orientation,
                order,
            } => (source_orientation, order),
            TriangleDisposition::Culled(_) => {
                report.culled_triangles = report.culled_triangles.saturating_add(1);
                continue;
            }
            TriangleDisposition::Degenerate => {
                report.degenerate_triangles = report.degenerate_triangles.saturating_add(1);
                continue;
            }
            TriangleDisposition::Invalid => {
                report.invalid_triangles = report.invalid_triangles.saturating_add(1);
                continue;
            }
        };
        report.submitted_triangles = report.submitted_triangles.saturating_add(1);
        if !draw_enabled {
            continue;
        }
        let screen_positions = [first, second, third].map(viewport_screen_point);
        let ordered_positions = order.map(|index| screen_positions[index]);
        let (wireframe_positions, edge_colors) = match winding_debug_mode {
            WindingDebugMode::VertexColor => {
                // 제출 geometry는 positive winding이지만, 8장 debug golden의 Bresenham
                // 방향과 edge 덮어쓰기 순서를 보존하기 위해 이 wireframe만 원본 index
                // 순서로 그린다.
                let colors = triangle.map(|vertex_index| {
                    clip_vertices
                        .get(vertex_index)
                        .map_or(Color::rgb(255, 210, 72), |vertex| debug_color(vertex.color))
                });
                (screen_positions, colors)
            }
            WindingDebugMode::Facing => {
                let color = match source_orientation {
                    FaceOrientation::Front => Color::rgb(72, 232, 112),
                    FaceOrientation::Back => Color::rgb(255, 82, 92),
                };
                (ordered_positions, [color; 3])
            }
        };
        report.debug_pixels = report
            .debug_pixels
            .saturating_add(target.draw_wireframe_triangle(wireframe_positions, edge_colors));
    }
    report
}

fn debug_color(color: Vec4) -> Color {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::rgb(channel(color.x), channel(color.y), channel(color.z))
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
                .all(|depth| *depth == f32::INFINITY)
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
        let viewport_cache_pointer = renderer.mesh_scene.viewport_positions.as_ptr();

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
        assert_eq!(first.clipped_triangles, 0);
        assert_eq!(first.rasterized_triangles, 0);
        assert_eq!(first.shaded_samples, 0);
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
            renderer.mesh_scene.viewport_positions.as_ptr(),
            viewport_cache_pointer
        );

        renderer.set_debug_lines_enabled(false);
        let negative = renderer.update_and_render(-1.0, InputSnapshot::default());
        assert_eq!(negative.dt_seconds, 0.0);
        assert_eq!(negative.submitted_triangles, 4);
        assert_eq!(negative.culled_triangles, 8);
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
        assert_eq!(tiny.color_buffer(), [0, 0, 220, 255]);
    }

    #[test]
    fn chapter_eight_indexed_mesh_wireframe_matches_64_by_64_golden_hash() {
        let mut renderer = Renderer::new(64, 64).expect("golden renderer should be valid");
        renderer.set_cull_mode(CullMode::None);
        renderer.update_and_render(0.0, InputSnapshot::default());
        assert_eq!(fnv1a(renderer.color_buffer()), 0xf1ef_5933);
    }

    #[test]
    fn chapter_nine_backface_culled_wireframe_matches_64_by_64_golden_hash() {
        let renderer = Renderer::new(64, 64).expect("golden renderer should be valid");
        assert_eq!(fnv1a(renderer.color_buffer()), 0x647c_0b11);
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
        assert_eq!(fnv1a(renderer.color_buffer()), 0xf1ef_5933);
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
        assert_eq!(
            draw_mesh_wireframe(
                &mut target,
                true,
                CullMode::Back,
                WindingDebugMode::VertexColor,
                &empty,
                &empty_scene.viewport_positions,
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
            draw_mesh_wireframe(
                &mut target,
                true,
                CullMode::Back,
                WindingDebugMode::VertexColor,
                &degenerate,
                &degenerate_scene.viewport_positions,
                &degenerate_scene.clip_vertices,
            ),
            FrameDrawReport {
                degenerate_triangles: 1,
                ..FrameDrawReport::default()
            }
        );
        assert_eq!(target.color(), color_before_degenerate);
        assert_eq!(target.depth(), depth_before_degenerate);

        let invalid_positions = [Some(ViewportPosition {
            x: f32::NAN,
            y: 0.0,
            z_ndc: 0.5,
        })];
        assert_eq!(
            draw_mesh_wireframe(
                &mut target,
                true,
                CullMode::None,
                WindingDebugMode::VertexColor,
                &degenerate,
                &invalid_positions,
                degenerate_scene.clip_vertices.as_slice(),
            ),
            FrameDrawReport {
                invalid_triangles: 1,
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
        assert_eq!(invalid.invalid_values, 96);
        assert_eq!(invalid.invalid_triangles, 12);
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
