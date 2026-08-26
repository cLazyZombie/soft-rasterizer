//! RGBA8 저장 색과 19장의 linear sampling/material shader 상태를 묶는 texture 저장소.

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{
    color::srgb_decode_rgba,
    math::{Vec2, Vec3, Vec4},
};

/// 한 texture가 소유할 수 있는 최대 texel 수다.
pub const MAX_TEXTURE_PIXEL_COUNT: usize = 16_777_216;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextureColorSpace {
    Srgb,
    Linear,
}

impl TextureColorSpace {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Srgb => "sRGB base color",
            Self::Linear => "linear data",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AddressMode {
    #[default]
    Repeat,
    ClampToEdge,
}

impl AddressMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Repeat => "repeat",
            Self::ClampToEdge => "clamp-to-edge",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FilterMode {
    #[default]
    Nearest,
    Bilinear,
}

impl FilterMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Bilinear => "bilinear",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SamplerState {
    pub address_u: AddressMode,
    pub address_v: AddressMode,
    pub filter: FilterMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NormalMode {
    #[default]
    Smooth,
    Flat,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShaderMode {
    #[default]
    Unlit,
    Lambert,
    BlinnPhong,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AlphaMode {
    #[default]
    Opaque,
    Mask,
    Blend,
}

impl AlphaMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Opaque => "opaque",
            Self::Mask => "mask",
            Self::Blend => "blend",
        }
    }
}

impl ShaderMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unlit => "unlit",
            Self::Lambert => "Lambert",
            Self::BlinnPhong => "Blinn-Phong",
        }
    }
}

impl NormalMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Smooth => "smooth",
            Self::Flat => "flat",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub base_color_texture: Option<TextureId>,
    pub sampler: SamplerState,
    pub base_color: Vec4,
    pub ambient: f32,
    pub shader_mode: ShaderMode,
    pub normal_mode: NormalMode,
    pub specular_color: Vec3,
    pub shininess: f32,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color_texture: None,
            sampler: SamplerState::default(),
            base_color: Vec4::new(1.0, 1.0, 1.0, 1.0),
            ambient: 0.18,
            shader_mode: ShaderMode::Unlit,
            normal_mode: NormalMode::Smooth,
            specular_color: Vec3::new(1.0, 1.0, 1.0),
            shininess: 32.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
        }
    }
}

impl SamplerState {
    pub fn sample(self, texture: &Texture, uv: Vec2) -> Option<Vec4> {
        self.sample_with_decode(texture, uv, true)
    }

    /// 잘못된 "encoded 값 filter" 비교 view에서만 사용하는 교육용 경로다.
    pub fn sample_encoded(self, texture: &Texture, uv: Vec2) -> Option<Vec4> {
        self.sample_with_decode(texture, uv, false)
    }

    fn sample_with_decode(self, texture: &Texture, uv: Vec2, decode: bool) -> Option<Vec4> {
        if !uv.x.is_finite() || !uv.y.is_finite() {
            return None;
        }
        match self.filter {
            FilterMode::Nearest => self.sample_nearest(texture, uv, decode),
            FilterMode::Bilinear => self.sample_bilinear(texture, uv, decode),
        }
    }

    fn sample_nearest(self, texture: &Texture, uv: Vec2, decode: bool) -> Option<Vec4> {
        let u = address_normalized(uv.x, self.address_u);
        let v = address_normalized(uv.y, self.address_v);
        let x = ((u * texture.width as f32).floor() as usize).min(texture.width - 1);
        let y = ((v * texture.height as f32).floor() as usize).min(texture.height - 1);
        texture.fetch_for_sampling(x, y, decode)
    }

