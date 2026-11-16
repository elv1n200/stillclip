//! Local Discord exports: budget for the entire selection, then check the
//! finished MP4 before publishing it. Never truncate a video to meet a limit.
use crate::{capture::command, export::Snapshot, recorder};
use std::{fs, path::Path};

const AUDIO_BPS: u64 = 128_000;
const MIN_VIDEO_BPS: u64 = 240_000;
const MAX_VIDEO_BPS: u64 = 12_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Free,
    Nitro,
}

impl Profile {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "discord_free" => Some(Self::Free),
            "discord_nitro" => Some(Self::Nitro),
            _ => None,
        }
    }

    // Decimal bytes are conservative even when a service counts MiB/GiB.
    // Discord File Attachments FAQ, checked 2026-11-16: free 20 MB, Nitro 1 GB.
    pub fn limit_bytes(self) -> u64 {
        match self {
            Self::Free => 20_000_000,
            Self::Nitro => 1_000_000_000,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Free => crate::i18n::text("Discord ohne Nitro (20 MB)", "Discord Free (20 MB)"),
            Self::Nitro => "Discord Nitro (1 GB)",
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Self::Free => "Discord",
            Self::Nitro => "Discord Nitro",
        }
    }
}

#[derive(Debug)]
struct Plan {
    video_bps: u64,
    width: u32,
    height: u32,
    fps: u32,
}

impl Plan {
    fn new(limit: u64, seconds: f64, audio: bool) -> Result<Self, String> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(crate::i18n::text("Wähle einen gültigen Clipbereich.", "Choose a valid clip range.").into());
        }
        // Leave five percent plus 64 kB for MP4 tables, AAC padding and encoder
        // variance. The final file, not this estimate, decides success.
        let usable = limit.saturating_sub(limit / 20 + 64_000);
        let total_bps = (usable as f64 * 8.0 / seconds) as u64;
        Self::with_video_bps(total_bps.saturating_sub(if audio { AUDIO_BPS } else { 0 }))
    }

    fn with_video_bps(video_bps: u64) -> Result<Self, String> {
        if video_bps < MIN_VIDEO_BPS {
            return Err(crate::i18n::text("Der Ausschnitt ist für dieses Dateilimit zu lang. Wähle einen kürzeren Bereich oder einen Export ohne Dateilimit.", "The selection is too long for this file size limit. Choose a shorter range or an export without a size limit.").into());
        }
        let video_bps = video_bps.min(MAX_VIDEO_BPS);
        let (width, height, fps) = match video_bps {
            8_000_000.. => (1920, 1080, 60),
            4_000_000.. => (1280, 720, 60),
            2_000_000.. => (1280, 720, 30),
            900_000.. => (854, 480, 30),
            _ => (640, 360, 30),
        };
        Ok(Self {
            video_bps,
            width,
            height,
            fps,
        })
    }

    fn filter(&self) -> String {
        // Fit inside the budget's resolution without upscaling, stretching or
        // increasing the source frame rate; x264 needs even dimensions.
        let ratio = format!(
            "min(1,if(gte(iw,ih),min({}/iw,{}/ih),min({}/iw,{}/ih)))",
            self.width, self.height, self.height, self.width
        );
        format!(
            "scale=w='max(2,trunc(iw*{ratio}/2)*2)':h='max(2,trunc(ih*{ratio}/2)*2)',fps=fps='if(isnan(source_fps),30,min(source_fps,{}))'",
            self.fps
        )
    }
}

pub fn export_clip(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    start: f64,
    end: f64,
    profile: Profile,
) -> Result<u64, String> {
    export_with_limit(ffmpeg, input, output, start, end, profile.limit_bytes())
}

pub fn export_controlled(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    start: f64,
    end: f64,
    profile: Profile,
    crop: Option<crate::portrait::Crop>,
    control: &crate::jobs::Control,
) -> Result<u64, String> {
    encode_with_limit(
        ffmpeg,
        input,
        output,
        start,
        end,
        profile.limit_bytes(),
        crop,
        control,
    )
}

