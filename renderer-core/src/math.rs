//! 렌더링 좌표 규약을 드러내는 최소 벡터와 4x4 행렬 타입.

use std::ops::{Add, Div, Mul, Sub};

/// 이 길이 이하의 벡터는 방향을 안정적으로 정할 수 없는 것으로 본다.
pub const NORMALIZE_EPSILON: f32 = 1.0e-6;

fn inverse_length(length_squared: f32) -> Option<f32> {
    if length_squared.is_finite() && length_squared > NORMALIZE_EPSILON * NORMALIZE_EPSILON {
        Some(length_squared.sqrt().recip())
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y
    }

    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    pub fn normalized(self) -> Option<Self> {
        inverse_length(self.length_squared()).map(|scale| self * scale)
    }
}

impl Add for Vec2 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;

    fn mul(self, scalar: f32) -> Self::Output {
        Self::new(self.x * scalar, self.y * scalar)
    }
}

impl Mul<Vec2> for f32 {
    type Output = Vec2;

    fn mul(self, vector: Vec2) -> Self::Output {
        vector * self
    }
}

impl Div<f32> for Vec2 {
    type Output = Self;

    fn div(self, scalar: f32) -> Self::Output {
        Self::new(self.x / scalar, self.y / scalar)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    pub fn normalized(self) -> Option<Self> {
        inverse_length(self.length_squared()).map(|scale| self * scale)
    }
}

impl Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;

    fn mul(self, scalar: f32) -> Self::Output {
        Self::new(self.x * scalar, self.y * scalar, self.z * scalar)
    }
}

impl Mul<Vec3> for f32 {
    type Output = Vec3;

    fn mul(self, vector: Vec3) -> Self::Output {
        vector * self
    }
}

impl Div<f32> for Vec3 {
    type Output = Self;

    fn div(self, scalar: f32) -> Self::Output {
        Self::new(self.x / scalar, self.y / scalar, self.z / scalar)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub const fn point(position: Vec3) -> Self {
        Self::new(position.x, position.y, position.z, 1.0)
    }

    pub const fn direction(direction: Vec3) -> Self {
        Self::new(direction.x, direction.y, direction.z, 0.0)
    }

    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z + self.w * rhs.w
    }

    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    pub fn normalized(self) -> Option<Self> {
        inverse_length(self.length_squared()).map(|scale| self * scale)
    }
}

impl Add for Vec4 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(
            self.x + rhs.x,
            self.y + rhs.y,
            self.z + rhs.z,
            self.w + rhs.w,
        )
    }
}

impl Sub for Vec4 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(
            self.x - rhs.x,
            self.y - rhs.y,
            self.z - rhs.z,
            self.w - rhs.w,
        )
    }
}

impl Mul<f32> for Vec4 {
    type Output = Self;

    fn mul(self, scalar: f32) -> Self::Output {
        Self::new(
            self.x * scalar,
            self.y * scalar,
            self.z * scalar,
            self.w * scalar,
        )
    }
}

impl Mul<Vec4> for f32 {
    type Output = Vec4;

    fn mul(self, vector: Vec4) -> Self::Output {
        vector * self
    }
}

impl Div<f32> for Vec4 {
    type Output = Self;

    fn div(self, scalar: f32) -> Self::Output {
        Self::new(
            self.x / scalar,
            self.y / scalar,
            self.z / scalar,
            self.w / scalar,
        )
    }
}

/// 논리적 행/열 접근과 열벡터 곱을 제공하는 4x4 행렬이다.
///
/// 저장 순서는 private이며 외부 코드는 [`Mat4::get`]으로만 성분을 읽는다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    rows: [[f32; 4]; 4],
}

impl Mat4 {
    pub const fn from_rows(rows: [[f32; 4]; 4]) -> Self {
        Self { rows }
    }

