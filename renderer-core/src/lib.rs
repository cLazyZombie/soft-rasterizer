//! Browser APIs에 의존하지 않는 소프트웨어 래스터라이저의 순수 Rust 코어.
//!
//! 2장에서는 브라우저 타입 없이 프레임 입력을 받고, 한 번의 호출로 렌더링한다.

use std::error::Error;
use std::fmt::{Display, Formatter};

/// 4096 × 4096 RGBA8와 깊이 버퍼까지만 허용한다.
pub const MAX_PIXEL_COUNT: usize = 16_777_216;
const MAX_FRAME_DT_SECONDS: f32 = 0.1;
const BACKGROUND_CYCLE_SECONDS: f32 = 2.0;

/// 렌더 타깃 생성 또는 크기 변경이 거부된 이유다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderTargetError {
    ZeroDimension,
    DimensionOverflow,
    PixelLimitExceeded { requested: usize, maximum: usize },
}

impl Display for RenderTargetError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroDimension => formatter.write_str("렌더 타깃 크기는 0보다 커야 합니다"),
            Self::DimensionOverflow => {
                formatter.write_str("렌더 타깃 크기 계산에서 정수 overflow가 발생했습니다")
            }
            Self::PixelLimitExceeded { requested, maximum } => write!(
                formatter,
                "렌더 타깃 픽셀 수 {requested}이 최대 허용치 {maximum}을 넘었습니다"
            ),
        }
    }
}

impl Error for RenderTargetError {}

/// 같은 픽셀 인덱스를 공유하는 RGBA8 색 버퍼와 f32 깊이 버퍼다.
#[derive(Debug)]
pub struct RenderTarget {
    width: usize,
    height: usize,
    color: Vec<u8>,
    depth: Vec<f32>,
}

impl RenderTarget {
    pub fn new(width: usize, height: usize) -> Result<Self, RenderTargetError> {
        let (pixel_count, color_byte_count) = checked_buffer_lengths(width, height)?;
        Ok(Self {
            width,
            height,
            color: vec![0; color_byte_count],
            depth: vec![f32::INFINITY; pixel_count],
        })
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub fn color(&self) -> &[u8] {
        &self.color
    }

    pub fn depth(&self) -> &[f32] {
        &self.depth
    }

    pub fn clear(&mut self, rgb: [u8; 3]) {
        for pixel in self.color.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        self.depth.fill(f32::INFINITY);
    }
}

/// 한 프레임의 작은 값만 복사하는 통계 snapshot이다.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    pub frame_index: u32,
    pub dt_seconds: f32,
    pub input_bits: u32,
    pub input_vertices: u32,
    pub input_triangles: u32,
    pub clipped_triangles: u32,
    pub rasterized_triangles: u32,
    pub shaded_samples: u32,
    pub invalid_values: u32,
}

/// 아직 의미를 부여하지 않은 장치 입력을 한 프레임 단위로 전달하는 작은 값이다.
///
/// 실제 키/포인터 비트 배치는 입력 카메라를 구현하는 20장에서 정한다.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputSnapshot {
    packed_bits: u32,
}

impl InputSnapshot {
    pub const fn from_packed(packed_bits: u32) -> Self {
        Self { packed_bits }
    }

    pub const fn packed_bits(self) -> u32 {
        self.packed_bits
    }
}

/// 렌더 타깃과 시간 상태를 소유하는 2장 Renderer다.
#[derive(Debug)]
pub struct Renderer {
    target: RenderTarget,
    elapsed_seconds: f32,
    stats: FrameStats,
    framebuffer_generation: u32,
}

impl Renderer {
    pub fn new(width: usize, height: usize) -> Result<Self, RenderTargetError> {
        let mut renderer = Self {
            target: RenderTarget::new(width, height)?,
            elapsed_seconds: 0.0,
            stats: FrameStats::default(),
            framebuffer_generation: 0,
        };
        renderer.target.clear(background_rgb(0.0));
        Ok(renderer)
    }

