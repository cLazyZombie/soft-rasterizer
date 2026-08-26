//! 8장의 immutable indexed mesh와 정점 속성 계약.

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::math::{Vec2, Vec3, Vec4};
use crate::transform::Transform;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position_object: Vec3,
    pub normal_object: Vec3,
    pub uv: Vec2,
    /// clipping과 perspective 보간을 포함한 모든 계산에서 사용하는 linear RGBA다.
    pub color: Vec4,
}

impl Vertex {
    pub const fn new(position_object: Vec3, normal_object: Vec3, uv: Vec2, color: Vec4) -> Self {
        Self {
            position_object,
            normal_object,
            uv,
            color,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VertexAttribute {
    Position,
    Normal,
    Uv,
    Color,
}

impl VertexAttribute {
    const fn label(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::Normal => "normal",
            Self::Uv => "uv",
            Self::Color => "color",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeshValidationError {
    IndicesNotTriangles {
        index_count: usize,
    },
    IndexOutOfRange {
        index_offset: usize,
        index: u32,
        vertex_count: usize,
    },
    NonFiniteVertex {
        vertex_index: usize,
        attribute: VertexAttribute,
    },
    ZeroNormal {
        vertex_index: usize,
    },
}

impl Display for MeshValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IndicesNotTriangles { index_count } => write!(
                formatter,
                "mesh index 수 {index_count}은 삼각형을 이루는 3의 배수여야 합니다"
            ),
            Self::IndexOutOfRange {
                index_offset,
                index,
                vertex_count,
            } => write!(
                formatter,
                "mesh index[{index_offset}]={index}가 정점 수 {vertex_count}의 범위를 벗어났습니다"
            ),
            Self::NonFiniteVertex {
                vertex_index,
                attribute,
            } => write!(
                formatter,
                "mesh vertex[{vertex_index}]의 {} 속성은 유한해야 합니다",
                attribute.label()
            ),
            Self::ZeroNormal { vertex_index } => write!(
                formatter,
                "mesh vertex[{vertex_index}]의 normal은 안정적으로 정규화할 수 있어야 합니다"
            ),
        }
    }
}

impl Error for MeshValidationError {}

/// 생성 시 검증을 끝낸 뒤 geometry를 바꾸지 않는 indexed mesh다.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl Mesh {
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Result<Self, MeshValidationError> {
        if !indices.len().is_multiple_of(3) {
            return Err(MeshValidationError::IndicesNotTriangles {
                index_count: indices.len(),
            });
        }
        for (vertex_index, vertex) in vertices.iter().enumerate() {
            for (attribute, finite) in [
                (
                    VertexAttribute::Position,
                    vec3_is_finite(vertex.position_object),
                ),
                (
                    VertexAttribute::Normal,
                    vec3_is_finite(vertex.normal_object),
                ),
                (VertexAttribute::Uv, vec2_is_finite(vertex.uv)),
                (VertexAttribute::Color, vec4_is_finite(vertex.color)),
            ] {
                if !finite {
                    return Err(MeshValidationError::NonFiniteVertex {
                        vertex_index,
                        attribute,
                    });
                }
            }
            if vertex.normal_object.normalized().is_none() {
                return Err(MeshValidationError::ZeroNormal { vertex_index });
            }
        }
        for (index_offset, &index) in indices.iter().enumerate() {
            if index as usize >= vertices.len() {
                return Err(MeshValidationError::IndexOutOfRange {
                    index_offset,
                    index,
                    vertex_count: vertices.len(),
                });
            }
        }
        Ok(Self { vertices, indices })
    }

    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn triangles(&self) -> impl ExactSizeIterator<Item = [usize; 3]> + '_ {
        self.indices.chunks_exact(3).map(|triangle| {
            [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ]
        })
    }

    pub const fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeshId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaterialId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawItem {
    pub mesh_id: MeshId,
    pub material_id: MaterialId,
    pub model: Transform,
}

impl DrawItem {
    pub const fn new(mesh_id: MeshId, material_id: MaterialId, model: Transform) -> Self {
        Self {
            mesh_id,
            material_id,
            model,
        }
    }
}

/// clipping에서 정점 전체를 같은 `t`로 보간하는 vertex-stage 출력이다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipVertex {
    pub clip_pos: crate::transform::ClipPosition,
    pub world_pos: Vec3,
    pub normal_world: Vec3,
    pub uv: Vec2,
    pub color: Vec4,
}

