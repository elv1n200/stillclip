//! Capture rules recovered from MedalEncoder.RecordingSession.CreateConfig.
//! See MECHANICS.md for the original RVAs, desktop defaults and engine differences.

/// The original desktop defaults: 1280×720 at 15 Mbit/s.
pub const CONFIGURED_WIDTH: u32 = 1280;
pub const CONFIGURED_HEIGHT: u32 = 720;
pub const BITRATE_MBPS: u32 = 15;

/// Default AspectRatio=2 preserves aspect by scaling to the configured height.
/// CalculateOutputResolution avoids upscaling and aligns encoder dimensions.
pub fn capture_size(input_width: u32, input_height: u32, configured_height: u32) -> (u32, u32) {
    let ratio = f64::from(configured_height) / f64::from(input_height.max(1));
    let (mut width, mut height) = (
        (f64::from(input_width) * ratio) as u32,
        (f64::from(input_height) * ratio) as u32,
    );
    if input_width <= width && input_height <= height {
        (width, height) = (input_width, input_height);
    }
    // Two pixels is our encoder safeguard, not a recovered GPU limit.
    (width.max(2) & !1, height.max(2) & !1)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaptureEncoding {
    pub bitrate: u32,
    pub max_bitrate: u32,
    pub buffer_size: u32,
    pub gop_frames: u32,
}

impl CaptureEncoding {
    /// `nominal = mbps * 1e6`, raised by the output/configured pixel-area ratio
    /// when above one; VBV and the ABR maximum are twice the nominal rate.
    pub fn for_output(
        width: u32,
        height: u32,
        fps: u32,
        configured_width: u32,
        configured_height: u32,
        mbps: u32,
    ) -> Self {
        let nominal = mbps.saturating_mul(1_000_000);
        let configured_area = (u64::from(configured_width) * u64::from(configured_height)).max(1);
        let area_ratio = (u64::from(width) * u64::from(height)) as f32 / configured_area as f32;
        let bitrate = if area_ratio > 1.0 {
            (nominal as f32 * area_ratio) as u32
        } else {
            nominal
        };
        Self {
            bitrate,
            // Config.get_AverageBitrateMaxRatio fallback is 2; no remote config here.
            max_bitrate: bitrate.saturating_mul(2),
            buffer_size: bitrate.saturating_mul(2),
            gop_frames: fps.saturating_mul(4) / 10,
        }
    }

    /// The original defaults, kept for the documented 720p formulas.
    #[cfg(test)]
    pub fn default_profile(width: u32, height: u32, fps: u32) -> Self {
        Self::for_output(
            width,
            height,
            fps,
            CONFIGURED_WIDTH,
            CONFIGURED_HEIGHT,
            BITRATE_MBPS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_rate_tracks_original_area_and_frame_formulas() {
        assert_eq!(
            CaptureEncoding::default_profile(1280, 720, 60),
            CaptureEncoding {
                bitrate: 15_000_000,
                max_bitrate: 30_000_000,
                buffer_size: 30_000_000,
                gop_frames: 24,
            }
        );
        assert_eq!(
            CaptureEncoding::default_profile(1920, 1080, 30),
            CaptureEncoding {
                bitrate: 33_750_000,
                max_bitrate: 67_500_000,
                buffer_size: 67_500_000,
                gop_frames: 12,
            }
        );
        assert_eq!(
            CaptureEncoding::default_profile(640, 360, 24).bitrate,
            15_000_000
        );
        assert_eq!(
            CaptureEncoding::default_profile(1280, 720, 24).gop_frames,
            9
        );
        // Other quality presets scale from their own configured size.
        assert_eq!(
            CaptureEncoding::for_output(2560, 1440, 60, 1920, 1080, 25).bitrate,
            44_444_444
        );
    }

    #[test]
    fn capture_size_preserves_aspect_without_upscaling() {
        assert_eq!(capture_size(1920, 1080, 720), (1280, 720));
        assert_eq!(capture_size(1600, 1200, 720), (960, 720));
        assert_eq!(capture_size(1920, 1200, 720), (1152, 720));
        assert_eq!(capture_size(3440, 1440, 720), (1720, 720));
        assert_eq!(capture_size(640, 360, 720), (640, 360));
        assert_eq!(capture_size(1365, 767, 720), (1280, 720));
        assert_eq!(capture_size(801, 601, 720), (800, 600));
        // Preserve the original floating-point multiply/truncate sequence.
        assert_eq!(capture_size(1920, 1142, 720), (1210, 718));
        assert_eq!(capture_size(2560, 1440, 1080), (1920, 1080));
        assert_eq!(capture_size(3840, 2160, 1440), (2560, 1440));
    }
}
