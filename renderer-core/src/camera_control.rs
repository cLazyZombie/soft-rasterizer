//! 장치 이벤트와 분리된 Orbit/Fly 카메라 상태와 프레임 단위 갱신.

use crate::math::Vec3;
use std::error::Error;
use std::fmt::{Display, Formatter};

pub const ORBIT_ROTATE_RADIANS_PER_PIXEL: f32 = 0.005;
pub const ORBIT_ZOOM_PER_WHEEL_UNIT: f32 = 0.001;
pub const ORBIT_MIN_RADIUS: f32 = 0.5;
pub const ORBIT_MAX_RADIUS: f32 = 20.0;
pub const FLY_SPEED_UNITS_PER_SECOND: f32 = 3.0;
pub const PITCH_LIMIT_RADIANS: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraControlInput {
    pub move_right: f32,
    pub move_up: f32,
    pub move_forward: f32,
    pub pointer_dx: f32,
    pub pointer_dy: f32,
    pub wheel_delta: f32,
    pub dragging: bool,
}

impl CameraControlInput {
    fn is_valid(self) -> bool {
        let movement_axes_are_valid = [self.move_right, self.move_up, self.move_forward]
            .into_iter()
            .all(|axis| axis.is_finite() && (-1.0..=1.0).contains(&axis));
        [self.pointer_dx, self.pointer_dy, self.wheel_delta]
            .into_iter()
            .all(f32::is_finite)
            && movement_axes_are_valid
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraControlError {
    InvalidPosition,
    InvalidRadius,
    InvalidAngles,
    UnrepresentablePose,
    InvalidInput,
    InvalidDeltaTime,
}

impl Display for CameraControlError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPosition => "카메라 위치와 target은 유한해야 합니다",
            Self::InvalidRadius => "orbit radius는 허용 범위 안의 유한한 값이어야 합니다",
            Self::InvalidAngles => "카메라 yaw/pitch는 유한하고 pitch 제한 안이어야 합니다",
            Self::UnrepresentablePose => {
                "카메라 eye와 target은 f32에서 서로 구분되는 유효한 pose여야 합니다"
            }
            Self::InvalidInput => {
                "카메라 입력의 이동 축은 -1..1이고 pointer/wheel delta는 유한해야 합니다"
            }
            Self::InvalidDeltaTime => "카메라 dt는 유한한 음이 아닌 값이어야 합니다",
        })
    }
}

impl Error for CameraControlError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraMode {
    Orbit,
    Fly,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub eye: Vec3,
    pub target: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
}

fn spherical_forward(yaw: f32, pitch: f32) -> Vec3 {
    let cos_pitch = pitch.cos();
    Vec3::new(cos_pitch * yaw.sin(), pitch.sin(), cos_pitch * yaw.cos())
}

fn camera_basis(yaw: f32, pitch: f32) -> (Vec3, Vec3, Vec3) {
    let forward = spherical_forward(yaw, pitch);
    let right = Vec3::Y
        .cross(forward)
        .normalized()
        .expect("pitch 제한은 카메라 right 기저를 0이 아니게 보장한다");
    let up = forward.cross(right);
    (forward, right, up)
}

fn yaw_pitch_from_forward(forward: Vec3) -> (f32, f32) {
    let forward = forward
        .normalized()
        .expect("카메라 pose forward는 0이 아니어야 한다");
    (
        forward.x.atan2(forward.z),
        forward
            .y
            .asin()
            .clamp(-PITCH_LIMIT_RADIANS, PITCH_LIMIT_RADIANS),
    )
}

