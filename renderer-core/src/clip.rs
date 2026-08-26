//! 10장의 동차 clip 공간 Sutherland-Hodgman clipping 계약.

use crate::mesh::ClipVertex;
use crate::transform::ClipPosition;

/// 삼각형을 convex frustum의 여섯 평면으로 자를 때 가능한 최대 정점 수다.
pub const MAX_CLIPPED_POLYGON_VERTICES: usize = 9;
/// 교점 계산의 반올림만 허용하는 clip 결과 debug postcondition ULP 배수다.
const CLIP_POSTCONDITION_ULPS: f32 = 8.0;

/// 내부 거리가 0 이상이 되도록 정의한 동차 clip 평면이다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipPlane {
    Left,
    Right,
    Bottom,
    Top,
    Near,
    Far,
}

impl ClipPlane {
    pub const ALL: [Self; 6] = [
        Self::Left,
        Self::Right,
        Self::Bottom,
        Self::Top,
        Self::Near,
        Self::Far,
    ];

    pub const fn distance(self, clip_pos: ClipPosition) -> f32 {
        let position = clip_pos.0;
        match self {
            Self::Left => position.x + position.w,
            Self::Right => position.w - position.x,
            Self::Bottom => position.y + position.w,
            Self::Top => position.w - position.y,
            Self::Near => position.z,
            Self::Far => position.w - position.z,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipStatus {
    Visible,
    FullyClipped,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipTriangleResult<'a> {
    pub status: ClipStatus,
    pub triangles: &'a [[ClipVertex; 3]],
    pub polygon_vertices: usize,
    pub max_polygon_vertices: usize,
}

/// 프레임 사이 capacity를 재사용하는 삼각형 clip scratch storage다.
#[derive(Clone, Debug)]
pub struct TriangleClipper {
    polygon_a: Vec<ClipVertex>,
    polygon_b: Vec<ClipVertex>,
    triangles: Vec<[ClipVertex; 3]>,
}

impl Default for TriangleClipper {
    fn default() -> Self {
        Self {
            polygon_a: Vec::with_capacity(MAX_CLIPPED_POLYGON_VERTICES),
            polygon_b: Vec::with_capacity(MAX_CLIPPED_POLYGON_VERTICES),
            triangles: Vec::with_capacity(MAX_CLIPPED_POLYGON_VERTICES - 2),
        }
    }
}

impl TriangleClipper {
    pub fn clip_triangle(&mut self, triangle: [ClipVertex; 3]) -> ClipTriangleResult<'_> {
        self.polygon_a.clear();
        self.polygon_b.clear();
        self.triangles.clear();

        if !triangle.into_iter().all(vertex_is_finite) {
            return self.result(ClipStatus::Invalid, 0, 0);
        }

        self.polygon_a.extend_from_slice(&triangle);
        let mut max_polygon_vertices = triangle.len();

        for plane in ClipPlane::ALL {
            self.polygon_b.clear();
            if self.polygon_a.is_empty() {
                return self.result(ClipStatus::FullyClipped, 0, max_polygon_vertices);
            }

            let mut previous = *self
                .polygon_a
                .last()
                .expect("비어 있지 않은 polygon에는 마지막 정점이 있어야 한다");
            let mut previous_distance = plane.distance(previous.clip_pos);
            if !previous_distance.is_finite() {
                return self.result(ClipStatus::Invalid, 0, max_polygon_vertices);
            }

            for current in self.polygon_a.iter().copied() {
                let current_distance = plane.distance(current.clip_pos);
                if !current_distance.is_finite() {
                    return self.result(ClipStatus::Invalid, 0, max_polygon_vertices);
                }
                let previous_inside = previous_distance >= 0.0;
                let current_inside = current_distance >= 0.0;

                if previous_inside != current_inside
                    && previous_distance != 0.0
                    && current_distance != 0.0
                {
                    let denominator = previous_distance - current_distance;
                    if !denominator.is_finite() || denominator == 0.0 {
                        return self.result(ClipStatus::Invalid, 0, max_polygon_vertices);
                    }
                    let t = previous_distance / denominator;
                    debug_assert!(t.is_finite() && (0.0..=1.0).contains(&t));
                    let intersection = previous.lerp(current, t);
                    if !vertex_is_finite(intersection) {
                        return self.result(ClipStatus::Invalid, 0, max_polygon_vertices);
                    }
                    self.polygon_b.push(intersection);
                }
                if current_inside {
                    self.polygon_b.push(current);
                }

                previous = current;
                previous_distance = current_distance;
            }

            debug_assert!(self.polygon_b.len() <= MAX_CLIPPED_POLYGON_VERTICES);
            max_polygon_vertices = max_polygon_vertices.max(self.polygon_b.len());
            std::mem::swap(&mut self.polygon_a, &mut self.polygon_b);
        }

        if self.polygon_a.len() < 3 {
            return self.result(ClipStatus::FullyClipped, 0, max_polygon_vertices);
        }
        debug_assert!(self.polygon_a.iter().all(|vertex| {
            ClipPlane::ALL.into_iter().all(|plane| {
                plane.distance(vertex.clip_pos) >= -postcondition_tolerance(vertex.clip_pos)
            })
        }));
        let anchor = self.polygon_a[0];
        for index in 1..self.polygon_a.len() - 1 {
            self.triangles
                .push([anchor, self.polygon_a[index], self.polygon_a[index + 1]]);
        }
        self.result(
            ClipStatus::Visible,
            self.polygon_a.len(),
            max_polygon_vertices,
        )
    }

    fn result(
        &self,
        status: ClipStatus,
        polygon_vertices: usize,
        max_polygon_vertices: usize,
    ) -> ClipTriangleResult<'_> {
        ClipTriangleResult {
            status,
            triangles: &self.triangles,
            polygon_vertices,
            max_polygon_vertices,
        }
    }
}

fn postcondition_tolerance(clip_pos: ClipPosition) -> f32 {
    let position = clip_pos.0;
    let coordinate_scale = position
        .x
        .abs()
        .max(position.y.abs())
        .max(position.z.abs())
        .max(position.w.abs())
        .max(1.0);
    coordinate_scale * f32::EPSILON * CLIP_POSTCONDITION_ULPS
}

fn vertex_is_finite(vertex: ClipVertex) -> bool {
    let clip = vertex.clip_pos.0;
    let world = vertex.world_pos;
    let normal = vertex.normal_world;
    let uv = vertex.uv;
    let color = vertex.color;
    [clip.x, clip.y, clip.z, clip.w]
        .into_iter()
        .chain([vertex.view_depth])
        .chain([world.x, world.y, world.z])
        .chain([normal.x, normal.y, normal.z])
        .chain([uv.x, uv.y])
        .chain([color.x, color.y, color.z, color.w])
        .all(f32::is_finite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Vec2, Vec3, Vec4};

    fn vertex(clip: Vec4, value: f32) -> ClipVertex {
        ClipVertex {
            clip_pos: ClipPosition(clip),
            view_depth: value + 0.5,
            world_pos: Vec3::new(value, value + 1.0, value + 2.0),
            normal_world: Vec3::new(value + 3.0, value + 4.0, value + 5.0),
            uv: Vec2::new(value + 6.0, value + 7.0),
            color: Vec4::new(value + 8.0, value + 9.0, value + 10.0, value + 11.0),
        }
    }

    fn inside_triangle() -> [ClipVertex; 3] {
        [
            vertex(Vec4::new(-0.25, -0.25, 0.5, 1.0), 0.0),
            vertex(Vec4::new(0.25, -0.25, 0.5, 1.0), 10.0),
            vertex(Vec4::new(0.0, 0.25, 0.5, 1.0), 20.0),
        ]
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1.0e-6,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn plane_distances_follow_lh_zero_to_one_clip_contract() {
        let position = ClipPosition(Vec4::new(2.0, 3.0, 5.0, 7.0));
        assert_eq!(
            ClipPlane::ALL.map(|plane| plane.distance(position)),
            [9.0, 5.0, 10.0, 4.0, 5.0, 2.0]
        );
    }

    #[test]
    fn fully_inside_triangle_preserves_values_and_triangle_count() {
        let input = inside_triangle();
        let mut clipper = TriangleClipper::default();
        let result = clipper.clip_triangle(input);
        assert_eq!(result.status, ClipStatus::Visible);
        assert_eq!(result.triangles, &[input]);
        assert_eq!(result.polygon_vertices, 3);
        assert_eq!(result.max_polygon_vertices, 3);
    }

    #[test]
    fn each_plane_clips_one_outside_vertex_into_two_triangles() {
        let outside_positions = [
            Vec4::new(-2.0, 0.25, 0.5, 1.0),
            Vec4::new(2.0, 0.25, 0.5, 1.0),
            Vec4::new(0.0, -2.0, 0.5, 1.0),
            Vec4::new(0.0, 2.0, 0.5, 1.0),
            Vec4::new(0.0, 0.25, -0.5, 1.0),
            Vec4::new(0.0, 0.25, 2.0, 1.0),
        ];
        for (outside_position, plane) in outside_positions.into_iter().zip(ClipPlane::ALL) {
            let mut input = inside_triangle();
            input[2] = vertex(outside_position, 20.0);
            let mut clipper = TriangleClipper::default();
            let result = clipper.clip_triangle(input);
            assert_eq!(result.status, ClipStatus::Visible, "{plane:?}");
            assert_eq!(result.triangles.len(), 2, "{plane:?}");
            assert_eq!(result.polygon_vertices, 4, "{plane:?}");
            assert!(result.triangles.iter().flatten().all(|vertex| {
                ClipPlane::ALL
                    .into_iter()
                    .all(|candidate| candidate.distance(vertex.clip_pos) >= -1.0e-6)
            }));
        }
    }

    #[test]
    fn near_intersections_lerp_every_attribute_with_the_same_t() {
        let outside = vertex(Vec4::new(0.0, 0.5, -0.5, 1.0), 20.0);
        let first_inside = vertex(Vec4::new(-0.5, -0.5, 0.5, 1.0), 0.0);
        let second_inside = vertex(Vec4::new(0.5, -0.5, 0.5, 1.0), 10.0);
        let mut clipper = TriangleClipper::default();
        let result = clipper.clip_triangle([first_inside, second_inside, outside]);
        assert_eq!(result.status, ClipStatus::Visible);
        assert_eq!(result.triangles.len(), 2);

        let intersection = result.triangles[0][0];
        assert_eq!(intersection.clip_pos.0.z, 0.0);
        assert_close(intersection.clip_pos.0.y, 0.0);
        assert_close(intersection.world_pos.x, 10.0);
        assert_close(intersection.normal_world.x, 13.0);
        assert_close(intersection.uv.x, 16.0);
        assert_close(intersection.color.x, 18.0);
    }

    #[test]
    fn triangle_with_a_negative_w_vertex_is_clipped_before_safe_divide() {
        let triangle = [
            vertex(Vec4::new(-0.25, -0.25, 0.5, 1.0), 0.0),
            vertex(Vec4::new(0.25, -0.25, 0.5, 1.0), 1.0),
            vertex(Vec4::new(0.0, 0.25, -0.5, -0.5), 2.0),
        ];
        assert!(triangle[2].clip_pos.0.w < 0.0);
        let mut clipper = TriangleClipper::default();
        let result = clipper.clip_triangle(triangle);
        assert_eq!(result.status, ClipStatus::Visible);
        assert!(!result.triangles.is_empty());
        assert!(result.triangles.iter().flatten().all(|vertex| {
            crate::camera::perspective_divide(vertex.clip_pos).is_ok()
                && ClipPlane::ALL
                    .into_iter()
                    .all(|plane| plane.distance(vertex.clip_pos) >= -1.0e-6)
        }));
    }

    #[test]
    fn fully_outside_and_behind_camera_triangles_emit_nothing() {
        for (plane, outside) in ClipPlane::ALL.into_iter().zip([
            Vec4::new(-2.0, 0.0, 0.5, 1.0),
            Vec4::new(2.0, 0.0, 0.5, 1.0),
            Vec4::new(0.0, -2.0, 0.5, 1.0),
            Vec4::new(0.0, 2.0, 0.5, 1.0),
            Vec4::new(0.0, 0.0, -0.5, 1.0),
            Vec4::new(0.0, 0.0, 2.0, 1.0),
        ]) {
            let triangle = [
                vertex(outside, 0.0),
                vertex(outside, 1.0),
                vertex(outside, 2.0),
            ];
            let mut clipper = TriangleClipper::default();
            let result = clipper.clip_triangle(triangle);
            assert_eq!(result.status, ClipStatus::FullyClipped, "{plane:?}");
            assert!(result.triangles.is_empty());
        }

        let behind = [
            vertex(Vec4::new(-0.5, -0.5, -1.0, -1.0), 0.0),
            vertex(Vec4::new(0.5, -0.5, -1.0, -1.0), 1.0),
            vertex(Vec4::new(0.0, 0.5, -1.0, -1.0), 2.0),
        ];
        let mut clipper = TriangleClipper::default();
        assert_eq!(
            clipper.clip_triangle(behind).status,
            ClipStatus::FullyClipped
        );
    }

    #[test]
    fn corner_crossing_stays_inside_all_six_planes() {
        let triangle = [
            vertex(Vec4::new(-2.0, 2.0, -0.5, 1.0), 0.0),
            vertex(Vec4::new(0.75, -0.75, 0.5, 1.0), 1.0),
            vertex(Vec4::new(-0.25, 0.25, 0.5, 1.0), 2.0),
        ];
        let mut clipper = TriangleClipper::default();
        let result = clipper.clip_triangle(triangle);
        assert_eq!(result.status, ClipStatus::Visible);
        assert!(!result.triangles.is_empty());
        assert!(result.triangles.iter().flatten().all(|vertex| {
            ClipPlane::ALL
                .into_iter()
                .all(|plane| plane.distance(vertex.clip_pos) >= -1.0e-6)
        }));
    }

    #[test]
    fn boundary_endpoints_are_not_duplicated_on_any_plane_in_either_order() {
        let cases = [
            (
                Vec4::new(-2.0, 0.0, 0.5, 1.0),
                Vec4::new(-1.0, 0.25, 0.5, 1.0),
                Vec4::new(0.0, -0.25, 0.5, 1.0),
            ),
            (
                Vec4::new(2.0, 0.0, 0.5, 1.0),
                Vec4::new(1.0, 0.25, 0.5, 1.0),
                Vec4::new(0.0, -0.25, 0.5, 1.0),
            ),
            (
                Vec4::new(0.0, -2.0, 0.5, 1.0),
                Vec4::new(0.25, -1.0, 0.5, 1.0),
                Vec4::new(-0.25, 0.0, 0.5, 1.0),
            ),
            (
                Vec4::new(0.0, 2.0, 0.5, 1.0),
                Vec4::new(0.25, 1.0, 0.5, 1.0),
                Vec4::new(-0.25, 0.0, 0.5, 1.0),
            ),
            (
                Vec4::new(0.0, 0.0, -1.0, 1.0),
                Vec4::new(0.25, 0.25, 0.0, 1.0),
                Vec4::new(-0.25, -0.25, 0.5, 1.0),
            ),
            (
                Vec4::new(0.0, 0.0, 2.0, 1.0),
                Vec4::new(0.25, 0.25, 1.0, 1.0),
                Vec4::new(-0.25, -0.25, 0.5, 1.0),
            ),
        ];
        for ((outside, boundary, inside), plane) in cases.into_iter().zip(ClipPlane::ALL) {
            for positions in [[outside, boundary, inside], [inside, boundary, outside]] {
                let triangle = positions.map(|position| vertex(position, position.x));
                let mut clipper = TriangleClipper::default();
                let result = clipper.clip_triangle(triangle);
                assert_eq!(result.status, ClipStatus::Visible, "{plane:?}");
                assert_eq!(result.polygon_vertices, 3, "{plane:?}");
                assert_eq!(result.triangles.len(), 1, "{plane:?}");
                assert_ne!(result.triangles[0][0], result.triangles[0][1], "{plane:?}");
                assert_ne!(result.triangles[0][1], result.triangles[0][2], "{plane:?}");
            }
        }
    }

    #[test]
    fn postcondition_tolerance_scales_with_large_homogeneous_coordinates() {
        for scale in [1.0, 100.0, 1.0e20] {
            let triangle = [
                vertex(Vec4::new(-2.0, 2.0, -0.5, 1.0) * scale, 0.0),
                vertex(Vec4::new(0.75, -0.5, 0.5, 1.0) * scale, 1.0),
                vertex(Vec4::new(-0.25, -0.25, 0.5, 1.0) * scale, 2.0),
            ];
            let mut clipper = TriangleClipper::default();
            let result = clipper.clip_triangle(triangle);
            assert_eq!(result.status, ClipStatus::Visible, "scale {scale}");
            assert!(result.triangles.iter().flatten().all(|vertex| {
                ClipPlane::ALL.into_iter().all(|plane| {
                    plane.distance(vertex.clip_pos) >= -postcondition_tolerance(vertex.clip_pos)
                })
            }));
        }
    }

    #[test]
    fn clipping_reuses_preallocated_polygon_and_fan_scratch() {
        let triangle = [
            vertex(Vec4::new(-2.0, 2.0, -0.5, 1.0), 0.0),
            vertex(Vec4::new(0.75, -0.5, 0.5, 1.0), 1.0),
            vertex(Vec4::new(-0.25, -0.25, 0.5, 1.0), 2.0),
        ];
        let mut clipper = TriangleClipper::default();
        let pointers = (
            clipper.polygon_a.as_ptr(),
            clipper.polygon_b.as_ptr(),
            clipper.triangles.as_ptr(),
        );
        let capacities = (
            clipper.polygon_a.capacity(),
            clipper.polygon_b.capacity(),
            clipper.triangles.capacity(),
        );
        for _ in 0..16 {
            let result = clipper.clip_triangle(triangle);
            assert_eq!(result.status, ClipStatus::Visible);
            assert_eq!(result.polygon_vertices, 5);
        }
        assert_eq!(
            (
                clipper.polygon_a.as_ptr(),
                clipper.polygon_b.as_ptr(),
                clipper.triangles.as_ptr(),
            ),
            pointers
        );
        assert_eq!(
            (
                clipper.polygon_a.capacity(),
                clipper.polygon_b.capacity(),
                clipper.triangles.capacity(),
            ),
            capacities
        );
    }

    #[test]
    fn non_finite_vertex_or_distance_is_invalid_without_output() {
        let mut invalid_attribute = inside_triangle();
        invalid_attribute[0].uv.x = f32::NAN;
        let mut clipper = TriangleClipper::default();
        let result = clipper.clip_triangle(invalid_attribute);
        assert_eq!(result.status, ClipStatus::Invalid);
        assert!(result.triangles.is_empty());

        let mut overflowing_distance = inside_triangle();
        overflowing_distance[0].clip_pos.0.x = f32::MAX;
        overflowing_distance[0].clip_pos.0.w = f32::MAX;
        assert_eq!(
            clipper.clip_triangle(overflowing_distance).status,
            ClipStatus::Invalid
        );

        let mut invalid_previous_distance = inside_triangle();
        invalid_previous_distance[2].clip_pos.0.x = f32::MAX;
        invalid_previous_distance[2].clip_pos.0.w = f32::MAX;
        assert_eq!(
            clipper.clip_triangle(invalid_previous_distance).status,
            ClipStatus::Invalid
        );

        let mut overflowing_denominator = inside_triangle();
        for vertex in &mut overflowing_denominator {
            vertex.clip_pos.0.w = f32::MAX;
            vertex.clip_pos.0.z = f32::MAX;
        }
        overflowing_denominator[2].clip_pos.0.z = -f32::MAX;
        assert_eq!(
            clipper.clip_triangle(overflowing_denominator).status,
            ClipStatus::Invalid
        );

        let mut overflowing_attribute = inside_triangle();
        overflowing_attribute[0].world_pos.x = -f32::MAX;
        overflowing_attribute[2].world_pos.x = f32::MAX;
        overflowing_attribute[2].clip_pos.0.z = -0.5;
        assert_eq!(
            clipper.clip_triangle(overflowing_attribute).status,
            ClipStatus::Invalid
        );
    }
}
