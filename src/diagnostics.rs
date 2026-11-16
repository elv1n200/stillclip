//! Read bounded tails of FFmpeg's progress log. Missing values stay unknown.
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Default, Debug, Serialize)]
pub struct Progress {
    pub fps: Option<f64>,
    pub speed: Option<f64>,
    pub frames: Option<u64>,
    pub dropped: Option<u64>,
    pub duplicated: Option<u64>,
    pub lag_seconds: Option<f64>,
    pub age_seconds: Option<u64>,
}

pub fn parse(text: &str, elapsed: f64) -> Progress {
    let mut result = Progress::default();
    let mut complete = Progress::default();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let number = value
            .trim()
            .trim_end_matches('x')
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0);
        match key {
            "fps" => result.fps = number,
            "speed" => result.speed = number,
            "frame" => result.frames = value.trim().parse().ok(),
            "drop_frames" => result.dropped = value.trim().parse().ok(),
            "dup_frames" => result.duplicated = value.trim().parse().ok(),
            "out_time_us" => {
                result.lag_seconds = number.map(|n| (elapsed - n / 1_000_000.0).max(0.0))
            }
            "progress" => {
                complete = result;
                result = Progress::default();
            }
            _ => {}
        }
    }
    complete
}

pub fn read(path: &Path, elapsed: f64) -> Option<Progress> {
    let mut file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    file.seek(SeekFrom::Start(metadata.len().saturating_sub(8192)))
        .ok()?;
    let mut text = String::new();
    file.take(8192).read_to_string(&mut text).ok()?;
    let mut progress = parse(&text, elapsed);
    progress.age_seconds = metadata
        .modified()
        .ok()?
        .elapsed()
        .ok()
        .map(|age| age.as_secs());
    Some(progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_complete_samples_are_reported_and_unknown_is_not_zero() {
        let p = parse("frame=120\nfps=60.0\nout_time_us=2000000\nspeed=1.0x\ndrop_frames=2\nprogress=continue\nframe=999\nfps=0", 4.0);
        assert_eq!(p.frames, Some(120));
        assert_eq!(p.fps, Some(60.0));
        assert_eq!(p.lag_seconds, Some(2.0));
        assert_eq!(p.dropped, Some(2));
        let unknown = parse("fps=N/A\nspeed=NaN\nprogress=continue\n", 0.0);
        assert_eq!(unknown.fps, None);
        assert_eq!(unknown.speed, None);
    }
}
