//! 7장의 왼손 look-at, zero-to-one 원근 투영과 viewport 변환.

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::math::{Mat4, Vec3};
use crate::transform::ClipPosition;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraError {
    NonFiniteLookAt,
    EyeTargetTooClose,
    InvalidWorldUp,
    NonFiniteViewMatrix,
    InvalidFovY,
    InvalidAspect,
    InvalidDepthRange,
    InvalidClipPosition,
    InvalidViewport,
}

impl Display for CameraError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::NonFiniteLookAt => "카메라 eye와 target은 유한해야 합니다",
            Self::EyeTargetTooClose => {
                "카메라 eye와 target은 안정적인 방향을 정할 만큼 떨어져야 합니다"
            }
            Self::InvalidWorldUp => "카메라 world_up은 유한한 0이 아닌 벡터여야 합니다",
            Self::NonFiniteViewMatrix => "카메라 입력이 유한한 view 행렬을 만들 수 없습니다",
            Self::InvalidFovY => "세로 시야각은 유한한 0과 pi 사이여야 합니다",
            Self::InvalidAspect => "화면 종횡비는 유한한 양수여야 합니다",
            Self::InvalidDepthRange => "깊이 범위는 유한한 0 < near < far여야 합니다",
            Self::InvalidClipPosition => {
                "perspective divide에는 유한하고 0이 아닌 clip w가 필요합니다"
            }
            Self::InvalidViewport => "viewport 크기는 유한한 양수여야 합니다",
        };
        formatter.write_str(message)
    }
}

