//! object/world/view/clip 공간, MVP 캐시와 18장 normal matrix 변환.

use crate::math::{Mat3, Mat4, Vec3, Vec4};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectPosition(pub Vec3);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldPosition(pub Vec4);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewPosition(pub Vec4);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPosition(pub Vec4);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoordinateSpace {
    Object,
    World,
    View,
    Clip,
}

impl CoordinateSpace {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Object => "Object",
            Self::World => "World",
            Self::View => "View",
            Self::Clip => "Clip",
        }
    }
}

/// translation, Euler rotation, scale 순으로 모델 배치를 표현한다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation_radians: Vec3,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation_radians: Vec3::ZERO,
        scale: Vec3::new(1.0, 1.0, 1.0),
    };

    /// 열벡터에 scale, X/Y/Z 회전, translation 순으로 적용한다.
    pub fn model_matrix(self) -> Mat4 {
        Mat4::translation(self.translation)
            * Mat4::rotation_z(self.rotation_radians.z)
            * Mat4::rotation_y(self.rotation_radians.y)
            * Mat4::rotation_x(self.rotation_radians.x)
            * Mat4::scale(self.scale)
    }
}

/// 한 모델의 M, 프레임 공통 VP, 최종 MVP를 한 번만 합성해 보관한다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformPipeline {
    model: Mat4,
    view: Mat4,
    projection: Mat4,
    view_projection: Mat4,
    model_view_projection: Mat4,
    normal_matrix: Option<Mat3>,
}

impl TransformPipeline {
    pub fn new(model: Mat4, view: Mat4, projection: Mat4) -> Self {
        let view_projection = projection * view;
        Self {
            model,
            view,
            projection,
            view_projection,
            model_view_projection: view_projection * model,
            normal_matrix: model.upper_left_3x3().inverse().map(Mat3::transpose),
        }
    }

    pub fn trace(self, object_pos: ObjectPosition) -> VertexTrace {
        let object_vec = Vec4::point(object_pos.0);
        let world_pos = WorldPosition(self.model * object_vec);
        let view_pos = ViewPosition(self.view * world_pos.0);
        let clip_pos = ClipPosition(self.projection * view_pos.0);
        VertexTrace {
            object_pos,
            world_pos,
            view_pos,
            clip_pos,
        }
    }

    pub fn transform_mvp(self, object_pos: ObjectPosition) -> ClipPosition {
        ClipPosition(self.model_view_projection * Vec4::point(object_pos.0))
    }

