//! 21장의 외부 모델 경계: 제한된 OBJ baseline과 glTF 좌표 변환 adapter.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::math::{Mat4, Vec2, Vec3, Vec4};
use crate::mesh::{Mesh, MeshValidationError, Vertex};

pub const MAX_OBJ_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_OBJ_POSITIONS: usize = 262_144;
pub const MAX_OBJ_TEXCOORDS: usize = 262_144;
pub const MAX_OBJ_NORMALS: usize = 262_144;
pub const MAX_OBJ_FACES: usize = 262_144;
pub const MAX_OBJ_INTERNAL_VERTICES: usize = 262_144;
pub const MAX_OBJ_TRIANGLES: usize = 262_144;
pub const MAX_OBJ_FACE_VERTICES: usize = 8;

const NORMALIZED_HALF_EXTENT: f32 = 0.75;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObjImportError {
    InputTooLarge { bytes: usize },
    InvalidUtf8,
    LimitExceeded { kind: &'static str, max: usize },
    Parse { line: usize, message: String },
    MissingFaces,
    DegenerateBounds,
    MeshValidation(MeshValidationError),
}

impl Display for ObjImportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputTooLarge { bytes } => write!(
                formatter,
                "OBJ 입력 {bytes} bytes가 최대 {MAX_OBJ_BYTES} bytes를 초과했습니다"
            ),
            Self::InvalidUtf8 => formatter.write_str("OBJ 입력은 유효한 UTF-8이어야 합니다"),
            Self::LimitExceeded { kind, max } => {
                write!(formatter, "OBJ {kind} 수가 최대 {max}개를 초과했습니다")
            }
            Self::Parse { line, message } => {
                write!(formatter, "OBJ {line}행을 해석하지 못했습니다: {message}")
            }
            Self::MissingFaces => {
                formatter.write_str("OBJ에는 렌더링할 face가 하나 이상 필요합니다")
            }
            Self::DegenerateBounds => {
                formatter.write_str("OBJ geometry의 bounding box는 0이 아닌 유한한 크기여야 합니다")
            }
            Self::MeshValidation(error) => write!(formatter, "OBJ mesh 검증 실패: {error}"),
        }
    }
}

impl Error for ObjImportError {}