    pub fn resize(&mut self, width: usize, height: usize) -> Result<(), RenderTargetError> {
        let mut replacement = RenderTarget::new(width, height)?;
        replacement.clear(background_rgb(self.elapsed_seconds));
        self.target = replacement;
        self.framebuffer_generation = self.framebuffer_generation.wrapping_add(1);
        Ok(())
    }

    pub fn update_and_render(&mut self, dt_seconds: f32, input: InputSnapshot) -> FrameStats {
        let (dt_seconds, invalid_dt) = sanitize_dt(dt_seconds);
        self.elapsed_seconds =
            (self.elapsed_seconds + dt_seconds).rem_euclid(BACKGROUND_CYCLE_SECONDS);
        self.target.clear(background_rgb(self.elapsed_seconds));
        self.stats = FrameStats {
            frame_index: self.stats.frame_index.wrapping_add(1),
            dt_seconds,
            input_bits: input.packed_bits(),
            invalid_values: u32::from(invalid_dt),
            ..FrameStats::default()
        };
        self.stats
    }

    pub fn clear(&mut self, rgb: [u8; 3]) {
        self.target.clear(rgb);
    }

    pub const fn width(&self) -> usize {
        self.target.width()
    }

    pub const fn height(&self) -> usize {
        self.target.height()
    }

    pub fn color_buffer(&self) -> &[u8] {
        self.target.color()
    }

    pub fn depth_buffer(&self) -> &[f32] {
        self.target.depth()
    }

    pub const fn stats(&self) -> FrameStats {
        self.stats
    }

    pub const fn framebuffer_generation(&self) -> u32 {
        self.framebuffer_generation
    }
}

fn checked_buffer_lengths(
    width: usize,
    height: usize,
) -> Result<(usize, usize), RenderTargetError> {
    if width == 0 || height == 0 {
        return Err(RenderTargetError::ZeroDimension);
    }
    let pixel_count = width
        .checked_mul(height)
        .ok_or(RenderTargetError::DimensionOverflow)?;
    if pixel_count > MAX_PIXEL_COUNT {
        return Err(RenderTargetError::PixelLimitExceeded {
            requested: pixel_count,
            maximum: MAX_PIXEL_COUNT,
        });
    }
    let color_byte_count = pixel_count
        .checked_mul(4)
        .ok_or(RenderTargetError::DimensionOverflow)?;
    Ok((pixel_count, color_byte_count))
}

fn sanitize_dt(dt_seconds: f32) -> (f32, bool) {
    if dt_seconds.is_finite() {
        (dt_seconds.clamp(0.0, MAX_FRAME_DT_SECONDS), false)
    } else {
        (0.0, true)
    }
}