fn export_with_limit(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    start: f64,
    end: f64,
    limit: u64,
) -> Result<u64, String> {
    encode_with_limit(
        ffmpeg,
        input,
        output,
        start,
        end,
        limit,
        None,
        &crate::jobs::Control::new(),
    )
}

fn encode_with_limit(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    start: f64,
    end: f64,
    limit: u64,
    crop: Option<crate::portrait::Crop>,
    control: &crate::jobs::Control,
) -> Result<u64, String> {
    control.check()?;
    let crop = crop.map(|c| c.validate()).transpose()?;
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
        return Err(crate::i18n::text("Wähle einen gültigen Clipbereich mit Ende nach dem Anfang.", "Choose a valid clip range with the end after the start.").into());
    }
    if output.exists() {
        return Err(crate::i18n::text("Die Zieldatei existiert bereits. Wähle einen neuen Dateinamen.", "The output file already exists. Choose a different filename.").into());
    }
    let info = recorder::probe_media(ffmpeg, input)?;
    if start >= info.duration || end > info.duration + 0.1 {
        return Err(crate::i18n::text("Der Clipbereich liegt außerhalb des Videos.", "The selected range is outside the video.").into());
    }
    let seconds = end.min(info.duration) - start;
    let mut plan = Plan::new(limit, seconds, info.audio)?;
    let workspace = Snapshot::new(output.parent().ok_or(crate::i18n::text("Exportordner fehlt.", "Export folder is missing."))?, &[])?;
    let staged = workspace.dir.join("discord.mp4");
    let passlog = workspace.dir.join("video-pass");
    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(1, 4))
        .unwrap_or(2);

    for attempt in 0..2 {
        // Two-pass x264 provides predictable sizes across GPU vendors. Keep
        // worker threads bounded so a live recording retains CPU headroom.
        for pass in [1, 2] {
            let mut encode = command(ffmpeg);
            encode
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-y",
                    "-ss",
                ])
                .arg(format!("{start:.6}"))
                .arg("-i")
                .arg(input)
                .arg("-t")
                .arg(format!("{seconds:.6}"))
                .args([
                    "-map",
                    "0:v:0",
                    "-map_metadata",
                    "-1",
                    "-map_chapters",
                    "-1",
                    "-vf",
                ])
                .arg(crop.map_or_else(
                    || plan.filter(),
                    |c| format!("{},{}", c.filter(), plan.filter()),
                ))
                .args([
                    "-filter_threads",
                    "2",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-pix_fmt",
                    "yuv420p",
                    "-qmin",
                    "18",
                    "-b:v",
                ])
                .arg(plan.video_bps.to_string())
                .arg("-threads")
                .arg(threads.to_string())
                .arg("-pass:v")
                .arg(pass.to_string())
                .arg("-passlogfile:v")
                .arg(&passlog);
            // Pass statistics belong to video only; AAC is encoded once.
            if pass == 1 {
                encode.args(["-an", "-f", "null"]).arg(if cfg!(windows) {
                    "NUL"
                } else {
                    "/dev/null"
                });
            } else {
                // Stillclip's first audio track already mixes game and mic.
                // The optional second mic track must not be mixed in again.
                encode
                    .args(["-map", "0:a:0?", "-c:a", "aac", "-b:a"])
                    .arg(AUDIO_BPS.to_string())
                    .args([
                        "-ac",
                        "2",
                        "-ar",
                        "48000",
                        "-movflags",
                        "+faststart",
                        "-f",
                        "mp4",
                    ])
                    .arg(&staged);
            }
            let base = if attempt == 0 { 0.0 } else { 0.9 };
            let step = if attempt == 0 { 0.45 } else { 0.045 };
            let result = control.part(base + (pass - 1) as f64 * step, step).run(
                &mut encode,
                seconds,
                if pass == 1 {
                    crate::i18n::text("Discord: Video analysieren", "Discord: analyzing video")
                } else {
                    crate::i18n::text("Discord: Video komprimieren", "Discord: compressing video")
                },
            )?;
            if !result.status.success() {
                return Err(crate::tr_format!(
                    "Discord-Export fehlgeschlagen. {}", "Discord export failed. {}",
                    recorder::diagnostic(&result.stderr)
                ));
            }
        }

        let bytes = fs::metadata(&staged)
            .map_err(|e| crate::tr_format!("Exportdatei fehlt: {e}", "Export file is missing: {e}"))?
            .len();
        if bytes == 0 {
            return Err(crate::i18n::text("Der Export hat keine Videodaten erzeugt.", "The export produced no video data.").into());
        }
        if bytes < limit {
            let exported = recorder::probe_media(ffmpeg, &staged)?;
            if (exported.duration - seconds).abs() > 0.25 || exported.audio != info.audio {
                return Err(crate::i18n::text("Der Discord-Export enthält nicht den vollständigen Ausschnitt. Bitte erneut versuchen.", "The Discord export does not contain the full selection. Please try again.").into());
            }
            control.check()?;
            workspace.publish(&staged, output)?;
            return Ok(bytes);
        }
        // An unusually complex selection can overshoot even with two passes.
        // Retry once with extra room; never publish an oversized/partial file.
        if attempt == 0 {
            let reduced = (plan.video_bps as f64 * limit as f64 / bytes as f64 * 0.85) as u64;
            plan = Plan::with_video_bps(reduced)?;
        }
    }
    Err(crate::i18n::text("Der Clip passt noch nicht unter das Dateilimit. Wähle einen kürzeren Bereich oder einen Export ohne Dateilimit.", "The clip still exceeds the file size limit. Choose a shorter range or an export without a size limit.").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_include_audio_and_margin_and_adapt_to_length() {
        for profile in [Profile::Free, Profile::Nitro] {
            let limit = profile.limit_bytes();
            for seconds in [2.0, 30.0, 120.0] {
                let plan = Plan::new(limit, seconds, true).unwrap();
                let estimated = (plan.video_bps + AUDIO_BPS) as f64 * seconds / 8.0;
                assert!(estimated + 64_000.0 <= limit as f64 * 0.95);
                assert!(plan.video_bps <= MAX_VIDEO_BPS);
            }
        }
        let short = Plan::new(Profile::Free.limit_bytes(), 15.0, true).unwrap();
        let long = Plan::new(Profile::Free.limit_bytes(), 120.0, true).unwrap();
        assert!(long.height < short.height);
        assert!(long.fps < short.fps);
        let silent = Plan::new(Profile::Free.limit_bytes(), 120.0, false).unwrap();
        assert_eq!(silent.video_bps - long.video_bps, AUDIO_BPS);
        assert_eq!(Profile::Free.limit_bytes(), 20_000_000);
        assert_eq!(Profile::Nitro.limit_bytes(), 1_000_000_000);
    }

    #[test]
    fn impossible_budgets_and_invalid_lengths_fail_before_encoding() {
        for seconds in [0.0, -1.0, f64::NAN, f64::INFINITY, 86_400.0] {
            assert!(Plan::new(Profile::Free.limit_bytes(), seconds, true).is_err());
        }
        assert!(Plan::new(1, 2.0, false).is_err());
    }

    #[test]
    #[ignore = "requires local FFmpeg and FFprobe; generated video, never screen capture"]
    fn synthetic_discord_exports_fit_and_preserve_the_complete_selection() {
        let ffmpeg = recorder::find_ffmpeg().expect("local FFmpeg is required");
        let ffprobe = recorder::ffprobe_for(&ffmpeg).expect("local FFprobe is required");
        let root =
            std::env::temp_dir().join(format!("stillclip-discord-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.mp4");
        let result = command(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=960x540:rate=60",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:sample_rate=48000",
                "-t",
                "4",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-map",
                "2:a",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "15",
                "-c:a",
                "aac",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            recorder::diagnostic(&result.stderr)
        );
        let original = fs::read(&source).unwrap();

        // Test both real presets plus a deliberately tight budget to exercise
        // rate control (a tiny clip alone would fit either preset by chance).
        for (name, limit) in [
            ("free", Profile::Free.limit_bytes()),
            ("nitro", Profile::Nitro.limit_bytes()),
            ("tight", 250_000),
        ] {
            let output = root.join(format!("{name}.mp4"));
            let _ = fs::remove_file(&output);
            let bytes = export_with_limit(&ffmpeg, &source, &output, 1.0, 3.5, limit).unwrap();
            assert!(bytes > 0 && bytes < limit, "{name}: {bytes} / {limit}");
            let info = recorder::probe_media(&ffmpeg, &output).unwrap();
            assert!((info.duration - 2.5).abs() < 0.1, "{}", info.duration);
            assert_eq!(
                (info.width, info.height),
                if name == "tight" {
                    (640, 360)
                } else {
                    (960, 540)
                }
            );
            assert!(info.audio);
            let metadata = command(&ffprobe)
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=codec_type,codec_name,avg_frame_rate",
                    "-of",
                    "json",
                ])
                .arg(&output)
                .output()
                .unwrap();
            let streams: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
            let streams = streams["streams"].as_array().unwrap();
            assert_eq!(streams.len(), 2, "one video and one mixed audio track");
            assert_eq!(streams[0]["codec_name"], "h264");
            assert_eq!(
                streams[0]["avg_frame_rate"],
                if name == "tight" { "30/1" } else { "60/1" },
                "adapt frame rate to the budget"
            );
            assert_eq!(streams[1]["codec_name"], "aac");
            let decoded = command(&ffmpeg)
                .args(["-v", "error", "-xerror", "-i"])
                .arg(&output)
                .args(["-f", "null", "-"])
                .output()
                .unwrap();
            assert!(
                decoded.status.success(),
                "{}",
                recorder::diagnostic(&decoded.stderr)
            );
            let saved = fs::read(&output).unwrap();
            assert!(export_with_limit(&ffmpeg, &source, &output, 1.0, 3.5, limit).is_err());
            assert_eq!(fs::read(&output).unwrap(), saved, "never overwrite");
        }
        // Silent imports work too, and low source frame rates stay unchanged.
        let silent = root.join("silent.mp4");
        let result = command(&ffmpeg)
            .args(["-v", "error", "-y", "-i"])
            .arg(&source)
            .args([
                "-an",
                "-vf",
                "fps=24",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ])
            .arg(&silent)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            recorder::diagnostic(&result.stderr)
        );
        let output = root.join("silent-discord.mp4");
        let _ = fs::remove_file(&output);
        export_clip(&ffmpeg, &silent, &output, 1.0, 3.5, Profile::Free).unwrap();
        let info = recorder::probe_media(&ffmpeg, &output).unwrap();
        assert!(!info.audio);
        assert!((info.duration - 2.5).abs() < 0.1);
        let rate = command(&ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=avg_frame_rate",
                "-of",
                "csv=p=0",
            ])
            .arg(&output)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&rate.stdout).trim(), "24/1");
        let rejected = root.join("too-long.mp4");
        assert!(export_with_limit(&ffmpeg, &source, &rejected, 0.0, 4.0, 100_000).is_err());
        assert!(!rejected.exists());
        assert_eq!(
            fs::read(&source).unwrap(),
            original,
            "never change the original"
        );
        assert_eq!(
            fs::read_dir(root.join(".stillclip-exports"))
                .unwrap()
                .count(),
            0
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