impl From<MeshValidationError> for ObjImportError {
    fn from(error: MeshValidationError) -> Self {
        Self::MeshValidation(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshBounds {
    pub source_min: Vec3,
    pub source_max: Vec3,
    pub source_center: Vec3,
    pub source_half_extent: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedMesh {
    mesh: Mesh,
    bounds: MeshBounds,
    source_position_count: usize,
    source_face_count: usize,
}

impl ImportedMesh {
    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }

    pub fn into_mesh(self) -> Mesh {
        self.mesh
    }

    pub const fn bounds(&self) -> MeshBounds {
        self.bounds
    }

    pub const fn source_position_count(&self) -> usize {
        self.source_position_count
    }

    pub const fn source_face_count(&self) -> usize {
        self.source_face_count
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ObjVertexKey {
    position: usize,
    texcoord: Option<usize>,
    normal: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
struct ObjFace {
    line: usize,
    vertices: Vec<ObjVertexKey>,
}

fn parse_error(line: usize, message: impl Into<String>) -> ObjImportError {
    ObjImportError::Parse {
        line,
        message: message.into(),
    }
}

fn ensure_count(current_len: usize, max: usize, kind: &'static str) -> Result<(), ObjImportError> {
    if current_len >= max {
        return Err(ObjImportError::LimitExceeded { kind, max });
    }
    Ok(())
}

fn next_triangle_count(current: usize, additional: usize) -> Result<usize, ObjImportError> {
    let next = current + additional;
    if next > MAX_OBJ_TRIANGLES {
        return Err(ObjImportError::LimitExceeded {
            kind: "triangle",
            max: MAX_OBJ_TRIANGLES,
        });
    }
    Ok(next)
}

fn new_internal_index(current_len: usize) -> Result<u32, ObjImportError> {
    ensure_count(current_len, MAX_OBJ_INTERNAL_VERTICES, "internal vertex")?;
    Ok(current_len as u32)
}

fn parse_f32(token: Option<&str>, line: usize, label: &str) -> Result<f32, ObjImportError> {
    let token = token.ok_or_else(|| parse_error(line, format!("{label} 값이 없습니다")))?;
    let value = token
        .parse::<f32>()
        .map_err(|_| parse_error(line, format!("{label} '{token}'은 f32가 아닙니다")))?;
    if !value.is_finite() {
        return Err(parse_error(line, format!("{label}은 유한해야 합니다")));
    }
    Ok(value)
}

fn require_no_more_fields<'a>(
    mut fields: impl Iterator<Item = &'a str>,
    line: usize,
    tag: &str,
) -> Result<(), ObjImportError> {
    if fields.next().is_some() {
        return Err(parse_error(
            line,
            format!("{tag} 필드 수가 지원 범위를 벗어났습니다"),
        ));
    }
    Ok(())
}

fn resolve_index(
    token: &str,
    count: usize,
    line: usize,
    label: &str,
) -> Result<usize, ObjImportError> {
    let raw = token
        .parse::<i64>()
        .map_err(|_| parse_error(line, format!("{label} index '{token}'이 정수가 아닙니다")))?;
    if raw == 0 {
        return Err(parse_error(
            line,
            format!("{label} index 0은 유효하지 않습니다"),
        ));
    }
    let resolved = if raw > 0 {
        usize::try_from(raw - 1).ok()
    } else {
        i64::try_from(count)
            .ok()
            .and_then(|count| count.checked_add(raw))
            .and_then(|index| usize::try_from(index).ok())
    };
    resolved.filter(|&index| index < count).ok_or_else(|| {
        parse_error(
            line,
            format!("{label} index {raw}가 현재 {count}개 범위를 벗어났습니다"),
        )
    })
}

fn parse_face_vertex(
    token: &str,
    counts: [usize; 3],
    line: usize,
) -> Result<ObjVertexKey, ObjImportError> {
    let mut parts = token.split('/');
    let position_token = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| parse_error(line, "face position index가 없습니다"))?;
    let texcoord_token = parts.next();
    let normal_token = parts.next();
    if parts.next().is_some() {
        return Err(parse_error(
            line,
            format!("face token '{token}'의 '/' 수가 너무 많습니다"),
        ));
    }
    if texcoord_token == Some("") && normal_token.is_none() {
        return Err(parse_error(
            line,
            format!("face token '{token}'의 texture index가 비었습니다"),
        ));
    }
    if normal_token == Some("") {
        return Err(parse_error(
            line,
            format!("face token '{token}'의 normal index가 비었습니다"),
        ));
    }
    Ok(ObjVertexKey {
        position: resolve_index(position_token, counts[0], line, "position")?,
        texcoord: texcoord_token
            .filter(|token| !token.is_empty())
            .map(|token| resolve_index(token, counts[1], line, "texture"))
            .transpose()?,
        normal: normal_token
            .filter(|token| !token.is_empty())
            .map(|token| resolve_index(token, counts[2], line, "normal"))
            .transpose()?,
    })
}

fn referenced_bounds(positions: &[Vec3], faces: &[ObjFace]) -> Result<MeshBounds, ObjImportError> {
    let mut referenced = faces
        .iter()
        .flat_map(|face| face.vertices.iter())
        .map(|key| positions[key.position]);
    let first = referenced.next().ok_or(ObjImportError::MissingFaces)?;
    let mut min = first;
    let mut max = first;
    for position in referenced {
        min.x = min.x.min(position.x);
        min.y = min.y.min(position.y);
        min.z = min.z.min(position.z);
        max.x = max.x.max(position.x);
        max.y = max.y.max(position.y);
        max.z = max.z.max(position.z);
    }
    let center = min * 0.5 + max * 0.5;
    let mut half_extent = 0.0_f32;
    for position in faces
        .iter()
        .flat_map(|face| face.vertices.iter())
        .map(|key| positions[key.position])
    {
        let offset = position - center;
        half_extent = half_extent.max(offset.x.abs().max(offset.y.abs()).max(offset.z.abs()));
    }
    if !half_extent.is_finite() || half_extent <= 0.0 {
        return Err(ObjImportError::DegenerateBounds);
    }
    Ok(MeshBounds {
        source_min: min,
        source_max: max,
        source_center: center,
        source_half_extent: half_extent,
    })
}

fn normalized_positions(positions: &[Vec3], bounds: MeshBounds) -> Vec<Vec3> {
    positions
        .iter()
        .map(|&position| {
            (position - bounds.source_center) / bounds.source_half_extent * NORMALIZED_HALF_EXTENT
        })
        .collect()
}

fn face_normal(face: &ObjFace, positions: &[Vec3]) -> Result<Vec3, ObjImportError> {
    let origin = positions[face.vertices[0].position];
    for index in 1..face.vertices.len() - 1 {
        let edge_a = positions[face.vertices[index].position] - origin;
        let edge_b = positions[face.vertices[index + 1].position] - origin;
        if let Some(normal) = edge_a.cross(edge_b).normalized() {
            return Ok(normal);
        }
    }
    Err(parse_error(
        face.line,
        "face가 퇴화해 geometric normal을 만들 수 없습니다",
    ))
}

fn validate_convex_face(face: &ObjFace, positions: &[Vec3]) -> Result<Vec3, ObjImportError> {
    let normal = face_normal(face, positions)?;
    let origin = positions[face.vertices[0].position];
    for key in &face.vertices[1..] {
        let distance = (positions[key.position] - origin).dot(normal).abs();
        if !distance.is_finite() || distance > 1.0e-4 {
            return Err(parse_error(
                face.line,
                "face 정점이 하나의 평면에 있지 않습니다",
            ));
        }
    }
    let mut sign = 0.0_f32;
    for edge_index in 0..face.vertices.len() {
        let next_edge_index = (edge_index + 1) % face.vertices.len();
        let a = positions[face.vertices[edge_index].position];
        let b = positions[face.vertices[next_edge_index].position];
        for vertex_index in 0..face.vertices.len() {
            if vertex_index == edge_index || vertex_index == next_edge_index {
                continue;
            }
            let point = positions[face.vertices[vertex_index].position];
            let turn = (b - a).cross(point - a).dot(normal);
            if !turn.is_finite() || turn.abs() <= 1.0e-6 {
                return Err(parse_error(
                    face.line,
                    "face에 중복되거나 일직선인 꼭짓점이 있습니다",
                ));
            }
            if sign == 0.0 {
                sign = turn.signum();
            } else if turn.signum() != sign {
                return Err(parse_error(
                    face.line,
                    "오목하거나 자기 교차하는 face는 OBJ baseline에서 지원하지 않습니다",
                ));
            }
        }
    }
    Ok(normal)
}

pub fn import_obj(bytes: &[u8]) -> Result<ImportedMesh, ObjImportError> {
    if bytes.len() > MAX_OBJ_BYTES {
        return Err(ObjImportError::InputTooLarge { bytes: bytes.len() });
    }
    let source = std::str::from_utf8(bytes).map_err(|_| ObjImportError::InvalidUtf8)?;
    let mut positions = Vec::new();
    let mut texcoords = Vec::new();
    let mut normals = Vec::new();
    let mut faces = Vec::new();

    for (line_index, raw_line) in source.lines().enumerate() {
        let line_number = line_index + 1;
        let content = raw_line.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let mut fields = content.split_ascii_whitespace();
        let tag = fields
            .next()
            .expect("비어 있지 않은 line에는 tag가 있어야 한다");
        match tag {
            "v" => {
                ensure_count(positions.len(), MAX_OBJ_POSITIONS, "position")?;
                let position = Vec3::new(
                    parse_f32(fields.next(), line_number, "v.x")?,
                    parse_f32(fields.next(), line_number, "v.y")?,
                    parse_f32(fields.next(), line_number, "v.z")?,
                );
                require_no_more_fields(fields, line_number, "v")?;
                positions.push(position);
            }
            "vt" => {
                ensure_count(texcoords.len(), MAX_OBJ_TEXCOORDS, "texture coordinate")?;
                let u = parse_f32(fields.next(), line_number, "vt.u")?;
                let v = parse_f32(fields.next(), line_number, "vt.v")?;
                require_no_more_fields(fields, line_number, "vt")?;
                texcoords.push(Vec2::new(u, 1.0 - v));
            }
            "vn" => {
                ensure_count(normals.len(), MAX_OBJ_NORMALS, "normal")?;
                let normal = Vec3::new(
                    parse_f32(fields.next(), line_number, "vn.x")?,
                    parse_f32(fields.next(), line_number, "vn.y")?,
                    parse_f32(fields.next(), line_number, "vn.z")?,
                );
                normals.push(normal.normalized().ok_or_else(|| {
                    parse_error(line_number, "vn은 0이 아닌 정규화 가능한 벡터여야 합니다")
                })?);
                require_no_more_fields(fields, line_number, "vn")?;
            }
            "f" => {
                ensure_count(faces.len(), MAX_OBJ_FACES, "face")?;
                let mut vertices = Vec::with_capacity(MAX_OBJ_FACE_VERTICES);
                for token in fields {
                    if vertices.len() == MAX_OBJ_FACE_VERTICES {
                        return Err(parse_error(
                            line_number,
                            format!("face는 최대 {MAX_OBJ_FACE_VERTICES}개 정점만 지원합니다"),
                        ));
                    }
                    vertices.push(parse_face_vertex(
                        token,
                        [positions.len(), texcoords.len(), normals.len()],
                        line_number,
                    )?);
                }
                if vertices.len() < 3 {
                    return Err(parse_error(
                        line_number,
                        "face에는 정점이 3개 이상 필요합니다",
                    ));
                }
                faces.push(ObjFace {
                    line: line_number,
                    vertices,
                });
            }
            "o" | "g" | "s" | "usemtl" | "mtllib" => {}
            _ => {
                return Err(parse_error(
                    line_number,
                    format!("지원하지 않는 record '{tag}'"),
                ));
            }
        }
    }

    if faces.is_empty() {
        return Err(ObjImportError::MissingFaces);
    }
    let bounds = referenced_bounds(&positions, &faces)?;
    let positions = normalized_positions(&positions, bounds);
    let mut dedup = HashMap::new();
    let mut internal_keys = Vec::new();
    let mut indices = Vec::new();
    let mut generated_normals = vec![Vec3::ZERO; positions.len()];
    let mut triangle_count = 0_usize;

    for face in &faces {
        validate_convex_face(face, &positions)?;
        let additional_triangles = face.vertices.len() - 2;
        triangle_count = next_triangle_count(triangle_count, additional_triangles)?;
        let mut face_indices = Vec::with_capacity(face.vertices.len());
        for &key in &face.vertices {
            let index = if let Some(&index) = dedup.get(&key) {
                index
            } else {
                let index = new_internal_index(internal_keys.len())?;
                internal_keys.push(key);
                dedup.insert(key, index);
                index
            };
            face_indices.push(index);
        }
        for index in 1..face.vertices.len() - 1 {
            let triangle_keys = [
                face.vertices[0],
                face.vertices[index],
                face.vertices[index + 1],
            ];
            let a = positions[triangle_keys[0].position];
            let b = positions[triangle_keys[1].position];
            let c = positions[triangle_keys[2].position];
            let area_normal = (b - a).cross(c - a);
            for key in triangle_keys {
                generated_normals[key.position] = generated_normals[key.position] + area_normal;
            }
            indices.extend_from_slice(&[
                face_indices[0],
                face_indices[index],
                face_indices[index + 1],
            ]);
        }
    }

    let vertices = internal_keys
        .into_iter()
        .map(|key| {
            let normal = key
                .normal
                .map(|index| normals[index])
                .or_else(|| generated_normals[key.position].normalized())
                .ok_or_else(|| parse_error(0, "누락 normal을 geometry에서 생성하지 못했습니다"))?;
            Ok(Vertex::new(
                positions[key.position],
                normal,
                key.texcoord.map_or(Vec2::ZERO, |index| texcoords[index]),
                Vec4::new(1.0, 1.0, 1.0, 1.0),
            ))
        })
        .collect::<Result<Vec<_>, ObjImportError>>()?;
    let mesh = Mesh::new(vertices, indices)?;
    Ok(ImportedMesh {
        mesh,
        bounds,
        source_position_count: positions.len(),
        source_face_count: faces.len(),
    })
}

pub const fn gltf_position_to_lh(position: Vec3) -> Vec3 {
    Vec3::new(-position.x, position.y, position.z)
}

pub fn gltf_normal_to_lh(normal: Vec3) -> Option<Vec3> {
    gltf_position_to_lh(normal).normalized()
}

pub const fn gltf_tangent_to_lh(tangent: Vec4) -> Vec4 {
    Vec4::new(-tangent.x, tangent.y, tangent.z, -tangent.w)
}

pub const fn gltf_triangle_to_lh(triangle: [u32; 3]) -> [u32; 3] {
    [triangle[0], triangle[2], triangle[1]]
}

pub fn gltf_matrix_to_lh(matrix: Mat4) -> Mat4 {
    let reflection = Mat4::scale(Vec3::new(-1.0, 1.0, 1.0));
    reflection * matrix * reflection
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import(source: &str) -> ImportedMesh {
        import_obj(source.as_bytes()).unwrap()
    }

    #[test]
    fn positive_negative_indices_tuple_seams_and_v_flip_are_preserved() {
        let imported = import(
            "v 0 0 0\n\
             v 1 0 0\n\
             v 1 1 0\n\
             v 0 1 0\n\
             vt 0 0\n\
             vt 1 0\n\
             vt 1 1\n\
             vt 0 1\n\
             vt 0.5 0.5\n\
             vn 0 0 2\n\
             f 1/1/1 2/2/1 3/3/1\n\
             f -4/5/1 -2/3/1 -1/4/1\n",
        );
        assert_eq!(imported.source_position_count(), 4);
        assert_eq!(imported.source_face_count(), 2);
        assert_eq!(imported.mesh().triangle_count(), 2);
        assert_eq!(imported.mesh().vertices().len(), 5);
        assert!(
            imported
                .mesh()
                .vertices()
                .iter()
                .any(|vertex| vertex.uv == Vec2::new(0.0, 1.0))
        );
        assert!(
            imported
                .mesh()
                .vertices()
                .iter()
                .all(|vertex| vertex.normal_object == Vec3::Z)
        );
    }

    #[test]
    fn convex_ngon_fans_and_generates_area_weighted_unit_normals() {
        let imported = import(
            "v 0 0 0\n\
             v 2 0 0\n\
             v 2 1 0\n\
             v 1 2 0\n\
             v 0 1 0\n\
             f 1 2 3 4 5\n",
        );
        assert_eq!(imported.mesh().triangle_count(), 3);
        assert_eq!(imported.mesh().indices(), &[0, 1, 2, 0, 2, 3, 0, 3, 4]);
        for vertex in imported.mesh().vertices() {
            assert!((vertex.normal_object.length() - 1.0).abs() <= 1.0e-6);
            assert_eq!(vertex.normal_object, Vec3::Z);
            assert_eq!(vertex.uv, Vec2::ZERO);
        }
    }

    #[test]
    fn extreme_small_and_large_bounds_normalize_to_a_stable_render_range() {
        for source in [
            "v 0 0 0\nv 1e-38 0 0\nv 0 1e-38 0\nf 1 2 3\n",
            "v -1e38 -1e38 0\nv 1e38 -1e38 0\nv 0 1e38 0\nf 1 2 3\n",
        ] {
            let imported = import(source);
            assert!(imported.bounds().source_half_extent > 0.0);
            for vertex in imported.mesh().vertices() {
                let position = vertex.position_object;
                assert!(position.x.abs() <= NORMALIZED_HALF_EXTENT);
                assert!(position.y.abs() <= NORMALIZED_HALF_EXTENT);
                assert!(position.z.abs() <= NORMALIZED_HALF_EXTENT);
            }
        }
    }

    #[test]
    fn malformed_records_indices_faces_and_bounds_are_explicit_errors() {
        for (source, expected) in [
            ("v 0 0\n", "값이 없습니다"),
            ("v nope 0 0\n", "f32"),
            ("v NaN 0 0\n", "유한"),
            ("v 0 0 0 1\n", "필드 수"),
            ("vt 0\n", "값이 없습니다"),
            ("vt 0 0 0\n", "필드 수"),
            ("vn 0 0 0\n", "0이 아닌"),
            ("vn 0 0 1 1\n", "필드 수"),
            ("f 1 2\n", "position index"),
            ("v 0 0 0\nf 1 1\n", "3개 이상"),
            ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 2 3\n", "index 0"),
            ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf -4 2 3\n", "범위"),
            ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf x 2 3\n", "정수"),
            ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf /1 2 3\n", "position"),
            ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1/ 2 3\n", "비었습니다"),
            ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1// 2 3\n", "비었습니다"),
            ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1/1/1/1 2 3\n", "너무 많"),
            ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1/1 2 3\n", "texture"),
            ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1//1 2 3\n", "normal"),
            ("l 1 2\n", "지원하지 않는"),
            ("v 0 0 0\n", "face"),
            ("v 0 0 0\nv 0 0 0\nv 0 0 0\nf 1 2 3\n", "bounding box"),
            ("v 0 0 0\nv 1 0 0\nv 2 0 0\nf 1 2 3\n", "퇴화"),
            (
                "v 0 0 0\nv 2 0 0\nv 1 1 0\nv 2 2 0\nv 0 2 0\nf 1 2 3 4 5\n",
                "오목",
            ),
            (
                "v 0 1 0\nv 0.9511 0.309 0\nv 0.5878 -0.809 0\nv -0.5878 -0.809 0\nv -0.9511 0.309 0\nf 1 3 5 2 4\n",
                "자기 교차",
            ),
            ("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0.1\nf 1 2 3 4\n", "평면"),
            (
                "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0.5 1 0\nv 0 1 0\nf 1 2 3 4 5\n",
                "일직선",
            ),
        ] {
            let error = import_obj(source.as_bytes()).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn size_face_vertex_and_internal_limits_are_bounded() {
        assert_eq!(
            import_obj(&vec![b' '; MAX_OBJ_BYTES + 1]),
            Err(ObjImportError::InputTooLarge {
                bytes: MAX_OBJ_BYTES + 1
            })
        );
        assert_eq!(import_obj(&[0xff]), Err(ObjImportError::InvalidUtf8));
        assert!(
            ObjImportError::InputTooLarge {
                bytes: MAX_OBJ_BYTES + 1,
            }
            .to_string()
            .contains("초과")
        );
        assert!(ObjImportError::InvalidUtf8.to_string().contains("UTF-8"));
        for (current, max, kind) in [
            (MAX_OBJ_POSITIONS, MAX_OBJ_POSITIONS, "position"),
            (MAX_OBJ_TEXCOORDS, MAX_OBJ_TEXCOORDS, "texture coordinate"),
            (MAX_OBJ_NORMALS, MAX_OBJ_NORMALS, "normal"),
            (MAX_OBJ_FACES, MAX_OBJ_FACES, "face"),
            (
                MAX_OBJ_INTERNAL_VERTICES,
                MAX_OBJ_INTERNAL_VERTICES,
                "internal vertex",
            ),
        ] {
            assert_eq!(
                ensure_count(current, max, kind),
                Err(ObjImportError::LimitExceeded { kind, max })
            );
        }
        let too_many_face_vertices = format!(
            "{}f {}\n",
            (0..9).map(|_| "v 0 0 0\n").collect::<String>(),
            (1..=9)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(
            import_obj(too_many_face_vertices.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("최대 8개")
        );
        assert!(
            ObjImportError::LimitExceeded {
                kind: "triangle",
                max: MAX_OBJ_TRIANGLES,
            }
            .to_string()
            .contains("triangle")
        );
        assert_eq!(
            next_triangle_count(MAX_OBJ_TRIANGLES, 1),
            Err(ObjImportError::LimitExceeded {
                kind: "triangle",
                max: MAX_OBJ_TRIANGLES,
            })
        );
        assert_eq!(next_triangle_count(4, 3), Ok(7));
        assert_eq!(new_internal_index(7), Ok(7));
        assert_eq!(
            new_internal_index(MAX_OBJ_INTERNAL_VERTICES),
            Err(ObjImportError::LimitExceeded {
                kind: "internal vertex",
                max: MAX_OBJ_INTERNAL_VERTICES,
            })
        );
    }

    #[test]
    fn ignored_obj_metadata_does_not_change_geometry() {
        let imported = import(
            "# comment\n\
             mtllib ignored.mtl\n\
             o object\n\
             g group\n\
             s off\n\
             usemtl ignored\n\
             v 0 0 0 # inline\n\
             v 1 0 0\n\
             v 0 1 0\n\
             f 1 2 3\n",
        );
        assert_eq!(imported.mesh().triangle_count(), 1);
    }

    #[test]
    fn mesh_validation_error_is_wrapped_with_obj_context() {
        let error =
            ObjImportError::from(MeshValidationError::IndicesNotTriangles { index_count: 1 });
        assert!(error.to_string().contains("mesh 검증 실패"));
    }

    #[test]
    fn gltf_basis_adapter_preserves_transform_and_orientation_contracts() {
        let point = Vec3::new(2.0, -3.0, 4.0);
        let matrix = Mat4::translation(Vec3::new(5.0, 6.0, 7.0))
            * Mat4::rotation_y(0.4)
            * Mat4::scale(Vec3::new(2.0, 3.0, 4.0));
        let source_world = matrix.transform_point(point);
        let converted_point = gltf_position_to_lh(point);
        let converted_matrix = gltf_matrix_to_lh(matrix);
        let converted_world = converted_matrix.transform_point(converted_point);
        assert!((converted_world.x + source_world.x).abs() <= 1.0e-5);
        assert!((converted_world.y - source_world.y).abs() <= 1.0e-5);
        assert!((converted_world.z - source_world.z).abs() <= 1.0e-5);

        let source = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let converted = source.map(gltf_position_to_lh);
        let triangle = gltf_triangle_to_lh([0, 1, 2]);
        let geometric = (converted[triangle[1] as usize] - converted[triangle[0] as usize])
            .cross(converted[triangle[2] as usize] - converted[triangle[0] as usize])
            .normalized()
            .unwrap();
        let normal = gltf_normal_to_lh(Vec3::Z).unwrap();
        assert!(geometric.dot(normal) > 0.9999);
        assert_eq!(gltf_normal_to_lh(Vec3::ZERO), None);

        let tangent = gltf_tangent_to_lh(Vec4::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(tangent, Vec4::new(-1.0, 0.0, 0.0, -1.0));
        let bitangent = normal.cross(Vec3::new(tangent.x, tangent.y, tangent.z)) * tangent.w;
        let source_bitangent = Vec3::Z.cross(Vec3::X);
        assert_eq!(bitangent, gltf_position_to_lh(source_bitangent));
    }
}
