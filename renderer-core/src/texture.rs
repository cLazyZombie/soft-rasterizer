//! 16장의 브라우저 디코드 결과를 소유하는 RGBA8 texture 저장소.

use std::error::Error;
use std::fmt::{Display, Formatter};

/// 한 texture가 소유할 수 있는 최대 texel 수다.
pub const MAX_TEXTURE_PIXEL_COUNT: usize = 16_777_216;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextureColorSpace {
    Srgb,
    Linear,
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
}
