//! Recording settings recovered from the original; see ../MECHANICS.md for evidence.

pub const DEFAULT_REPLAY_SECONDS: u32 = 30;
pub const MIN_REPLAY_SECONDS: u32 = 15;
pub const MAX_REPLAY_SECONDS: u32 = 1200;
pub const REPLAY_PRESETS: [u32; 6] = [15, 30, 60, 120, 180, 300];
pub const DEFAULT_FPS: u32 = 60;
pub const MIN_EDITOR_SECONDS: f64 = 2.0;
pub const EDITOR_TOLERANCE_SECONDS: f64 = 0.000_001;
// FFmpeg's text duration fallback rounds to hundredths, unlike WebView metadata.
// This is a Stillclip adapter tolerance, not an original editor rule.
pub const PROBE_DURATION_TOLERANCE_SECONDS: f64 = 0.02;

// The minimum remains Stillclip's policy; 144 FPS is the highest recovered UI choice.
pub const MIN_FPS: u32 = 15;
pub const MAX_FPS: u32 = 144;

pub fn replay_seconds(value: u64) -> u32 {
    value.clamp(u64::from(MIN_REPLAY_SECONDS), u64::from(MAX_REPLAY_SECONDS)) as u32
}

pub fn fps(value: u64) -> u32 {
    value.clamp(u64::from(MIN_FPS), u64::from(MAX_FPS)) as u32
}

/// The editor's two-second minimum does not apply to short replay exports.
pub fn valid_editor_range(start: f64, end: f64, duration: f64) -> bool {
    start.is_finite()
        && end.is_finite()
        && duration.is_finite()
        && duration > 0.0
        && start >= 0.0
        && end > start
        && end <= duration + EDITOR_TOLERANCE_SECONDS
        && end - start + EDITOR_TOLERANCE_SECONDS >= MIN_EDITOR_SECONDS.min(duration)
}

/// Reconcile the browser's source endpoint with the backend's duration probe.
pub fn editor_range(start: f64, end: f64, duration: f64) -> Option<(f64, f64)> {
    if !start.is_finite()
        || !end.is_finite()
        || !duration.is_finite()
        || duration <= 0.0
        || start < 0.0
        || end <= start
    {
        return None;
    }
    let (start, end) = if (end - duration).abs()
        <= PROBE_DURATION_TOLERANCE_SECONDS + EDITOR_TOLERANCE_SECONDS
    {
        // If a valid two-second tail shifts slightly earlier, move its start too.
        // Only a selection already at least two seconds may move its start.
        let start = if end - start + EDITOR_TOLERANCE_SECONDS >= MIN_EDITOR_SECONDS {
            start.min((duration - MIN_EDITOR_SECONDS).max(0.0))
        } else {
            start
        };
        (start, duration)
    } else {
        (start, end)
    };
    valid_editor_range(start, end, duration).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_ranges_require_two_seconds_or_the_whole_short_video() {
        assert!(valid_editor_range(3.0, 5.0, 10.0));
        assert!(valid_editor_range(0.0, 1.25, 1.25));
        assert!(!valid_editor_range(3.0, 4.9, 10.0));
        assert!(!valid_editor_range(0.1, 1.25, 1.25));
        assert!(!valid_editor_range(-1.0, 2.0, 10.0));
        assert!(!valid_editor_range(8.1, 10.1, 10.0));
        assert!(!valid_editor_range(0.0, f64::NAN, 10.0));
    }

    #[test]
    fn editor_endpoints_reconcile_probe_rounding_without_short_selections() {
        assert_eq!(editor_range(0.0, 1.234, 1.23), Some((0.0, 1.23)));
        assert_eq!(editor_range(0.0, 1.234, 1.24), Some((0.0, 1.24)));
        assert_eq!(editor_range(8.005, 10.005, 10.0), Some((8.0, 10.0)));
        assert_eq!(editor_range(0.1, 1.234, 1.24), None);
        assert_eq!(editor_range(0.0, 1.9, 10.0), None);
        assert_eq!(editor_range(8.1, 10.005, 10.0), None);
        assert_eq!(editor_range(0.0, 1.234, 1.30), None);
        assert_eq!(editor_range(0.0, f64::NAN, 1.23), None);
    }
}