fn background_rgb(elapsed_seconds: f32) -> [u8; 3] {
    let phase = elapsed_seconds.rem_euclid(BACKGROUND_CYCLE_SECONDS);
    let triangle_wave = if phase <= 1.0 { phase } else { 2.0 - phase };
    let green = 48.0 + triangle_wave * 96.0;
    [24, green.round() as u8, 88]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_by_two_target_has_expected_lengths_and_clear_values() {
        let mut target = RenderTarget::new(3, 2).expect("3x2 target should be valid");
        assert_eq!((target.width(), target.height()), (3, 2));
        assert_eq!(target.color().len(), 24);
        assert_eq!(target.depth().len(), 6);

        target.clear([7, 11, 13]);
        assert!(
            target
                .color()
                .chunks_exact(4)
                .all(|pixel| pixel == [7, 11, 13, 255])
        );
        assert!(target.depth().iter().all(|depth| *depth == f32::INFINITY));
    }

    #[test]
    fn dimensions_reject_zero_overflow_and_excessive_targets() {
        assert_eq!(
            RenderTarget::new(0, 1).unwrap_err(),
            RenderTargetError::ZeroDimension
        );
        assert_eq!(
            RenderTarget::new(1, 0).unwrap_err(),
            RenderTargetError::ZeroDimension
        );
        assert_eq!(
            RenderTarget::new(usize::MAX, 2).unwrap_err(),
            RenderTargetError::DimensionOverflow
        );

        let error = RenderTarget::new(MAX_PIXEL_COUNT + 1, 1).unwrap_err();
        assert_eq!(
            error,
            RenderTargetError::PixelLimitExceeded {
                requested: MAX_PIXEL_COUNT + 1,
                maximum: MAX_PIXEL_COUNT,
            }
        );
        assert!(error.to_string().contains("최대 허용치"));
        assert!(
            RenderTargetError::DimensionOverflow
                .to_string()
                .contains("overflow")
        );
        assert!(
            RenderTargetError::ZeroDimension
                .to_string()
                .contains("0보다")
        );
    }

    #[test]
    fn resize_is_atomic_and_increments_generation_only_on_success() {
        let mut renderer = Renderer::new(3, 2).expect("renderer should be valid");
        renderer.clear([1, 2, 3]);

        let error = renderer.resize(MAX_PIXEL_COUNT + 1, 1).unwrap_err();
        assert!(matches!(
            error,
            RenderTargetError::PixelLimitExceeded { .. }
        ));
        assert_eq!((renderer.width(), renderer.height()), (3, 2));
        assert_eq!(renderer.color_buffer()[..4], [1, 2, 3, 255]);
        assert_eq!(renderer.framebuffer_generation(), 0);

        renderer.resize(2, 4).expect("resize should succeed");
        assert_eq!((renderer.width(), renderer.height()), (2, 4));
        assert_eq!(renderer.color_buffer().len(), 32);
        assert_eq!(renderer.depth_buffer().len(), 8);
        assert!(
            renderer
                .depth_buffer()
                .iter()
                .all(|depth| *depth == f32::INFINITY)
        );
        assert_eq!(renderer.framebuffer_generation(), 1);
    }

    #[test]
    fn frame_clamps_dt_resets_stage_counts_and_animates_clear_color() {
        let mut renderer = Renderer::new(1, 1).expect("renderer should be valid");
        assert_eq!(renderer.color_buffer(), [24, 48, 88, 255]);

        let first = renderer.update_and_render(0.25, InputSnapshot::from_packed(0xa5));
        assert_eq!(first.frame_index, 1);
        assert_eq!(first.dt_seconds, 0.1);
        assert_eq!(first.input_bits, 0xa5);
        assert_eq!(first.input_vertices, 0);
        assert_eq!(first.input_triangles, 0);
        assert_eq!(first.clipped_triangles, 0);
        assert_eq!(first.rasterized_triangles, 0);
        assert_eq!(first.shaded_samples, 0);
        assert_eq!(first.invalid_values, 0);
        assert_eq!(renderer.stats(), first);
        assert_eq!(renderer.color_buffer(), [24, 58, 88, 255]);

        let negative = renderer.update_and_render(-1.0, InputSnapshot::default());
        assert_eq!(negative.dt_seconds, 0.0);
        assert_eq!(negative.invalid_values, 0);
        for invalid_dt in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let stats = renderer.update_and_render(invalid_dt, InputSnapshot::default());
            assert_eq!(stats.dt_seconds, 0.0);
            assert_eq!(stats.invalid_values, 1);
        }
    }

    #[test]
    fn background_triangle_wave_reverses_and_wraps() {
        assert_eq!(background_rgb(1.0), [24, 144, 88]);
        assert_eq!(background_rgb(1.5), [24, 96, 88]);
        assert_eq!(background_rgb(2.0), [24, 48, 88]);
    }

    #[test]
    fn input_snapshot_round_trips_all_packed_bits() {
        let snapshot = InputSnapshot::from_packed(u32::MAX);
        assert_eq!(snapshot.packed_bits(), u32::MAX);
        assert_eq!(InputSnapshot::default().packed_bits(), 0);
    }
}