    pub const fn identity() -> Self {
        Self::from_rows([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    pub const fn translation(offset: Vec3) -> Self {
        Self::from_rows([
            [1.0, 0.0, 0.0, offset.x],
            [0.0, 1.0, 0.0, offset.y],
            [0.0, 0.0, 1.0, offset.z],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    pub const fn scale(factors: Vec3) -> Self {
        Self::from_rows([
            [factors.x, 0.0, 0.0, 0.0],
            [0.0, factors.y, 0.0, 0.0],
            [0.0, 0.0, factors.z, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    pub fn rotation_x(angle_radians: f32) -> Self {
        let (sine, cosine) = angle_radians.sin_cos();
        Self::from_rows([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, cosine, -sine, 0.0],
            [0.0, sine, cosine, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    pub fn rotation_y(angle_radians: f32) -> Self {
        let (sine, cosine) = angle_radians.sin_cos();
        Self::from_rows([
            [cosine, 0.0, sine, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [-sine, 0.0, cosine, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    pub fn rotation_z(angle_radians: f32) -> Self {
        let (sine, cosine) = angle_radians.sin_cos();
        Self::from_rows([
            [cosine, -sine, 0.0, 0.0],
            [sine, cosine, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    pub const fn get(&self, row: usize, column: usize) -> f32 {
        self.rows[row][column]
    }

    pub fn transform_point(self, point: Vec3) -> Vec4 {
        self * Vec4::point(point)
    }

    pub fn transform_direction(self, direction: Vec3) -> Vec4 {
        self * Vec4::direction(direction)
    }
}

impl Mul<Vec4> for Mat4 {
    type Output = Vec4;

    fn mul(self, vector: Vec4) -> Self::Output {
        let component = |row| {
            self.get(row, 0) * vector.x
                + self.get(row, 1) * vector.y
                + self.get(row, 2) * vector.z
                + self.get(row, 3) * vector.w
        };
        Vec4::new(component(0), component(1), component(2), component(3))
    }
}

impl Mul for Mat4 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        let mut rows = [[0.0; 4]; 4];
        for (row_index, row) in rows.iter_mut().enumerate() {
            for (column_index, value) in row.iter_mut().enumerate() {
                *value = (0..4)
                    .map(|inner| self.get(row_index, inner) * rhs.get(inner, column_index))
                    .sum();
            }
        }
        Self::from_rows(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1.0e-5;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_vec4_close(actual: Vec4, expected: Vec4) {
        assert_close(actual.x, expected.x);
        assert_close(actual.y, expected.y);
        assert_close(actual.z, expected.z);
        assert_close(actual.w, expected.w);
    }

    fn assert_vec2_close(actual: Vec2, expected: Vec2) {
        assert_close(actual.x, expected.x);
        assert_close(actual.y, expected.y);
    }

    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert_close(actual.x, expected.x);
        assert_close(actual.y, expected.y);
        assert_close(actual.z, expected.z);
    }

    #[test]
    fn vector_arithmetic_dot_length_and_normalization_are_component_wise() {
        let two = Vec2::new(3.0, 4.0);
        assert_eq!(two + Vec2::new(1.0, 2.0), Vec2::new(4.0, 6.0));
        assert_eq!(two - Vec2::new(1.0, 2.0), Vec2::new(2.0, 2.0));
        assert_eq!(two * 2.0, Vec2::new(6.0, 8.0));
        assert_eq!(2.0 * two, Vec2::new(6.0, 8.0));
        assert_eq!(two / 2.0, Vec2::new(1.5, 2.0));
        assert_eq!(two.dot(Vec2::new(-4.0, 3.0)), 0.0);
        assert_eq!(two.length_squared(), 25.0);
        assert_eq!(two.length(), 5.0);
        assert_vec2_close(
            two.normalized().expect("non-zero Vec2 should normalize"),
            Vec2::new(0.6, 0.8),
        );

        let three = Vec3::new(2.0, 3.0, 6.0);
        assert_eq!(three + Vec3::new(1.0, 2.0, 3.0), Vec3::new(3.0, 5.0, 9.0));
        assert_eq!(three - Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 1.0, 3.0));
        assert_eq!(three * 2.0, Vec3::new(4.0, 6.0, 12.0));
        assert_eq!(2.0 * three, Vec3::new(4.0, 6.0, 12.0));
        assert_eq!(three / 2.0, Vec3::new(1.0, 1.5, 3.0));
        assert_eq!(three.dot(Vec3::new(0.5, 1.0, -2.0)), -8.0);
        assert_eq!(three.length_squared(), 49.0);
        assert_eq!(three.length(), 7.0);
        assert_vec3_close(
            three.normalized().expect("non-zero Vec3 should normalize"),
            Vec3::new(2.0 / 7.0, 3.0 / 7.0, 6.0 / 7.0),
        );

        let four = Vec4::new(1.0, 2.0, 2.0, 4.0);
        assert_eq!(
            four + Vec4::new(4.0, 3.0, 2.0, 1.0),
            Vec4::new(5.0, 5.0, 4.0, 5.0)
        );
        assert_eq!(
            four - Vec4::new(1.0, 1.0, 1.0, 1.0),
            Vec4::new(0.0, 1.0, 1.0, 3.0)
        );
        assert_eq!(four * 2.0, Vec4::new(2.0, 4.0, 4.0, 8.0));
        assert_eq!(2.0 * four, Vec4::new(2.0, 4.0, 4.0, 8.0));
        assert_eq!(four / 2.0, Vec4::new(0.5, 1.0, 1.0, 2.0));
        assert_eq!(four.dot(Vec4::new(1.0, -1.0, 2.0, 0.0)), 3.0);
        assert_eq!(four.length_squared(), 25.0);
        assert_eq!(four.length(), 5.0);
        assert_vec4_close(
            four.normalized().expect("non-zero Vec4 should normalize"),
            Vec4::new(0.2, 0.4, 0.4, 0.8),
        );
    }

    #[test]
    fn cross_uses_the_algebraic_order_and_zero_or_invalid_vectors_do_not_normalize() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::Y.cross(Vec3::X), Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(Vec3::X.dot(Vec3::Y), 0.0);
        assert_eq!(Vec2::ZERO.normalized(), None);
        assert_eq!(Vec3::ZERO.normalized(), None);
        assert_eq!(Vec4::ZERO.normalized(), None);
        assert_eq!(Vec2::new(NORMALIZE_EPSILON, 0.0).normalized(), None);
        assert_eq!(Vec3::new(f32::NAN, 0.0, 0.0).normalized(), None);
        assert_eq!(Vec4::new(f32::INFINITY, 0.0, 0.0, 0.0).normalized(), None);
    }

    #[test]
    fn identity_and_logical_row_column_access_preserve_a_column_vector() {
        let identity = Mat4::identity();
        for row in 0..4 {
            for column in 0..4 {
                let expected = if row == column { 1.0 } else { 0.0 };
                assert_eq!(identity.get(row, column), expected);
            }
        }
        let vector = Vec4::new(2.0, 3.0, 5.0, 7.0);
        assert_eq!(identity * vector, vector);
    }

    #[test]
    fn translation_moves_points_but_not_directions() {
        let translation = Mat4::translation(Vec3::new(5.0, -2.0, 7.0));
        assert_eq!(
            translation.transform_point(Vec3::new(1.0, 2.0, 3.0)),
            Vec4::new(6.0, 0.0, 10.0, 1.0)
        );
        assert_eq!(
            translation.transform_direction(Vec3::new(1.0, 2.0, 3.0)),
            Vec4::new(1.0, 2.0, 3.0, 0.0)
        );
    }

    #[test]
    fn matrix_multiplication_applies_the_rightmost_transform_first() {
        let point = Vec3::new(1.0, 1.0, 1.0);
        let scale = Mat4::scale(Vec3::new(2.0, 3.0, 4.0));
        let translation = Mat4::translation(Vec3::new(10.0, 20.0, 30.0));
        assert_eq!(
            (translation * scale).transform_point(point),
            Vec4::new(12.0, 23.0, 34.0, 1.0)
        );
        assert_eq!(
            (scale * translation).transform_point(point),
            Vec4::new(22.0, 63.0, 124.0, 1.0)
        );
    }

    #[test]
    fn positive_quarter_turns_follow_the_fixed_axis_convention() {
        let quarter_turn = std::f32::consts::FRAC_PI_2;
        assert_vec4_close(
            Mat4::rotation_x(quarter_turn).transform_direction(Vec3::Y),
            Vec4::direction(Vec3::Z),
        );
        assert_vec4_close(
            Mat4::rotation_y(quarter_turn).transform_direction(Vec3::Z),
            Vec4::direction(Vec3::X),
        );
        assert_vec4_close(
            Mat4::rotation_z(quarter_turn).transform_direction(Vec3::X),
            Vec4::direction(Vec3::Y),
        );
    }
}
