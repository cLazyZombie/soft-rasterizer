//! 19장의 저장 색(sRGB)과 계산 색(linear) 경계를 고정한다.

use crate::math::Vec4;

/// 정규화된 sRGB 채널을 linear 밝기로 decode한다.
pub fn srgb_decode_channel(channel: f32) -> f32 {
    let channel = channel.clamp(0.0, 1.0);
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// 정규화된 linear 채널을 framebuffer용 sRGB 채널로 encode한다.
pub fn srgb_encode_channel(channel: f32) -> f32 {
    let channel = channel.clamp(0.0, 1.0);
    if channel <= 0.003_130_8 {
        12.92 * channel
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

/// RGB만 decode하고 alpha는 transfer function을 적용하지 않는다.
pub fn srgb_decode_rgba(color: Vec4) -> Vec4 {
    Vec4::new(
        srgb_decode_channel(color.x),
        srgb_decode_channel(color.y),
        srgb_decode_channel(color.z),
        color.w.clamp(0.0, 1.0),
    )
}

/// RGB만 encode하고 alpha는 linear coverage 값으로 유지한다.
pub fn srgb_encode_rgba(color: Vec4) -> Vec4 {
    Vec4::new(
        srgb_encode_channel(color.x),
        srgb_encode_channel(color.y),
        srgb_encode_channel(color.z),
        color.w.clamp(0.0, 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f32, expected: f32, epsilon: f32) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn srgb_reference_points_and_alpha_contract_are_exact() {
        assert_eq!(srgb_decode_channel(0.0), 0.0);
        assert_eq!(srgb_decode_channel(1.0), 1.0);
        assert_eq!(srgb_encode_channel(0.0), 0.0);
        close(srgb_encode_channel(1.0), 1.0, 1.0e-7);
        close(srgb_decode_channel(0.04045), 0.003_130_805, 1.0e-8);
        close(srgb_encode_channel(0.003_130_8), 0.040_449_936, 1.0e-8);

        let decoded = srgb_decode_rgba(Vec4::new(0.5, 0.25, 0.75, 0.37));
        assert_eq!(decoded.w, 0.37);
        assert_eq!(srgb_encode_rgba(decoded).w, 0.37);
    }

    #[test]
    fn every_rgba8_channel_round_trips_within_one_quantization_step() {
        for byte in 0_u16..=255 {
            let encoded = f32::from(byte) / 255.0;
            let round_trip = srgb_encode_channel(srgb_decode_channel(encoded));
            close(round_trip, encoded, 0.5 / 255.0);
        }
    }

    #[test]
    fn transfer_functions_clamp_finite_range_and_propagate_nan() {
        assert_eq!(srgb_decode_channel(-1.0), 0.0);
        assert_eq!(srgb_decode_channel(2.0), 1.0);
        assert_eq!(srgb_encode_channel(-1.0), 0.0);
        close(srgb_encode_channel(2.0), 1.0, 1.0e-7);
        assert!(srgb_decode_channel(f32::NAN).is_nan());
        assert!(srgb_encode_channel(f32::NAN).is_nan());
    }
}