impl Error for CameraError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraBasis {
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
    pub used_fallback_up: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NdcPosition(pub Vec3);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportPosition {
    pub x: f32,
    pub y: f32,
    pub z_ndc: f32,
}

fn vec3_is_finite(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

fn mat4_is_finite(matrix: Mat4) -> bool {
    (0..4).all(|row| (0..4).all(|column| matrix.get(row, column).is_finite()))
}

/// 통상적인 cross 식으로 LH/+Z 카메라의 정규직교 기저를 만든다.
pub fn look_at_basis_lh(
    eye: Vec3,
    target: Vec3,
    world_up: Vec3,
) -> Result<CameraBasis, CameraError> {
    if !vec3_is_finite(eye) || !vec3_is_finite(target) {
        return Err(CameraError::NonFiniteLookAt);
    }
    let forward_delta = target - eye;
    if !vec3_is_finite(forward_delta) {
        return Err(CameraError::NonFiniteViewMatrix);
    }
    if !forward_delta.length_squared().is_finite() {
        return Err(CameraError::NonFiniteViewMatrix);
    }
    let forward = forward_delta
        .normalized()
        .ok_or(CameraError::EyeTargetTooClose)?;
    let world_up = if vec3_is_finite(world_up) {
        world_up.normalized().ok_or(CameraError::InvalidWorldUp)?
    } else {
        return Err(CameraError::InvalidWorldUp);
    };
    let (right, used_fallback_up) = if let Some(right) = world_up.cross(forward).normalized() {
        (right, false)
    } else {
        let fallback_up = if forward.y.abs() < 0.999 {
            Vec3::Y
        } else {
            Vec3::X
        };
        (
            fallback_up
                .cross(forward)
                .normalized()
                .ok_or(CameraError::InvalidWorldUp)?,
            true,
        )
    };
    let up = forward.cross(right);
    Ok(CameraBasis {
        right,
        up,
        forward,
        used_fallback_up,
    })
}

/// 열벡터와 LH/+Z 규약의 world-to-view 행렬을 만든다.
pub fn look_at_lh(eye: Vec3, target: Vec3, world_up: Vec3) -> Result<Mat4, CameraError> {
    let basis = look_at_basis_lh(eye, target, world_up)?;
    let view = Mat4::from_rows([
        [
            basis.right.x,
            basis.right.y,
            basis.right.z,
            -basis.right.dot(eye),
        ],
        [basis.up.x, basis.up.y, basis.up.z, -basis.up.dot(eye)],
        [
            basis.forward.x,
            basis.forward.y,
            basis.forward.z,
            -basis.forward.dot(eye),
        ],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    if mat4_is_finite(view) {
        Ok(view)
    } else {
        Err(CameraError::NonFiniteViewMatrix)
    }
}

/// `w_clip=z_view`, NDC 깊이 `0..1`인 왼손 원근 투영 행렬을 만든다.
pub fn perspective_lh_zo(
    fov_y_radians: f32,
    aspect: f32,
    near: f32,
    far: f32,
) -> Result<Mat4, CameraError> {
    if !fov_y_radians.is_finite() || fov_y_radians <= 0.0 || fov_y_radians >= std::f32::consts::PI {
        return Err(CameraError::InvalidFovY);
    }
    if !aspect.is_finite() || aspect <= 0.0 {
        return Err(CameraError::InvalidAspect);
    }
    if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
        return Err(CameraError::InvalidDepthRange);
    }
    let q = (0.5 * fov_y_radians).tan().recip();
    if !q.is_finite() {
        return Err(CameraError::InvalidFovY);
    }
    let x_scale = q / aspect;
    if !x_scale.is_finite() {
        return Err(CameraError::InvalidAspect);
    }
    let depth_scale = far / (far - near);
    let depth_translation = -near * depth_scale;
    if !depth_scale.is_finite() || !depth_translation.is_finite() {
        return Err(CameraError::InvalidDepthRange);
    }
    Ok(Mat4::from_rows([
        [x_scale, 0.0, 0.0, 0.0],
        [0.0, q, 0.0, 0.0],
        [0.0, 0.0, depth_scale, depth_translation],
        [0.0, 0.0, 1.0, 0.0],
    ]))
}

/// clipping 뒤의 동차 좌표를 w로 나눠 NDC로 옮긴다.
pub fn perspective_divide(clip_position: ClipPosition) -> Result<NdcPosition, CameraError> {
    let clip = clip_position.0;
    if !clip.x.is_finite()
        || !clip.y.is_finite()
        || !clip.z.is_finite()
        || !clip.w.is_finite()
        || clip.w == 0.0
    {
        return Err(CameraError::InvalidClipPosition);
    }
    let ndc = Vec3::new(clip.x / clip.w, clip.y / clip.w, clip.z / clip.w);
    if vec3_is_finite(ndc) {
        Ok(NdcPosition(ndc))
    } else {
        Err(CameraError::InvalidClipPosition)
    }
}

/// NDC를 픽셀 셀 경계 기준 `0..width`, `0..height` 화면 좌표로 옮긴다.
pub fn viewport(
    ndc_position: NdcPosition,
    width: f32,
    height: f32,
) -> Result<ViewportPosition, CameraError> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(CameraError::InvalidViewport);
    }
    let ndc = ndc_position.0;
    if !vec3_is_finite(ndc) {
        return Err(CameraError::InvalidClipPosition);
    }
    let position = ViewportPosition {
        x: (0.5 + 0.5 * ndc.x) * width,
        y: (0.5 - 0.5 * ndc.y) * height,
        z_ndc: ndc.z,
    };
    if position.x.is_finite() && position.y.is_finite() {
        Ok(position)
    } else {
        Err(CameraError::InvalidViewport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec4;

    const EPSILON: f32 = 1.0e-5;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert_close(actual.x, expected.x);
        assert_close(actual.y, expected.y);
        assert_close(actual.z, expected.z);
    }

    #[test]
    fn canonical_look_at_is_identity_and_maps_eye_and_forward_distance() {
        let eye = Vec3::new(0.0, 0.0, -3.0);
        let target = Vec3::new(0.0, 0.0, -2.0);
        let view = look_at_lh(eye, target, Vec3::Y).expect("canonical camera should be valid");
        assert_eq!(
            look_at_lh(Vec3::ZERO, Vec3::Z, Vec3::Y).unwrap(),
            Mat4::identity()
        );
        assert_eq!(view.transform_point(eye), Vec4::new(0.0, 0.0, 0.0, 1.0));
        assert_eq!(
            view.transform_point(Vec3::new(0.0, 0.0, 2.0)),
            Vec4::new(0.0, 0.0, 5.0, 1.0)
        );
    }

    #[test]
    fn look_at_rejects_finite_inputs_when_view_translation_overflows() {
        let eye = Vec3::new(f32::MAX, f32::MAX, 0.0);
        let target = Vec3::new(f32::MAX, f32::MAX, 1.0);
        assert_eq!(
            look_at_lh(eye, target, Vec3::new(1.0, 1.0, 0.0)),
            Err(CameraError::NonFiniteViewMatrix)
        );
        assert_eq!(
            look_at_lh(
                Vec3::new(-f32::MAX, 0.0, 0.0),
                Vec3::new(f32::MAX, 0.0, 0.0),
                Vec3::Y,
            ),
            Err(CameraError::NonFiniteViewMatrix)
        );
        assert_eq!(
            look_at_lh(Vec3::ZERO, Vec3::new(f32::MAX, 0.0, 0.0), Vec3::Y),
            Err(CameraError::NonFiniteViewMatrix)
        );
    }

    #[test]
    fn oblique_look_at_basis_is_orthonormal_and_parallel_up_has_a_fallback() {
        let basis = look_at_basis_lh(Vec3::new(2.0, 1.0, -4.0), Vec3::ZERO, Vec3::Y).unwrap();
        assert_close(basis.right.length(), 1.0);
        assert_close(basis.up.length(), 1.0);
        assert_close(basis.forward.length(), 1.0);
        assert_close(basis.right.dot(basis.up), 0.0);
        assert_close(basis.up.dot(basis.forward), 0.0);
        assert_close(basis.forward.dot(basis.right), 0.0);
        assert!(!basis.used_fallback_up);

        let fallback = look_at_basis_lh(Vec3::ZERO, Vec3::Y, Vec3::Y).unwrap();
        assert!(fallback.used_fallback_up);
        assert_vec3_close(fallback.forward, Vec3::Y);
        assert_close(fallback.right.dot(fallback.forward), 0.0);
        assert_close(fallback.up.dot(fallback.forward), 0.0);

        let vertical_fallback = look_at_basis_lh(Vec3::ZERO, Vec3::Z, Vec3::Z).unwrap();
        assert!(vertical_fallback.used_fallback_up);
        assert_vec3_close(vertical_fallback.forward, Vec3::Z);
        assert_vec3_close(vertical_fallback.right, Vec3::X);
        assert_vec3_close(vertical_fallback.up, Vec3::Y);
    }

    #[test]
    fn perspective_maps_near_far_and_uses_view_z_as_clip_w() {
        let near = 0.25;
        let far = 50.0;
        let projection = perspective_lh_zo(std::f32::consts::FRAC_PI_2, 2.0, near, far).unwrap();
        let near_clip = projection.transform_point(Vec3::new(0.0, 0.0, near));
        let far_clip = projection.transform_point(Vec3::new(0.0, 0.0, far));
        assert_close(near_clip.w, near);
        assert_close(far_clip.w, far);
        assert_close(
            perspective_divide(ClipPosition(near_clip)).unwrap().0.z,
            0.0,
        );
        assert_close(perspective_divide(ClipPosition(far_clip)).unwrap().0.z, 1.0);
    }

    #[test]
    fn tiny_positive_near_and_scaled_homogeneous_positions_still_divide() {
        let near = 0.5 * f32::EPSILON;
        let projection = perspective_lh_zo(1.0, 1.0, near, 1.0).unwrap();
        let near_ndc = perspective_divide(ClipPosition(
            projection.transform_point(Vec3::new(0.0, 0.0, near)),
        ))
        .unwrap();
        assert_close(near_ndc.0.z, 0.0);

        let homogeneous = Vec4::new(0.25, -0.5, 0.75, 1.0);
        let scale = f32::MIN_POSITIVE;
        let scaled = homogeneous * scale;
        let expected = perspective_divide(ClipPosition(homogeneous)).unwrap();
        let actual = perspective_divide(ClipPosition(scaled)).unwrap();
        assert_vec3_close(actual.0, expected.0);
    }

    #[test]
    fn points_ahead_and_behind_have_opposite_clip_w_before_divide() {
        let projection = perspective_lh_zo(1.0, 1.0, 0.1, 10.0).unwrap();
        let ahead = projection.transform_point(Vec3::new(0.0, 0.0, 2.0));
        let behind = projection.transform_point(Vec3::new(0.0, 0.0, -2.0));
        assert!(ahead.w > 0.0);
        assert!(behind.w < 0.0);
        assert_eq!(ahead.w, 2.0);
        assert_eq!(behind.w, -2.0);
    }

    #[test]
    fn perspective_shrinks_xy_with_distance_and_aspect_only_changes_x_scale() {
        let square = perspective_lh_zo(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 10.0).unwrap();
        let wide = perspective_lh_zo(std::f32::consts::FRAC_PI_2, 2.0, 0.1, 10.0).unwrap();
        let near = perspective_divide(ClipPosition(
            square.transform_point(Vec3::new(0.5, 0.5, 1.0)),
        ))
        .unwrap();
        let far = perspective_divide(ClipPosition(
            square.transform_point(Vec3::new(0.5, 0.5, 2.0)),
        ))
        .unwrap();
        let wide_near =
            perspective_divide(ClipPosition(wide.transform_point(Vec3::new(0.5, 0.5, 1.0))))
                .unwrap();
        assert_close(far.0.x, near.0.x * 0.5);
        assert_close(far.0.y, near.0.y * 0.5);
        assert_close(wide_near.0.x, near.0.x * 0.5);
        assert_close(wide_near.0.y, near.0.y);
    }

    #[test]
    fn viewport_maps_all_ndc_corners_and_flips_y() {
        let top_left = viewport(NdcPosition(Vec3::new(-1.0, 1.0, 0.25)), 640.0, 360.0).unwrap();
        let bottom_right = viewport(NdcPosition(Vec3::new(1.0, -1.0, 0.75)), 640.0, 360.0).unwrap();
        assert_eq!(
            top_left,
            ViewportPosition {
                x: 0.0,
                y: 0.0,
                z_ndc: 0.25
            }
        );
        assert_eq!(
            bottom_right,
            ViewportPosition {
                x: 640.0,
                y: 360.0,
                z_ndc: 0.75
            }
        );
    }

    #[test]
    fn invalid_camera_projection_divide_and_viewport_inputs_are_explicit_errors() {
        assert_eq!(
            look_at_lh(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::Z, Vec3::Y),
            Err(CameraError::NonFiniteLookAt)
        );
        assert_eq!(
            look_at_lh(Vec3::ZERO, Vec3::ZERO, Vec3::Y),
            Err(CameraError::EyeTargetTooClose)
        );
        assert_eq!(
            look_at_lh(
                Vec3::ZERO,
                Vec3::new(0.5 * crate::math::NORMALIZE_EPSILON, 0.0, 0.0),
                Vec3::Y,
            ),
            Err(CameraError::EyeTargetTooClose)
        );
        for world_up in [Vec3::ZERO, Vec3::new(0.0, f32::INFINITY, 0.0)] {
            assert_eq!(
                look_at_lh(Vec3::ZERO, Vec3::Z, world_up),
                Err(CameraError::InvalidWorldUp)
            );
        }
        for fov in [0.0, std::f32::consts::PI, f32::NAN] {
            assert_eq!(
                perspective_lh_zo(fov, 1.0, 0.1, 10.0),
                Err(CameraError::InvalidFovY)
            );
        }
        for aspect in [0.0, -1.0, f32::INFINITY] {
            assert_eq!(
                perspective_lh_zo(1.0, aspect, 0.1, 10.0),
                Err(CameraError::InvalidAspect)
            );
        }
        assert_eq!(
            perspective_lh_zo(f32::from_bits(1), 1.0, 0.1, 10.0),
            Err(CameraError::InvalidFovY)
        );
        assert_eq!(
            perspective_lh_zo(1.0, f32::from_bits(1), 0.1, 10.0),
            Err(CameraError::InvalidAspect)
        );
        for (near, far) in [(0.0, 1.0), (1.0, 1.0), (2.0, 1.0), (f32::NAN, 1.0)] {
            assert_eq!(
                perspective_lh_zo(1.0, 1.0, near, far),
                Err(CameraError::InvalidDepthRange)
            );
        }
        assert_eq!(
            perspective_lh_zo(1.0, 1.0, f32::MAX * 0.75, f32::MAX),
            Err(CameraError::InvalidDepthRange)
        );
        for clip in [
            Vec4::ZERO,
            Vec4::new(f32::NAN, 0.0, 0.0, 1.0),
            Vec4::new(f32::MAX, 0.0, 0.0, 2.0 * f32::EPSILON),
        ] {
            assert_eq!(
                perspective_divide(ClipPosition(clip)),
                Err(CameraError::InvalidClipPosition)
            );
        }
        assert_eq!(
            viewport(NdcPosition(Vec3::ZERO), 0.0, 1.0),
            Err(CameraError::InvalidViewport)
        );
        assert_eq!(
            viewport(NdcPosition(Vec3::new(f32::NAN, 0.0, 0.0)), 1.0, 1.0),
            Err(CameraError::InvalidClipPosition)
        );
        assert_eq!(
            viewport(NdcPosition(Vec3::new(2.0, 0.0, 0.0)), f32::MAX, 1.0),
            Err(CameraError::InvalidViewport)
        );

        let errors = [
            CameraError::NonFiniteLookAt,
            CameraError::EyeTargetTooClose,
            CameraError::InvalidWorldUp,
            CameraError::NonFiniteViewMatrix,
            CameraError::InvalidFovY,
            CameraError::InvalidAspect,
            CameraError::InvalidDepthRange,
            CameraError::InvalidClipPosition,
            CameraError::InvalidViewport,
        ];
        assert!(
            errors
                .into_iter()
                .all(|error| !error.to_string().is_empty())
        );
    }
}