fn pose_is_representable(pose: CameraPose) -> bool {
    let finite = [
        pose.eye.x,
        pose.eye.y,
        pose.eye.z,
        pose.target.x,
        pose.target.y,
        pose.target.z,
    ]
    .into_iter()
    .all(f32::is_finite);
    let Some(actual_forward) = (pose.target - pose.eye).normalized() else {
        return false;
    };
    finite && (actual_forward - pose.forward).length() <= 1.0e-4
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitController {
    target: Vec3,
    radius: f32,
    yaw: f32,
    pitch: f32,
}

impl OrbitController {
    const fn new_unchecked(target: Vec3, radius: f32, yaw: f32, pitch: f32) -> Self {
        Self {
            target,
            radius,
            yaw,
            pitch,
        }
    }

    pub fn new(
        target: Vec3,
        radius: f32,
        yaw: f32,
        pitch: f32,
    ) -> Result<Self, CameraControlError> {
        if ![target.x, target.y, target.z]
            .into_iter()
            .all(f32::is_finite)
        {
            return Err(CameraControlError::InvalidPosition);
        }
        if !radius.is_finite() || !(ORBIT_MIN_RADIUS..=ORBIT_MAX_RADIUS).contains(&radius) {
            return Err(CameraControlError::InvalidRadius);
        }
        if !yaw.is_finite()
            || !pitch.is_finite()
            || !(-PITCH_LIMIT_RADIANS..=PITCH_LIMIT_RADIANS).contains(&pitch)
        {
            return Err(CameraControlError::InvalidAngles);
        }
        let controller =
            Self::new_unchecked(target, radius, yaw.rem_euclid(std::f32::consts::TAU), pitch);
        if !pose_is_representable(controller.pose()) {
            return Err(CameraControlError::UnrepresentablePose);
        }
        Ok(controller)
    }

    fn from_pose(pose: CameraPose, radius: f32) -> Self {
        let (yaw, pitch) = yaw_pitch_from_forward(pose.forward);
        Self::new_unchecked(pose.eye + pose.forward * radius, radius, yaw, pitch)
    }

    pub fn update(&mut self, input: CameraControlInput) -> Result<(), CameraControlError> {
        if !input.is_valid() {
            return Err(CameraControlError::InvalidInput);
        }
        let mut yaw = self.yaw;
        let mut pitch = self.pitch;
        if input.dragging {
            yaw = (yaw + input.pointer_dx * ORBIT_ROTATE_RADIANS_PER_PIXEL)
                .rem_euclid(std::f32::consts::TAU);
            pitch = (pitch + input.pointer_dy * ORBIT_ROTATE_RADIANS_PER_PIXEL)
                .clamp(-PITCH_LIMIT_RADIANS, PITCH_LIMIT_RADIANS);
        }
        let radius = (self.radius * (input.wheel_delta * ORBIT_ZOOM_PER_WHEEL_UNIT).exp())
            .clamp(ORBIT_MIN_RADIUS, ORBIT_MAX_RADIUS);
        let next = Self::new_unchecked(self.target, radius, yaw, pitch);
        if !pose_is_representable(next.pose()) {
            return Err(CameraControlError::UnrepresentablePose);
        }
        *self = next;
        Ok(())
    }

    pub fn pose(self) -> CameraPose {
        let (forward, right, up) = camera_basis(self.yaw, self.pitch);
        CameraPose {
            eye: self.target - forward * self.radius,
            target: self.target,
            forward,
            right,
            up,
        }
    }

    pub const fn radius(self) -> f32 {
        self.radius
    }

    pub const fn yaw(self) -> f32 {
        self.yaw
    }

    pub const fn pitch(self) -> f32 {
        self.pitch
    }
}

impl Default for OrbitController {
    fn default() -> Self {
        Self::new_unchecked(Vec3::ZERO, 3.0, 0.0, 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlyController {
    position: Vec3,
    yaw: f32,
    pitch: f32,
}

impl FlyController {
    const fn new_unchecked(position: Vec3, yaw: f32, pitch: f32) -> Self {
        Self {
            position,
            yaw,
            pitch,
        }
    }

    pub fn new(position: Vec3, yaw: f32, pitch: f32) -> Result<Self, CameraControlError> {
        if ![position.x, position.y, position.z]
            .into_iter()
            .all(f32::is_finite)
        {
            return Err(CameraControlError::InvalidPosition);
        }
        if !yaw.is_finite()
            || !pitch.is_finite()
            || !(-PITCH_LIMIT_RADIANS..=PITCH_LIMIT_RADIANS).contains(&pitch)
        {
            return Err(CameraControlError::InvalidAngles);
        }
        let controller =
            Self::new_unchecked(position, yaw.rem_euclid(std::f32::consts::TAU), pitch);
        if !pose_is_representable(controller.pose()) {
            return Err(CameraControlError::UnrepresentablePose);
        }
        Ok(controller)
    }

    fn from_pose(pose: CameraPose) -> Self {
        let (yaw, pitch) = yaw_pitch_from_forward(pose.forward);
        Self::new_unchecked(pose.eye, yaw, pitch)
    }

    pub fn update(
        &mut self,
        dt_seconds: f32,
        input: CameraControlInput,
    ) -> Result<(), CameraControlError> {
        if !dt_seconds.is_finite() || dt_seconds < 0.0 {
            return Err(CameraControlError::InvalidDeltaTime);
        }
        if !input.is_valid() {
            return Err(CameraControlError::InvalidInput);
        }
        let mut yaw = self.yaw;
        let mut pitch = self.pitch;
        if input.dragging {
            yaw = (yaw + input.pointer_dx * ORBIT_ROTATE_RADIANS_PER_PIXEL)
                .rem_euclid(std::f32::consts::TAU);
            pitch = (pitch + input.pointer_dy * ORBIT_ROTATE_RADIANS_PER_PIXEL)
                .clamp(-PITCH_LIMIT_RADIANS, PITCH_LIMIT_RADIANS);
        }
        let (forward, right, up) = camera_basis(yaw, pitch);
        let direction =
            right * input.move_right + up * input.move_up + forward * input.move_forward;
        let position = direction.normalized().map_or(self.position, |direction| {
            self.position + direction * (FLY_SPEED_UNITS_PER_SECOND * dt_seconds)
        });
        if ![position.x, position.y, position.z]
            .into_iter()
            .all(f32::is_finite)
        {
            return Err(CameraControlError::InvalidDeltaTime);
        }
        let next = Self::new_unchecked(position, yaw, pitch);
        if !pose_is_representable(next.pose()) {
            return Err(CameraControlError::UnrepresentablePose);
        }
        *self = next;
        Ok(())
    }

    pub fn pose(self) -> CameraPose {
        let (forward, right, up) = camera_basis(self.yaw, self.pitch);
        CameraPose {
            eye: self.position,
            target: self.position + forward,
            forward,
            right,
            up,
        }
    }

    pub const fn yaw(self) -> f32 {
        self.yaw
    }

    pub const fn pitch(self) -> f32 {
        self.pitch
    }
}

impl Default for FlyController {
    fn default() -> Self {
        Self::new_unchecked(Vec3::new(0.0, 0.0, -3.0), 0.0, 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraController {
    mode: CameraMode,
    orbit: OrbitController,
    fly: FlyController,
}

impl CameraController {
    pub fn set_mode(&mut self, mode: CameraMode) -> Result<(), CameraControlError> {
        if self.mode == mode {
            return Ok(());
        }
        let pose = self.pose();
        let (orbit, fly) = match mode {
            CameraMode::Orbit => {
                let orbit = OrbitController::from_pose(pose, self.orbit.radius());
                (orbit, self.fly)
            }
            CameraMode::Fly => {
                let fly = FlyController::from_pose(pose);
                (self.orbit, fly)
            }
        };
        let next_pose = match mode {
            CameraMode::Orbit => orbit.pose(),
            CameraMode::Fly => fly.pose(),
        };
        if !pose_is_representable(next_pose) {
            return Err(CameraControlError::UnrepresentablePose);
        }
        self.orbit = orbit;
        self.fly = fly;
        self.mode = mode;
        Ok(())
    }

    pub fn update(
        &mut self,
        dt_seconds: f32,
        input: CameraControlInput,
    ) -> Result<(), CameraControlError> {
        match self.mode {
            CameraMode::Orbit => self.orbit.update(input),
            CameraMode::Fly => self.fly.update(dt_seconds, input),
        }
    }

    pub fn pose(self) -> CameraPose {
        match self.mode {
            CameraMode::Orbit => self.orbit.pose(),
            CameraMode::Fly => self.fly.pose(),
        }
    }

    pub const fn mode(self) -> CameraMode {
        self.mode
    }

    pub const fn yaw(self) -> f32 {
        match self.mode {
            CameraMode::Orbit => self.orbit.yaw(),
            CameraMode::Fly => self.fly.yaw(),
        }
    }

    pub const fn pitch(self) -> f32 {
        match self.mode {
            CameraMode::Orbit => self.orbit.pitch(),
            CameraMode::Fly => self.fly.pitch(),
        }
    }

    pub const fn orbit_radius(self) -> f32 {
        self.orbit.radius()
    }
}

impl Default for CameraController {
    fn default() -> Self {
        Self {
            mode: CameraMode::Orbit,
            orbit: OrbitController::default(),
            fly: FlyController::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length() <= 1.0e-5,
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn orbit_zero_angles_face_positive_z_and_positive_dx_turns_toward_positive_x() {
        let mut orbit = OrbitController::default();
        let initial = orbit.pose();
        assert_eq!(initial.forward, Vec3::Z);
        assert_eq!(initial.eye, Vec3::new(0.0, 0.0, -3.0));
        assert_eq!(initial.target, Vec3::ZERO);
        assert_eq!(initial.right, Vec3::X);
        assert_eq!(initial.up, Vec3::Y);

        orbit
            .update(CameraControlInput {
                pointer_dx: 20.0,
                dragging: true,
                ..CameraControlInput::default()
            })
            .unwrap();
        assert!(orbit.pose().forward.x > 0.0);
        assert!(orbit.yaw() > 0.0);
    }

    #[test]
    fn orbit_clamps_pitch_and_radius_without_a_singular_basis() {
        let mut orbit = OrbitController::default();
        orbit
            .update(CameraControlInput {
                pointer_dy: f32::MAX,
                wheel_delta: f32::MAX,
                dragging: true,
                ..CameraControlInput::default()
            })
            .unwrap();
        let far = orbit.pose();
        assert_eq!(orbit.pitch(), PITCH_LIMIT_RADIANS);
        assert_eq!(orbit.radius(), ORBIT_MAX_RADIUS);
        assert!((far.right.length() - 1.0).abs() <= 1.0e-5);

        orbit
            .update(CameraControlInput {
                pointer_dy: -f32::MAX,
                wheel_delta: -f32::MAX,
                dragging: true,
                ..CameraControlInput::default()
            })
            .unwrap();
        assert_eq!(orbit.pitch(), -PITCH_LIMIT_RADIANS);
        assert_eq!(orbit.radius(), ORBIT_MIN_RADIUS);
    }

    #[test]
    fn fly_forward_right_and_diagonal_have_normalized_dt_scaled_speed() {
        let mut forward = FlyController::default();
        forward
            .update(
                1.0,
                CameraControlInput {
                    move_forward: 1.0,
                    ..CameraControlInput::default()
                },
            )
            .unwrap();
        assert_vec3_close(forward.pose().eye, Vec3::ZERO);

        let mut right = FlyController::default();
        right
            .update(
                1.0,
                CameraControlInput {
                    move_right: 1.0,
                    ..CameraControlInput::default()
                },
            )
            .unwrap();
        assert_vec3_close(right.pose().eye, Vec3::new(3.0, 0.0, -3.0));

        let mut diagonal = FlyController::default();
        diagonal
            .update(
                1.0,
                CameraControlInput {
                    move_right: 1.0,
                    move_forward: 1.0,
                    ..CameraControlInput::default()
                },
            )
            .unwrap();
        assert!(((diagonal.pose().eye - Vec3::new(0.0, 0.0, -3.0)).length() - 3.0).abs() <= 1.0e-5);
    }

    #[test]
    fn fly_same_total_time_is_independent_of_frame_partition() {
        let input = CameraControlInput {
            move_forward: 1.0,
            ..CameraControlInput::default()
        };
        let mut sixty = FlyController::default();
        let mut thirty = FlyController::default();
        for _ in 0..60 {
            sixty.update(1.0 / 60.0, input).unwrap();
        }
        for _ in 0..30 {
            thirty.update(1.0 / 30.0, input).unwrap();
        }
        assert_vec3_close(sixty.pose().eye, thirty.pose().eye);
    }

    #[test]
    fn controller_mode_switch_preserves_pose_and_updates_only_active_mode() {
        let mut controller = CameraController::default();
        let orbit_pose = controller.pose();
        controller.set_mode(CameraMode::Fly).unwrap();
        assert_vec3_close(controller.pose().eye, orbit_pose.eye);
        assert_vec3_close(controller.pose().forward, orbit_pose.forward);
        controller
            .update(
                0.0,
                CameraControlInput {
                    pointer_dx: 20.0,
                    pointer_dy: -10.0,
                    dragging: true,
                    ..CameraControlInput::default()
                },
            )
            .unwrap();
        assert!((controller.yaw() - 0.1).abs() <= 1.0e-5);
        assert!((controller.pitch() + 0.05).abs() <= 1.0e-5);
        assert_vec3_close(controller.pose().eye, orbit_pose.eye);
        controller
            .update(
                0.5,
                CameraControlInput {
                    move_up: 1.0,
                    ..CameraControlInput::default()
                },
            )
            .unwrap();
        assert!(controller.pose().eye.y > orbit_pose.eye.y);
        controller.set_mode(CameraMode::Orbit).unwrap();
        let switched = controller.pose();
        assert_eq!(controller.mode(), CameraMode::Orbit);
        assert_vec3_close(switched.eye, controller.pose().eye);
        assert!((controller.orbit_radius() - 3.0).abs() <= 1.0e-5);
        controller.set_mode(CameraMode::Orbit).unwrap();
    }

    #[test]
    fn public_constructors_reject_invalid_positions_radii_and_angles() {
        assert_eq!(
            OrbitController::new(Vec3::new(f32::NAN, 0.0, 0.0), 3.0, 0.0, 0.0),
            Err(CameraControlError::InvalidPosition)
        );
        assert_eq!(
            OrbitController::new(Vec3::ZERO, f32::NAN, 0.0, 0.0),
            Err(CameraControlError::InvalidRadius)
        );
        assert_eq!(
            OrbitController::new(Vec3::ZERO, ORBIT_MIN_RADIUS - 0.1, 0.0, 0.0),
            Err(CameraControlError::InvalidRadius)
        );
        assert_eq!(
            OrbitController::new(Vec3::ZERO, 3.0, f32::INFINITY, 0.0),
            Err(CameraControlError::InvalidAngles)
        );
        assert_eq!(
            OrbitController::new(Vec3::ZERO, 3.0, 0.0, PITCH_LIMIT_RADIANS + 0.1),
            Err(CameraControlError::InvalidAngles)
        );
        assert_eq!(
            FlyController::new(Vec3::new(0.0, f32::INFINITY, 0.0), 0.0, 0.0),
            Err(CameraControlError::InvalidPosition)
        );
        assert_eq!(
            FlyController::new(Vec3::ZERO, 0.0, f32::NAN),
            Err(CameraControlError::InvalidAngles)
        );
        assert_eq!(
            OrbitController::new(Vec3::new(f32::MAX, f32::MAX, f32::MAX), 3.0, 0.0, 0.0),
            Err(CameraControlError::UnrepresentablePose)
        );
        assert_eq!(
            FlyController::new(Vec3::new(f32::MAX, f32::MAX, f32::MAX), 0.0, 0.0),
            Err(CameraControlError::UnrepresentablePose)
        );
        assert_eq!(
            OrbitController::new(Vec3::ZERO, ORBIT_MAX_RADIUS, std::f32::consts::TAU, 0.0)
                .unwrap()
                .yaw(),
            0.0
        );
        assert_eq!(
            FlyController::new(Vec3::ZERO, std::f32::consts::TAU, PITCH_LIMIT_RADIANS)
                .unwrap()
                .yaw(),
            0.0
        );
    }

    #[test]
    fn invalid_updates_are_rejected_without_mutating_controller_state() {
        let mut orbit = OrbitController::default();
        let initial_orbit = orbit;
        assert_eq!(
            orbit.update(CameraControlInput {
                pointer_dx: f32::NAN,
                dragging: true,
                ..CameraControlInput::default()
            }),
            Err(CameraControlError::InvalidInput)
        );
        assert_eq!(orbit, initial_orbit);

        let mut fly = FlyController::default();
        let initial_fly = fly;
        assert_eq!(
            fly.update(-0.1, CameraControlInput::default()),
            Err(CameraControlError::InvalidDeltaTime)
        );
        assert_eq!(fly, initial_fly);
        assert_eq!(
            fly.update(
                0.25,
                CameraControlInput {
                    move_forward: f32::MAX,
                    pointer_dx: 20.0,
                    dragging: true,
                    ..CameraControlInput::default()
                }
            ),
            Err(CameraControlError::InvalidInput)
        );
        assert_eq!(fly, initial_fly);

        let mut partially_quantized =
            FlyController::new(Vec3::new(16_777_216.0, 0.0, 0.0), 0.0, 0.0).unwrap();
        let initial_partially_quantized = partially_quantized;
        assert_eq!(
            partially_quantized.update(
                0.0,
                CameraControlInput {
                    pointer_dx: std::f32::consts::FRAC_PI_4 / ORBIT_ROTATE_RADIANS_PER_PIXEL,
                    dragging: true,
                    ..CameraControlInput::default()
                }
            ),
            Err(CameraControlError::UnrepresentablePose)
        );
        assert_eq!(partially_quantized, initial_partially_quantized);
        assert_eq!(
            fly.update(f32::NAN, CameraControlInput::default()),
            Err(CameraControlError::InvalidDeltaTime)
        );
        assert_eq!(fly, initial_fly);
        assert_eq!(
            fly.update(
                0.0,
                CameraControlInput {
                    wheel_delta: f32::INFINITY,
                    ..CameraControlInput::default()
                }
            ),
            Err(CameraControlError::InvalidInput)
        );
        assert_eq!(fly, initial_fly);
        assert_eq!(
            fly.update(
                f32::MAX,
                CameraControlInput {
                    move_forward: 1.0,
                    ..CameraControlInput::default()
                }
            ),
            Err(CameraControlError::InvalidDeltaTime)
        );
        assert_eq!(fly, initial_fly);
    }

    #[test]
    fn pose_precision_failures_are_atomic_for_rotation_and_mode_switches() {
        let mut orbit =
            OrbitController::new(Vec3::new(16_777_216.0, 0.0, 0.0), 0.5, 0.0, 0.0).unwrap();
        let initial_orbit = orbit;
        assert_eq!(
            orbit.update(CameraControlInput {
                pointer_dx: std::f32::consts::FRAC_PI_2 / ORBIT_ROTATE_RADIANS_PER_PIXEL,
                dragging: true,
                ..CameraControlInput::default()
            }),
            Err(CameraControlError::UnrepresentablePose)
        );
        assert_eq!(orbit, initial_orbit);

        let mut fly = FlyController::new(Vec3::new(16_777_216.0, 0.0, 0.0), 0.0, 0.0).unwrap();
        let initial_fly = fly;
        assert_eq!(
            fly.update(
                0.0,
                CameraControlInput {
                    pointer_dx: std::f32::consts::FRAC_PI_2 / ORBIT_ROTATE_RADIANS_PER_PIXEL,
                    dragging: true,
                    ..CameraControlInput::default()
                }
            ),
            Err(CameraControlError::UnrepresentablePose)
        );
        assert_eq!(fly, initial_fly);

        let mut controller = CameraController {
            mode: CameraMode::Orbit,
            orbit: OrbitController::new(
                Vec3::new(16_777_220.0, 0.0, 0.0),
                3.0,
                std::f32::consts::FRAC_PI_2,
                0.0,
            )
            .unwrap(),
            fly: FlyController::default(),
        };
        let initial_controller = controller;
        assert_eq!(
            controller.set_mode(CameraMode::Fly),
            Err(CameraControlError::UnrepresentablePose)
        );
        assert_eq!(controller, initial_controller);
    }

    #[test]
    fn camera_control_errors_have_actionable_messages() {
        for (error, expected) in [
            (CameraControlError::InvalidPosition, "위치"),
            (CameraControlError::InvalidRadius, "radius"),
            (CameraControlError::InvalidAngles, "yaw/pitch"),
            (CameraControlError::UnrepresentablePose, "f32"),
            (CameraControlError::InvalidInput, "입력"),
            (CameraControlError::InvalidDeltaTime, "dt"),
        ] {
            assert!(error.to_string().contains(expected));
        }
    }
}
