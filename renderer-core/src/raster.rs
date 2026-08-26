//! 9장의 screen-space winding/culling과 11장의 fixed-point coverage 계약.

use crate::camera::ViewportPosition;

/// Wireframe 단계의 조기 분류에만 쓰는 float 면적 기준이다.
///
/// 최종 coverage 퇴화 판정과 edge equality는 S=256 고정소수점으로 다시 계산한다.
pub const WIREFRAME_AREA_EPSILON: f32 = 1.0e-5;
pub const SUBPIXEL_BITS: u32 = 8;
pub const SUBPIXEL_SCALE: i64 = 1_i64 << SUBPIXEL_BITS;
const SUBPIXEL_HALF: i64 = SUBPIXEL_SCALE / 2;
const I64_UPPER_EXCLUSIVE: f64 = 9_223_372_036_854_775_808.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaceOrientation {
    Front,
    Back,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CullMode {
    None,
    Back,
    Front,
}

impl CullMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Back => "back",
            Self::Front => "front",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindingDebugMode {
    VertexColor,
    Facing,
}

impl WindingDebugMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::VertexColor => "vertex color",
            Self::Facing => "front green / back red",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriangleDisposition {
    Submit {
        source_orientation: FaceOrientation,
        order: [usize; 3],
    },
    Culled(FaceOrientation),
    Degenerate,
    Invalid,
}

pub fn orient2d(first: ViewportPosition, second: ViewportPosition, third: ViewportPosition) -> f32 {
    (second.x - first.x) * (third.y - first.y) - (second.y - first.y) * (third.x - first.x)
}

pub fn classify_triangle(
    vertices: [ViewportPosition; 3],
    cull_mode: CullMode,
) -> TriangleDisposition {
    let area = orient2d(vertices[0], vertices[1], vertices[2]);
    if !area.is_finite() {
        return TriangleDisposition::Invalid;
    }
    if area.abs() <= WIREFRAME_AREA_EPSILON {
        return TriangleDisposition::Degenerate;
    }

    let source_orientation = if area > 0.0 {
        FaceOrientation::Front
    } else {
        FaceOrientation::Back
    };
    let should_cull = matches!(
        (cull_mode, source_orientation),
        (CullMode::Back, FaceOrientation::Back) | (CullMode::Front, FaceOrientation::Front)
    );
    if should_cull {
        return TriangleDisposition::Culled(source_orientation);
    }

    let order = match source_orientation {
        FaceOrientation::Front => [0, 1, 2],
        FaceOrientation::Back => [0, 2, 1],
    };
    TriangleDisposition::Submit {
        source_orientation,
        order,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriangleSetupError {
    InvalidTarget,
    NonFinitePosition,
    FixedPointOverflow,
    ArithmeticOverflow,
    Degenerate,
    BackFacing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FixedPointPosition {
    x: i64,
    y: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelBounds {
    pub min_x: usize,
    pub min_y: usize,
    pub max_x: usize,
    pub max_y: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EdgeEquation {
    origin: FixedPointPosition,
    dx: i64,
    dy: i64,
    step_x: i64,
    step_y: i64,
    inclusive: bool,
}

impl EdgeEquation {
    fn new(
        first: FixedPointPosition,
        second: FixedPointPosition,
    ) -> Result<Self, TriangleSetupError> {
        let dx = i64::try_from(i128::from(second.x) - i128::from(first.x))
            .map_err(|_| TriangleSetupError::ArithmeticOverflow)?;
        let dy = i64::try_from(i128::from(second.y) - i128::from(first.y))
            .map_err(|_| TriangleSetupError::ArithmeticOverflow)?;
        let step_x = i64::try_from(-i128::from(dy) * i128::from(SUBPIXEL_SCALE))
            .map_err(|_| TriangleSetupError::ArithmeticOverflow)?;
        let step_y = i64::try_from(i128::from(dx) * i128::from(SUBPIXEL_SCALE))
            .map_err(|_| TriangleSetupError::ArithmeticOverflow)?;
        Ok(Self {
            origin: first,
            dx,
            dy,
            step_x,
            step_y,
            inclusive: dy < 0 || (dy == 0 && dx > 0),
        })
    }

    fn value_at_fixed(self, point: FixedPointPosition) -> Result<i64, TriangleSetupError> {
        let relative_x = i128::from(point.x) - i128::from(self.origin.x);
        let relative_y = i128::from(point.y) - i128::from(self.origin.y);
        checked_edge_value(
            i128::from(self.dx),
            i128::from(self.dy),
            relative_x,
            relative_y,
        )
    }

    fn value_at_sample(self, x: usize, y: usize) -> Result<i64, TriangleSetupError> {
        self.value_at_fixed(sample_center(x, y)?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoveredSample {
    pub x: usize,
    pub y: usize,
    pub edge_values: [i64; 3],
}

/// 양자화, bbox clamp, edge 계수와 top-left flag를 프레임 hot loop 전에 고정한다.
///
/// 정상 파이프라인의 screen 정점은 `0..=width`, `0..=height`에 있고 RenderTarget은
/// `width * height <= 16_777_216`이다. 따라서 각 교차항은 최대
/// `width * height * 256^2 = 1_099_511_627_776`으로 i64에 안전하다. 공개 setup은
/// 더 넓은 입력도 받을 수 있으므로 i128로 네 bbox 모서리를 preflight한 뒤 i64만 저장한다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TriangleSetup {
    bounds: Option<PixelBounds>,
    area: i64,
    edges: [EdgeEquation; 3],
}

impl TriangleSetup {
    pub fn new(
        vertices: [ViewportPosition; 3],
        width: usize,
        height: usize,
    ) -> Result<Self, TriangleSetupError> {
        validate_target_size(width, height)?;
        let [first, second, third] = vertices;
        let vertices = [
            quantize_position(first)?,
            quantize_position(second)?,
            quantize_position(third)?,
        ];
        let area = edge_value(vertices[0], vertices[1], vertices[2])?;
        if area == 0 {
            return Err(TriangleSetupError::Degenerate);
        }
        if area < 0 {
            return Err(TriangleSetupError::BackFacing);
        }

        let edges = [
            EdgeEquation::new(vertices[1], vertices[2])?,
            EdgeEquation::new(vertices[2], vertices[0])?,
            EdgeEquation::new(vertices[0], vertices[1])?,
        ];
        let bounds = pixel_bounds(vertices, width, height);
        if let Some(bounds) = bounds {
            for edge in edges {
                for (x, y) in [
                    (bounds.min_x, bounds.min_y),
                    (bounds.max_x, bounds.min_y),
                    (bounds.min_x, bounds.max_y),
                    (bounds.max_x, bounds.max_y),
                ] {
                    edge.value_at_sample(x, y)?;
                }
            }
        }
        Ok(Self {
            bounds,
            area,
            edges,
        })
    }

    pub const fn area(&self) -> i64 {
        self.area
    }

    pub const fn bounds(&self) -> Option<PixelBounds> {
        self.bounds
    }

    pub const fn top_left_flags(&self) -> [bool; 3] {
        [
            self.edges[0].inclusive,
            self.edges[1].inclusive,
            self.edges[2].inclusive,
        ]
    }

    pub fn edge_values_at(&self, x: usize, y: usize) -> Result<[i64; 3], TriangleSetupError> {
        Ok([
            self.edges[0].value_at_sample(x, y)?,
            self.edges[1].value_at_sample(x, y)?,
            self.edges[2].value_at_sample(x, y)?,
        ])
    }

    pub fn accepts(&self, edge_values: [i64; 3]) -> bool {
        edge_values
            .into_iter()
            .zip(self.edges)
            .all(|(value, edge)| value > 0 || (value == 0 && edge.inclusive))
    }

    pub fn rasterize(&self, mut visit: impl FnMut(CoveredSample)) -> u32 {
        let Some(bounds) = self.bounds else {
            return 0;
        };
        let mut row_values = self
            .edge_values_at(bounds.min_x, bounds.min_y)
            .expect("triangle setup preflight가 bbox 시작 edge 범위를 보장해야 한다");
        let mut covered_samples = 0_u32;
        for y in bounds.min_y..=bounds.max_y {
            let mut edge_values = row_values;
            for x in bounds.min_x..=bounds.max_x {
                if self.accepts(edge_values) {
                    visit(CoveredSample { x, y, edge_values });
                    covered_samples = covered_samples.saturating_add(1);
                }
                if x != bounds.max_x {
                    for (value, edge) in edge_values.iter_mut().zip(self.edges) {
                        *value = value
                            .checked_add(edge.step_x)
                            .expect("triangle setup preflight가 x edge 증가 범위를 보장해야 한다");
                    }
                }
            }
            if y != bounds.max_y {
                for (value, edge) in row_values.iter_mut().zip(self.edges) {
                    *value = value
                        .checked_add(edge.step_y)
                        .expect("triangle setup preflight가 y edge 증가 범위를 보장해야 한다");
                }
            }
        }
        covered_samples
    }
}

fn validate_target_size(width: usize, height: usize) -> Result<(), TriangleSetupError> {
    let max_sample_coordinate = ((i64::MAX - SUBPIXEL_HALF) / SUBPIXEL_SCALE) as usize;
    if width == 0
        || height == 0
        || width - 1 > max_sample_coordinate
        || height - 1 > max_sample_coordinate
    {
        return Err(TriangleSetupError::InvalidTarget);
    }
    Ok(())
}

fn quantize_position(position: ViewportPosition) -> Result<FixedPointPosition, TriangleSetupError> {
    if !position.x.is_finite() || !position.y.is_finite() || !position.z_ndc.is_finite() {
        return Err(TriangleSetupError::NonFinitePosition);
    }
    Ok(FixedPointPosition {
        x: quantize_component(position.x)?,
        y: quantize_component(position.y)?,
    })
}

fn quantize_component(value: f32) -> Result<i64, TriangleSetupError> {
    let rounded = (f64::from(value) * SUBPIXEL_SCALE as f64).round();
    if rounded < i64::MIN as f64 || rounded >= I64_UPPER_EXCLUSIVE {
        return Err(TriangleSetupError::FixedPointOverflow);
    }
    Ok(rounded as i64)
}

fn sample_center(x: usize, y: usize) -> Result<FixedPointPosition, TriangleSetupError> {
    let x = i64::try_from(x).map_err(|_| TriangleSetupError::ArithmeticOverflow)?;
    let y = i64::try_from(y).map_err(|_| TriangleSetupError::ArithmeticOverflow)?;
    Ok(FixedPointPosition {
        x: x.checked_mul(SUBPIXEL_SCALE)
            .and_then(|value| value.checked_add(SUBPIXEL_HALF))
            .ok_or(TriangleSetupError::ArithmeticOverflow)?,
        y: y.checked_mul(SUBPIXEL_SCALE)
            .and_then(|value| value.checked_add(SUBPIXEL_HALF))
            .ok_or(TriangleSetupError::ArithmeticOverflow)?,
    })
}

fn edge_value(
    first: FixedPointPosition,
    second: FixedPointPosition,
    point: FixedPointPosition,
) -> Result<i64, TriangleSetupError> {
    let dx = i128::from(second.x) - i128::from(first.x);
    let dy = i128::from(second.y) - i128::from(first.y);
    let point_x = i128::from(point.x) - i128::from(first.x);
    let point_y = i128::from(point.y) - i128::from(first.y);
    checked_edge_value(dx, dy, point_x, point_y)
}

fn checked_edge_value(
    dx: i128,
    dy: i128,
    point_x: i128,
    point_y: i128,
) -> Result<i64, TriangleSetupError> {
    let first_product = dx
        .checked_mul(point_y)
        .ok_or(TriangleSetupError::ArithmeticOverflow)?;
    let second_product = dy
        .checked_mul(point_x)
        .ok_or(TriangleSetupError::ArithmeticOverflow)?;
    let value = first_product
        .checked_sub(second_product)
        .ok_or(TriangleSetupError::ArithmeticOverflow)?;
    i64::try_from(value).map_err(|_| TriangleSetupError::ArithmeticOverflow)
}

fn pixel_bounds(
    vertices: [FixedPointPosition; 3],
    width: usize,
    height: usize,
) -> Option<PixelBounds> {
    let min_x = vertices
        .iter()
        .map(|vertex| vertex.x)
        .min()?
        .div_euclid(SUBPIXEL_SCALE);
    let min_y = vertices
        .iter()
        .map(|vertex| vertex.y)
        .min()?
        .div_euclid(SUBPIXEL_SCALE);
    let max_x = vertices
        .iter()
        .map(|vertex| vertex.x)
        .max()?
        .div_euclid(SUBPIXEL_SCALE);
    let max_y = vertices
        .iter()
        .map(|vertex| vertex.y)
        .max()?
        .div_euclid(SUBPIXEL_SCALE);
    let target_max_x = (width - 1) as i64;
    let target_max_y = (height - 1) as i64;
    if max_x < 0 || max_y < 0 || min_x > target_max_x || min_y > target_max_y {
        return None;
    }
    Some(PixelBounds {
        min_x: min_x.clamp(0, target_max_x) as usize,
        min_y: min_y.clamp(0, target_max_y) as usize,
        max_x: max_x.clamp(0, target_max_x) as usize,
        max_y: max_y.clamp(0, target_max_y) as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32) -> ViewportPosition {
        ViewportPosition { x, y, z_ndc: 0.5 }
    }

    fn front_triangle() -> [ViewportPosition; 3] {
        [point(1.0, 1.0), point(5.0, 1.0), point(1.0, 4.0)]
    }

    fn direct_samples(setup: &TriangleSetup) -> Vec<CoveredSample> {
        let Some(bounds) = setup.bounds() else {
            return Vec::new();
        };
        let mut samples = Vec::new();
        for y in bounds.min_y..=bounds.max_y {
            for x in bounds.min_x..=bounds.max_x {
                let edge_values = setup.edge_values_at(x, y).unwrap();
                if setup.accepts(edge_values) {
                    samples.push(CoveredSample { x, y, edge_values });
                }
            }
        }
        samples
    }

    fn owner_counts(width: usize, height: usize, triangles: [[ViewportPosition; 3]; 2]) -> Vec<u8> {
        let mut owners = vec![0_u8; width * height];
        for triangle in triangles {
            TriangleSetup::new(triangle, width, height)
                .unwrap()
                .rasterize(|sample| {
                    let index = sample.y * width + sample.x;
                    owners[index] += 1;
                });
        }
        owners
    }

    #[test]
    fn orient2d_uses_positive_clockwise_winding_in_y_down_screen_space() {
        let [first, second, third] = front_triangle();
        assert_eq!(orient2d(first, second, third), 12.0);
        assert_eq!(orient2d(first, third, second), -12.0);
    }

    #[test]
    fn back_culling_rejects_back_faces_and_submits_front_faces() {
        assert_eq!(
            classify_triangle(front_triangle(), CullMode::Back),
            TriangleDisposition::Submit {
                source_orientation: FaceOrientation::Front,
                order: [0, 1, 2],
            }
        );
        let [first, second, third] = front_triangle();
        assert_eq!(
            classify_triangle([first, third, second], CullMode::Back),
            TriangleDisposition::Culled(FaceOrientation::Back)
        );
    }

    #[test]
    fn double_sided_and_front_culling_normalize_submitted_back_faces() {
        let [first, second, third] = front_triangle();
        let back = [first, third, second];
        let normalized = TriangleDisposition::Submit {
            source_orientation: FaceOrientation::Back,
            order: [0, 2, 1],
        };
        assert_eq!(classify_triangle(back, CullMode::None), normalized);
        assert_eq!(classify_triangle(back, CullMode::Front), normalized);
        assert_eq!(
            classify_triangle(front_triangle(), CullMode::Front),
            TriangleDisposition::Culled(FaceOrientation::Front)
        );
    }

    #[test]
    fn collinear_near_zero_and_non_finite_triangles_are_rejected() {
        assert_eq!(
            classify_triangle(
                [point(0.0, 0.0), point(1.0, 1.0), point(2.0, 2.0)],
                CullMode::None,
            ),
            TriangleDisposition::Degenerate
        );
        assert_eq!(
            classify_triangle(
                [
                    point(0.0, 0.0),
                    point(1.0, 0.0),
                    point(1.0, WIREFRAME_AREA_EPSILON / 2.0),
                ],
                CullMode::None,
            ),
            TriangleDisposition::Degenerate
        );
        assert_eq!(
            classify_triangle(
                [point(0.0, 0.0), point(f32::NAN, 1.0), point(1.0, 0.0)],
                CullMode::None,
            ),
            TriangleDisposition::Invalid
        );
    }

    #[test]
    fn mode_labels_are_stable_for_the_debug_overlay() {
        assert_eq!(CullMode::None.label(), "none");
        assert_eq!(CullMode::Back.label(), "back");
        assert_eq!(CullMode::Front.label(), "front");
        assert_eq!(WindingDebugMode::VertexColor.label(), "vertex color");
        assert_eq!(WindingDebugMode::Facing.label(), "front green / back red");
    }

    #[test]
    fn fixed_point_setup_uses_pixel_centers_and_y_down_top_left_edges() {
        let setup = TriangleSetup::new(front_triangle(), 8, 8).unwrap();
        assert_eq!(setup.area(), 12 * SUBPIXEL_SCALE * SUBPIXEL_SCALE);
        assert_eq!(setup.top_left_flags(), [false, true, true]);
        assert_eq!(
            setup.bounds(),
            Some(PixelBounds {
                min_x: 1,
                min_y: 1,
                max_x: 5,
                max_y: 4,
            })
        );
        let samples = direct_samples(&setup);
        assert!(samples.iter().any(|sample| (sample.x, sample.y) == (1, 1)));
        assert!(!samples.iter().any(|sample| (sample.x, sample.y) == (4, 3)));
    }

    #[test]
    fn incremental_edges_match_direct_oracle_for_boundaries_and_edge_directions() {
        let fixtures = [
            front_triangle(),
            [point(-2.0, -2.0), point(4.0, -2.0), point(-2.0, 4.0)],
            [point(2.25, 0.5), point(7.75, 3.5), point(1.5, 6.75)],
            [point(0.0, 7.0), point(0.0, 0.0), point(7.0, 7.0)],
        ];
        for triangle in fixtures {
            let setup = TriangleSetup::new(triangle, 8, 8).unwrap();
            let expected = direct_samples(&setup);
            let mut actual = Vec::new();
            let count = setup.rasterize(|sample| actual.push(sample));
            assert_eq!(count as usize, expected.len());
            assert_eq!(actual, expected);
            assert!(actual.iter().all(|sample| sample.x < 8 && sample.y < 8));
        }
    }

    #[test]
    fn axis_aligned_quad_has_one_owner_for_both_diagonals_and_cyclic_orders() {
        let (top_left, top_right, bottom_right, bottom_left) = (
            point(1.0, 1.0),
            point(7.0, 1.0),
            point(7.0, 6.0),
            point(1.0, 6.0),
        );
        let first_diagonal = owner_counts(
            9,
            8,
            [
                [top_left, top_right, bottom_right],
                [bottom_right, bottom_left, top_left],
            ],
        );
        let second_diagonal = owner_counts(
            9,
            8,
            [
                [top_right, bottom_left, top_left],
                [bottom_left, top_right, bottom_right],
            ],
        );
        assert_eq!(first_diagonal, second_diagonal);
        for y in 0..8 {
            for x in 0..9 {
                let expected = u8::from((1..7).contains(&x) && (1..6).contains(&y));
                assert_eq!(first_diagonal[y * 9 + x], expected, "sample ({x}, {y})");
            }
        }
    }

    #[test]
    fn rotated_quad_coverage_is_diagonal_independent_without_gaps_or_overlaps() {
        let (top, right, bottom, left) = (
            point(4.0, 1.0),
            point(7.0, 4.0),
            point(4.0, 7.0),
            point(1.0, 4.0),
        );
        let vertical = owner_counts(9, 9, [[top, right, bottom], [bottom, left, top]]);
        let horizontal = owner_counts(9, 9, [[left, top, right], [right, bottom, left]]);
        assert_eq!(vertical, horizontal);
        let expected_owned_samples = [
            (3, 1),
            (2, 2),
            (3, 2),
            (4, 2),
            (1, 3),
            (2, 3),
            (3, 3),
            (4, 3),
            (5, 3),
            (1, 4),
            (2, 4),
            (3, 4),
            (4, 4),
            (5, 4),
            (2, 5),
            (3, 5),
            (4, 5),
            (3, 6),
        ];
        for y in 0..9 {
            for x in 0..9 {
                let expected = u8::from(expected_owned_samples.contains(&(x, y)));
                assert_eq!(vertical[y * 9 + x], expected, "sample ({x}, {y})");
            }
        }
    }

    #[test]
    fn setup_reports_invalid_degenerate_backfacing_and_overflow_inputs() {
        assert_eq!(
            TriangleSetup::new(front_triangle(), 0, 8),
            Err(TriangleSetupError::InvalidTarget)
        );
        assert_eq!(
            TriangleSetup::new(front_triangle(), usize::MAX, 8),
            Err(TriangleSetupError::InvalidTarget)
        );
        for invalid in [
            [point(f32::NAN, 0.0), point(1.0, 0.0), point(0.0, 1.0)],
            [point(0.0, f32::INFINITY), point(1.0, 0.0), point(0.0, 1.0)],
            [
                ViewportPosition {
                    x: 0.0,
                    y: 0.0,
                    z_ndc: f32::NAN,
                },
                point(1.0, 0.0),
                point(0.0, 1.0),
            ],
        ] {
            assert_eq!(
                TriangleSetup::new(invalid, 8, 8),
                Err(TriangleSetupError::NonFinitePosition)
            );
        }
        assert_eq!(
            TriangleSetup::new(
                [point(0.0, 0.0), point(f32::MAX, 0.0), point(0.0, 1.0)],
                8,
                8,
            ),
            Err(TriangleSetupError::FixedPointOverflow)
        );
        assert_eq!(
            TriangleSetup::new(
                [point(0.0, 0.0), point(0.001, 0.0), point(0.0, 0.001)],
                8,
                8,
            ),
            Err(TriangleSetupError::Degenerate)
        );
        let [first, second, third] = front_triangle();
        assert_eq!(
            TriangleSetup::new([first, third, second], 8, 8),
            Err(TriangleSetupError::BackFacing)
        );
        assert_eq!(
            TriangleSetup::new(
                [
                    point(-1.0e15, -1.0e15),
                    point(1.0e15, -1.0e15),
                    point(-1.0e15, 1.0e15),
                ],
                8,
                8,
            ),
            Err(TriangleSetupError::ArithmeticOverflow)
        );
        let quantized_lower_bound = -2.0_f32.powi(55);
        let quantized_upper_bound = f32::from_bits(2.0_f32.powi(55).to_bits() - 1);
        assert_eq!(
            TriangleSetup::new(
                [
                    point(quantized_lower_bound, quantized_lower_bound),
                    point(quantized_upper_bound, quantized_lower_bound),
                    point(quantized_lower_bound, quantized_upper_bound),
                ],
                8,
                8,
            ),
            Err(TriangleSetupError::ArithmeticOverflow)
        );
    }

    #[test]
    fn fully_offscreen_setup_is_a_safe_no_op_and_sample_overflow_is_reported() {
        let setup = TriangleSetup::new(
            [point(-8.0, -8.0), point(-4.0, -8.0), point(-8.0, -4.0)],
            4,
            4,
        )
        .unwrap();
        assert_eq!(setup.bounds(), None);
        assert!(direct_samples(&setup).is_empty());
        assert_eq!(
            setup.rasterize(|_| panic!("offscreen triangle must not visit")),
            0
        );
        assert_eq!(
            setup.edge_values_at(usize::MAX, 0),
            Err(TriangleSetupError::ArithmeticOverflow)
        );
    }

    #[test]
    fn maximum_render_target_contract_keeps_fixed_point_edges_in_i64_range() {
        let maximum = 16_777_216.0;
        let setup = TriangleSetup::new(
            [point(0.0, 0.0), point(maximum, 0.0), point(0.0, 1.0)],
            maximum as usize,
            1,
        )
        .unwrap();
        assert_eq!(setup.bounds().unwrap().max_x, maximum as usize - 1);
        assert!(setup.area() > 0);
        assert!(setup.edge_values_at(maximum as usize - 1, 0).is_ok());
    }
}
