//! 9장의 screen-space winding과 backface culling 계약.

use crate::camera::ViewportPosition;

/// 11장의 고정소수점 양자화 전, wireframe 제출 단계에서만 사용하는 면적 기준이다.
pub const WIREFRAME_AREA_EPSILON: f32 = 1.0e-5;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32) -> ViewportPosition {
        ViewportPosition { x, y, z_ndc: 0.5 }
    }

    fn front_triangle() -> [ViewportPosition; 3] {
        [point(1.0, 1.0), point(5.0, 1.0), point(1.0, 4.0)]
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
}