    fn sample_bilinear(self, texture: &Texture, uv: Vec2, decode: bool) -> Option<Vec4> {
        let u = address_normalized(uv.x, self.address_u);
        let v = address_normalized(uv.y, self.address_v);
        let x = u * texture.width as f32 - 0.5;
        let y = v * texture.height as f32 - 0.5;
        let x0 = x.floor() as i64;
        let y0 = y.floor() as i64;
        let fraction_x = x - x.floor();
        let fraction_y = y - y.floor();
        let x0_index = address_texel(x0, texture.width, self.address_u);
        let x1_index = address_texel(x0 + 1, texture.width, self.address_u);
        let y0_index = address_texel(y0, texture.height, self.address_v);
        let y1_index = address_texel(y0 + 1, texture.height, self.address_v);
        let top = lerp_vec4(
            texture.fetch_for_sampling(x0_index, y0_index, decode)?,
            texture.fetch_for_sampling(x1_index, y0_index, decode)?,
            fraction_x,
        );
        let bottom = lerp_vec4(
            texture.fetch_for_sampling(x0_index, y1_index, decode)?,
            texture.fetch_for_sampling(x1_index, y1_index, decode)?,
            fraction_x,
        );
        Some(lerp_vec4(top, bottom, fraction_y))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextureId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextureError {
    ZeroDimension,
    DimensionOverflow,
    PixelLimitExceeded { requested: usize, maximum: usize },
    ByteLengthMismatch { expected: usize, actual: usize },
    TextureIdExhausted,
    InvalidTextureId(TextureId),
}

impl Display for TextureError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroDimension => formatter.write_str("texture 크기는 0보다 커야 합니다"),
            Self::DimensionOverflow => {
                formatter.write_str("texture 크기 계산에서 정수 overflow가 발생했습니다")
            }
            Self::PixelLimitExceeded { requested, maximum } => write!(
                formatter,
                "texture texel 수 {requested}이 최대 허용치 {maximum}을 넘었습니다"
            ),
            Self::ByteLengthMismatch { expected, actual } => write!(
                formatter,
                "texture RGBA byte 길이는 {expected}이어야 하지만 {actual}입니다"
            ),
            Self::TextureIdExhausted => formatter.write_str("texture ID 공간을 모두 사용했습니다"),
            Self::InvalidTextureId(id) => write!(formatter, "유효하지 않은 texture ID {}", id.0),
        }
    }
}

impl Error for TextureError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Texture {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    color_space: TextureColorSpace,
}

impl Texture {
    pub fn from_rgba8(
        width: usize,
        height: usize,
        pixels: &[u8],
        color_space: TextureColorSpace,
    ) -> Result<Self, TextureError> {
        let expected = checked_texture_byte_len(width, height)?;
        if pixels.len() != expected {
            return Err(TextureError::ByteLengthMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels: pixels.to_vec(),
            color_space,
        })
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub const fn color_space(&self) -> TextureColorSpace {
        self.color_space
    }

    pub fn texel_rgba8(&self, x: usize, y: usize) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let byte_index = 4 * (y * self.width + x);
        Some(self.pixels[byte_index..byte_index + 4].try_into().unwrap())
    }

    pub fn fetch(&self, x: usize, y: usize) -> Option<Vec4> {
        self.fetch_for_sampling(x, y, true)
    }

    pub fn fetch_encoded(&self, x: usize, y: usize) -> Option<Vec4> {
        self.fetch_for_sampling(x, y, false)
    }

    fn fetch_for_sampling(&self, x: usize, y: usize, decode: bool) -> Option<Vec4> {
        self.texel_rgba8(x, y).map(|rgba| {
            let encoded = Vec4::new(
                f32::from(rgba[0]) / 255.0,
                f32::from(rgba[1]) / 255.0,
                f32::from(rgba[2]) / 255.0,
                f32::from(rgba[3]) / 255.0,
            );
            if decode && self.color_space == TextureColorSpace::Srgb {
                srgb_decode_rgba(encoded)
            } else {
                encoded
            }
        })
    }
}

#[derive(Debug)]
pub struct TextureStore {
    textures: Vec<Texture>,
}

