//! Assemble selected ranges in the requested order. All intermediates are
//! private; originals and existing destination files are never overwritten.
use crate::{capture::command, discord, export::Snapshot, mechanics, recorder};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const MAX_PIECES: usize = 20;
pub struct Piece {
    pub crop: Option<crate::portrait::Crop>,
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
    pub markers: Vec<f64>,
}

pub fn export(
    ffmpeg: &Path,
    pieces: &[Piece],
    output: &Path,
    profile: Option<discord::Profile>,
) -> Result<Vec<f64>, String> {
    export_controlled(
        ffmpeg,
        pieces,
        output,
        profile,
        None,
        &crate::jobs::Control::new(),
    )
}

pub fn export_controlled(
    ffmpeg: &Path,
    pieces: &[Piece],
    output: &Path,
    profile: Option<discord::Profile>,
    portrait_width: Option<u32>,
    control: &crate::jobs::Control,
) -> Result<Vec<f64>, String> {
    control.check()?;
    if portrait_width.is_some_and(|w| ![720, 1080].contains(&w)) {
        return Err(crate::i18n::text("Ungültiges Hochkantformat.", "Invalid portrait format.").into());
    }
    if pieces.is_empty() || pieces.len() > MAX_PIECES {
        return Err(crate::tr_format!(
            "Wähle zwischen einem und {MAX_PIECES} Ausschnitten.", "Choose between one and {MAX_PIECES} excerpts."
        ));
    }
    if output.exists() {
        return Err(crate::i18n::text("Die Zieldatei existiert bereits.", "The output file already exists.").into());
    }
    // Validate the entire request before doing any expensive work.
    let ranges: Vec<_> = pieces
        .iter()
        .map(|piece| {
            control.check()?;
            if let Some(crop) = piece.crop {
                crop.validate()?;
            }
            let info = recorder::probe_media(ffmpeg, &piece.path)?;
            let (start, end) = mechanics::editor_range(piece.start, piece.end, info.duration)
                .ok_or(crate::i18n::text("Ein Highlight-Ausschnitt ist ungültig oder zu kurz.", "A highlight excerpt is invalid or too short."))?;
            Ok((start, end, info.audio))
        })
        .collect::<Result<_, String>>()?;
    let total: f64 = ranges.iter().map(|(s, e, _)| e - s).sum();
    if total > 1800.0 {
        return Err(crate::i18n::text("Ein Highlight darf höchstens 30 Minuten lang sein.", "A highlight must be no longer than 30 minutes.").into());
    }
    let workspace = Snapshot::new(output.parent().ok_or(crate::i18n::text("Exportordner fehlt.", "Export folder is missing."))?, &[])?;
    let mut concat = String::from("ffconcat version 1.0\n");
    let mut markers = Vec::new();
    let mut offset = 0.0;
    for (index, (piece, (start, end, audio))) in pieces.iter().zip(&ranges).enumerate() {
        control.check()?;
        let name = format!("part-{index:02}.mp4");
        let part = workspace.dir.join(&name);
        let mut encode = command(ffmpeg);
        encode
            .args(["-hide_banner", "-v", "error", "-nostdin", "-n", "-ss"])
            .arg(format!("{start:.6}"))
            .arg("-i")
            .arg(&piece.path);
        if !audio {
            encode.args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo"]);
        }
        let filter = if let Some(width) = portrait_width {
            let crop = crate::portrait::Crop {
                width,
                ..piece.crop.unwrap_or(crate::portrait::Crop {
                    x: 0.5,
                    y: 0.5,
                    width,
                })
            };
            format!("{},fps=60,setpts=PTS-STARTPTS", crop.filter())
        } else {
            "scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=60,setpts=PTS-STARTPTS".into()
        };
        encode
            .arg("-t")
            .arg(format!("{:.6}", end - start))
            .args([
                "-map",
                "0:v:0",
                "-map",
                if *audio { "0:a:0" } else { "1:a:0" },
                "-map_metadata",
                "-1",
                "-map_chapters",
                "-1",
                "-filter_threads",
                "2",
                "-vf",
                &filter,
                "-af",
                "aresample=48000,asetpts=PTS-STARTPTS,apad",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "20",
                "-threads",
                "4",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-ar",
                "48000",
                "-ac",
                "2",
                "-video_track_timescale",
                "90000",
            ])
            .arg(&part);
        let result = control
            .part(
                index as f64 / pieces.len() as f64 * 0.8,
                0.8 / pieces.len() as f64,
            )
            .run(
                &mut encode,
                end - start,
                &format!("Ausschnitt {} / {}", index + 1, pieces.len()),
            )?;
        if !result.status.success() {
            return Err(crate::tr_format!(
                "Ausschnitt {} konnte nicht exportiert werden: {}", "Could not export excerpt {}: {}",
                index + 1,
                recorder::diagnostic(&result.stderr)
            ));
        }
        let length = recorder::probe_duration(ffmpeg, &part)?;
        if (length - (end - start)).abs() > 0.25 {
            return Err(crate::i18n::text("Ein Highlight-Ausschnitt wurde nicht vollständig exportiert.", "A highlight excerpt was not exported in full.").into());
        }
        markers.extend(
            piece
                .markers
                .iter()
                .filter(|m| m.is_finite() && **m >= *start && **m <= *end)
                .map(|m| offset + m - start),
        );
        offset += length;
        // Filenames are generated above, never user input or external paths.
        concat.push_str(&format!("file '{name}'\n"));
    }
    let list = workspace.dir.join("parts.txt");
    fs::write(&list, concat).map_err(|e| format!("Highlight-Liste: {e}"))?;
    let joined = workspace.dir.join("highlight.mp4");
    let mut join = command(ffmpeg);
    join.args([
        "-hide_banner",
        "-v",
        "error",
        "-nostdin",
        "-n",
        "-f",
        "concat",
        "-safe",
        "1",
        "-i",
    ])
    .arg(&list)
    .args([
        "-map",
        "0:v:0",
        "-map",
        "0:a:0",
        "-c",
        "copy",
        "-movflags",
        "+faststart",
    ])
    .arg(&joined);
    let result = control
        .part(0.8, 0.05)
        .run(&mut join, offset, crate::i18n::text("Ausschnitte verbinden", "Joining excerpts"))?;
    if !result.status.success() {
        return Err(crate::tr_format!(
            "Highlights konnten nicht verbunden werden: {}", "Could not join highlights: {}",
            recorder::diagnostic(&result.stderr)
        ));
    }
    let duration = recorder::probe_duration(ffmpeg, &joined)?;
    if (duration - offset).abs() > 0.5 {
        return Err(crate::i18n::text("Der Highlight-Clip ist unvollständig.", "The highlight clip is incomplete.").into());
    }
    let staged = if let Some(profile) = profile {
        let compressed = workspace.dir.join("discord-highlight.mp4");
        discord::export_controlled(
            ffmpeg,
            &joined,
            &compressed,
            0.0,
            duration,
            profile,
            None,
            &control.part(0.85, 0.14),
        )?;
        compressed
    } else {
        joined
    };
    control.check()?;
    workspace.publish(&staged, output)?;
    Ok(markers)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires FFmpeg; generated video only"]
    fn synthetic_highlights_combine_mixed_sources_and_keep_markers() {
        let ffmpeg = recorder::find_ffmpeg().unwrap();
        let root =
            std::env::temp_dir().join(format!("stillclip-highlights-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut pieces = Vec::new();
        for (i, filter) in [
            "testsrc2=size=320x180:rate=24",
            "color=c=blue:size=180x320:rate=30",
        ]
        .iter()
        .enumerate()
        {
            let source = root.join(format!("source-{i}.mp4"));
            let mut cmd = command(&ffmpeg);
            cmd.args(["-v", "error", "-y", "-f", "lavfi", "-i", filter]);
            if i == 0 {
                cmd.args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"]);
            }
            let result = cmd
                .args([
                    "-t",
                    "3",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
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
            pieces.push(Piece {
                crop: None,
                path: source,
                start: 0.5,
                end: 2.5,
                markers: vec![0.1, 1.5],
            });
        }
        let original: Vec<_> = pieces.iter().map(|p| fs::read(&p.path).unwrap()).collect();
        let output = root.join("output.mp4");
        let _ = fs::remove_file(&output);
        let marks = export(&ffmpeg, &pieces, &output, Some(discord::Profile::Free)).unwrap();
        assert_eq!(marks.len(), 2);
        assert!((marks[0] - 1.0).abs() < 0.05);
        assert!((marks[1] - 3.0).abs() < 0.1);
        let info = recorder::probe_media(&ffmpeg, &output).unwrap();
        assert!((info.duration - 4.0).abs() < 0.2);
        assert!(info.audio);
        assert!(fs::metadata(&output).unwrap().len() < discord::Profile::Free.limit_bytes());
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
        assert!(export(&ffmpeg, &pieces, &output, None).is_err());
        for (piece, bytes) in pieces.iter().zip(original) {
            assert_eq!(fs::read(&piece.path).unwrap(), bytes);
        }
        assert_eq!(
            fs::read_dir(root.join(".stillclip-exports"))
                .unwrap()
                .count(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }
}