    /// model upper 3x3의 inverse-transpose로 object normal을 world 공간에 둔다.
    /// singular model이면 `None`을 반환해 NaN이 vertex 속성으로 퍼지지 않게 한다.
    pub fn transform_model_normal(self, object_normal: Vec3) -> Option<Vec3> {
        (self.normal_matrix? * object_normal).normalized()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VertexTrace {
    pub object_pos: ObjectPosition,
    pub world_pos: WorldPosition,
    pub view_pos: ViewPosition,
    pub clip_pos: ClipPosition,
}

impl VertexTrace {
    pub const fn value(self, space: CoordinateSpace) -> Vec4 {
        match space {
            CoordinateSpace::Object => Vec4::point(self.object_pos.0),
            CoordinateSpace::World => self.world_pos.0,
            CoordinateSpace::View => self.view_pos.0,
            CoordinateSpace::Clip => self.clip_pos.0,
        }
    }
}

/// `x+w`, `w-x`, `y+w`, `w-y`, `z`, `w-z` 순서의 동차 clip 평면 거리다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPlaneDistances(pub [f32; 6]);

impl ClipPlaneDistances {
    pub const fn from_position(clip_pos: ClipPosition) -> Self {
        use crate::clip::ClipPlane;
        Self([
            ClipPlane::Left.distance(clip_pos),
            ClipPlane::Right.distance(clip_pos),
            ClipPlane::Bottom.distance(clip_pos),
            ClipPlane::Top.distance(clip_pos),
            ClipPlane::Near.distance(clip_pos),
            ClipPlane::Far.distance(clip_pos),
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateBounds {
    pub min: Vec4,
    pub max: Vec4,
}

impl CoordinateBounds {
    const fn empty() -> Self {
        Self {
            min: Vec4::new(f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY),
            max: Vec4::new(
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ),
        }
    }

    fn include(&mut self, value: Vec4) {
        self.min = Vec4::new(
            self.min.x.min(value.x),
            self.min.y.min(value.y),
            self.min.z.min(value.z),
            self.min.w.min(value.w),
        );
        self.max = Vec4::new(
            self.max.x.max(value.x),
            self.max.y.max(value.y),
            self.max.z.max(value.z),
            self.max.w.max(value.w),
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateDiagnostics {
    pub bounds: [CoordinateBounds; 4],
    pub invalid_values: u32,
    pub first_invalid_space: Option<CoordinateSpace>,
}

impl CoordinateDiagnostics {
    pub fn from_traces(traces: &[VertexTrace]) -> Self {
        let spaces = [
            CoordinateSpace::Object,
            CoordinateSpace::World,
            CoordinateSpace::View,
            CoordinateSpace::Clip,
        ];
        let mut bounds = [CoordinateBounds::empty(); 4];
        let mut invalid_values = 0_u32;
        let mut first_invalid_space = None;
        for trace in traces {
            for (index, space) in spaces.into_iter().enumerate() {
                let value = trace.value(space);
                if vec4_is_finite(value) {
                    bounds[index].include(value);
                } else {
                    invalid_values = invalid_values.saturating_add(1);
                    first_invalid_space.get_or_insert(space);
                }
            }
        }
        Self {
            bounds,
            invalid_values,
            first_invalid_space,
        }
    }
}

pub const fn vec4_is_finite(value: Vec4) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite() && value.w.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1.0e-5;

    fn assert_vec4_close(actual: Vec4, expected: Vec4) {
        for (actual, expected) in [
            (actual.x, expected.x),
            (actual.y, expected.y),
            (actual.z, expected.z),
            (actual.w, expected.w),
        ] {
            assert!(
                (actual - expected).abs() <= EPSILON,
                "{actual} != {expected}"
            );
        }
    }

    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert_vec4_close(Vec4::direction(actual), Vec4::direction(expected));
    }

    #[test]
    fn identity_pipeline_preserves_every_named_space() {
        let object = ObjectPosition(Vec3::new(0.25, -0.5, 0.75));
        let pipeline = TransformPipeline::new(
            Transform::IDENTITY.model_matrix(),
            Mat4::identity(),
            Mat4::identity(),
        );
        let trace = pipeline.trace(object);
        let expected = Vec4::point(object.0);
        for space in [
            CoordinateSpace::Object,
            CoordinateSpace::World,
            CoordinateSpace::View,
            CoordinateSpace::Clip,
        ] {
            assert_eq!(trace.value(space), expected);
        }
        assert_eq!(pipeline.view_projection, Mat4::identity());
        assert_eq!(pipeline.model_view_projection, Mat4::identity());
    }

    #[test]
    fn translation_changes_world_and_later_spaces_without_changing_object() {
        let object = ObjectPosition(Vec3::new(1.0, 2.0, 3.0));
        let transform = Transform {
            translation: Vec3::new(4.0, -2.0, 1.0),
            ..Transform::IDENTITY
        };
        let trace =
            TransformPipeline::new(transform.model_matrix(), Mat4::identity(), Mat4::identity())
                .trace(object);
        assert_eq!(trace.object_pos, object);
        let expected = Vec4::new(5.0, 0.0, 4.0, 1.0);
        assert_eq!(trace.world_pos.0, expected);
        assert_eq!(trace.view_pos.0, expected);
        assert_eq!(trace.clip_pos.0, expected);
    }

    #[test]
    fn cached_mvp_matches_sequential_non_identity_transforms() {
        let transform = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation_radians: Vec3::new(0.2, -0.4, 0.7),
            scale: Vec3::new(2.0, 0.5, 1.5),
        };
        let view = Mat4::translation(Vec3::new(-0.5, 1.0, 2.0));
        let projection = Mat4::scale(Vec3::new(0.75, 1.25, 0.5));
        let pipeline = TransformPipeline::new(transform.model_matrix(), view, projection);
        let object = ObjectPosition(Vec3::new(-0.3, 0.8, 1.1));
        let sequential = pipeline.trace(object).clip_pos.0;
        assert_vec4_close(pipeline.transform_mvp(object).0, sequential);
        assert_eq!(pipeline.view_projection, projection * view);
        assert_eq!(
            pipeline.model_view_projection,
            projection * view * pipeline.model
        );
        let expected_normal = pipeline
            .model
            .upper_left_3x3()
            .inverse()
            .unwrap()
            .transpose()
            * Vec3::Y;
        assert_vec3_close(
            pipeline.transform_model_normal(Vec3::Y).unwrap(),
            expected_normal.normalized().unwrap(),
        );
        let singular = TransformPipeline::new(
            Mat4::scale(Vec3::new(1.0, 0.0, 1.0)),
            Mat4::identity(),
            Mat4::identity(),
        );
        assert_eq!(singular.transform_model_normal(Vec3::Y), None);
    }

    #[test]
    fn adding_one_turn_returns_a_transformed_vertex_to_the_same_position() {
        let object = ObjectPosition(Vec3::new(0.3, -0.7, 1.2));
        let base = Transform {
            rotation_radians: Vec3::new(0.2, -0.5, 0.9),
            ..Transform::IDENTITY
        };
        let one_turn_later = Transform {
            rotation_radians: Vec3::new(
                base.rotation_radians.x,
                base.rotation_radians.y + std::f32::consts::TAU,
                base.rotation_radians.z,
            ),
            ..base
        };
        assert_vec4_close(
            base.model_matrix().transform_point(object.0),
            one_turn_later.model_matrix().transform_point(object.0),
        );
    }

    #[test]
    fn clip_distances_follow_the_six_plane_contract() {
        assert_eq!(
            ClipPlaneDistances::from_position(ClipPosition(Vec4::new(0.25, -0.5, 0.75, 2.0))).0,
            [2.25, 1.75, 1.5, 2.5, 0.75, 1.25]
        );
    }

    #[test]
    fn diagnostics_collect_bounds_and_report_the_first_invalid_space() {
        let valid_pipeline =
            TransformPipeline::new(Mat4::identity(), Mat4::identity(), Mat4::identity());
        let valid = [
            valid_pipeline.trace(ObjectPosition(Vec3::new(-1.0, 2.0, 0.25))),
            valid_pipeline.trace(ObjectPosition(Vec3::new(3.0, -4.0, 0.75))),
        ];
        let diagnostics = CoordinateDiagnostics::from_traces(&valid);
        assert_eq!(diagnostics.invalid_values, 0);
        assert_eq!(diagnostics.first_invalid_space, None);
        assert_eq!(diagnostics.bounds[0].min, Vec4::new(-1.0, -4.0, 0.25, 1.0));
        assert_eq!(diagnostics.bounds[3].max, Vec4::new(3.0, 2.0, 0.75, 1.0));

        let invalid_pipeline = TransformPipeline::new(
            Mat4::rotation_y(f32::NAN),
            Mat4::identity(),
            Mat4::identity(),
        );
        let invalid = [invalid_pipeline.trace(ObjectPosition(Vec3::ZERO))];
        let diagnostics = CoordinateDiagnostics::from_traces(&invalid);
        assert_eq!(diagnostics.invalid_values, 3);
        assert_eq!(
            diagnostics.first_invalid_space,
            Some(CoordinateSpace::World)
        );
        assert_eq!(diagnostics.first_invalid_space.unwrap().label(), "World");
        assert!(!vec4_is_finite(invalid[0].clip_pos.0));
        assert_eq!(CoordinateSpace::Object.label(), "Object");
        assert_eq!(CoordinateSpace::View.label(), "View");
        assert_eq!(CoordinateSpace::Clip.label(), "Clip");
    }
}