impl Default for TextureStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TextureStore {
    pub fn new() -> Self {
        let fallback = Texture::from_rgba8(
            2,
            2,
            &[
                255, 0, 255, 255, 24, 24, 24, 255, 24, 24, 24, 255, 255, 0, 255, 255,
            ],
            TextureColorSpace::Srgb,
        )
        .expect("내장 checkerboard fallback texture는 유효해야 한다");
        Self {
            textures: vec![fallback],
        }
    }

    pub const fn fallback_id(&self) -> TextureId {
        TextureId(0)
    }

    pub fn upload_rgba8(
        &mut self,
        width: usize,
        height: usize,
        pixels: &[u8],
        color_space: TextureColorSpace,
    ) -> Result<TextureId, TextureError> {
        let id = texture_id_for_len(self.textures.len())?;
        let texture = Texture::from_rgba8(width, height, pixels, color_space)?;
        self.textures.push(texture);
        Ok(id)
    }

    pub fn get(&self, id: TextureId) -> Result<&Texture, TextureError> {
        self.textures
            .get(id.0 as usize)
            .ok_or(TextureError::InvalidTextureId(id))
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

fn checked_texture_byte_len(width: usize, height: usize) -> Result<usize, TextureError> {
    if width == 0 || height == 0 {
        return Err(TextureError::ZeroDimension);
    }
    let pixel_count = width
        .checked_mul(height)
        .ok_or(TextureError::DimensionOverflow)?;
    if pixel_count > MAX_TEXTURE_PIXEL_COUNT {
        return Err(TextureError::PixelLimitExceeded {
            requested: pixel_count,
            maximum: MAX_TEXTURE_PIXEL_COUNT,
        });
    }
    pixel_count
        .checked_mul(4)
        .ok_or(TextureError::DimensionOverflow)
}

fn texture_id_for_len(texture_count: usize) -> Result<TextureId, TextureError> {
    u32::try_from(texture_count)
        .map(TextureId)
        .map_err(|_| TextureError::TextureIdExhausted)
}

fn address_normalized(value: f32, mode: AddressMode) -> f32 {
    match mode {
        AddressMode::Repeat => value - value.floor(),
        AddressMode::ClampToEdge => value.clamp(0.0, 1.0),
    }
}

fn address_texel(index: i64, extent: usize, mode: AddressMode) -> usize {
    let extent = extent as i64;
    match mode {
        AddressMode::Repeat => index.rem_euclid(extent) as usize,
        AddressMode::ClampToEdge => index.clamp(0, extent - 1) as usize,
    }
}

fn lerp_vec4(first: Vec4, second: Vec4, amount: f32) -> Vec4 {
    first * (1.0 - amount) + second * amount
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORNERS: [u8; 16] = [
        255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0,
    ];

    #[test]
    fn texture_owns_top_to_bottom_rgba_bytes_and_metadata() {
        let mut source = CORNERS;
        let texture = Texture::from_rgba8(2, 2, &source, TextureColorSpace::Linear).unwrap();
        source.fill(0);

        assert_eq!(texture.width(), 2);
        assert_eq!(texture.height(), 2);
        assert_eq!(texture.color_space(), TextureColorSpace::Linear);
        assert_eq!(texture.pixels(), CORNERS);
        assert_eq!(texture.texel_rgba8(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(texture.texel_rgba8(1, 0), Some([0, 255, 0, 128]));
        assert_eq!(texture.texel_rgba8(0, 1), Some([0, 0, 255, 64]));
        assert_eq!(texture.texel_rgba8(1, 1), Some([255, 255, 255, 0]));
        assert_eq!(texture.texel_rgba8(2, 0), None);
        assert_eq!(texture.texel_rgba8(0, 2), None);
        assert_eq!(texture.fetch(0, 0), Some(Vec4::new(1.0, 0.0, 0.0, 1.0)));
        assert_eq!(texture.fetch(2, 0), None);
    }

    #[test]
    fn texture_validation_reports_every_size_and_length_failure() {
        assert_eq!(
            Texture::from_rgba8(0, 1, &[], TextureColorSpace::Srgb),
            Err(TextureError::ZeroDimension)
        );
        assert_eq!(
            Texture::from_rgba8(usize::MAX, 2, &[], TextureColorSpace::Srgb),
            Err(TextureError::DimensionOverflow)
        );
        assert_eq!(
            Texture::from_rgba8(MAX_TEXTURE_PIXEL_COUNT + 1, 1, &[], TextureColorSpace::Srgb),
            Err(TextureError::PixelLimitExceeded {
                requested: MAX_TEXTURE_PIXEL_COUNT + 1,
                maximum: MAX_TEXTURE_PIXEL_COUNT,
            })
        );
        for actual in [CORNERS.len() - 1, CORNERS.len() + 1] {
            assert_eq!(
                Texture::from_rgba8(2, 2, &vec![0; actual], TextureColorSpace::Srgb),
                Err(TextureError::ByteLengthMismatch {
                    expected: CORNERS.len(),
                    actual,
                })
            );
        }
    }

    #[test]
    fn store_keeps_fallback_and_assigns_stable_monotonic_ids() {
        let mut store = TextureStore::default();
        assert_eq!(store.fallback_id(), TextureId(0));
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
        assert_eq!(store.get(TextureId(0)).unwrap().width(), 2);

        let first = store
            .upload_rgba8(2, 2, &CORNERS, TextureColorSpace::Srgb)
            .unwrap();
        let second = store
            .upload_rgba8(1, 1, &[1, 2, 3, 4], TextureColorSpace::Linear)
            .unwrap();
        assert_eq!(
            (first, second, store.len()),
            (TextureId(1), TextureId(2), 3)
        );
        assert_eq!(store.get(second).unwrap().pixels(), [1, 2, 3, 4]);
        assert_eq!(
            store.get(TextureId(99)),
            Err(TextureError::InvalidTextureId(TextureId(99)))
        );
    }

    #[test]
    fn failed_upload_does_not_consume_an_id_or_change_existing_textures() {
        let mut store = TextureStore::new();
        assert!(
            store
                .upload_rgba8(2, 2, &CORNERS[..15], TextureColorSpace::Srgb)
                .is_err()
        );
        let id = store
            .upload_rgba8(1, 1, &[9, 8, 7, 6], TextureColorSpace::Srgb)
            .unwrap();
        assert_eq!(id, TextureId(1));
        assert_eq!(store.get(TextureId(0)).unwrap().pixels()[0], 255);
    }

    #[test]
    fn texture_errors_have_actionable_messages() {
        let errors = [
            TextureError::ZeroDimension,
            TextureError::DimensionOverflow,
            TextureError::PixelLimitExceeded {
                requested: 10,
                maximum: 4,
            },
            TextureError::ByteLengthMismatch {
                expected: 4,
                actual: 3,
            },
            TextureError::TextureIdExhausted,
            TextureError::InvalidTextureId(TextureId(7)),
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
            let as_error: &dyn Error = &error;
            assert!(as_error.source().is_none());
        }
    }

    #[test]
    fn texture_id_conversion_reports_native_counts_beyond_u32() {
        assert_eq!(texture_id_for_len(7), Ok(TextureId(7)));
        let too_many = (u32::MAX as usize)
            .checked_add(1)
            .expect("coverage native target는 64-bit usize를 사용해야 한다");
        assert_eq!(
            texture_id_for_len(too_many),
            Err(TextureError::TextureIdExhausted)
        );
    }

    fn sampler(filter: FilterMode, address_u: AddressMode, address_v: AddressMode) -> SamplerState {
        SamplerState {
            address_u,
            address_v,
            filter,
        }
    }

    fn corner_texture() -> Texture {
        Texture::from_rgba8(2, 2, &CORNERS, TextureColorSpace::Srgb).unwrap()
    }

    fn assert_vec4_close(actual: Vec4, expected: Vec4) {
        for (actual, expected) in [
            (actual.x, expected.x),
            (actual.y, expected.y),
            (actual.z, expected.z),
            (actual.w, expected.w),
        ] {
            assert!(
                (actual - expected).abs() <= 1.0e-6,
                "{actual} != {expected}"
            );
        }
    }

    #[test]
    fn nearest_sampling_fixes_corners_negative_repeat_and_large_clamp() {
        let texture = corner_texture();
        let repeat = sampler(
            FilterMode::Nearest,
            AddressMode::Repeat,
            AddressMode::Repeat,
        );
        let clamp = sampler(
            FilterMode::Nearest,
            AddressMode::ClampToEdge,
            AddressMode::ClampToEdge,
        );
        assert_vec4_close(
            repeat.sample(&texture, Vec2::new(0.0, 0.0)).unwrap(),
            texture.fetch(0, 0).unwrap(),
        );
        assert_vec4_close(
            repeat.sample(&texture, Vec2::new(0.75, 0.25)).unwrap(),
            texture.fetch(1, 0).unwrap(),
        );
        assert_vec4_close(
            repeat.sample(&texture, Vec2::new(-0.25, 0.25)).unwrap(),
            texture.fetch(1, 0).unwrap(),
        );
        assert_vec4_close(
            repeat.sample(&texture, Vec2::new(0.25, 0.75)).unwrap(),
            texture.fetch(0, 1).unwrap(),
        );
        assert_vec4_close(
            clamp.sample(&texture, Vec2::new(1.0, 1.0)).unwrap(),
            texture.fetch(1, 1).unwrap(),
        );
        assert_vec4_close(
            clamp.sample(&texture, Vec2::new(1.0e20, -1.0e20)).unwrap(),
            texture.fetch(1, 0).unwrap(),
        );
        assert_eq!(repeat.sample(&texture, Vec2::new(f32::NAN, 0.0)), None);
        assert_eq!(repeat.sample(&texture, Vec2::new(0.0, f32::INFINITY)), None);
    }

    #[test]
    fn bilinear_center_is_four_texel_average_and_edges_obey_address_modes() {
        let texture = Texture::from_rgba8(
            2,
            2,
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
            TextureColorSpace::Srgb,
        )
        .unwrap();
        let repeat = sampler(
            FilterMode::Bilinear,
            AddressMode::Repeat,
            AddressMode::Repeat,
        );
        let clamp = sampler(
            FilterMode::Bilinear,
            AddressMode::ClampToEdge,
            AddressMode::ClampToEdge,
        );
        assert_vec4_close(
            repeat.sample(&texture, Vec2::new(0.5, 0.5)).unwrap(),
            Vec4::new(0.5, 0.5, 0.5, 1.0),
        );
        assert_vec4_close(
            clamp.sample(&texture, Vec2::new(0.0, 0.0)).unwrap(),
            Vec4::new(1.0, 0.0, 0.0, 1.0),
        );
        assert_vec4_close(
            repeat.sample(&texture, Vec2::new(0.0, 0.0)).unwrap(),
            Vec4::new(0.5, 0.5, 0.5, 1.0),
        );
    }

    #[test]
    fn srgb_texels_decode_before_bilinear_filtering_while_linear_data_stays_raw() {
        let srgb = Texture::from_rgba8(
            2,
            1,
            &[0, 0, 0, 64, 255, 255, 255, 192],
            TextureColorSpace::Srgb,
        )
        .unwrap();
        let linear =
            Texture::from_rgba8(1, 1, &[128, 64, 32, 16], TextureColorSpace::Linear).unwrap();
        let bilinear = sampler(
            FilterMode::Bilinear,
            AddressMode::ClampToEdge,
            AddressMode::ClampToEdge,
        );
        assert_vec4_close(
            bilinear.sample(&srgb, Vec2::new(0.5, 0.5)).unwrap(),
            Vec4::new(0.5, 0.5, 0.5, 128.0 / 255.0),
        );
        let correct_display = crate::color::srgb_encode_channel(0.5);
        assert!((correct_display - 0.735_357).abs() <= 1.0e-6);
        assert!((correct_display - 0.5).abs() > 0.2);
        assert_vec4_close(
            bilinear.sample_encoded(&srgb, Vec2::new(0.5, 0.5)).unwrap(),
            Vec4::new(0.5, 0.5, 0.5, 128.0 / 255.0),
        );
        assert_vec4_close(
            SamplerState::default()
                .sample(&linear, Vec2::new(0.5, 0.5))
                .unwrap(),
            Vec4::new(128.0 / 255.0, 64.0 / 255.0, 32.0 / 255.0, 16.0 / 255.0),
        );
        assert_eq!(srgb.fetch_encoded(0, 0), srgb.fetch(0, 0));
        assert_eq!(TextureColorSpace::Srgb.label(), "sRGB base color");
        assert_eq!(TextureColorSpace::Linear.label(), "linear data");
    }

    #[test]
    fn one_by_one_and_non_power_of_two_textures_are_safe_for_every_sampler() {
        let one = Texture::from_rgba8(1, 1, &[12, 34, 56, 78], TextureColorSpace::Linear).unwrap();
        let non_power = Texture::from_rgba8(3, 2, &[128; 24], TextureColorSpace::Linear).unwrap();
        let expected = one.fetch(0, 0).unwrap();
        for filter in [FilterMode::Nearest, FilterMode::Bilinear] {
            for address in [AddressMode::Repeat, AddressMode::ClampToEdge] {
                let state = sampler(filter, address, address);
                assert_vec4_close(
                    state.sample(&one, Vec2::new(-99.25, 123.75)).unwrap(),
                    expected,
                );
                assert_vec4_close(
                    state.sample(&non_power, Vec2::new(0.37, 0.61)).unwrap(),
                    Vec4::new(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0),
                );
            }
        }
        assert_eq!(AddressMode::Repeat.label(), "repeat");
        assert_eq!(AddressMode::ClampToEdge.label(), "clamp-to-edge");
        assert_eq!(FilterMode::Nearest.label(), "nearest");
        assert_eq!(FilterMode::Bilinear.label(), "bilinear");
    }

    #[test]
    fn shader_normal_mode_labels_and_material_defaults_are_stable() {
        assert_eq!(NormalMode::Smooth.label(), "smooth");
        assert_eq!(NormalMode::Flat.label(), "flat");
        let material = Material::default();
        assert_eq!(material.base_color_texture, None);
        assert_eq!(material.base_color, Vec4::new(1.0, 1.0, 1.0, 1.0));
        assert_eq!(material.ambient, 0.18);
        assert_eq!(ShaderMode::Unlit.label(), "unlit");
        assert_eq!(ShaderMode::Lambert.label(), "Lambert");
        assert_eq!(ShaderMode::BlinnPhong.label(), "Blinn-Phong");
        assert_eq!(material.shader_mode, ShaderMode::Unlit);
        assert_eq!(material.alpha_mode, AlphaMode::Opaque);
        assert_eq!(material.alpha_cutoff, 0.5);
        assert_eq!(AlphaMode::Opaque.label(), "opaque");
        assert_eq!(AlphaMode::Mask.label(), "mask");
        assert_eq!(AlphaMode::Blend.label(), "blend");
        assert_eq!(material.normal_mode, NormalMode::Smooth);
        assert_eq!(material.specular_color, Vec3::new(1.0, 1.0, 1.0));
        assert_eq!(material.shininess, 32.0);
    }
}