impl ClipVertex {
    /// 동차 clipping 교점에서 모든 정점 속성을 같은 `t`로 보간한다.
    pub fn lerp(self, rhs: Self, t: f32) -> Self {
        Self {
            clip_pos: crate::transform::ClipPosition(
                self.clip_pos.0 + (rhs.clip_pos.0 - self.clip_pos.0) * t,
            ),
            world_pos: self.world_pos + (rhs.world_pos - self.world_pos) * t,
            normal_world: self.normal_world + (rhs.normal_world - self.normal_world) * t,
            uv: self.uv + (rhs.uv - self.uv) * t,
            color: self.color + (rhs.color - self.color) * t,
        }
    }
}

pub fn unit_cube_mesh() -> Mesh {
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    let faces = [
        (
            Vec3::new(0.0, 0.0, -1.0),
            [
                Vec3::new(-0.5, -0.5, -0.5),
                Vec3::new(-0.5, 0.5, -0.5),
                Vec3::new(0.5, 0.5, -0.5),
                Vec3::new(0.5, -0.5, -0.5),
            ],
            Vec4::new(1.0, 0.35, 0.25, 1.0),
        ),
        (
            Vec3::Z,
            [
                Vec3::new(-0.5, -0.5, 0.5),
                Vec3::new(0.5, -0.5, 0.5),
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(-0.5, 0.5, 0.5),
            ],
            Vec4::new(0.25, 0.75, 1.0, 1.0),
        ),
        (
            Vec3::new(-1.0, 0.0, 0.0),
            [
                Vec3::new(-0.5, -0.5, 0.5),
                Vec3::new(-0.5, 0.5, 0.5),
                Vec3::new(-0.5, 0.5, -0.5),
                Vec3::new(-0.5, -0.5, -0.5),
            ],
            Vec4::new(0.55, 1.0, 0.45, 1.0),
        ),
        (
            Vec3::X,
            [
                Vec3::new(0.5, -0.5, -0.5),
                Vec3::new(0.5, 0.5, -0.5),
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(0.5, -0.5, 0.5),
            ],
            Vec4::new(1.0, 0.75, 0.25, 1.0),
        ),
        (
            Vec3::new(0.0, -1.0, 0.0),
            [
                Vec3::new(-0.5, -0.5, 0.5),
                Vec3::new(-0.5, -0.5, -0.5),
                Vec3::new(0.5, -0.5, -0.5),
                Vec3::new(0.5, -0.5, 0.5),
            ],
            Vec4::new(0.75, 0.45, 1.0, 1.0),
        ),
        (
            Vec3::Y,
            [
                Vec3::new(-0.5, 0.5, -0.5),
                Vec3::new(-0.5, 0.5, 0.5),
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(0.5, 0.5, -0.5),
            ],
            Vec4::new(1.0, 0.45, 0.75, 1.0),
        ),
    ];
    let uvs = [
        Vec2::new(0.0, 1.0),
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
    ];
    for (normal, positions, color) in faces {
        let base = vertices.len() as u32;
        vertices.extend(
            positions
                .into_iter()
                .zip(uvs)
                .map(|(position, uv)| Vertex::new(position, normal, uv, color)),
        );
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(vertices, indices).expect("내장 cube mesh는 정적 validation 계약을 만족해야 한다")
}

const fn vec2_is_finite(value: Vec2) -> bool {
    value.x.is_finite() && value.y.is_finite()
}

const fn vec3_is_finite(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

const fn vec4_is_finite(value: Vec4) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite() && value.w.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{look_at_lh, perspective_divide, perspective_lh_zo, viewport};
    use crate::transform::{ObjectPosition, TransformPipeline};

    fn sample_vertex() -> Vertex {
        Vertex::new(
            Vec3::ZERO,
            Vec3::Z,
            Vec2::ZERO,
            Vec4::new(1.0, 1.0, 1.0, 1.0),
        )
    }

    #[test]
    fn cube_has_face_vertices_indices_unit_normals_uvs_and_outward_winding() {
        let cube = unit_cube_mesh();
        assert_eq!(cube.vertices().len(), 24);
        assert_eq!(cube.indices().len(), 36);
        assert_eq!(cube.triangle_count(), 12);
        assert_eq!(cube.triangles().len(), 12);
        for face in cube.vertices().chunks_exact(4) {
            assert!(
                face.iter()
                    .all(|vertex| vertex.normal_object == face[0].normal_object)
            );
            assert_eq!(face[0].normal_object.length(), 1.0);
            assert_eq!(
                face.iter().map(|vertex| vertex.uv).collect::<Vec<_>>(),
                [
                    Vec2::new(0.0, 1.0),
                    Vec2::new(0.0, 0.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(1.0, 1.0),
                ]
            );
        }
        for triangle in cube.triangles() {
            let [a, b, c] = triangle.map(|index| cube.vertices()[index]);
            let geometric_normal = (b.position_object - a.position_object)
                .cross(c.position_object - a.position_object)
                .normalized()
                .expect("cube triangle should not be degenerate");
            assert!(geometric_normal.dot(a.normal_object) > 0.0);
            assert_eq!(a.normal_object, b.normal_object);
            assert_eq!(a.normal_object, c.normal_object);
        }
        assert_eq!(cube.vertices()[0].normal_object, Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn camera_side_face_is_positive_winding_after_y_down_viewport_mapping() {
        let cube = unit_cube_mesh();
        let view = look_at_lh(Vec3::new(0.0, 0.0, -3.0), Vec3::ZERO, Vec3::Y).unwrap();
        let projection = perspective_lh_zo(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 10.0).unwrap();
        let pipeline = TransformPipeline::new(Transform::IDENTITY.model_matrix(), view, projection);
        let triangle = cube.triangles().next().unwrap();
        let screen = triangle.map(|index| {
            let trace = pipeline.trace(ObjectPosition(cube.vertices()[index].position_object));
            viewport(perspective_divide(trace.clip_pos).unwrap(), 64.0, 64.0).unwrap()
        });
        let orient2d = (screen[1].x - screen[0].x) * (screen[2].y - screen[0].y)
            - (screen[1].y - screen[0].y) * (screen[2].x - screen[0].x);
        assert!(orient2d > 0.0);
        assert_eq!(
            cube.vertices()[triangle[0]].normal_object,
            Vec3::new(0.0, 0.0, -1.0)
        );
    }

    #[test]
    fn validation_rejects_non_triangle_indices_and_out_of_range_indices() {
        let non_triangles = Mesh::new(vec![sample_vertex()], vec![0]).unwrap_err();
        assert_eq!(
            non_triangles,
            MeshValidationError::IndicesNotTriangles { index_count: 1 }
        );
        assert!(non_triangles.to_string().contains("3의 배수"));
        let error = Mesh::new(vec![sample_vertex()], vec![0, 1, 0]).unwrap_err();
        assert_eq!(
            error,
            MeshValidationError::IndexOutOfRange {
                index_offset: 1,
                index: 1,
                vertex_count: 1,
            }
        );
        assert!(error.to_string().contains("범위를 벗어났습니다"));
    }

    #[test]
    fn validation_rejects_every_non_finite_attribute_and_zero_normal() {
        for attribute in [
            VertexAttribute::Position,
            VertexAttribute::Normal,
            VertexAttribute::Uv,
            VertexAttribute::Color,
        ] {
            let mut vertex = sample_vertex();
            match attribute {
                VertexAttribute::Position => vertex.position_object.x = f32::NAN,
                VertexAttribute::Normal => vertex.normal_object.y = f32::INFINITY,
                VertexAttribute::Uv => vertex.uv.x = f32::NEG_INFINITY,
                VertexAttribute::Color => vertex.color.w = f32::NAN,
            }
            let error = Mesh::new(vec![vertex], vec![]).unwrap_err();
            assert_eq!(
                error,
                MeshValidationError::NonFiniteVertex {
                    vertex_index: 0,
                    attribute,
                }
            );
            assert!(error.to_string().contains(attribute.label()));
        }
        let mut vertex = sample_vertex();
        vertex.normal_object = Vec3::ZERO;
        let error = Mesh::new(vec![vertex], vec![]).unwrap_err();
        assert_eq!(error, MeshValidationError::ZeroNormal { vertex_index: 0 });
        assert!(error.to_string().contains("정규화"));
    }

    #[test]
    fn empty_and_degenerate_meshes_remain_valid_inputs() {
        let empty = Mesh::new(vec![], vec![]).expect("empty mesh is a valid no-op");
        assert!(empty.vertices().is_empty());
        assert_eq!(empty.triangle_count(), 0);
        let degenerate = Mesh::new(vec![sample_vertex()], vec![0, 0, 0])
            .expect("degenerate rejection belongs to chapter 9");
        assert_eq!(degenerate.triangles().collect::<Vec<_>>(), [[0, 0, 0]]);
    }

    #[test]
    fn draw_item_keeps_geometry_material_and_model_references_separate() {
        let item = DrawItem::new(MeshId(3), MaterialId(7), Transform::IDENTITY);
        assert_eq!(item.mesh_id, MeshId(3));
        assert_eq!(item.material_id, MaterialId(7));
        assert_eq!(item.model, Transform::IDENTITY);
    }
}
