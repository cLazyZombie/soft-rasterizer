//! 26장의 GLB 2.0 scene/material/animation/skinning importer와 순수 Rust runtime.

use std::error::Error;
use std::fmt::{Display, Formatter};

use gltf::accessor::{Accessor, DataType, Dimensions};
use gltf::animation::Interpolation as GltfInterpolation;
use gltf::animation::Property;
use gltf::animation::util::ReadOutputs;
use gltf::mesh::{Mode, Semantic};

use crate::clip::MAX_CLIPPED_POLYGON_VERTICES;
use crate::color::srgb_encode_rgba;
use crate::import::{
    gltf_matrix_to_lh, gltf_normal_to_lh, gltf_position_to_lh, gltf_triangle_to_lh,
};
use crate::math::{Mat3, Mat4, Vec2, Vec3, Vec4};
use crate::mesh::{Mesh, MeshValidationError, Vertex};
use crate::texture::{
    AddressMode, AlphaMode, FilterMode, Material, SamplerState, ShaderMode, TextureId,
};

pub const MAX_GLB_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_GLB_VERTICES: usize = 262_144;
pub const MAX_GLB_TRIANGLES: usize = 262_144;
pub const MAX_GLB_NODES: usize = 4_096;
pub const MAX_GLB_PRIMITIVES: usize = 4_096;
pub const MAX_GLB_MATERIALS: usize = 512;
pub const MAX_GLB_IMAGES: usize = 64;
pub const MAX_GLB_SKINS: usize = 128;
pub const MAX_GLB_JOINTS_PER_SKIN: usize = 256;
pub const MAX_GLB_JOINT_MATRICES_PER_FRAME: usize = 65_536;
pub const MAX_GLB_ANIMATIONS: usize = 64;
pub const MAX_GLB_ANIMATION_CHANNELS: usize = 4_096;
pub const MAX_GLB_KEYFRAMES: usize = 1_048_576;
pub const MAX_GLB_TRANSPARENT_GENERATED_TRIANGLES: usize = 65_536;
const MAX_GLB_VALIDATION_ERRORS_REPORTED: usize = 8;
const MAX_GLB_VALIDATION_ERROR_CHARS: usize = 256;
const NORMALIZED_HALF_EXTENT: f32 = 0.9;

struct BoundedText {
    value: String,
    remaining_chars: usize,
}

impl std::fmt::Write for BoundedText {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        for character in text.chars().take(self.remaining_chars) {
            self.value.push(character);
            self.remaining_chars -= 1;
        }
        Ok(())
    }
}

fn bounded_format(arguments: std::fmt::Arguments<'_>, max_chars: usize) -> String {
    let mut output = BoundedText {
        value: String::with_capacity(max_chars),
        remaining_chars: max_chars,
    };
    std::fmt::Write::write_fmt(&mut output, arguments)
        .expect("String에 쓰는 formatting은 실패하지 않는다");
    output.value
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GlbImportError {
    InputTooLarge { bytes: usize },
    InvalidHeader(&'static str),
    Parse(String),
    MissingBinaryChunk,
    Unsupported(String),
    LimitExceeded { kind: &'static str, max: usize },
    InvalidData(String),
    MeshValidation(MeshValidationError),
}

impl Display for GlbImportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputTooLarge { bytes } => write!(
                formatter,
                "GLB 입력 {bytes} bytes가 최대 {MAX_GLB_BYTES} bytes를 초과했습니다"
            ),
            Self::InvalidHeader(message) => {
                write!(formatter, "GLB header가 유효하지 않습니다: {message}")
            }
            Self::Parse(message) => write!(
                formatter,
                "gltf crate가 GLB를 해석하지 못했습니다: {message}"
            ),
            Self::MissingBinaryChunk => formatter.write_str("GLB에 내장 BIN chunk가 없습니다"),
            Self::Unsupported(message) => {
                write!(formatter, "지원하지 않는 GLB 기능입니다: {message}")
            }
            Self::LimitExceeded { kind, max } => {
                write!(formatter, "GLB {kind} 수가 최대 {max}개를 초과했습니다")
            }
            Self::InvalidData(message) => {
                write!(formatter, "GLB 데이터가 유효하지 않습니다: {message}")
            }
            Self::MeshValidation(error) => write!(formatter, "GLB mesh 검증 실패: {error}"),
        }
    }
}

impl Error for GlbImportError {}

impl From<MeshValidationError> for GlbImportError {
    fn from(error: MeshValidationError) -> Self {
        Self::MeshValidation(error)
    }
}

fn limit(current: usize, max: usize, kind: &'static str) -> Result<(), GlbImportError> {
    if current > max {
        Err(GlbImportError::LimitExceeded { kind, max })
    } else {
        Ok(())
    }
}

fn advance_limited(
    current: &mut usize,
    added: usize,
    max: usize,
    kind: &'static str,
) -> Result<(), GlbImportError> {
    let next = current
        .checked_add(added)
        .ok_or(GlbImportError::LimitExceeded { kind, max })?;
    limit(next, max, kind)?;
    *current = next;
    Ok(())
}

#[cfg(test)]
fn declared_animation_totals(
    key_counts: impl IntoIterator<Item = usize>,
) -> Result<(usize, usize), GlbImportError> {
    let mut channels = 0;
    let mut keyframes = 0;
    for key_count in key_counts {
        advance_limited(&mut channels, 1, MAX_GLB_ANIMATION_CHANNELS, "channel")?;
        advance_limited(&mut keyframes, key_count, MAX_GLB_KEYFRAMES, "keyframe")?;
    }
    Ok((channels, keyframes))
}

fn checked_subrange_end(
    offset: usize,
    stride: usize,
    count: usize,
    item_size: usize,
    container_len: usize,
    label: &str,
) -> Result<usize, GlbImportError> {
    if count == 0 || item_size == 0 || stride < item_size {
        return Err(GlbImportError::InvalidData(format!(
            "{label} range의 count/item size/stride가 유효하지 않습니다"
        )));
    }
    let end = count
        .checked_sub(1)
        .and_then(|steps| stride.checked_mul(steps))
        .and_then(|span| span.checked_add(item_size))
        .and_then(|span| offset.checked_add(span))
        .ok_or_else(|| GlbImportError::InvalidData(format!("{label} range가 overflow했습니다")))?;
    if end > container_len {
        return Err(GlbImportError::InvalidData(format!(
            "{label} range가 bufferView 범위를 벗어났습니다"
        )));
    }
    Ok(end)
}

fn checked_range_end(offset: usize, length: usize, label: &str) -> Result<usize, GlbImportError> {
    offset
        .checked_add(length)
        .ok_or_else(|| GlbImportError::InvalidData(format!("{label} range가 overflow했습니다")))
}

fn checked_blob_range<'a>(
    blob: &'a [u8],
    offset: usize,
    length: usize,
    label: &str,
) -> Result<&'a [u8], GlbImportError> {
    let end = checked_range_end(offset, length, label)?;
    blob.get(offset..end)
        .ok_or_else(|| GlbImportError::InvalidData(format!("{label}가 BIN 범위를 벗어났습니다")))
}

fn validate_accessor_storage(accessor: &Accessor<'_>, label: &str) -> Result<(), GlbImportError> {
    let item_size = accessor.size();
    if let Some(view) = accessor.view() {
        checked_subrange_end(
            accessor.offset(),
            view.stride().unwrap_or(item_size),
            accessor.count(),
            item_size,
            view.length(),
            label,
        )?;
    }
    Ok(())
}

fn validate_accessor_profile(
    accessor: &Accessor<'_>,
    label: &str,
    profiles: &[(DataType, Dimensions, bool)],
) -> Result<(), GlbImportError> {
    if accessor.count() == 0 {
        return Err(GlbImportError::InvalidData(format!(
            "{label} accessor count는 0보다 커야 합니다"
        )));
    }
    if accessor.sparse().is_some() {
        return Err(GlbImportError::Unsupported(format!(
            "{label} sparse accessor"
        )));
    }
    let profile = (
        accessor.data_type(),
        accessor.dimensions(),
        accessor.normalized(),
    );
    if !profiles.contains(&profile) {
        return Err(GlbImportError::InvalidData(format!(
            "{label} accessor profile {profile:?}을 지원하지 않습니다"
        )));
    }
    validate_accessor_storage(accessor, label)?;
    Ok(())
}

fn validate_primitive_declaration(
    primitive: &gltf::Primitive<'_>,
    skin_index: Option<usize>,
) -> Result<(usize, usize), GlbImportError> {
    if primitive.mode() != Mode::Triangles {
        return Err(GlbImportError::Unsupported(format!(
            "primitive mode {:?}; TRIANGLES만 허용됩니다",
            primitive.mode()
        )));
    }
    if primitive.morph_targets().next().is_some() {
        return Err(GlbImportError::Unsupported("morph target".into()));
    }
    let positions = primitive.get(&Semantic::Positions).ok_or_else(|| {
        GlbImportError::InvalidData("primitive POSITION accessor가 없습니다".into())
    })?;
    validate_accessor_profile(
        &positions,
        "POSITION",
        &[(DataType::F32, Dimensions::Vec3, false)],
    )?;
    let vertex_count = positions.count();
    let mut has_joints = false;
    let mut has_weights = false;
    for (semantic, accessor) in primitive.attributes() {
        let profiles: &[(DataType, Dimensions, bool)] = match semantic {
            Semantic::Positions | Semantic::Normals => &[(DataType::F32, Dimensions::Vec3, false)],
            Semantic::TexCoords(0) => &[
                (DataType::F32, Dimensions::Vec2, false),
                (DataType::U8, Dimensions::Vec2, true),
                (DataType::U16, Dimensions::Vec2, true),
            ],
            Semantic::Colors(0) => &[
                (DataType::F32, Dimensions::Vec3, false),
                (DataType::F32, Dimensions::Vec4, false),
                (DataType::U8, Dimensions::Vec3, true),
                (DataType::U8, Dimensions::Vec4, true),
                (DataType::U16, Dimensions::Vec3, true),
                (DataType::U16, Dimensions::Vec4, true),
            ],
            Semantic::Joints(0) => {
                has_joints = true;
                &[
                    (DataType::U8, Dimensions::Vec4, false),
                    (DataType::U16, Dimensions::Vec4, false),
                ]
            }
            Semantic::Weights(0) => {
                has_weights = true;
                &[
                    (DataType::F32, Dimensions::Vec4, false),
                    (DataType::U8, Dimensions::Vec4, true),
                    (DataType::U16, Dimensions::Vec4, true),
                ]
            }
            other => {
                return Err(GlbImportError::Unsupported(format!(
                    "vertex attribute {other:?}"
                )));
            }
        };
        validate_accessor_profile(&accessor, &format!("{semantic:?}"), profiles)?;
        if accessor.count() != vertex_count {
            return Err(GlbImportError::InvalidData(format!(
                "{semantic:?} 수가 POSITION 수와 다릅니다"
            )));
        }
    }
    if has_joints != has_weights || skin_index.is_some() != (has_joints && has_weights) {
        return Err(GlbImportError::InvalidData(
            "skin node와 JOINTS_0/WEIGHTS_0 존재 여부가 일치해야 합니다".into(),
        ));
    }
    let index_count = if let Some(indices) = primitive.indices() {
        validate_accessor_profile(
            &indices,
            "indices",
            &[
                (DataType::U8, Dimensions::Scalar, false),
                (DataType::U16, Dimensions::Scalar, false),
                (DataType::U32, Dimensions::Scalar, false),
            ],
        )?;
        indices.count()
    } else {
        vertex_count
    };
    if !index_count.is_multiple_of(3) {
        return Err(GlbImportError::InvalidData(
            "TRIANGLES index/vertex 수는 3의 배수여야 합니다".into(),
        ));
    }
    Ok((vertex_count, index_count / 3))
}

fn validate_animation_declarations(document: &gltf::Document) -> Result<(), GlbImportError> {
    let mut channels = 0usize;
    let mut keyframes = 0usize;
    for animation in document.animations() {
        let mut targeted_properties = vec![false; document.nodes().count() * 3];
        for channel in animation.channels() {
            let target = channel.target();
            let (property_index, profiles): (usize, &[(DataType, Dimensions, bool)]) =
                match target.property() {
                    Property::Translation => (0, &[(DataType::F32, Dimensions::Vec3, false)]),
                    Property::Rotation => (
                        1,
                        &[
                            (DataType::F32, Dimensions::Vec4, false),
                            (DataType::I8, Dimensions::Vec4, true),
                            (DataType::U8, Dimensions::Vec4, true),
                            (DataType::I16, Dimensions::Vec4, true),
                            (DataType::U16, Dimensions::Vec4, true),
                        ],
                    ),
                    Property::Scale => (2, &[(DataType::F32, Dimensions::Vec3, false)]),
                    Property::MorphTargetWeights => {
                        return Err(GlbImportError::Unsupported(
                            "animation morph weights".into(),
                        ));
                    }
                };
            let target_index = target.node().index() * 3 + property_index;
            if std::mem::replace(&mut targeted_properties[target_index], true) {
                return Err(GlbImportError::InvalidData(
                    "한 animation에서 같은 node/property를 둘 이상의 channel이 대상으로 삼습니다"
                        .into(),
                ));
            }
            let input = channel.sampler().input();
            validate_accessor_profile(
                &input,
                "animation input",
                &[(DataType::F32, Dimensions::Scalar, false)],
            )?;
            advance_limited(&mut channels, 1, MAX_GLB_ANIMATION_CHANNELS, "channel")?;
            advance_limited(&mut keyframes, input.count(), MAX_GLB_KEYFRAMES, "keyframe")?;
            let output = channel.sampler().output();
            validate_accessor_profile(&output, "animation output", profiles)?;
            let multiplier = if channel.sampler().interpolation() == GltfInterpolation::CubicSpline
            {
                3
            } else {
                1
            };
            let expected =
                input
                    .count()
                    .checked_mul(multiplier)
                    .ok_or(GlbImportError::LimitExceeded {
                        kind: "animation output",
                        max: MAX_GLB_KEYFRAMES * 3,
                    })?;
            if output.count() != expected {
                return Err(GlbImportError::InvalidData(format!(
                    "animation output {}개가 key/interpolation 기대값 {expected}와 다릅니다",
                    output.count()
                )));
            }
        }
    }
    Ok(())
}

fn finite(values: impl IntoIterator<Item = f32>) -> bool {
    values.into_iter().all(f32::is_finite)
}

fn matrix_is_finite(matrix: Mat4) -> bool {
    (0..4).all(|row| (0..4).all(|column| matrix.get(row, column).is_finite()))
}

fn parse_gltf_version(value: &str) -> Option<(u32, u32)> {
    let (major, minor) = value.split_once('.')?;
    if major.is_empty() || minor.is_empty() || minor.contains('.') {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

fn validate_asset_version(version: &str, minimum: Option<&str>) -> Result<(), GlbImportError> {
    if parse_gltf_version(version) != Some((2, 0)) {
        return Err(GlbImportError::Unsupported(format!(
            "asset.version {version}; GLB profile은 glTF 2.0만 지원합니다"
        )));
    }
    if let Some(minimum) = minimum {
        let minimum = parse_gltf_version(minimum).ok_or_else(|| {
            GlbImportError::InvalidData("asset.minVersion 형식은 major.minor여야 합니다".into())
        })?;
        if minimum > (2, 0) {
            return Err(GlbImportError::Unsupported(format!(
                "asset.minVersion {}.{}; runtime 지원 버전은 2.0입니다",
                minimum.0, minimum.1
            )));
        }
    }
    Ok(())
}

fn validate_glb_chunk_layout(bytes: &[u8]) -> Result<usize, GlbImportError> {
    let json_header = checked_blob_range(bytes, 12, 8, "JSON chunk header")?;
    let json_length = u32::from_le_bytes(json_header[..4].try_into().unwrap()) as usize;
    if &json_header[4..] != b"JSON" || !json_length.is_multiple_of(4) {
        return Err(GlbImportError::InvalidHeader(
            "첫 chunk는 4-byte 정렬된 JSON이어야 합니다",
        ));
    }
    let json_end = checked_range_end(20, json_length, "JSON chunk")?;
    checked_blob_range(bytes, 20, json_length, "JSON chunk")?;
    let bin_header = checked_blob_range(bytes, json_end, 8, "BIN chunk header")?;
    let bin_length = u32::from_le_bytes(bin_header[..4].try_into().unwrap()) as usize;
    if &bin_header[4..] != b"BIN\0" || !bin_length.is_multiple_of(4) {
        return Err(GlbImportError::InvalidHeader(
            "두 번째 chunk는 4-byte 정렬된 BIN이어야 합니다",
        ));
    }
    let bin_start = json_end + 8;
    let bin_end = checked_range_end(bin_start, bin_length, "BIN chunk")?;
    checked_blob_range(bytes, bin_start, bin_length, "BIN chunk")?;
    if bin_end != bytes.len() {
        return Err(GlbImportError::InvalidHeader(
            "BIN 뒤에 추가 chunk 또는 trailing bytes가 있습니다",
        ));
    }
    Ok(bin_length)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self::new(0.0, 0.0, 0.0, 1.0);

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub fn normalized(self) -> Option<Self> {
        Vec4::new(self.x, self.y, self.z, self.w)
            .normalized()
            .map(|q| Self::new(q.x, q.y, q.z, q.w))
    }

    pub fn to_matrix(self) -> Mat4 {
        let q = self.normalized().unwrap_or(Self::IDENTITY);
        let (x2, y2, z2) = (q.x + q.x, q.y + q.y, q.z + q.z);
        let (xx, xy, xz) = (q.x * x2, q.x * y2, q.x * z2);
        let (yy, yz, zz) = (q.y * y2, q.y * z2, q.z * z2);
        let (wx, wy, wz) = (q.w * x2, q.w * y2, q.w * z2);
        Mat4::from_rows([
            [1.0 - (yy + zz), xy - wz, xz + wy, 0.0],
            [xy + wz, 1.0 - (xx + zz), yz - wx, 0.0],
            [xz - wy, yz + wx, 1.0 - (xx + yy), 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    pub fn shortest_slerp(self, rhs: Self, t: f32) -> Self {
        let first = self.normalized().unwrap_or(Self::IDENTITY);
        let mut second = rhs.normalized().unwrap_or(Self::IDENTITY);
        let mut dot =
            first.x * second.x + first.y * second.y + first.z * second.z + first.w * second.w;
        if dot < 0.0 {
            second = second * -1.0;
            dot = -dot;
        }
        if dot > 0.9995 {
            return (first * (1.0 - t) + second * t)
                .normalized()
                .unwrap_or(first);
        }
        let angle = dot.clamp(-1.0, 1.0).acos();
        let denominator = angle.sin();
        (first * (((1.0 - t) * angle).sin() / denominator)
            + second * ((t * angle).sin() / denominator))
            .normalized()
            .unwrap_or(first)
    }
}

impl std::ops::Add for Quat {
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

impl std::ops::Mul<f32> for Quat {
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct NodePose {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

impl NodePose {
    fn matrix(self) -> Mat4 {
        Mat4::translation(self.translation) * self.rotation.to_matrix() * Mat4::scale(self.scale)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Node {
    parent: Option<usize>,
    base_pose: NodePose,
    pose: NodePose,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinVertex {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
struct Skin {
    joints: Vec<usize>,
    inverse_bind_matrices: Vec<Mat4>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EncodedGlbImage {
    mime_type: String,
    bytes: Vec<u8>,
}

impl EncodedGlbImage {
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MaterialTemplate {
    material: Material,
    image_index: Option<usize>,
    double_sided: bool,
    sampler_downgraded: bool,
    forced_unlit: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct PrimitiveAsset {
    mesh: Mesh,
    node_index: usize,
    skin_index: Option<usize>,
    material_index: usize,
    skin_vertices: Option<Vec<SkinVertex>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Interpolation {
    Step,
    Linear,
    CubicSpline,
}

#[derive(Clone, Debug, PartialEq)]
enum ChannelValues {
    Translation(Vec<Vec3>),
    Rotation(Vec<Quat>),
    Scale(Vec<Vec3>),
}

#[derive(Clone, Debug, PartialEq)]
struct AnimationChannel {
    node_index: usize,
    interpolation: Interpolation,
    times: Vec<f32>,
    values: ChannelValues,
}

#[derive(Clone, Debug, PartialEq)]
struct AnimationClip {
    name: String,
    duration: f32,
    channels: Vec<AnimationChannel>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlbAsset {
    images: Vec<EncodedGlbImage>,
    materials: Vec<MaterialTemplate>,
    primitives: Vec<PrimitiveAsset>,
    nodes: Vec<Node>,
    skins: Vec<Skin>,
    clips: Vec<AnimationClip>,
    source_vertices: usize,
    source_triangles: usize,
}

impl GlbAsset {
    pub fn images(&self) -> &[EncodedGlbImage] {
        &self.images
    }
    pub fn image_count(&self) -> usize {
        self.images.len()
    }
    pub fn primitive_count(&self) -> usize {
        self.primitives.len()
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn skin_count(&self) -> usize {
        self.skins.len()
    }
    pub fn animation_count(&self) -> usize {
        self.clips.len()
    }
    pub fn source_vertices(&self) -> usize {
        self.source_vertices
    }
    pub fn source_triangles(&self) -> usize {
        self.source_triangles
    }
}

fn matrix_from_columns(columns: [[f32; 4]; 4]) -> Mat4 {
    Mat4::from_rows(std::array::from_fn(|row| {
        std::array::from_fn(|column| columns[column][row])
    }))
}

fn gltf_quat_components_to_lh(value: [f32; 4]) -> Result<Quat, GlbImportError> {
    if !finite(value) {
        return Err(GlbImportError::InvalidData(
            "node/animation quaternion은 유한해야 합니다".into(),
        ));
    }
    Ok(Quat::new(value[0], -value[1], -value[2], value[3]))
}

fn gltf_quat_to_lh(value: [f32; 4]) -> Result<Quat, GlbImportError> {
    gltf_quat_components_to_lh(value)?
        .normalized()
        .ok_or_else(|| {
            GlbImportError::InvalidData(
                "node/animation quaternion은 0이 아닌 길이를 가져야 합니다".into(),
            )
        })
}

fn node_pose(node: gltf::Node<'_>) -> Result<NodePose, GlbImportError> {
    let (translation, rotation, scale) = node.transform().decomposed();
    Ok(NodePose {
        translation: gltf_position_to_lh(Vec3::new(translation[0], translation[1], translation[2])),
        rotation: gltf_quat_to_lh(rotation)?,
        scale: Vec3::new(scale[0], scale[1], scale[2]),
    })
}

fn buffer_data<'a>(buffer: gltf::Buffer<'_>, blob: &'a [u8]) -> Option<&'a [u8]> {
    matches!(buffer.source(), gltf::buffer::Source::Bin).then_some(blob)
}

fn sampler_state(sampler: gltf::texture::Sampler<'_>) -> (SamplerState, bool) {
    use gltf::texture::{MagFilter, MinFilter, WrappingMode};
    let address = |mode| match mode {
        WrappingMode::ClampToEdge => AddressMode::ClampToEdge,
        WrappingMode::MirroredRepeat => AddressMode::MirroredRepeat,
        WrappingMode::Repeat => AddressMode::Repeat,
    };
    let mag = sampler.mag_filter();
    let min = sampler.min_filter();
    let mag_linear = !matches!(mag, Some(MagFilter::Nearest));
    let min_linear = !matches!(
        min,
        Some(MinFilter::Nearest | MinFilter::NearestMipmapNearest | MinFilter::NearestMipmapLinear)
    );
    let filter = if mag_linear || min_linear {
        FilterMode::Bilinear
    } else {
        FilterMode::Nearest
    };
    let downgraded = mag_linear != min_linear
        || matches!(
            min,
            Some(MinFilter::NearestMipmapLinear | MinFilter::LinearMipmapLinear)
        );
    (
        SamplerState {
            address_u: address(sampler.wrap_s()),
            address_v: address(sampler.wrap_t()),
            filter,
        },
        downgraded,
    )
}

fn parse_material(material: gltf::Material<'_>) -> Result<MaterialTemplate, GlbImportError> {
    let pbr = material.pbr_metallic_roughness();
    let factor = pbr.base_color_factor();
    let (image_index, sampler, downgraded) = if let Some(info) = pbr.base_color_texture() {
        if info.tex_coord() != 0 {
            return Err(GlbImportError::Unsupported(format!(
                "material baseColorTexture TEXCOORD_{}",
                info.tex_coord()
            )));
        }
        let texture = info.texture();
        let (sampler, downgraded) = sampler_state(texture.sampler());
        (Some(texture.source().index()), sampler, downgraded)
    } else {
        (None, SamplerState::default(), false)
    };
    let alpha_mode = match material.alpha_mode() {
        gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
        gltf::material::AlphaMode::Mask => AlphaMode::Mask,
        gltf::material::AlphaMode::Blend => AlphaMode::Blend,
    };
    let linear = Vec4::new(
        factor[0],
        factor[1],
        factor[2],
        if alpha_mode == AlphaMode::Opaque {
            1.0
        } else {
            factor[3]
        },
    );
    Ok(MaterialTemplate {
        material: Material {
            base_color: srgb_encode_rgba(linear),
            sampler,
            shader_mode: if material.unlit() {
                ShaderMode::Unlit
            } else {
                ShaderMode::Lambert
            },
            alpha_mode,
            alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
            ..Material::default()
        },
        image_index,
        double_sided: material.double_sided(),
        sampler_downgraded: downgraded,
        forced_unlit: material.unlit(),
    })
}

fn generate_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for triangle in indices.as_chunks::<3>().0.iter() {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let face = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
        if finite([face.x, face.y, face.z]) {
            normals[a] = normals[a] + face;
            normals[b] = normals[b] + face;
            normals[c] = normals[c] + face;
        }
    }
    normals
        .into_iter()
        .map(|normal| normal.normalized().unwrap_or(Vec3::Y))
        .collect()
}

fn parse_primitive(
    primitive: gltf::Primitive<'_>,
    blob: &[u8],
    node_index: usize,
    skin_index: Option<usize>,
    default_material_index: usize,
) -> Result<PrimitiveAsset, GlbImportError> {
    if primitive.mode() != Mode::Triangles {
        return Err(GlbImportError::Unsupported(format!(
            "primitive mode {:?}; TRIANGLES만 허용됩니다",
            primitive.mode()
        )));
    }
    if primitive.morph_targets().next().is_some() {
        return Err(GlbImportError::Unsupported("morph target".into()));
    }
    for (semantic, _) in primitive.attributes() {
        match semantic {
            Semantic::Positions
            | Semantic::Normals
            | Semantic::TexCoords(0)
            | Semantic::Colors(0)
            | Semantic::Joints(0)
            | Semantic::Weights(0) => {}
            other => {
                return Err(GlbImportError::Unsupported(format!(
                    "vertex attribute {other:?}"
                )));
            }
        }
    }
    let reader = primitive.reader(|buffer| buffer_data(buffer, blob));
    let positions = reader
        .read_positions()
        .ok_or_else(|| {
            GlbImportError::InvalidData("primitive POSITION accessor가 없습니다".into())
        })?
        .map(|position| gltf_position_to_lh(Vec3::new(position[0], position[1], position[2])))
        .collect::<Vec<_>>();
    if positions.is_empty() || !positions.iter().all(|p| finite([p.x, p.y, p.z])) {
        return Err(GlbImportError::InvalidData(
            "POSITION은 비어 있지 않은 유한한 값이어야 합니다".into(),
        ));
    }
    let mut indices = reader.read_indices().map_or_else(
        || {
            (0..positions.len())
                .map(|index| u32::try_from(index).unwrap())
                .collect::<Vec<_>>()
        },
        |indices| indices.into_u32().collect::<Vec<_>>(),
    );
    if !indices.len().is_multiple_of(3) {
        return Err(GlbImportError::InvalidData(
            "TRIANGLES index/vertex 수는 3의 배수여야 합니다".into(),
        ));
    }
    if let Some((index_offset, &index)) = indices
        .iter()
        .enumerate()
        .find(|(_, index)| **index as usize >= positions.len())
    {
        return Err(MeshValidationError::IndexOutOfRange {
            index_offset,
            index,
            vertex_count: positions.len(),
        }
        .into());
    }
    for triangle in indices.as_chunks_mut::<3>().0.iter_mut() {
        *triangle = gltf_triangle_to_lh(*triangle);
    }
    let normals = if let Some(normals) = reader.read_normals() {
        let normals = normals
            .map(|normal| gltf_normal_to_lh(Vec3::new(normal[0], normal[1], normal[2])))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                GlbImportError::InvalidData(
                    "NORMAL은 유한하고 0이 아닌 길이를 가져야 합니다".into(),
                )
            })?;
        if normals.len() != positions.len() {
            return Err(GlbImportError::InvalidData(
                "NORMAL 수가 POSITION 수와 다릅니다".into(),
            ));
        }
        normals
    } else {
        generate_normals(&positions, &indices)
    };
    let texcoords = reader
        .read_tex_coords(0)
        .map(|values| {
            values
                .into_f32()
                .map(|uv| Vec2::new(uv[0], uv[1]))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![Vec2::ZERO; positions.len()]);
    let colors = reader
        .read_colors(0)
        .map(|values| {
            values
                .into_rgba_f32()
                .map(|color| Vec4::new(color[0], color[1], color[2], color[3]))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![Vec4::new(1.0, 1.0, 1.0, 1.0); positions.len()]);
    if texcoords.len() != positions.len()
        || colors.len() != positions.len()
        || !texcoords.iter().all(|uv| finite([uv.x, uv.y]))
        || !colors
            .iter()
            .all(|color| finite([color.x, color.y, color.z, color.w]))
    {
        return Err(GlbImportError::InvalidData(
            "TEXCOORD_0/COLOR_0 수와 유한성 계약이 깨졌습니다".into(),
        ));
    }
    let joints = reader
        .read_joints(0)
        .map(|values| values.into_u16().collect::<Vec<_>>());
    let weights = reader
        .read_weights(0)
        .map(|values| values.into_f32().collect::<Vec<_>>());
    let skin_vertices = match (joints, weights) {
        (None, None) => None,
        (Some(joints), Some(weights))
            if joints.len() == positions.len() && weights.len() == positions.len() =>
        {
            let mut result = Vec::with_capacity(positions.len());
            for (joints, mut weights) in joints.into_iter().zip(weights) {
                if !finite(weights) {
                    return Err(GlbImportError::InvalidData(
                        "WEIGHTS_0는 유한해야 합니다".into(),
                    ));
                }
                if weights.iter().any(|&weight| weight < 0.0) {
                    return Err(GlbImportError::InvalidData(
                        "WEIGHTS_0는 음수가 아니어야 합니다".into(),
                    ));
                }
                for first in 0..4 {
                    for second in (first + 1)..4 {
                        if weights[first] > 0.0
                            && weights[second] > 0.0
                            && joints[first] == joints[second]
                        {
                            return Err(GlbImportError::InvalidData(
                                "한 vertex의 non-zero skin influence는 서로 다른 joint여야 합니다"
                                    .into(),
                            ));
                        }
                    }
                }
                let sum = weights.into_iter().sum::<f32>();
                if !sum.is_finite() || sum <= 1.0e-6 {
                    return Err(GlbImportError::InvalidData(
                        "WEIGHTS_0 합은 0보다 커야 합니다".into(),
                    ));
                }
                for weight in &mut weights {
                    *weight /= sum;
                }
                result.push(SkinVertex { joints, weights });
            }
            Some(result)
        }
        _ => {
            return Err(GlbImportError::InvalidData(
                "JOINTS_0와 WEIGHTS_0는 POSITION과 같은 수로 함께 있어야 합니다".into(),
            ));
        }
    };
    if skin_index.is_some() != skin_vertices.is_some() {
        return Err(GlbImportError::InvalidData(
            "skin node와 JOINTS_0/WEIGHTS_0 존재 여부가 일치해야 합니다".into(),
        ));
    }
    let vertices = positions
        .into_iter()
        .zip(normals)
        .zip(texcoords)
        .zip(colors)
        .map(|(((position, normal), uv), color)| Vertex::new(position, normal, uv, color))
        .collect();
    let material_index = primitive
        .material()
        .index()
        .unwrap_or(default_material_index);
    Ok(PrimitiveAsset {
        mesh: Mesh::new(vertices, indices)?,
        node_index,
        skin_index,
        material_index,
        skin_vertices,
    })
}

fn parse_channel(
    channel: gltf::animation::Channel<'_>,
    blob: &[u8],
) -> Result<AnimationChannel, GlbImportError> {
    let interpolation = match channel.sampler().interpolation() {
        GltfInterpolation::Step => Interpolation::Step,
        GltfInterpolation::Linear => Interpolation::Linear,
        GltfInterpolation::CubicSpline => Interpolation::CubicSpline,
    };
    let reader = channel.reader(|buffer| buffer_data(buffer, blob));
    let times = reader
        .read_inputs()
        .ok_or_else(|| {
            GlbImportError::InvalidData("animation input accessor를 읽지 못했습니다".into())
        })?
        .collect::<Vec<_>>();
    if times.is_empty()
        || !finite(times.iter().copied())
        || times.windows(2).any(|pair| pair[0] >= pair[1])
        || times[0] < 0.0
    {
        return Err(GlbImportError::InvalidData(
            "animation key time은 0 이상이고 엄격히 증가하는 유한 값이어야 합니다".into(),
        ));
    }
    let expected = times.len()
        * if interpolation == Interpolation::CubicSpline {
            3
        } else {
            1
        };
    let outputs = reader.read_outputs().ok_or_else(|| {
        GlbImportError::InvalidData("animation output accessor를 읽지 못했습니다".into())
    })?;
    let values = match outputs {
        ReadOutputs::Translations(values) => {
            let values = values
                .map(|v| gltf_position_to_lh(Vec3::new(v[0], v[1], v[2])))
                .collect::<Vec<_>>();
            if !values
                .iter()
                .all(|value| finite([value.x, value.y, value.z]))
            {
                return Err(GlbImportError::InvalidData(
                    "animation translation 값과 tangent는 유한해야 합니다".into(),
                ));
            }
            ChannelValues::Translation(values)
        }
        ReadOutputs::Scales(values) => {
            let values = values
                .map(|v| Vec3::new(v[0], v[1], v[2]))
                .collect::<Vec<_>>();
            if !values
                .iter()
                .all(|value| finite([value.x, value.y, value.z]))
            {
                return Err(GlbImportError::InvalidData(
                    "animation scale 값과 tangent는 유한해야 합니다".into(),
                ));
            }
            ChannelValues::Scale(values)
        }
        ReadOutputs::Rotations(values) => {
            let mut rotations = values
                .into_f32()
                .map(gltf_quat_components_to_lh)
                .collect::<Result<Vec<_>, _>>()?;
            if interpolation == Interpolation::CubicSpline {
                for value in rotations.iter_mut().skip(1).step_by(3) {
                    *value = value.normalized().ok_or_else(|| {
                        GlbImportError::InvalidData(
                            "CUBICSPLINE rotation key quaternion 길이는 0보다 커야 합니다".into(),
                        )
                    })?;
                }
            } else {
                for value in &mut rotations {
                    *value = value.normalized().ok_or_else(|| {
                        GlbImportError::InvalidData(
                            "rotation key quaternion 길이는 0보다 커야 합니다".into(),
                        )
                    })?;
                }
            }
            ChannelValues::Rotation(rotations)
        }
        ReadOutputs::MorphTargetWeights(_) => {
            return Err(GlbImportError::Unsupported(
                "animation morph weights".into(),
            ));
        }
    };
    let actual = match &values {
        ChannelValues::Translation(v) | ChannelValues::Scale(v) => v.len(),
        ChannelValues::Rotation(v) => v.len(),
    };
    if actual != expected {
        return Err(GlbImportError::InvalidData(format!(
            "animation output {actual}개가 key/interpolation 기대값 {expected}와 다릅니다"
        )));
    }
    Ok(AnimationChannel {
        node_index: channel.target().node().index(),
        interpolation,
        times,
        values,
    })
}

pub fn import_glb(bytes: &[u8]) -> Result<GlbAsset, GlbImportError> {
    if bytes.len() > MAX_GLB_BYTES {
        return Err(GlbImportError::InputTooLarge { bytes: bytes.len() });
    }
    if bytes.len() < 12 || &bytes[..4] != b"glTF" {
        return Err(GlbImportError::InvalidHeader("magic은 glTF여야 합니다"));
    }
    if u32::from_le_bytes(bytes[4..8].try_into().unwrap()) != 2 {
        return Err(GlbImportError::InvalidHeader("version은 2여야 합니다"));
    }
    if u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize != bytes.len() {
        return Err(GlbImportError::InvalidHeader(
            "선언 길이와 입력 길이가 다릅니다",
        ));
    }
    let bin_chunk_length = validate_glb_chunk_layout(bytes)?;
    let gltf = gltf::Gltf::from_slice_without_validation(bytes)
        .map_err(|error| GlbImportError::Parse(error.to_string()))?;
    let blob = gltf
        .blob
        .as_deref()
        .ok_or(GlbImportError::MissingBinaryChunk)?;
    let asset = &gltf.document.as_json().asset;
    validate_asset_version(&asset.version, asset.min_version.as_deref())?;
    for required in gltf.document.extensions_required() {
        if required != "KHR_materials_unlit" {
            return Err(GlbImportError::Unsupported(format!(
                "required extension {required}"
            )));
        }
    }
    if gltf.document.buffers().count() != 1 {
        return Err(GlbImportError::Unsupported(
            "GLB profile은 URI 없는 BIN buffer 하나만 지원합니다".into(),
        ));
    }
    for buffer in gltf.document.buffers() {
        if !matches!(buffer.source(), gltf::buffer::Source::Bin) {
            return Err(GlbImportError::Unsupported("외부/data URI buffer".into()));
        }
    }
    {
        use gltf::json::validation::Validate;
        let root = gltf.document.as_json();
        let mut errors = Vec::new();
        let mut error_count = 0usize;
        root.validate(root, gltf::json::Path::new, &mut |path, error| {
            error_count = error_count.saturating_add(1);
            if errors.len() < MAX_GLB_VALIDATION_ERRORS_REPORTED {
                errors.push(bounded_format(
                    format_args!("{}: {error:?}", path()),
                    MAX_GLB_VALIDATION_ERROR_CHARS,
                ));
            }
        });
        if error_count > 0 {
            let mut summary = errors.join(", ");
            if error_count > errors.len() {
                summary.push_str(&format!(", ... ({error_count} document validation errors)"));
            }
            return Err(GlbImportError::Parse(summary));
        }
    }
    for buffer in gltf.document.buffers() {
        if buffer.length() > bin_chunk_length {
            return Err(GlbImportError::InvalidData(format!(
                "BIN chunk {} bytes가 buffer {} 선언 길이 {} bytes보다 짧습니다",
                blob.len(),
                buffer.index(),
                buffer.length()
            )));
        }
        if bin_chunk_length - buffer.length() > 3 {
            return Err(GlbImportError::InvalidData(format!(
                "BIN chunk {} bytes가 buffer {} 선언 길이 {} bytes보다 padding 3 bytes를 초과해 깁니다",
                bin_chunk_length,
                buffer.index(),
                buffer.length()
            )));
        }
        if blob[buffer.length()..bin_chunk_length]
            .iter()
            .any(|&byte| byte != 0)
        {
            return Err(GlbImportError::InvalidData(
                "BIN chunk padding은 0x00이어야 합니다".into(),
            ));
        }
    }
    for view in gltf.document.views() {
        let declared_length = view.buffer().length();
        if view.offset() > declared_length
            || view.length() > declared_length.saturating_sub(view.offset())
        {
            return Err(GlbImportError::InvalidData(format!(
                "bufferView {}가 buffer 선언 범위를 벗어났습니다",
                view.index()
            )));
        }
    }
    limit(gltf.document.nodes().count(), MAX_GLB_NODES, "node")?;
    let material_count = gltf.document.materials().count();
    limit(material_count, MAX_GLB_MATERIALS, "material")?;
    limit(gltf.document.images().count(), MAX_GLB_IMAGES, "image")?;
    limit(gltf.document.skins().count(), MAX_GLB_SKINS, "skin")?;
    let animation_count = gltf.document.animations().count();
    limit(animation_count, MAX_GLB_ANIMATIONS, "animation")?;
    validate_animation_declarations(&gltf.document)?;

    let image_count = gltf.document.images().count();
    let mut image_sources = Vec::with_capacity(image_count);
    let mut encoded_image_bytes = 0usize;
    for image in gltf.document.images() {
        match image.source() {
            gltf::image::Source::View { view, mime_type } => {
                if !matches!(mime_type, "image/png" | "image/jpeg") {
                    return Err(GlbImportError::Unsupported(format!(
                        "image MIME {mime_type}"
                    )));
                }
                advance_limited(
                    &mut encoded_image_bytes,
                    view.length(),
                    MAX_GLB_BYTES,
                    "encoded image byte",
                )?;
                image_sources.push((view.offset(), view.length(), mime_type.to_owned()));
            }
            gltf::image::Source::Uri { .. } => {
                return Err(GlbImportError::Unsupported("외부/data URI image".into()));
            }
        }
    }

    let images = image_sources
        .into_iter()
        .map(|(offset, length, mime_type)| {
            let data = checked_blob_range(blob, offset, length, "image bufferView")?;
            Ok(EncodedGlbImage {
                mime_type,
                bytes: data.to_vec(),
            })
        })
        .collect::<Result<Vec<_>, GlbImportError>>()?;
    let mut materials = gltf
        .document
        .materials()
        .map(parse_material)
        .collect::<Result<Vec<_>, _>>()?;
    let default_material_index = materials.len();
    materials.push(MaterialTemplate {
        material: Material {
            shader_mode: ShaderMode::Lambert,
            ..Material::default()
        },
        image_index: None,
        double_sided: false,
        sampler_downgraded: false,
        forced_unlit: false,
    });
    let mut parents = vec![None; gltf.document.nodes().count()];
    for node in gltf.document.nodes() {
        for child in node.children() {
            if parents[child.index()].replace(node.index()).is_some() {
                return Err(GlbImportError::InvalidData(
                    "node가 둘 이상의 parent를 가집니다".into(),
                ));
            }
        }
    }
    let nodes = gltf
        .document
        .nodes()
        .map(|node| {
            let pose = node_pose(node.clone())?;
            Ok(Node {
                parent: parents[node.index()],
                base_pose: pose,
                pose,
            })
        })
        .collect::<Result<Vec<_>, GlbImportError>>()?;
    let scene = gltf
        .document
        .default_scene()
        .or_else(|| gltf.document.scenes().next())
        .ok_or_else(|| GlbImportError::InvalidData("scene이 없습니다".into()))?;
    let mut reachable = vec![false; nodes.len()];
    let root_count = scene.nodes().count();
    limit(root_count, MAX_GLB_NODES, "scene root")?;
    let mut stack = Vec::with_capacity(root_count);
    stack.extend(scene.nodes().map(|node| node.index()));
    while let Some(index) = stack.pop() {
        if std::mem::replace(&mut reachable[index], true) {
            continue;
        }
        stack.extend(
            gltf.document
                .nodes()
                .nth(index)
                .unwrap()
                .children()
                .map(|child| child.index()),
        );
    }

    let skins = gltf
        .document
        .skins()
        .map(|skin| {
            let joint_count = skin.joints().count();
            limit(joint_count, MAX_GLB_JOINTS_PER_SKIN, "joint/skin")?;
            let mut joints = Vec::with_capacity(joint_count);
            joints.extend(skin.joints().map(|node| node.index()));
            if let Some(accessor) = skin.inverse_bind_matrices() {
                validate_accessor_profile(
                    &accessor,
                    "inverseBindMatrices",
                    &[(DataType::F32, Dimensions::Mat4, false)],
                )?;
                if accessor.count() != joints.len() {
                    return Err(GlbImportError::InvalidData(
                        "inverseBindMatrices 수가 joint 수와 다릅니다".into(),
                    ));
                }
            }
            let reader = skin.reader(|buffer| buffer_data(buffer, blob));
            let inverse_bind_matrices = reader
                .read_inverse_bind_matrices()
                .map(|values| {
                    values
                        .map(|matrix| {
                            if !finite(matrix.into_iter().flatten()) {
                                return Err(GlbImportError::InvalidData(
                                    "inverseBindMatrices 성분은 유한해야 합니다".into(),
                                ));
                            }
                            if matrix[0][3].abs() > 1.0e-6
                                || matrix[1][3].abs() > 1.0e-6
                                || matrix[2][3].abs() > 1.0e-6
                                || (matrix[3][3] - 1.0).abs() > 1.0e-6
                            {
                                return Err(GlbImportError::InvalidData(
                                    "inverseBindMatrices는 affine 행렬이어야 합니다".into(),
                                ));
                            }
                            Ok(gltf_matrix_to_lh(matrix_from_columns(matrix)))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_else(|| vec![Mat4::identity(); joints.len()]);
            Ok(Skin {
                joints,
                inverse_bind_matrices,
            })
        })
        .collect::<Result<Vec<_>, GlbImportError>>()?;

    let mut source_vertices = 0usize;
    let mut source_triangles = 0usize;
    let mut primitive_count = 0usize;
    let mut transparent_generated_triangles = 0usize;
    let mut joint_matrices_per_frame = 0usize;
    for node in gltf.document.nodes().filter(|node| reachable[node.index()]) {
        let Some(mesh) = node.mesh() else {
            continue;
        };
        for primitive in mesh.primitives() {
            let (vertices, triangles) =
                validate_primitive_declaration(&primitive, node.skin().map(|skin| skin.index()))?;
            advance_limited(&mut source_vertices, vertices, MAX_GLB_VERTICES, "vertex")?;
            advance_limited(
                &mut source_triangles,
                triangles,
                MAX_GLB_TRIANGLES,
                "triangle",
            )?;
            advance_limited(&mut primitive_count, 1, MAX_GLB_PRIMITIVES, "primitive")?;
            if let Some(skin) = node.skin() {
                advance_limited(
                    &mut joint_matrices_per_frame,
                    skins[skin.index()].joints.len(),
                    MAX_GLB_JOINT_MATRICES_PER_FRAME,
                    "joint matrix/frame",
                )?;
            }
            let material_index = primitive
                .material()
                .index()
                .unwrap_or(default_material_index);
            if materials[material_index].material.alpha_mode == AlphaMode::Blend {
                let worst_case = triangles
                    .checked_mul(MAX_CLIPPED_POLYGON_VERTICES - 2)
                    .ok_or(GlbImportError::LimitExceeded {
                        kind: "transparent clipped triangle",
                        max: MAX_GLB_TRANSPARENT_GENERATED_TRIANGLES,
                    })?;
                advance_limited(
                    &mut transparent_generated_triangles,
                    worst_case,
                    MAX_GLB_TRANSPARENT_GENERATED_TRIANGLES,
                    "transparent clipped triangle",
                )?;
            }
        }
    }

    let mut primitives = Vec::with_capacity(primitive_count);
    for node in gltf.document.nodes().filter(|node| reachable[node.index()]) {
        let Some(mesh) = node.mesh() else {
            continue;
        };
        for primitive in mesh.primitives() {
            let parsed_primitive = parse_primitive(
                primitive,
                blob,
                node.index(),
                node.skin().map(|skin| skin.index()),
                default_material_index,
            )?;
            primitives.push(parsed_primitive);
            let parsed = primitives.last().unwrap();
            if let (Some(skin_index), Some(skin_vertices)) =
                (parsed.skin_index, parsed.skin_vertices.as_ref())
            {
                let joint_count = skins[skin_index].joints.len();
                if skin_vertices
                    .iter()
                    .flat_map(|vertex| vertex.joints)
                    .any(|joint| joint as usize >= joint_count)
                {
                    return Err(GlbImportError::InvalidData(
                        "JOINTS_0가 skin joint 범위를 벗어났습니다".into(),
                    ));
                }
            }
        }
    }
    if primitives.is_empty() {
        return Err(GlbImportError::InvalidData(
            "선택 scene에 렌더링할 primitive가 없습니다".into(),
        ));
    }

    let clips = gltf
        .document
        .animations()
        .map(|animation| {
            let mut channels = Vec::new();
            let mut duration = 0.0f32;
            for channel in animation.channels() {
                let parsed = parse_channel(channel, blob)?;
                duration = duration.max(*parsed.times.last().unwrap());
                channels.push(parsed);
            }
            Ok(AnimationClip {
                name: animation
                    .name()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Animation {}", animation.index())),
                duration,
                channels,
            })
        })
        .collect::<Result<Vec<_>, GlbImportError>>()?;

    Ok(GlbAsset {
        images,
        materials,
        primitives,
        nodes,
        skins,
        clips,
        source_vertices,
        source_triangles,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlbRuntimeMaterial {
    pub material: Material,
    pub double_sided: bool,
    pub sampler_downgraded: bool,
    pub forced_unlit: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlbRuntimePrimitive {
    asset: PrimitiveAsset,
    evaluated_vertices: Vec<Vertex>,
    position_palette: Vec<Mat4>,
    normal_palette: Vec<Mat3>,
    winding_reversed: bool,
}

impl GlbRuntimePrimitive {
    pub fn mesh(&self) -> &Mesh {
        &self.asset.mesh
    }
    pub fn vertices(&self) -> &[Vertex] {
        &self.evaluated_vertices
    }
    pub fn material_index(&self) -> usize {
        self.asset.material_index
    }
    pub fn is_skinned(&self) -> bool {
        self.asset.skin_index.is_some()
    }
    pub const fn winding_reversed(&self) -> bool {
        self.winding_reversed
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlbSceneStats {
    pub draw_items: usize,
    pub nodes: usize,
    pub materials: usize,
    pub images: usize,
    pub skins: usize,
    pub joints: usize,
    pub animations: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub skinned_vertices: usize,
    pub sampler_downgrades: usize,
    pub animated_nodes: usize,
    pub joint_matrices_per_frame: usize,
}

fn animated_node_count(clip: Option<&AnimationClip>, node_count: usize) -> usize {
    let mut targeted = vec![false; node_count];
    if let Some(clip) = clip {
        for channel in &clip.channels {
            targeted[channel.node_index] = true;
        }
    }
    targeted.into_iter().filter(|&value| value).count()
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlbScene {
    nodes: Vec<Node>,
    skins: Vec<Skin>,
    clips: Vec<AnimationClip>,
    primitives: Vec<GlbRuntimePrimitive>,
    materials: Vec<GlbRuntimeMaterial>,
    global_matrices: Vec<Mat4>,
    node_visit_states: Vec<u8>,
    node_visit_stack: Vec<usize>,
    root_transform: Mat4,
    selected_clip: Option<usize>,
    time_seconds: f32,
    playing: bool,
    looping: bool,
    stats: GlbSceneStats,
}

impl GlbScene {
    pub fn new(asset: GlbAsset, image_texture_ids: &[TextureId]) -> Result<Self, GlbImportError> {
        if image_texture_ids.len() != asset.images.len() {
            return Err(GlbImportError::InvalidData(
                "decode된 image/texture ID 수가 GLB image 수와 다릅니다".into(),
            ));
        }
        let materials = asset
            .materials
            .iter()
            .map(|template| {
                let mut material = template.material;
                material.base_color_texture =
                    template.image_index.map(|index| image_texture_ids[index]);
                GlbRuntimeMaterial {
                    material,
                    double_sided: template.double_sided,
                    sampler_downgraded: template.sampler_downgraded,
                    forced_unlit: template.forced_unlit,
                }
            })
            .collect::<Vec<_>>();
        let primitives = asset
            .primitives
            .into_iter()
            .map(|asset| GlbRuntimePrimitive {
                evaluated_vertices: asset.mesh.vertices().to_vec(),
                position_palette: Vec::new(),
                normal_palette: Vec::new(),
                winding_reversed: false,
                asset,
            })
            .collect::<Vec<_>>();
        let stats = GlbSceneStats {
            draw_items: primitives.len(),
            nodes: asset.nodes.len(),
            materials: materials.len().saturating_sub(1),
            images: asset.images.len(),
            skins: asset.skins.len(),
            joints: asset.skins.iter().map(|skin| skin.joints.len()).sum(),
            animations: asset.clips.len(),
            vertices: asset.source_vertices,
            triangles: asset.source_triangles,
            skinned_vertices: primitives
                .iter()
                .filter_map(|primitive| primitive.asset.skin_vertices.as_ref())
                .map(Vec::len)
                .sum(),
            sampler_downgrades: materials
                .iter()
                .filter(|material| material.sampler_downgraded)
                .count(),
            animated_nodes: animated_node_count(asset.clips.first(), asset.nodes.len()),
            joint_matrices_per_frame: primitives
                .iter()
                .filter_map(|primitive| primitive.asset.skin_index)
                .map(|skin_index| asset.skins[skin_index].joints.len())
                .sum(),
        };
        let has_clips = !asset.clips.is_empty();
        let mut scene = Self {
            global_matrices: vec![Mat4::identity(); asset.nodes.len()],
            node_visit_states: vec![0; asset.nodes.len()],
            node_visit_stack: Vec::with_capacity(asset.nodes.len()),
            nodes: asset.nodes,
            skins: asset.skins,
            clips: asset.clips,
            primitives,
            materials,
            root_transform: Mat4::identity(),
            selected_clip: has_clips.then_some(0),
            time_seconds: 0.0,
            playing: has_clips,
            looping: true,
            stats,
        };
        scene.evaluate_pose()?;
        let (minimum, maximum) = scene.evaluated_bounds()?;
        let center = (minimum + maximum) * 0.5;
        let half = (maximum - minimum) * 0.5;
        let half_extent = half.x.max(half.y).max(half.z);
        if !half_extent.is_finite() || half_extent <= 1.0e-8 {
            return Err(GlbImportError::InvalidData(
                "scene bounding box는 0이 아닌 유한한 크기여야 합니다".into(),
            ));
        }
        scene.root_transform = Mat4::scale(Vec3::new(
            NORMALIZED_HALF_EXTENT / half_extent,
            NORMALIZED_HALF_EXTENT / half_extent,
            NORMALIZED_HALF_EXTENT / half_extent,
        )) * Mat4::translation(center * -1.0);
        scene.evaluate_pose()?;
        Ok(scene)
    }

    pub fn primitives(&self) -> &[GlbRuntimePrimitive] {
        &self.primitives
    }
    pub fn materials(&self) -> &[GlbRuntimeMaterial] {
        &self.materials
    }
    pub const fn stats(&self) -> GlbSceneStats {
        self.stats
    }
    pub fn clip_count(&self) -> usize {
        self.clips.len()
    }
    pub fn clip_name(&self, index: usize) -> Option<&str> {
        self.clips.get(index).map(|clip| clip.name.as_str())
    }
    pub fn selected_clip(&self) -> Option<usize> {
        self.selected_clip
    }
    pub fn selected_clip_duration(&self) -> f32 {
        self.selected_clip
            .and_then(|index| self.clips.get(index))
            .map_or(0.0, |clip| clip.duration)
    }
    pub const fn time_seconds(&self) -> f32 {
        self.time_seconds
    }
    pub const fn playing(&self) -> bool {
        self.playing
    }
    pub const fn looping(&self) -> bool {
        self.looping
    }

    #[cfg(test)]
    pub(crate) fn clear_animations_for_test(&mut self) {
        self.clips.clear();
        self.selected_clip = None;
        self.playing = false;
        self.stats.animations = 0;
        self.stats.animated_nodes = 0;
    }

    #[cfg(test)]
    pub(crate) fn force_update_failure_for_test(&mut self) {
        let joint = self.skins[0].joints[0];
        self.nodes[joint].base_pose.scale = Vec3::ZERO;
    }

    pub fn set_lighting_enabled(&mut self, enabled: bool) {
        for runtime in &mut self.materials {
            runtime.material.shader_mode = if runtime.forced_unlit || !enabled {
                ShaderMode::Unlit
            } else if runtime.material.shader_mode == ShaderMode::Unlit {
                ShaderMode::Lambert
            } else {
                runtime.material.shader_mode
            };
        }
    }

    pub fn set_shader_mode(&mut self, mode: ShaderMode) {
        for runtime in &mut self.materials {
            if !runtime.forced_unlit {
                runtime.material.shader_mode = mode;
            }
        }
    }

    pub fn set_normal_mode(&mut self, mode: crate::texture::NormalMode) {
        for runtime in &mut self.materials {
            runtime.material.normal_mode = mode;
        }
    }

    pub fn set_specular(&mut self, color: Vec3, shininess: f32) {
        for runtime in &mut self.materials {
            runtime.material.specular_color = color;
            runtime.material.shininess = shininess;
        }
    }

    pub fn set_clip(&mut self, index: usize) -> Result<(), GlbImportError> {
        if index >= self.clips.len() {
            return Err(GlbImportError::InvalidData(format!(
                "animation clip index {index}가 범위를 벗어났습니다"
            )));
        }
        let previous = self.animation_control_state();
        self.selected_clip = Some(index);
        self.time_seconds = 0.0;
        self.playing = true;
        self.stats.animated_nodes = animated_node_count(self.clips.get(index), self.nodes.len());
        self.finish_animation_control_change(previous)
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing && self.selected_clip.is_some();
    }
    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    pub fn seek(&mut self, time_seconds: f32) -> Result<(), GlbImportError> {
        if !time_seconds.is_finite() {
            return Err(GlbImportError::InvalidData(
                "animation seek 시간은 유한해야 합니다".into(),
            ));
        }
        let previous = self.animation_control_state();
        self.time_seconds = time_seconds.clamp(0.0, self.selected_clip_duration());
        self.finish_animation_control_change(previous)
    }

    pub fn update(&mut self, dt_seconds: f32) -> Result<(), GlbImportError> {
        if !dt_seconds.is_finite() {
            return Err(GlbImportError::InvalidData(
                "animation update dt는 유한해야 합니다".into(),
            ));
        }
        let previous = self.animation_control_state();
        if self.playing {
            let duration = self.selected_clip_duration();
            if duration > 0.0 {
                let next = self.time_seconds + dt_seconds.max(0.0);
                if !next.is_finite() {
                    return Err(GlbImportError::InvalidData(
                        "animation update 결과 시간은 유한해야 합니다".into(),
                    ));
                }
                if self.looping {
                    self.time_seconds = next.rem_euclid(duration);
                } else if next >= duration {
                    self.time_seconds = duration;
                    self.playing = false;
                } else {
                    self.time_seconds = next;
                }
            }
        }
        self.finish_animation_control_change(previous)
    }

    fn animation_control_state(&self) -> AnimationControlState {
        AnimationControlState {
            selected_clip: self.selected_clip,
            time_seconds: self.time_seconds,
            playing: self.playing,
            animated_nodes: self.stats.animated_nodes,
        }
    }

    fn finish_animation_control_change(
        &mut self,
        previous: AnimationControlState,
    ) -> Result<(), GlbImportError> {
        match self.evaluate_pose() {
            Ok(()) => Ok(()),
            Err(error) => {
                self.selected_clip = previous.selected_clip;
                self.time_seconds = previous.time_seconds;
                self.playing = previous.playing;
                self.stats.animated_nodes = previous.animated_nodes;
                if let Err(rollback_error) = self.evaluate_pose() {
                    return Err(GlbImportError::InvalidData(format!(
                        "animation 제어 실패 뒤 pose rollback도 실패했습니다: {rollback_error}; 원래 오류: {error}"
                    )));
                }
                Err(error)
            }
        }
    }

    fn evaluated_bounds(&self) -> Result<(Vec3, Vec3), GlbImportError> {
        let mut positions = self.primitives.iter().flat_map(|primitive| {
            primitive
                .evaluated_vertices
                .iter()
                .map(|vertex| vertex.position_object)
        });
        let first = positions.next().ok_or_else(|| {
            GlbImportError::InvalidData("scene bounds를 계산할 정점이 없습니다".into())
        })?;
        let mut minimum = first;
        let mut maximum = first;
        for position in positions {
            minimum.x = minimum.x.min(position.x);
            minimum.y = minimum.y.min(position.y);
            minimum.z = minimum.z.min(position.z);
            maximum.x = maximum.x.max(position.x);
            maximum.y = maximum.y.max(position.y);
            maximum.z = maximum.z.max(position.z);
        }
        Ok((minimum, maximum))
    }

    fn evaluate_pose(&mut self) -> Result<(), GlbImportError> {
        for node in &mut self.nodes {
            node.pose = node.base_pose;
        }
        if let Some(clip_index) = self.selected_clip {
            let clip = &self.clips[clip_index];
            for channel in &clip.channels {
                let pose = &mut self.nodes[channel.node_index].pose;
                match &channel.values {
                    ChannelValues::Translation(values) => {
                        pose.translation = sample_vec3(channel, values, self.time_seconds)
                    }
                    ChannelValues::Scale(values) => {
                        pose.scale = sample_vec3(channel, values, self.time_seconds)
                    }
                    ChannelValues::Rotation(values) => {
                        pose.rotation = sample_quat(channel, values, self.time_seconds)
                    }
                }
            }
        }
        compute_globals(
            &self.nodes,
            &mut self.global_matrices,
            &mut self.node_visit_states,
            &mut self.node_visit_stack,
        )?;
        let root = self.root_transform;
        let root_normal = root
            .upper_left_3x3()
            .inverse()
            .map(|inverse| inverse.transpose())
            .ok_or_else(|| {
                GlbImportError::InvalidData("scene normalization matrix가 singular입니다".into())
            })?;
        for primitive in &mut self.primitives {
            let node_global = self.global_matrices[primitive.asset.node_index];
            let node_normal = if let Some(skin_index) = primitive.asset.skin_index {
                // Skinned positions are already evaluated in scene space as
                // global_joint * inverse_bind * position. The mesh node transform
                // is intentionally ignored by the glTF skinning profile.
                primitive.winding_reversed = false;
                let skin = &self.skins[skin_index];
                primitive.position_palette.clear();
                primitive.normal_palette.clear();
                primitive.position_palette.reserve(
                    skin.joints
                        .len()
                        .saturating_sub(primitive.position_palette.capacity()),
                );
                primitive.normal_palette.reserve(
                    skin.joints
                        .len()
                        .saturating_sub(primitive.normal_palette.capacity()),
                );
                for (&joint, &inverse_bind) in skin.joints.iter().zip(&skin.inverse_bind_matrices) {
                    let matrix = self.global_matrices[joint] * inverse_bind;
                    if !matrix_is_finite(matrix) {
                        return Err(GlbImportError::InvalidData(
                            "joint palette matrix 합성이 유한 범위를 벗어났습니다".into(),
                        ));
                    }
                    if matrix.upper_left_3x3().determinant() < 0.0 {
                        return Err(GlbImportError::Unsupported(
                            "negative-determinant skin pose".into(),
                        ));
                    }
                    let normal = matrix
                        .upper_left_3x3()
                        .inverse()
                        .map(|inverse| inverse.transpose())
                        .ok_or_else(|| {
                            GlbImportError::InvalidData(
                                "joint normal matrix가 singular입니다".into(),
                            )
                        })?;
                    primitive.position_palette.push(matrix);
                    primitive.normal_palette.push(normal);
                }
                None
            } else {
                primitive.winding_reversed = node_global.upper_left_3x3().determinant() < 0.0;
                Some(
                    node_global
                        .upper_left_3x3()
                        .inverse()
                        .map(|inverse| inverse.transpose())
                        .ok_or_else(|| {
                            GlbImportError::InvalidData(
                                "node normal matrix가 singular입니다".into(),
                            )
                        })?,
                )
            };
            for (index, (source, evaluated)) in primitive
                .asset
                .mesh
                .vertices()
                .iter()
                .zip(&mut primitive.evaluated_vertices)
                .enumerate()
            {
                let (world_position, world_normal) =
                    if let Some(skin_vertices) = primitive.asset.skin_vertices.as_ref() {
                        let skin_vertex = skin_vertices[index];
                        let mut position = Vec4::ZERO;
                        let mut normal = Vec3::ZERO;
                        for influence in 0..4 {
                            let weight = skin_vertex.weights[influence];
                            if weight == 0.0 {
                                continue;
                            }
                            let joint = skin_vertex.joints[influence] as usize;
                            position = position
                                + primitive.position_palette[joint]
                                    .transform_point(source.position_object)
                                    * weight;
                            normal = normal
                                + (primitive.normal_palette[joint] * source.normal_object) * weight;
                        }
                        (Vec3::new(position.x, position.y, position.z), normal)
                    } else {
                        let position = node_global.transform_point(source.position_object);
                        (
                            Vec3::new(position.x, position.y, position.z),
                            node_normal.expect("unskinned primitive에는 node normal이 있다")
                                * source.normal_object,
                        )
                    };
                if !finite([
                    world_position.x,
                    world_position.y,
                    world_position.z,
                    world_normal.x,
                    world_normal.y,
                    world_normal.z,
                ]) {
                    return Err(GlbImportError::InvalidData(
                        "평가된 world position/normal이 유한 범위를 벗어났습니다".into(),
                    ));
                }
                let normalized_position = root.transform_point(world_position);
                if !finite([
                    normalized_position.x,
                    normalized_position.y,
                    normalized_position.z,
                    normalized_position.w,
                ]) {
                    return Err(GlbImportError::InvalidData(
                        "정규화된 position이 유한 범위를 벗어났습니다".into(),
                    ));
                }
                evaluated.position_object = Vec3::new(
                    normalized_position.x,
                    normalized_position.y,
                    normalized_position.z,
                );
                evaluated.normal_object =
                    (root_normal * world_normal).normalized().ok_or_else(|| {
                        GlbImportError::InvalidData(
                            "평가된 normal이 0이거나 유효하지 않습니다".into(),
                        )
                    })?;
                evaluated.uv = source.uv;
                evaluated.color = source.color;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct AnimationControlState {
    selected_clip: Option<usize>,
    time_seconds: f32,
    playing: bool,
    animated_nodes: usize,
}

fn compute_globals(
    nodes: &[Node],
    globals: &mut [Mat4],
    states: &mut [u8],
    stack: &mut Vec<usize>,
) -> Result<(), GlbImportError> {
    states.fill(0);
    for start in 0..nodes.len() {
        if states[start] == 2 {
            continue;
        }
        stack.clear();
        let mut current = start;
        loop {
            match states[current] {
                2 => break,
                1 => {
                    return Err(GlbImportError::InvalidData(
                        "node hierarchy에 cycle이 있습니다".into(),
                    ));
                }
                _ => {
                    states[current] = 1;
                    stack.push(current);
                    let Some(parent) = nodes[current].parent else {
                        break;
                    };
                    current = parent;
                }
            }
        }
        while let Some(index) = stack.pop() {
            let local = nodes[index].pose.matrix();
            let global = nodes[index]
                .parent
                .map_or(local, |parent| globals[parent] * local);
            if !matrix_is_finite(global) {
                return Err(GlbImportError::InvalidData(
                    "node global transform 합성이 유한 범위를 벗어났습니다".into(),
                ));
            }
            globals[index] = global;
            states[index] = 2;
        }
    }
    Ok(())
}

fn sample_segment(times: &[f32], time: f32) -> (usize, usize, f32) {
    if time <= times[0] {
        return (0, 0, 0.0);
    }
    let last = times.len() - 1;
    if time >= times[last] {
        return (last, last, 0.0);
    }
    let right = times.partition_point(|&sample| sample <= time).min(last);
    let left = right - 1;
    (
        left,
        right,
        (time - times[left]) / (times[right] - times[left]),
    )
}

fn hermite_vec3(p0: Vec3, m0: Vec3, p1: Vec3, m1: Vec3, t: f32, duration: f32) -> Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (duration * (t3 - 2.0 * t2 + t))
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (duration * (t3 - t2))
}

fn hermite_quat(p0: Quat, m0: Quat, p1: Quat, m1: Quat, t: f32, duration: f32) -> Quat {
    let t2 = t * t;
    let t3 = t2 * t;
    (p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (duration * (t3 - 2.0 * t2 + t))
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (duration * (t3 - t2)))
        .normalized()
        .unwrap_or(p0)
}

fn sample_vec3(channel: &AnimationChannel, values: &[Vec3], time: f32) -> Vec3 {
    let (left, right, t) = sample_segment(&channel.times, time);
    let stride = if channel.interpolation == Interpolation::CubicSpline {
        3
    } else {
        1
    };
    if left == right {
        return values[left * stride + usize::from(stride == 3)];
    }
    match channel.interpolation {
        Interpolation::Step => values[left],
        Interpolation::Linear => values[left] * (1.0 - t) + values[right] * t,
        Interpolation::CubicSpline => hermite_vec3(
            values[left * 3 + 1],
            values[left * 3 + 2],
            values[right * 3 + 1],
            values[right * 3],
            t,
            channel.times[right] - channel.times[left],
        ),
    }
}

fn sample_quat(channel: &AnimationChannel, values: &[Quat], time: f32) -> Quat {
    let (left, right, t) = sample_segment(&channel.times, time);
    let stride = if channel.interpolation == Interpolation::CubicSpline {
        3
    } else {
        1
    };
    if left == right {
        return values[left * stride + usize::from(stride == 3)]
            .normalized()
            .unwrap_or(Quat::IDENTITY);
    }
    match channel.interpolation {
        Interpolation::Step => values[left],
        Interpolation::Linear => values[left].shortest_slerp(values[right], t),
        Interpolation::CubicSpline => hermite_quat(
            values[left * 3 + 1],
            values[left * 3 + 2],
            values[right * 3 + 1],
            values[right * 3],
            t,
            channel.times[right] - channel.times[left],
        ),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[derive(Default)]
    struct BinaryFixture {
        bytes: Vec<u8>,
        views: Vec<(usize, usize)>,
    }

    impl BinaryFixture {
        fn align(&mut self, alignment: usize) {
            while !self.bytes.len().is_multiple_of(alignment) {
                self.bytes.push(0);
            }
        }

        fn push_bytes(&mut self, bytes: &[u8], alignment: usize) -> usize {
            self.align(alignment);
            let offset = self.bytes.len();
            self.bytes.extend_from_slice(bytes);
            self.views.push((offset, bytes.len()));
            self.views.len() - 1
        }

        fn push_f32(&mut self, values: &[f32]) -> usize {
            let bytes = values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>();
            self.push_bytes(&bytes, 4)
        }

        fn push_u16(&mut self, values: &[u16]) -> usize {
            let bytes = values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>();
            self.push_bytes(&bytes, 2)
        }
    }

    fn identity_columns() -> [f32; 16] {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    fn build_glb(json: String, mut binary: Vec<u8>) -> Vec<u8> {
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + binary.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4e4f534au32.to_le_bytes());
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(binary.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x004e4942u32.to_le_bytes());
        glb.extend_from_slice(&binary);
        glb
    }

    fn glb_parts(bytes: &[u8]) -> (String, Vec<u8>) {
        let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let json_start = 20;
        let json_end = json_start + json_length;
        let binary_header = json_end;
        let binary_length =
            u32::from_le_bytes(bytes[binary_header..binary_header + 4].try_into().unwrap())
                as usize;
        let binary_start = binary_header + 8;
        (
            std::str::from_utf8(&bytes[json_start..json_end])
                .unwrap()
                .trim_end()
                .to_owned(),
            bytes[binary_start..binary_start + binary_length].to_vec(),
        )
    }

    fn replace_json(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
        let (json, binary) = glb_parts(bytes);
        assert!(
            json.contains(from),
            "fixture JSON에 교체 대상이 없습니다: {from}"
        );
        build_glb(json.replacen(from, to, 1), binary)
    }

    fn mutate_binary(bytes: &[u8], offset: usize, replacement: &[u8]) -> Vec<u8> {
        let (json, mut binary) = glb_parts(bytes);
        binary[offset..offset + replacement.len()].copy_from_slice(replacement);
        build_glb(json, binary)
    }

    fn unvalidated_gltf(bytes: &[u8]) -> gltf::Gltf {
        gltf::Gltf::from_slice_without_validation(bytes).unwrap()
    }

    fn parse_first_primitive_unvalidated(
        bytes: &[u8],
        skin_index: Option<usize>,
    ) -> Result<PrimitiveAsset, GlbImportError> {
        let gltf = unvalidated_gltf(bytes);
        let blob = gltf.blob.as_deref().unwrap();
        let primitive = gltf
            .document
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        parse_primitive(primitive, blob, 2, skin_index, 1)
    }

    fn parse_channel_unvalidated(
        bytes: &[u8],
        channel_index: usize,
    ) -> Result<AnimationChannel, GlbImportError> {
        let gltf = unvalidated_gltf(bytes);
        let blob = gltf.blob.as_deref().unwrap();
        let channel = gltf
            .document
            .animations()
            .next()
            .unwrap()
            .channels()
            .nth(channel_index)
            .unwrap();
        parse_channel(channel, blob)
    }

    fn parse_first_material_unvalidated(bytes: &[u8]) -> Result<MaterialTemplate, GlbImportError> {
        let gltf = unvalidated_gltf(bytes);
        parse_material(gltf.document.materials().next().unwrap())
    }

    pub(crate) fn canonical_glb(indexed: bool) -> Vec<u8> {
        let mut binary = BinaryFixture::default();
        let positions = binary.push_f32(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        let texcoords = binary.push_f32(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        let colors = binary.push_f32(&[1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
        let joints = binary.push_u16(&[0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0]);
        let weights =
            binary.push_f32(&[1.0, 0.0, 0.0, 0.0, 0.75, 0.25, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0]);
        let indices = binary.push_u16(&[0, 1, 2]);
        let inverse_bind = binary.push_f32(&[identity_columns(), identity_columns()].concat());
        let times = binary.push_f32(&[0.0, 1.0]);
        let translations = binary.push_f32(&[0.0, 0.0, 0.0, 0.0, 0.4, 0.0]);
        let rotations = binary.push_f32(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
        let scales = binary.push_f32(&[
            0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.2, 1.2, 1.2, 0.0, 0.0,
            0.0,
        ]);
        let image = binary.push_bytes(&[0x89, b'P', b'N', b'G'], 4);
        let byte_length = binary.bytes.len();
        let views = binary
            .views
            .iter()
            .map(|(offset, length)| {
                format!(r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{length}}}"#)
            })
            .collect::<Vec<_>>()
            .join(",");
        let index_field = if indexed { r#", "indices":5"# } else { "" };
        let json = format!(
            r#"{{
          "asset":{{"version":"2.0"}},
          "extensionsUsed":["KHR_materials_unlit"],
          "extensionsRequired":["KHR_materials_unlit"],
          "scene":0,
          "scenes":[{{"nodes":[0]}}],
          "nodes":[
            {{"children":[1,2]}},
            {{"children":[3]}},
            {{"mesh":0,"skin":0,"translation":[10,0,0]}},
            {{"translation":[0,0.5,0]}}
          ],
          "buffers":[{{"byteLength":{byte_length}}}],
          "bufferViews":[{views}],
          "accessors":[
            {{"bufferView":{positions},"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
            {{"bufferView":{texcoords},"componentType":5126,"count":3,"type":"VEC2"}},
            {{"bufferView":{colors},"componentType":5126,"count":3,"type":"VEC4"}},
            {{"bufferView":{joints},"componentType":5123,"count":3,"type":"VEC4"}},
            {{"bufferView":{weights},"componentType":5126,"count":3,"type":"VEC4"}},
            {{"bufferView":{indices},"componentType":5123,"count":3,"type":"SCALAR"}},
            {{"bufferView":{inverse_bind},"componentType":5126,"count":2,"type":"MAT4"}},
            {{"bufferView":{times},"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]}},
            {{"bufferView":{translations},"componentType":5126,"count":2,"type":"VEC3"}},
            {{"bufferView":{rotations},"componentType":5126,"count":2,"type":"VEC4"}},
            {{"bufferView":{scales},"componentType":5126,"count":6,"type":"VEC3"}}
          ],
          "images":[{{"bufferView":{image},"mimeType":"image/png"}}],
          "samplers":[{{"magFilter":9729,"minFilter":9987,"wrapS":33648,"wrapT":33071}}],
          "textures":[{{"sampler":0,"source":0}}],
          "materials":[{{
            "pbrMetallicRoughness":{{"baseColorFactor":[0.5,0.25,1,0.75],"baseColorTexture":{{"index":0}}}},
            "alphaMode":"BLEND","doubleSided":true,"extensions":{{"KHR_materials_unlit":{{}}}}
          }}],
          "meshes":[{{"primitives":[{{
            "attributes":{{"POSITION":0,"TEXCOORD_0":1,"COLOR_0":2,"JOINTS_0":3,"WEIGHTS_0":4}}
            {index_field}, "material":0
          }}]}}],
          "skins":[{{"joints":[1,3],"inverseBindMatrices":6}}],
          "animations":[{{"name":"Mixed","samplers":[
            {{"input":7,"output":8,"interpolation":"STEP"}},
            {{"input":7,"output":9,"interpolation":"LINEAR"}},
            {{"input":7,"output":10,"interpolation":"CUBICSPLINE"}}
          ],"channels":[
            {{"sampler":0,"target":{{"node":1,"path":"translation"}}}},
            {{"sampler":1,"target":{{"node":1,"path":"rotation"}}}},
            {{"sampler":2,"target":{{"node":3,"path":"scale"}}}}
          ]}}]
        }}"#
        );
        build_glb(json, binary.bytes)
    }

    pub(crate) fn canonical_opaque_glb() -> Vec<u8> {
        replace_json(
            &canonical_glb(true),
            "\"alphaMode\":\"BLEND\"",
            "\"alphaMode\":\"OPAQUE\"",
        )
    }

    pub(crate) fn canonical_lit_opaque_glb() -> Vec<u8> {
        replace_json(
            &canonical_opaque_glb(),
            r#","extensions":{"KHR_materials_unlit":{}}"#,
            "",
        )
    }

    pub(crate) fn canonical_mirrored_opaque_glb() -> Vec<u8> {
        let unskinned = replace_json(
            &canonical_opaque_glb(),
            r#""mesh":0,"skin":0,"translation":[10,0,0]"#,
            r#""mesh":0,"scale":[-1,1,1],"translation":[10,0,0]"#,
        );
        replace_json(&unskinned, r#","JOINTS_0":3,"WEIGHTS_0":4"#, "")
    }

    pub(crate) fn canonical_degenerate_glb() -> Vec<u8> {
        let mut bytes = canonical_glb(true);
        for offset in [0, 4, 8, 12, 16, 20, 24, 28, 32] {
            bytes = mutate_binary(&bytes, offset, &0.0f32.to_le_bytes());
        }
        for offset in [132, 148, 164] {
            let mut weights = Vec::from(1.0f32.to_le_bytes());
            weights.extend_from_slice(&[0; 12]);
            bytes = mutate_binary(&bytes, offset, &weights);
        }
        bytes
    }

    fn geometry_limit_glb(position_count: usize, index_count: Option<usize>) -> Vec<u8> {
        let mut binary = BinaryFixture::default();
        let positions = binary.push_f32(&vec![0.0; position_count * 3]);
        let indices = index_count.map(|count| binary.push_u16(&vec![0; count]));
        let byte_length = binary.bytes.len();
        let views = binary
            .views
            .iter()
            .map(|(offset, length)| {
                format!(r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{length}}}"#)
            })
            .collect::<Vec<_>>()
            .join(",");
        let index_accessor = indices.map_or_else(String::new, |view| {
            format!(
                r#",{{"bufferView":{view},"componentType":5123,"count":{},"type":"SCALAR"}}"#,
                index_count.unwrap()
            )
        });
        let index_field = indices.map_or_else(String::new, |_| r#", "indices":1"#.into());
        let json = format!(
            r#"{{
              "asset":{{"version":"2.0"}},
              "scene":0,
              "scenes":[{{"nodes":[0]}}],
              "nodes":[{{"mesh":0}}],
              "buffers":[{{"byteLength":{byte_length}}}],
              "bufferViews":[{views}],
              "accessors":[{{"bufferView":{positions},"componentType":5126,"count":{position_count},"type":"VEC3","min":[0,0,0],"max":[0,0,0]}}{index_accessor}],
              "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}{index_field}}}]}}]
            }}"#
        );
        build_glb(json, binary.bytes)
    }

    fn joint_palette_limit_glb(draw_items: usize, joint_count: usize) -> Vec<u8> {
        let mut binary = BinaryFixture::default();
        let positions = binary.push_f32(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        let joints = binary.push_u16(&[0; 12]);
        let weights =
            binary.push_f32(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        let views = binary
            .views
            .iter()
            .map(|(offset, length)| {
                format!(r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{length}}}"#)
            })
            .collect::<Vec<_>>()
            .join(",");
        let mut nodes = vec!["{}".to_owned(); joint_count];
        nodes.extend(std::iter::repeat_n(
            r#"{"mesh":0,"skin":0}"#.to_owned(),
            draw_items,
        ));
        let roots = (0..nodes.len())
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let skin_joints = (0..joint_count)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            r#"{{
              "asset":{{"version":"2.0"}},
              "scene":0,
              "scenes":[{{"nodes":[{roots}]}}],
              "nodes":[{}],
              "buffers":[{{"byteLength":{}}}],
              "bufferViews":[{views}],
              "accessors":[
                {{"bufferView":{positions},"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
                {{"bufferView":{joints},"componentType":5123,"count":3,"type":"VEC4"}},
                {{"bufferView":{weights},"componentType":5126,"count":3,"type":"VEC4"}}
              ],
              "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2}}}}]}}],
              "skins":[{{"joints":[{skin_joints}]}}]
            }}"#,
            nodes.join(","),
            binary.bytes.len(),
        );
        build_glb(json, binary.bytes)
    }

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1.0e-5,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn canonical_indexed_and_nonindexed_glb_parse_material_scene_animation_and_skin() {
        for indexed in [true, false] {
            let asset = import_glb(&canonical_glb(indexed)).unwrap();
            assert_eq!(asset.image_count(), 1);
            assert_eq!(asset.images()[0].mime_type(), "image/png");
            assert_eq!(asset.images()[0].bytes(), &[0x89, b'P', b'N', b'G']);
            assert_eq!(asset.primitive_count(), 1);
            assert_eq!(asset.node_count(), 4);
            assert_eq!(asset.skin_count(), 1);
            assert_eq!(asset.animation_count(), 1);
            assert_eq!(asset.source_vertices(), 3);
            assert_eq!(asset.source_triangles(), 1);
            assert_eq!(asset.primitives[0].mesh.indices(), &[0, 2, 1]);
            assert_eq!(
                asset.primitives[0].mesh.vertices()[0].normal_object,
                Vec3::Z
            );

            let mut scene = GlbScene::new(asset, &[TextureId(7)]).unwrap();
            assert_eq!(scene.clip_name(0), Some("Mixed"));
            assert_eq!(scene.stats().draw_items, 1);
            assert_eq!(scene.stats().skinned_vertices, 3);
            assert_eq!(scene.stats().sampler_downgrades, 1);
            assert_eq!(scene.stats().animated_nodes, 2);
            assert_eq!(scene.stats().joint_matrices_per_frame, 2);
            let material = scene.materials()[0];
            assert_eq!(material.material.base_color_texture, Some(TextureId(7)));
            assert_eq!(material.material.alpha_mode, AlphaMode::Blend);
            assert_eq!(material.material.sampler.filter, FilterMode::Bilinear);
            assert_eq!(
                material.material.sampler.address_u,
                AddressMode::MirroredRepeat
            );
            assert_eq!(
                material.material.sampler.address_v,
                AddressMode::ClampToEdge
            );
            assert!(material.double_sided);
            assert!(material.forced_unlit);
            assert!(scene.primitives()[0].is_skinned());
            assert!(
                scene.primitives()[0].vertices().iter().all(|vertex| vertex
                    .position_object
                    .x
                    .abs()
                    <= 0.9)
            );

            scene.set_clip(0).unwrap();
            scene.set_looping(false);
            scene.update(2.0).unwrap();
            assert_eq!(scene.time_seconds(), 1.0);
            assert!(!scene.playing());
            scene.seek(0.25).unwrap();
            close(scene.time_seconds(), 0.25);
            scene.set_playing(true);
            scene.set_looping(true);
            scene.update(1.0).unwrap();
            close(scene.time_seconds(), 0.25);
            let palette_capacity = scene.primitives[0].position_palette.capacity();
            let states_pointer = scene.node_visit_states.as_ptr();
            let stack_capacity = scene.node_visit_stack.capacity();
            scene.update(0.1).unwrap();
            assert_eq!(
                scene.primitives[0].position_palette.capacity(),
                palette_capacity
            );
            assert_eq!(scene.node_visit_states.as_ptr(), states_pointer);
            assert_eq!(scene.node_visit_stack.capacity(), stack_capacity);
            scene.set_lighting_enabled(true);
            assert_eq!(scene.materials()[0].material.shader_mode, ShaderMode::Unlit);
            scene.set_shader_mode(ShaderMode::BlinnPhong);
            assert_eq!(scene.materials()[0].material.shader_mode, ShaderMode::Unlit);
            scene.set_normal_mode(crate::texture::NormalMode::Flat);
            scene.set_specular(Vec3::new(0.2, 0.3, 0.4), 8.0);
            assert_eq!(
                scene.materials()[0].material.normal_mode,
                crate::texture::NormalMode::Flat
            );
            assert_eq!(scene.materials()[0].material.shininess, 8.0);
        }
    }

    #[test]
    fn quaternion_and_sampling_contracts_cover_step_linear_cubic_and_endpoints() {
        let quarter = Quat::new(
            0.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        let rotated = quarter.to_matrix().transform_direction(Vec3::X);
        close(rotated.x, 0.0);
        close(rotated.y, 1.0);
        let same_path = Quat::IDENTITY.shortest_slerp(Quat::new(0.0, 0.0, 0.0, -1.0), 0.4);
        assert_eq!(same_path, Quat::IDENTITY);

        let step = AnimationChannel {
            node_index: 0,
            interpolation: Interpolation::Step,
            times: vec![0.0, 1.0],
            values: ChannelValues::Translation(vec![Vec3::ZERO, Vec3::X]),
        };
        assert_eq!(sample_vec3(&step, &[Vec3::ZERO, Vec3::X], 0.75), Vec3::ZERO);
        let linear = AnimationChannel {
            interpolation: Interpolation::Linear,
            ..step.clone()
        };
        assert_eq!(
            sample_vec3(&linear, &[Vec3::ZERO, Vec3::X], 0.25),
            Vec3::new(0.25, 0.0, 0.0)
        );
        let cubic = AnimationChannel {
            interpolation: Interpolation::CubicSpline,
            ..step
        };
        let cubic_values = [
            Vec3::ZERO,
            Vec3::ZERO,
            Vec3::X,
            Vec3::X,
            Vec3::X,
            Vec3::ZERO,
        ];
        assert_eq!(sample_vec3(&cubic, &cubic_values, -1.0), Vec3::ZERO);
        assert_eq!(sample_vec3(&cubic, &cubic_values, 2.0), Vec3::X);
        let quat_values = [
            Quat::new(0.0, 0.0, 0.0, 0.0),
            Quat::IDENTITY,
            Quat::IDENTITY,
            Quat::IDENTITY,
            quarter,
            Quat::new(0.0, 0.0, 0.0, 0.0),
        ];
        assert!(
            sample_quat(&cubic, &quat_values, 0.5)
                .normalized()
                .is_some()
        );
    }

    #[test]
    fn invalid_headers_limits_errors_and_runtime_controls_are_explicit() {
        assert_eq!(
            import_glb(&vec![0; MAX_GLB_BYTES + 1]),
            Err(GlbImportError::InputTooLarge {
                bytes: MAX_GLB_BYTES + 1
            })
        );
        assert!(matches!(
            import_glb(b"glTF"),
            Err(GlbImportError::InvalidHeader(_))
        ));
        let mut wrong_version = vec![0; 12];
        wrong_version[..4].copy_from_slice(b"glTF");
        wrong_version[4..8].copy_from_slice(&1u32.to_le_bytes());
        wrong_version[8..12].copy_from_slice(&12u32.to_le_bytes());
        assert!(matches!(
            import_glb(&wrong_version),
            Err(GlbImportError::InvalidHeader(_))
        ));
        wrong_version[4..8].copy_from_slice(&2u32.to_le_bytes());
        wrong_version[8..12].copy_from_slice(&13u32.to_le_bytes());
        assert!(matches!(
            import_glb(&wrong_version),
            Err(GlbImportError::InvalidHeader(_))
        ));
        assert_eq!(
            limit(2, 1, "test"),
            Err(GlbImportError::LimitExceeded {
                kind: "test",
                max: 1
            })
        );
        assert_eq!(limit(1, 1, "test"), Ok(()));
        for error in [
            GlbImportError::InputTooLarge {
                bytes: MAX_GLB_BYTES + 1,
            },
            GlbImportError::InvalidHeader("x"),
            GlbImportError::Parse("x".into()),
            GlbImportError::MissingBinaryChunk,
            GlbImportError::Unsupported("x".into()),
            GlbImportError::LimitExceeded { kind: "x", max: 1 },
            GlbImportError::InvalidData("x".into()),
            GlbImportError::MeshValidation(MeshValidationError::IndicesNotTriangles {
                index_count: 1,
            }),
        ] {
            assert!(!error.to_string().is_empty());
        }
        assert!(matches!(
            GlbImportError::from(MeshValidationError::IndicesNotTriangles { index_count: 2 }),
            GlbImportError::MeshValidation(_)
        ));

        let asset = import_glb(&canonical_glb(true)).unwrap();
        assert!(
            GlbScene::new(asset.clone(), &[])
                .unwrap_err()
                .to_string()
                .contains("image/texture")
        );
        let mut scene = GlbScene::new(asset, &[TextureId(1)]).unwrap();
        assert!(scene.set_clip(9).is_err());
        assert!(scene.seek(f32::NAN).is_err());
    }

    #[test]
    fn importer_rejects_unsupported_primitives_and_corrupt_vertex_streams() {
        let canonical = canonical_glb(true);
        assert_eq!(bounded_format(format_args!("가나다"), 2), "가나");

        let cases = [
            (
                replace_json(
                    &canonical,
                    r#", "material":0"#,
                    r#", "mode":1,"material":0"#,
                ),
                Some(0),
                "primitive mode",
            ),
            (
                replace_json(
                    &canonical,
                    r#", "material":0"#,
                    r#", "targets":[{"POSITION":0}],"material":0"#,
                ),
                Some(0),
                "morph target",
            ),
            (
                replace_json(
                    &canonical,
                    r#""TEXCOORD_0":1"#,
                    r#""TEXCOORD_0":1,"TEXCOORD_1":1"#,
                ),
                Some(0),
                "vertex attribute",
            ),
            (
                replace_json(&canonical, r#""POSITION":0,"#, ""),
                Some(0),
                "POSITION accessor",
            ),
            (
                replace_json(
                    &canonical,
                    r#""count":3,"type":"SCALAR""#,
                    r#""count":2,"type":"SCALAR""#,
                ),
                Some(0),
                "3의 배수",
            ),
            (
                replace_json(
                    &canonical,
                    r#""TEXCOORD_0":1"#,
                    r#""TEXCOORD_0":1,"NORMAL":0"#,
                ),
                Some(0),
                "NORMAL",
            ),
            (
                replace_json(
                    &mutate_binary(&canonical, 332, &1.0f32.to_le_bytes()),
                    r#""TEXCOORD_0":1"#,
                    r#""TEXCOORD_0":1,"NORMAL":8"#,
                ),
                Some(0),
                "NORMAL 수",
            ),
            (
                replace_json(&canonical, r#""JOINTS_0":3,"#, ""),
                Some(0),
                "함께",
            ),
            (
                replace_json(&canonical, r#""mesh":0,"skin":0"#, r#""mesh":0"#),
                None,
                "존재 여부",
            ),
        ];
        for (bytes, skin_index, message) in cases {
            let error = parse_first_primitive_unvalidated(&bytes, skin_index).unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
        }

        let valid_normals = replace_json(
            &mutate_binary(&canonical, 8, &1.0f32.to_le_bytes()),
            r#""TEXCOORD_0":1"#,
            r#""TEXCOORD_0":1,"NORMAL":0"#,
        );
        assert!(parse_first_primitive_unvalidated(&valid_normals, Some(0)).is_ok());

        let non_finite_uv = mutate_binary(&canonical, 36, &f32::NAN.to_le_bytes());
        assert!(
            parse_first_primitive_unvalidated(&non_finite_uv, Some(0))
                .unwrap_err()
                .to_string()
                .contains("TEXCOORD_0")
        );
        let non_finite_position = mutate_binary(&canonical, 0, &f32::NAN.to_le_bytes());
        assert!(
            parse_first_primitive_unvalidated(&non_finite_position, Some(0))
                .unwrap_err()
                .to_string()
                .contains("POSITION")
        );
        let non_finite_weight = mutate_binary(&canonical, 132, &f32::NAN.to_le_bytes());
        assert!(
            parse_first_primitive_unvalidated(&non_finite_weight, Some(0))
                .unwrap_err()
                .to_string()
                .contains("유한")
        );
        let negative_weight = mutate_binary(&canonical, 132, &(-1.0f32).to_le_bytes());
        assert!(
            parse_first_primitive_unvalidated(&negative_weight, Some(0))
                .unwrap_err()
                .to_string()
                .contains("음수")
        );
        let duplicate_joint = mutate_binary(&canonical, 118, &0u16.to_le_bytes());
        assert!(
            parse_first_primitive_unvalidated(&duplicate_joint, Some(0))
                .unwrap_err()
                .to_string()
                .contains("서로 다른 joint")
        );
        let zero_weights = mutate_binary(&canonical, 132, &[0; 16]);
        assert!(
            parse_first_primitive_unvalidated(&zero_weights, Some(0))
                .unwrap_err()
                .to_string()
                .contains("합")
        );
        let bad_index = mutate_binary(&canonical, 180, &9u16.to_le_bytes());
        assert!(matches!(
            parse_first_primitive_unvalidated(&bad_index, Some(0)),
            Err(GlbImportError::MeshValidation(_))
        ));
    }

    #[test]
    fn importer_rejects_corrupt_animation_streams_and_unsupported_targets() {
        let canonical = canonical_glb(true);
        let morph = replace_json(&canonical, r#""path":"translation""#, r#""path":"weights""#);
        let morph = replace_json(
            &morph,
            r#""input":7,"output":8,"interpolation":"STEP""#,
            r#""input":7,"output":7,"interpolation":"STEP""#,
        );
        assert!(
            parse_channel_unvalidated(&morph, 0)
                .unwrap_err()
                .to_string()
                .contains("morph")
        );

        let missing_inputs = replace_json(
            &canonical,
            r#""buffers":[{"byteLength":456}]"#,
            r#""buffers":[{"byteLength":456,"uri":"missing.bin"}]"#,
        );
        assert!(
            parse_channel_unvalidated(&missing_inputs, 0)
                .unwrap_err()
                .to_string()
                .contains("input accessor")
        );
        let missing_outputs = replace_json(
            &canonical,
            r#"{"bufferView":8,"componentType":5126,"count":2,"type":"VEC3"}"#,
            r#"{"componentType":5126,"count":2,"type":"VEC3"}"#,
        );
        assert!(
            parse_channel_unvalidated(&missing_outputs, 0)
                .unwrap_err()
                .to_string()
                .contains("output accessor")
        );
        let descending_times = mutate_binary(&canonical, 316, &1.0f32.to_le_bytes());
        assert!(
            parse_channel_unvalidated(&descending_times, 0)
                .unwrap_err()
                .to_string()
                .contains("엄격히 증가")
        );
        let non_finite_translation = mutate_binary(&canonical, 324, &f32::NAN.to_le_bytes());
        assert!(
            parse_channel_unvalidated(&non_finite_translation, 0)
                .unwrap_err()
                .to_string()
                .contains("translation")
        );
        let non_finite_scale = mutate_binary(&canonical, 380, &f32::NAN.to_le_bytes());
        assert!(
            parse_channel_unvalidated(&non_finite_scale, 2)
                .unwrap_err()
                .to_string()
                .contains("scale")
        );
        let zero_rotation = mutate_binary(&canonical, 348, &[0; 16]);
        assert!(
            parse_channel_unvalidated(&zero_rotation, 1)
                .unwrap_err()
                .to_string()
                .contains("quaternion")
        );
        let mismatched_output = replace_json(
            &canonical,
            r#"{"bufferView":8,"componentType":5126,"count":2,"type":"VEC3"}"#,
            r#"{"bufferView":8,"componentType":5126,"count":1,"type":"VEC3"}"#,
        );
        assert!(
            parse_channel_unvalidated(&mismatched_output, 0)
                .unwrap_err()
                .to_string()
                .contains("기대값")
        );

        let cubic_rotation = replace_json(
            &canonical,
            r#"{"bufferView":10,"componentType":5126,"count":6,"type":"VEC3"}"#,
            r#"{"bufferView":6,"componentType":5126,"count":6,"type":"VEC4"}"#,
        );
        let cubic_rotation =
            replace_json(&cubic_rotation, r#""path":"scale""#, r#""path":"rotation""#);
        assert!(parse_channel_unvalidated(&cubic_rotation, 2).is_ok());
        let zero_cubic_key = mutate_binary(&cubic_rotation, 204, &[0; 16]);
        assert!(
            parse_channel_unvalidated(&zero_cubic_key, 2)
                .unwrap_err()
                .to_string()
                .contains("CUBICSPLINE rotation")
        );
    }

    #[test]
    fn importer_header_material_image_hierarchy_and_scene_failures_are_explicit() {
        let canonical = canonical_glb(true);
        let asset_v1 = replace_json(&canonical, r#""version":"2.0""#, r#""version":"1.0""#);
        assert!(
            import_glb(&asset_v1)
                .unwrap_err()
                .to_string()
                .contains("asset.version")
        );
        let minimum_v3 = replace_json(
            &canonical,
            r#""version":"2.0""#,
            r#""version":"2.0","minVersion":"3.0""#,
        );
        assert!(
            import_glb(&minimum_v3)
                .unwrap_err()
                .to_string()
                .contains("asset.minVersion")
        );
        assert!(validate_asset_version("2.0", Some("1.0")).is_ok());
        assert!(validate_asset_version("2", None).is_err());
        assert!(validate_asset_version(".", None).is_err());
        assert!(validate_asset_version("2.0", Some("future")).is_err());

        let mut wrong_json_chunk = canonical.clone();
        wrong_json_chunk[16..20].copy_from_slice(b"BIN\0");
        assert!(
            import_glb(&wrong_json_chunk)
                .unwrap_err()
                .to_string()
                .contains("첫 chunk")
        );
        let mut wrong_bin_chunk = canonical.clone();
        let canonical_json_length =
            u32::from_le_bytes(wrong_bin_chunk[12..16].try_into().unwrap()) as usize;
        let canonical_bin_header = 20 + canonical_json_length;
        wrong_bin_chunk[canonical_bin_header + 4..canonical_bin_header + 8]
            .copy_from_slice(b"JUNK");
        assert!(
            import_glb(&wrong_bin_chunk)
                .unwrap_err()
                .to_string()
                .contains("두 번째 chunk")
        );

        let mut duplicate_bin = canonical.clone();
        duplicate_bin.extend_from_slice(&0u32.to_le_bytes());
        duplicate_bin.extend_from_slice(b"BIN\0");
        let duplicate_length = duplicate_bin.len() as u32;
        duplicate_bin[8..12].copy_from_slice(&duplicate_length.to_le_bytes());
        assert!(
            import_glb(&duplicate_bin)
                .unwrap_err()
                .to_string()
                .contains("추가 chunk")
        );
        let mut excessive_padding = canonical.clone();
        let json_length =
            u32::from_le_bytes(excessive_padding[12..16].try_into().unwrap()) as usize;
        let bin_length_offset = 20 + json_length;
        let bin_length = u32::from_le_bytes(
            excessive_padding[bin_length_offset..bin_length_offset + 4]
                .try_into()
                .unwrap(),
        );
        excessive_padding[bin_length_offset..bin_length_offset + 4]
            .copy_from_slice(&(bin_length + 4).to_le_bytes());
        excessive_padding.extend_from_slice(&[0; 4]);
        let padded_total = excessive_padding.len() as u32;
        excessive_padding[8..12].copy_from_slice(&padded_total.to_le_bytes());
        assert!(
            import_glb(&excessive_padding)
                .unwrap_err()
                .to_string()
                .contains("padding 3 bytes")
        );
        let mut non_zero_padding = replace_json(
            &excessive_padding,
            r#""buffers":[{"byteLength":456}]"#,
            r#""buffers":[{"byteLength":457}]"#,
        );
        *non_zero_padding.last_mut().unwrap() = 1;
        assert!(
            import_glb(&non_zero_padding)
                .unwrap_err()
                .to_string()
                .contains("padding은 0x00")
        );

        let mut malformed = canonical.clone();
        malformed[20] = b'!';
        assert!(matches!(
            import_glb(&malformed),
            Err(GlbImportError::Parse(_))
        ));

        let unknown_extension = replace_json(
            &canonical,
            r#""extensionsRequired":["KHR_materials_unlit"]"#,
            r#""extensionsRequired":["EXT_unknown"]"#,
        );
        assert!(
            import_glb(&unknown_extension)
                .unwrap_err()
                .to_string()
                .contains("required extension")
        );
        let external_image = replace_json(
            &canonical,
            r#"{"bufferView":11,"mimeType":"image/png"}"#,
            r#"{"uri":"image.png"}"#,
        );
        assert!(
            import_glb(&external_image)
                .unwrap_err()
                .to_string()
                .contains("URI image")
        );
        let unsupported_mime = replace_json(&canonical, "image/png", "image/webp");
        assert!(
            import_glb(&unsupported_mime)
                .unwrap_err()
                .to_string()
                .contains("MIME")
        );
        let external_buffer = replace_json(
            &canonical,
            r#""buffers":[{"byteLength":456}]"#,
            r#""buffers":[{"byteLength":456,"uri":"mesh.bin"}]"#,
        );
        assert!(
            import_glb(&external_buffer)
                .unwrap_err()
                .to_string()
                .contains("URI buffer")
        );
        let invalid_document = replace_json(
            &canonical,
            r#""bufferView":0,"componentType":5126"#,
            r#""bufferView":99,"componentType":5126"#,
        );
        assert!(matches!(
            import_glb(&invalid_document),
            Err(GlbImportError::Parse(_))
        ));
        let (json, mut short_binary) = glb_parts(&canonical);
        short_binary.truncate(451);
        assert!(
            import_glb(&build_glb(json, short_binary))
                .unwrap_err()
                .to_string()
                .contains("BIN chunk")
        );
        let duplicate_root = replace_json(
            &canonical,
            r#""scenes":[{"nodes":[0]}]"#,
            r#""scenes":[{"nodes":[0,0]}]"#,
        );
        let duplicate_asset = import_glb(&duplicate_root).unwrap();
        assert_eq!(duplicate_asset.primitive_count(), 1);
        let oversized_roots = std::iter::repeat_n("0", MAX_GLB_NODES + 1)
            .collect::<Vec<_>>()
            .join(",");
        let oversized_roots = replace_json(
            &canonical,
            r#""scenes":[{"nodes":[0]}]"#,
            &format!(r#""scenes":[{{"nodes":[{oversized_roots}]}}]"#),
        );
        assert!(matches!(
            import_glb(&oversized_roots),
            Err(GlbImportError::LimitExceeded {
                kind: "scene root",
                ..
            })
        ));
        let multiple_parents = replace_json(
            &canonical,
            r#"{"children":[1,2]}"#,
            r#"{"children":[1,2,3]}"#,
        );
        assert!(
            import_glb(&multiple_parents)
                .unwrap_err()
                .to_string()
                .contains("둘 이상의 parent")
        );

        let bad_inverse_bind_count = replace_json(
            &canonical,
            r#""bufferView":6,"componentType":5126,"count":2,"type":"MAT4""#,
            r#""bufferView":6,"componentType":5126,"count":1,"type":"MAT4""#,
        );
        assert!(
            import_glb(&bad_inverse_bind_count)
                .unwrap_err()
                .to_string()
                .contains("inverseBindMatrices")
        );
        let non_finite_inverse_bind = mutate_binary(&canonical, 188, &f32::NAN.to_le_bytes());
        assert!(
            import_glb(&non_finite_inverse_bind)
                .unwrap_err()
                .to_string()
                .contains("성분은 유한")
        );
        let projective_inverse_bind = mutate_binary(&canonical, 200, &1.0f32.to_le_bytes());
        assert!(
            import_glb(&projective_inverse_bind)
                .unwrap_err()
                .to_string()
                .contains("affine")
        );
        let oversized_joints = std::iter::repeat_n("1", MAX_GLB_JOINTS_PER_SKIN + 1)
            .collect::<Vec<_>>()
            .join(",");
        let oversized_joints = replace_json(
            &canonical,
            r#""skins":[{"joints":[1,3],"inverseBindMatrices":6}]"#,
            &format!(r#""skins":[{{"joints":[{oversized_joints}],"inverseBindMatrices":6}}]"#),
        );
        assert!(matches!(
            import_glb(&oversized_joints),
            Err(GlbImportError::LimitExceeded {
                kind: "joint/skin",
                ..
            })
        ));
        let bad_joint = mutate_binary(&canonical, 108, &9u16.to_le_bytes());
        assert!(
            import_glb(&bad_joint)
                .unwrap_err()
                .to_string()
                .contains("joint 범위")
        );
        let no_primitives = replace_json(&canonical, r#""mesh":0,"skin":0,"#, "");
        assert!(
            import_glb(&no_primitives)
                .unwrap_err()
                .to_string()
                .contains("primitive")
        );
        let unsupported_mode = replace_json(
            &canonical,
            r#", "material":0"#,
            r#", "mode":1,"material":0"#,
        );
        assert!(
            import_glb(&unsupported_mode)
                .unwrap_err()
                .to_string()
                .contains("primitive mode")
        );

        let multiple_bin_buffers = replace_json(
            &canonical,
            r#""buffers":[{"byteLength":456}]"#,
            r#""buffers":[{"byteLength":456},{"byteLength":456}]"#,
        );
        assert!(
            import_glb(&multiple_bin_buffers)
                .unwrap_err()
                .to_string()
                .contains("BIN buffer 하나")
        );
    }

    #[test]
    fn importer_preflights_accessor_ranges_profiles_animation_and_image_amplification() {
        assert_eq!(checked_range_end(1, 2, "fixture"), Ok(3));
        assert!(checked_range_end(usize::MAX, 2, "fixture").is_err());
        assert_eq!(
            checked_blob_range(&[1, 2, 3, 4], 1, 2, "fixture").unwrap(),
            &[2, 3]
        );
        assert!(checked_blob_range(&[0; 4], usize::MAX, 2, "fixture").is_err());
        assert!(checked_blob_range(&[0; 4], 3, 2, "fixture").is_err());
        assert_eq!(checked_subrange_end(0, 4, 2, 4, 8, "fixture"), Ok(8));
        assert!(checked_subrange_end(0, 4, 0, 4, 8, "fixture").is_err());
        assert!(checked_subrange_end(0, 2, 2, 4, 8, "fixture").is_err());
        assert!(checked_subrange_end(usize::MAX, 4, 2, 4, usize::MAX, "fixture").is_err());
        assert!(checked_subrange_end(0, 4, 2, 4, 7, "fixture").is_err());

        let canonical = canonical_glb(true);
        let zero_count = replace_json(
            &canonical,
            r#"{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3""#,
            r#"{"bufferView":0,"componentType":5126,"count":0,"type":"VEC3""#,
        );
        let zero_count_document = unvalidated_gltf(&zero_count);
        let zero_count_accessor = zero_count_document.document.accessors().next().unwrap();
        assert!(
            validate_accessor_profile(
                &zero_count_accessor,
                "POSITION",
                &[(DataType::F32, Dimensions::Vec3, false)]
            )
            .unwrap_err()
            .to_string()
            .contains("count는 0보다")
        );
        let implicit_zero_storage = replace_json(
            &canonical,
            r#"{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3""#,
            r#"{"componentType":5126,"count":3,"type":"VEC3""#,
        );
        let implicit_zero_document = unvalidated_gltf(&implicit_zero_storage);
        let implicit_zero_accessor = implicit_zero_document.document.accessors().next().unwrap();
        assert!(validate_accessor_storage(&implicit_zero_accessor, "POSITION").is_ok());
        let sparse_position = replace_json(
            &canonical,
            r#""max":[1,1,0]}"#,
            r#""max":[1,1,0],"sparse":{"count":1,"indices":{"bufferView":5,"componentType":5123},"values":{"bufferView":0}}}"#,
        );
        assert!(
            import_glb(&sparse_position)
                .unwrap_err()
                .to_string()
                .contains("sparse accessor")
        );
        let unsupported_position_profile = replace_json(
            &canonical,
            r#"{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3""#,
            r#"{"bufferView":0,"componentType":5123,"count":3,"type":"VEC3""#,
        );
        assert!(
            import_glb(&unsupported_position_profile)
                .unwrap_err()
                .to_string()
                .contains("accessor profile")
        );
        let unsupported_weight_profile = replace_json(
            &canonical,
            r#"{"bufferView":4,"componentType":5126,"count":3,"type":"VEC4"}"#,
            r#"{"bufferView":4,"componentType":5123,"count":3,"type":"VEC4"}"#,
        );
        assert!(
            import_glb(&unsupported_weight_profile)
                .unwrap_err()
                .to_string()
                .contains("accessor profile")
        );
        let unsupported_rotation_profile = replace_json(
            &canonical,
            r#"{"bufferView":9,"componentType":5126,"count":2,"type":"VEC4"}"#,
            r#"{"bufferView":9,"componentType":5125,"count":2,"type":"VEC4"}"#,
        );
        assert!(
            import_glb(&unsupported_rotation_profile)
                .unwrap_err()
                .to_string()
                .contains("accessor profile")
        );
        let mismatched_normal = replace_json(
            &canonical,
            r#""TEXCOORD_0":1"#,
            r#""TEXCOORD_0":1,"NORMAL":8"#,
        );
        let mismatched_normal_error = import_glb(&mismatched_normal).unwrap_err();
        assert!(
            mismatched_normal_error.to_string().contains("POSITION 수"),
            "{mismatched_normal_error}"
        );
        let accessor_out_of_range = replace_json(
            &canonical,
            r#"{"bufferView":0,"componentType":5126"#,
            r#"{"bufferView":0,"byteOffset":999,"componentType":5126"#,
        );
        let accessor_out_of_range_document = unvalidated_gltf(&accessor_out_of_range);
        let accessor_out_of_range_value = accessor_out_of_range_document
            .document
            .accessors()
            .next()
            .unwrap();
        assert!(
            validate_accessor_profile(
                &accessor_out_of_range_value,
                "POSITION",
                &[(DataType::F32, Dimensions::Vec3, false)]
            )
            .is_err()
        );
        assert!(
            import_glb(&accessor_out_of_range)
                .unwrap_err()
                .to_string()
                .contains("bufferView 범위")
        );
        let unsupported_index_profile = replace_json(
            &canonical,
            r#"{"bufferView":5,"componentType":5123,"count":3,"type":"SCALAR"}"#,
            r#"{"bufferView":5,"componentType":5126,"count":3,"type":"SCALAR"}"#,
        );
        let unsupported_index_document = unvalidated_gltf(&unsupported_index_profile);
        let unsupported_index_primitive = unsupported_index_document
            .document
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        assert!(
            validate_primitive_declaration(&unsupported_index_primitive, Some(0))
                .unwrap_err()
                .to_string()
                .contains("accessor profile")
        );
        let view_out_of_range = replace_json(
            &canonical,
            r#"{"buffer":0,"byteOffset":452,"byteLength":4}"#,
            r#"{"buffer":0,"byteOffset":456,"byteLength":4}"#,
        );
        assert!(
            import_glb(&view_out_of_range)
                .unwrap_err()
                .to_string()
                .contains("buffer 선언 범위")
        );
        let mismatched_output = replace_json(
            &canonical,
            r#"{"bufferView":8,"componentType":5126,"count":2,"type":"VEC3"}"#,
            r#"{"bufferView":8,"componentType":5126,"count":1,"type":"VEC3"}"#,
        );
        assert!(
            import_glb(&mismatched_output)
                .unwrap_err()
                .to_string()
                .contains("기대값")
        );
        let duplicate_target = replace_json(
            &canonical,
            r#""path":"rotation""#,
            r#""path":"translation""#,
        );
        assert!(
            import_glb(&duplicate_target)
                .unwrap_err()
                .to_string()
                .contains("같은 node/property")
        );
        let unsupported_animation_input = replace_json(
            &canonical,
            r#"{"bufferView":7,"componentType":5126,"count":2,"type":"SCALAR""#,
            r#"{"bufferView":7,"componentType":5123,"count":2,"type":"SCALAR""#,
        );
        let unsupported_animation_document = unvalidated_gltf(&unsupported_animation_input);
        assert!(
            validate_animation_declarations(&unsupported_animation_document.document)
                .unwrap_err()
                .to_string()
                .contains("accessor profile")
        );
        let morph_animation =
            replace_json(&canonical, r#""path":"translation""#, r#""path":"weights""#);
        assert!(
            import_glb(&morph_animation)
                .unwrap_err()
                .to_string()
                .contains("morph weights")
        );
        let morph_primitive = replace_json(
            &canonical,
            r#""WEIGHTS_0":4}"#,
            r#""WEIGHTS_0":4},"targets":[{"POSITION":0}]"#,
        );
        assert!(
            import_glb(&morph_primitive)
                .unwrap_err()
                .to_string()
                .contains("morph target")
        );
        let missing_position = replace_json(&canonical, r#""POSITION":0,"#, "");
        let missing_position_document = unvalidated_gltf(&missing_position);
        let missing_position_primitive = missing_position_document
            .document
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap();
        assert!(
            validate_primitive_declaration(&missing_position_primitive, Some(0))
                .unwrap_err()
                .to_string()
                .contains("POSITION accessor")
        );
        let unsupported_attribute = replace_json(
            &canonical,
            r#""TEXCOORD_0":1"#,
            r#""TEXCOORD_0":1,"TEXCOORD_1":1"#,
        );
        assert!(
            import_glb(&unsupported_attribute)
                .unwrap_err()
                .to_string()
                .contains("vertex attribute")
        );
        let missing_joints = replace_json(&canonical, r#""JOINTS_0":3,"#, "");
        assert!(
            import_glb(&missing_joints)
                .unwrap_err()
                .to_string()
                .contains("존재 여부")
        );
        let unsupported_inverse_bind = replace_json(
            &canonical,
            r#"{"bufferView":6,"componentType":5126,"count":2,"type":"MAT4"}"#,
            r#"{"bufferView":6,"componentType":5123,"count":2,"type":"MAT4"}"#,
        );
        assert!(
            import_glb(&unsupported_inverse_bind)
                .unwrap_err()
                .to_string()
                .contains("accessor profile")
        );
        let implicit_inverse_bind = replace_json(&canonical, r#","inverseBindMatrices":6"#, "");
        assert!(import_glb(&implicit_inverse_bind).is_ok());
        let non_finite_import = mutate_binary(&canonical, 0, &f32::NAN.to_le_bytes());
        assert!(
            import_glb(&non_finite_import)
                .unwrap_err()
                .to_string()
                .contains("POSITION")
        );
        assert!(
            import_glb(&geometry_limit_glb(4, None))
                .unwrap_err()
                .to_string()
                .contains("3의 배수")
        );

        let (json, mut binary) = glb_parts(&canonical);
        binary.resize(600_452, 0);
        let json = json
            .replacen(
                r#""buffers":[{"byteLength":456}]"#,
                r#""buffers":[{"byteLength":600452}]"#,
                1,
            )
            .replacen(
                r#"{"buffer":0,"byteOffset":452,"byteLength":4}"#,
                r#"{"buffer":0,"byteOffset":452,"byteLength":600000}"#,
                1,
            );
        let repeated_images = std::iter::repeat_n(
            r#"{"bufferView":11,"mimeType":"image/png"}"#,
            MAX_GLB_IMAGES,
        )
        .collect::<Vec<_>>()
        .join(",");
        let json = json.replacen(
            r#""images":[{"bufferView":11,"mimeType":"image/png"}]"#,
            &format!(r#""images":[{repeated_images}]"#),
            1,
        );
        let amplified = import_glb(&build_glb(json, binary));
        assert!(matches!(
            amplified,
            Err(GlbImportError::LimitExceeded {
                kind: "encoded image byte",
                ..
            })
        ));

        let transparent_source_triangles =
            MAX_GLB_TRANSPARENT_GENERATED_TRIANGLES / (MAX_CLIPPED_POLYGON_VERTICES - 2) + 1;
        let transparent = geometry_limit_glb(3, Some(transparent_source_triangles * 3));
        let transparent = replace_json(
            &transparent,
            r#""meshes":"#,
            r#""materials":[{"alphaMode":"BLEND"}],"meshes":"#,
        );
        let transparent = replace_json(
            &transparent,
            r#""attributes":{"POSITION":0}, "indices":1"#,
            r#""attributes":{"POSITION":0}, "indices":1,"material":0"#,
        );
        assert!(matches!(
            import_glb(&transparent),
            Err(GlbImportError::LimitExceeded {
                kind: "transparent clipped triangle",
                ..
            })
        ));

        let palette_overflow = joint_palette_limit_glb(
            MAX_GLB_JOINT_MATRICES_PER_FRAME / MAX_GLB_JOINTS_PER_SKIN + 1,
            MAX_GLB_JOINTS_PER_SKIN,
        );
        assert!(matches!(
            import_glb(&palette_overflow),
            Err(GlbImportError::LimitExceeded {
                kind: "joint matrix/frame",
                ..
            })
        ));

        let invalid_accessor =
            r#"{"bufferView":999,"componentType":5126,"count":1,"type":"SCALAR"}"#;
        let invalid_accessors = std::iter::repeat_n(invalid_accessor, 16)
            .collect::<Vec<_>>()
            .join(",");
        let many_validation_errors = replace_json(
            &canonical,
            r#"{"bufferView":10,"componentType":5126,"count":6,"type":"VEC3"}"#,
            &format!(
                r#"{{"bufferView":10,"componentType":5126,"count":6,"type":"VEC3"}},{invalid_accessors}"#
            ),
        );
        let validation_error = import_glb(&many_validation_errors).unwrap_err();
        assert!(
            validation_error
                .to_string()
                .contains("document validation errors")
        );
        assert!(validation_error.to_string().chars().count() < 3_000);
    }

    #[test]
    fn material_sampler_profiles_map_to_supported_runtime_states() {
        let canonical = canonical_glb(true);
        let texcoord_one = replace_json(
            &canonical,
            r#""baseColorTexture":{"index":0}"#,
            r#""baseColorTexture":{"index":0,"texCoord":1}"#,
        );
        assert!(
            parse_first_material_unvalidated(&texcoord_one)
                .unwrap_err()
                .to_string()
                .contains("TEXCOORD_1")
        );

        let no_texture = replace_json(&canonical, r#","baseColorTexture":{"index":0}"#, "");
        let no_texture = replace_json(
            &no_texture,
            "\"alphaMode\":\"BLEND\"",
            "\"alphaMode\":\"MASK\"",
        );
        let material = parse_first_material_unvalidated(&no_texture).unwrap();
        assert_eq!(material.image_index, None);
        assert_eq!(material.material.alpha_mode, AlphaMode::Mask);
        assert_eq!(material.material.sampler, SamplerState::default());

        let opaque = replace_json(
            &canonical,
            "\"alphaMode\":\"BLEND\"",
            "\"alphaMode\":\"OPAQUE\"",
        );
        let material = parse_first_material_unvalidated(&opaque).unwrap();
        assert_eq!(material.material.alpha_mode, AlphaMode::Opaque);
        assert_eq!(material.material.base_color.w, 1.0);

        let min_linear = replace_json(
            &canonical,
            r#"{"magFilter":9729,"minFilter":9987,"wrapS":33648,"wrapT":33071}"#,
            r#"{"magFilter":9728,"minFilter":9729,"wrapS":10497,"wrapT":10497}"#,
        );
        let material = parse_first_material_unvalidated(&min_linear).unwrap();
        assert_eq!(material.material.sampler.filter, FilterMode::Bilinear);
        assert_eq!(material.material.sampler.address_u, AddressMode::Repeat);
        assert!(material.sampler_downgraded);

        let nearest = replace_json(
            &canonical,
            r#"{"magFilter":9729,"minFilter":9987,"wrapS":33648,"wrapT":33071}"#,
            r#"{"magFilter":9728,"minFilter":9728,"wrapS":10497,"wrapT":10497}"#,
        );
        assert_eq!(
            parse_first_material_unvalidated(&nearest)
                .unwrap()
                .material
                .sampler
                .filter,
            FilterMode::Nearest
        );
    }

    #[test]
    fn runtime_scene_handles_unlit_lit_animation_and_internal_invariants() {
        assert!(gltf_quat_components_to_lh([f32::NAN, 0.0, 0.0, 1.0]).is_err());
        assert!(gltf_quat_to_lh([0.0; 4]).is_err());
        assert!(matrix_is_finite(Mat4::identity()));
        assert!(!matrix_is_finite(Mat4::translation(Vec3::new(
            f32::INFINITY,
            0.0,
            0.0,
        ))));

        let canonical = canonical_glb(true);
        let lit = replace_json(
            &canonical,
            r#","extensions":{"KHR_materials_unlit":{}}"#,
            "",
        );
        let lit = replace_json(
            &lit,
            r#""extensionsRequired":["KHR_materials_unlit"]"#,
            r#""extensionsRequired":[]"#,
        );
        let mut scene = GlbScene::new(import_glb(&lit).unwrap(), &[TextureId(1)]).unwrap();
        scene.set_lighting_enabled(false);
        assert_eq!(scene.materials()[0].material.shader_mode, ShaderMode::Unlit);
        scene.set_lighting_enabled(true);
        assert_eq!(
            scene.materials()[0].material.shader_mode,
            ShaderMode::Lambert
        );
        scene.set_looping(false);
        scene.seek(0.25).unwrap();
        scene.update(0.25).unwrap();
        close(scene.time_seconds(), 0.5);

        let mut no_clip_asset = import_glb(&canonical).unwrap();
        no_clip_asset.clips.clear();
        let mut no_clip = GlbScene::new(no_clip_asset, &[TextureId(1)]).unwrap();
        assert_eq!(no_clip.clip_count(), 0);
        assert_eq!(no_clip.selected_clip(), None);
        assert_eq!(no_clip.selected_clip_duration(), 0.0);
        no_clip.set_playing(true);
        assert!(!no_clip.playing());
        no_clip.update(-1.0).unwrap();

        let mut empty_asset = import_glb(&canonical).unwrap();
        empty_asset.primitives.clear();
        assert!(
            GlbScene::new(empty_asset, &[TextureId(1)])
                .unwrap_err()
                .to_string()
                .contains("bounds")
        );
        let mut flat_asset = import_glb(&canonical).unwrap();
        let flat_vertices = flat_asset.primitives[0]
            .mesh
            .vertices()
            .iter()
            .cloned()
            .map(|mut vertex| {
                vertex.position_object = Vec3::ZERO;
                vertex
            })
            .collect();
        flat_asset.primitives[0].mesh = Mesh::new(
            flat_vertices,
            flat_asset.primitives[0].mesh.indices().to_vec(),
        )
        .unwrap();
        flat_asset.primitives[0].skin_index = None;
        flat_asset.primitives[0].skin_vertices = None;
        assert!(
            GlbScene::new(flat_asset, &[TextureId(1)])
                .unwrap_err()
                .to_string()
                .contains("bounding box")
        );

        let mut singular_joint =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        singular_joint.nodes[1].base_pose.scale = Vec3::ZERO;
        assert!(
            singular_joint
                .evaluate_pose()
                .unwrap_err()
                .to_string()
                .contains("joint normal")
        );
        let mut singular_root =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        singular_root.root_transform = Mat4::scale(Vec3::ZERO);
        assert!(
            singular_root
                .evaluate_pose()
                .unwrap_err()
                .to_string()
                .contains("normalization")
        );
        let mut singular_node =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        singular_node.primitives[0].asset.skin_index = None;
        singular_node.primitives[0].asset.skin_vertices = None;
        singular_node.nodes[2].base_pose.scale = Vec3::ZERO;
        assert!(
            singular_node
                .evaluate_pose()
                .unwrap_err()
                .to_string()
                .contains("node normal")
        );
        let mut zero_normal =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        zero_normal.primitives[0]
            .asset
            .skin_vertices
            .as_mut()
            .unwrap()[0]
            .weights = [0.0; 4];
        assert!(
            zero_normal
                .evaluate_pose()
                .unwrap_err()
                .to_string()
                .contains("평가된 normal")
        );
        let mut mirrored = GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        mirrored.primitives[0].asset.skin_index = None;
        mirrored.primitives[0].asset.skin_vertices = None;
        mirrored.nodes[2].base_pose.scale = Vec3::new(-1.0, 1.0, 1.0);
        mirrored.evaluate_pose().unwrap();
        assert!(mirrored.primitives()[0].winding_reversed());
        let mut mirrored_skinned =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        mirrored_skinned.nodes[2].base_pose.scale = Vec3::new(-1.0, 1.0, 1.0);
        mirrored_skinned.evaluate_pose().unwrap();
        assert!(!mirrored_skinned.primitives()[0].winding_reversed());
        let mut reflected_skin =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        reflected_skin.nodes[1].base_pose.scale = Vec3::new(-1.0, 1.0, 1.0);
        assert!(
            reflected_skin
                .evaluate_pose()
                .unwrap_err()
                .to_string()
                .contains("negative-determinant")
        );

        let mut palette_overflow =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        palette_overflow.nodes[1].base_pose.scale = Vec3::new(f32::MAX, 1.0, 1.0);
        palette_overflow.skins[0].inverse_bind_matrices[0] =
            Mat4::scale(Vec3::new(f32::MAX, 1.0, 1.0));
        assert!(
            palette_overflow
                .evaluate_pose()
                .unwrap_err()
                .to_string()
                .contains("joint palette matrix")
        );

        let mut world_overflow =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        world_overflow.primitives[0].asset.skin_index = None;
        world_overflow.primitives[0].asset.skin_vertices = None;
        world_overflow.root_transform = Mat4::identity();
        world_overflow.nodes[2].base_pose.scale = Vec3::new(-f32::MAX, 1.0, 1.0);
        world_overflow.nodes[2].base_pose.translation = Vec3::new(f32::MAX, 0.0, 0.0);
        assert!(
            world_overflow
                .evaluate_pose()
                .unwrap_err()
                .to_string()
                .contains("world position")
        );

        let mut normalized_overflow =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        normalized_overflow.primitives[0].asset.skin_index = None;
        normalized_overflow.primitives[0].asset.skin_vertices = None;
        normalized_overflow.root_transform = Mat4::scale(Vec3::new(2.0, 1.0, 1.0));
        normalized_overflow.nodes[2].base_pose.translation = Vec3::new(f32::MAX, 0.0, 0.0);
        assert!(
            normalized_overflow
                .evaluate_pose()
                .unwrap_err()
                .to_string()
                .contains("정규화된 position")
        );

        let mut animated_overflow =
            GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        for channel in &mut animated_overflow.clips[0].channels {
            if let ChannelValues::Translation(values) = &mut channel.values {
                values[1] = Vec3::new(f32::MAX, 0.0, 0.0);
            }
        }
        animated_overflow.clips[0].channels.push(AnimationChannel {
            node_index: 3,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0],
            values: ChannelValues::Translation(vec![Vec3::ZERO, Vec3::new(f32::MAX, 0.0, 0.0)]),
        });
        animated_overflow.seek(0.0).unwrap();
        animated_overflow.set_looping(false);
        let overflow_vertices = animated_overflow.primitives()[0].vertices().to_vec();
        let animated_overflow_error = animated_overflow.update(1.0).unwrap_err().to_string();
        assert!(
            animated_overflow_error.contains("유한 범위"),
            "unexpected animation overflow error: {animated_overflow_error}"
        );
        assert_eq!(animated_overflow.time_seconds(), 0.0);
        assert_eq!(
            animated_overflow.primitives()[0].vertices(),
            overflow_vertices
        );

        let mut atomic = GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        let original_vertices = atomic.primitives()[0].vertices().to_vec();
        for channel in &mut atomic.clips[0].channels {
            if let ChannelValues::Scale(values) = &mut channel.values {
                values[4] = Vec3::new(-1.0, 1.0, 1.0);
            }
        }
        assert!(atomic.seek(1.0).is_err());
        assert_eq!(atomic.time_seconds(), 0.0);
        assert_eq!(atomic.selected_clip(), Some(0));
        assert!(atomic.playing());
        assert_eq!(atomic.primitives()[0].vertices(), original_vertices);
        atomic.set_looping(false);
        assert!(atomic.update(1.0).is_err());
        assert_eq!(atomic.time_seconds(), 0.0);
        assert_eq!(atomic.primitives()[0].vertices(), original_vertices);

        let mut invalid_clip = atomic.clips[0].clone();
        invalid_clip.name = "Reflected".into();
        for channel in &mut invalid_clip.channels {
            if let ChannelValues::Scale(values) = &mut channel.values {
                values[1] = Vec3::new(-1.0, 1.0, 1.0);
            }
        }
        atomic.clips.push(invalid_clip);
        assert!(atomic.set_clip(1).is_err());
        assert_eq!(atomic.selected_clip(), Some(0));
        assert_eq!(atomic.time_seconds(), 0.0);
        assert_eq!(atomic.primitives()[0].vertices(), original_vertices);

        let mut cyclic = GlbScene::new(import_glb(&canonical).unwrap(), &[TextureId(1)]).unwrap();
        cyclic.nodes[0].parent = Some(1);
        cyclic.nodes[1].parent = Some(0);
        assert!(cyclic.evaluate_pose().is_err());
        scene.clips[0].duration = 0.0;
        scene.playing = true;
        scene.update(1.0).unwrap();
        scene.clips[0].duration = f32::MAX;
        scene.seek(f32::MAX).unwrap();
        let before_overflow = scene.primitives()[0].vertices().to_vec();
        assert!(
            scene
                .update(f32::MAX)
                .unwrap_err()
                .to_string()
                .contains("결과 시간은 유한")
        );
        assert_eq!(scene.time_seconds(), f32::MAX);
        assert_eq!(scene.primitives()[0].vertices(), before_overflow);
        for invalid_dt in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                scene
                    .update(invalid_dt)
                    .unwrap_err()
                    .to_string()
                    .contains("dt는 유한")
            );
        }

        let identity_pose = NodePose {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::new(1.0, 1.0, 1.0),
        };
        let cyclic_nodes = vec![
            Node {
                parent: Some(1),
                base_pose: identity_pose,
                pose: identity_pose,
            },
            Node {
                parent: Some(0),
                base_pose: identity_pose,
                pose: identity_pose,
            },
        ];
        let mut globals = vec![Mat4::identity(); 2];
        let mut states = vec![0; 2];
        let mut visit_stack = Vec::new();
        assert!(
            compute_globals(&cyclic_nodes, &mut globals, &mut states, &mut visit_stack).is_err()
        );

        let parent_after_child_nodes = vec![
            Node {
                parent: Some(1),
                base_pose: identity_pose,
                pose: identity_pose,
            },
            Node {
                parent: None,
                base_pose: identity_pose,
                pose: identity_pose,
            },
        ];
        let mut globals = vec![Mat4::identity(); 2];
        let mut states = vec![0; 2];
        compute_globals(
            &parent_after_child_nodes,
            &mut globals,
            &mut states,
            &mut visit_stack,
        )
        .unwrap();
        assert_eq!(states, vec![2, 2]);

        let overflow_nodes = vec![
            Node {
                parent: None,
                base_pose: NodePose {
                    translation: Vec3::new(f32::MAX, 0.0, 0.0),
                    ..identity_pose
                },
                pose: NodePose {
                    translation: Vec3::new(f32::MAX, 0.0, 0.0),
                    ..identity_pose
                },
            },
            Node {
                parent: Some(0),
                base_pose: NodePose {
                    translation: Vec3::new(f32::MAX, 0.0, 0.0),
                    ..identity_pose
                },
                pose: NodePose {
                    translation: Vec3::new(f32::MAX, 0.0, 0.0),
                    ..identity_pose
                },
            },
        ];
        assert!(
            compute_globals(&overflow_nodes, &mut globals, &mut states, &mut visit_stack)
                .unwrap_err()
                .to_string()
                .contains("node global transform")
        );

        let deep_nodes = (0..MAX_GLB_NODES)
            .map(|index| Node {
                parent: index.checked_sub(1),
                base_pose: identity_pose,
                pose: identity_pose,
            })
            .collect::<Vec<_>>();
        let mut deep_globals = vec![Mat4::identity(); deep_nodes.len()];
        let mut deep_states = vec![0; deep_nodes.len()];
        let mut deep_stack = Vec::with_capacity(deep_nodes.len());
        compute_globals(
            &deep_nodes,
            &mut deep_globals,
            &mut deep_states,
            &mut deep_stack,
        )
        .unwrap();
        assert!(deep_states.into_iter().all(|state| state == 2));
        assert_eq!(deep_globals.last(), Some(&Mat4::identity()));

        let step_rotation = AnimationChannel {
            node_index: 0,
            interpolation: Interpolation::Step,
            times: vec![0.0, 1.0],
            values: ChannelValues::Rotation(vec![Quat::IDENTITY; 2]),
        };
        assert_eq!(
            sample_quat(&step_rotation, &[Quat::IDENTITY; 2], 0.5),
            Quat::IDENTITY
        );
        assert!(!build_glb("{}".into(), vec![1]).is_empty());
        assert_eq!(declared_animation_totals([2, 3]), Ok((2, 5)));
        assert!(declared_animation_totals([MAX_GLB_KEYFRAMES + 1]).is_err());
        assert!(
            declared_animation_totals(std::iter::repeat_n(0, MAX_GLB_ANIMATION_CHANNELS + 1))
                .is_err()
        );
        let mut total = usize::MAX;
        assert!(advance_limited(&mut total, 1, usize::MAX, "overflow").is_err());
        let vertex_limit = import_glb(&geometry_limit_glb(MAX_GLB_VERTICES + 2, None));
        assert!(
            matches!(
                vertex_limit,
                Err(GlbImportError::LimitExceeded { kind: "vertex", .. })
            ),
            "{vertex_limit:?}"
        );
        let triangle_limit = import_glb(&geometry_limit_glb(3, Some((MAX_GLB_TRIANGLES + 1) * 3)));
        assert!(
            matches!(
                triangle_limit,
                Err(GlbImportError::LimitExceeded {
                    kind: "triangle",
                    ..
                })
            ),
            "{triangle_limit:?}"
        );
    }
}
