//! Automatic clips from the recorder's completed segments. A single, bounded
//! worker owns private hard links, so analysis never holds the recorder or
//! reads an unfinished segment. Capture/stop remain on their existing threads.
use crate::{
    export::Snapshot,
    i18n::text,
    jobs::Control,
    killfeed,
    recorder::{self, Segment},
    valorant, App, AppEvent,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};
use tao::event_loop::EventLoopProxy;

#[derive(Clone)]
pub struct Source {
    pub directory: PathBuf,
    pub list: PathBuf,
    pub clips: PathBuf,
    pub ffmpeg: PathBuf,
}
pub struct Spec {
    pub source: Source,
    pub preferences: valorant::Preferences,
    pub player: String,
    pub region: killfeed::Region,
}
pub enum Event {
    Progress(u64, &'static str, usize, f64, usize),
    Saved(u64, PathBuf, Vec<f64>, usize),
    Tail(u64, Result<Snapshot, String>),
    Finished(u64, Result<(), String>),
}
enum Command {
    Finish(Result<Snapshot, String>),
}
struct Worker {
    id: u64,
    control: Control,
    sender: mpsc::Sender<Command>,
}
#[derive(Default)]
pub struct State {
    worker: Option<Worker>,
    sequence: u64,
    phase: &'static str,
    saved: usize,
    detected: usize,
    skipped: usize,
    lag: f64,
    error: Option<String>,
}
impl State {
    pub fn running(&self) -> bool {
        self.worker.is_some()
    }
    pub fn tail_id(&self) -> Option<u64> {
        self.worker
            .as_ref()
            .filter(|w| !w.control.cancelled())
            .map(|w| w.id)
    }
    pub fn cancel(&mut self) {
        if let Some(worker) = &self.worker {
            worker.control.cancel();
            self.phase = "cancelling";
        }
    }
    pub fn state(&self) -> Value {
        json!({"running":self.running(),"phase":self.phase,"saved":self.saved,"detected":self.detected,
            "skipped":self.skipped,"lag_seconds":self.lag.round(),"error":self.error})
    }
    fn start(&mut self, spec: Spec, proxy: EventLoopProxy<AppEvent>) {
        if self.running() {
            return;
        }
        self.sequence += 1;
        let id = self.sequence;
        let control = Control::new();
        let (sender, receiver) = mpsc::channel();
        self.worker = Some(Worker {
            id,
            control: control.clone(),
            sender,
        });
        self.phase = "loading";
        self.saved = 0;
        self.detected = 0;
        self.skipped = 0;
        self.lag = 0.0;
        self.error = None;
        std::thread::spawn(move || {
            #[cfg(windows)]
            unsafe {
                windows_sys::Win32::System::Threading::SetThreadPriority(
                    windows_sys::Win32::System::Threading::GetCurrentThread(),
                    -1,
                );
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(spec, id, &receiver, &control, &mut |event| {
                    proxy
                        .send_event(AppEvent::AutoHighlights(event))
                        .map_err(|e| e.to_string())
                })
            }))
            .unwrap_or_else(|_| {
                Err(text(
                    "Die Highlight-Erkennung wurde unerwartet beendet.",
                    "Highlight detection stopped unexpectedly.",
                )
                .into())
            });
            let _ = proxy.send_event(AppEvent::AutoHighlights(Event::Finished(id, result)));
        });
    }
}

/// Read a bounded tail of the recorder-owned CSV; filenames are validated by
/// parse_segment_list. No desktop/client logs, credentials or network access.
fn completed(source: &Source) -> Result<Vec<Segment>, String> {
    let mut file = match fs::File::open(&source.list) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let offset = length.saturating_sub(256 * 1024);
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(256 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let content = String::from_utf8_lossy(&bytes);
    let content = if offset > 0 {
        content.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &content
    };
    // The recorder can still be appending the last CSV row. Only consume
    // newline-terminated records, even if a partial timestamp parses as a number.
    let content = content.rsplit_once('\n').map_or("", |(rows, _)| rows);
    Ok(recorder::parse_segment_list(content, &source.directory)
        .into_iter()
        .filter(|s| s.path.is_file())
        .collect())
}

fn preserve(archive: &mut Snapshot, source: &[Segment], latest: f64) -> Result<(), String> {
    for segment in source.iter().filter(|s| {
        s.end
            .is_some_and(|end| end >= latest - valorant::RETAIN_SECONDS as f64)
    }) {
        if archive.segments.iter().any(|s| s.index == segment.index) {
            continue;
        }
        let path = archive.dir.join(format!("seg-{:06}.ts", segment.index));
        match fs::hard_link(&segment.path, &path)
            .or_else(|_| fs::copy(&segment.path, &path).map(|_| ()))
        {
            Ok(()) => archive.segments.push(Segment {
                path,
                ..segment.clone()
            }),
            // The ring may expire a segment between listing and linking. Its
            // absence is surfaced as a gap rather than blocking the recording.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    archive.segments.sort_by_key(|s| s.index);
    archive.segments.retain(|s| {
        if s.end
            .is_some_and(|end| end < latest - valorant::RETAIN_SECONDS as f64)
        {
            let _ = fs::remove_file(&s.path);
            false
        } else {
            true
        }
    });
    Ok(())
}

fn save(
    source: &Source,
    archive: &Snapshot,
    moment: &valorant::Moment,
    control: &Control,
) -> Result<(PathBuf, Vec<f64>), String> {
    control.check()?;
    let segments = recorder::span_segments(&archive.segments, moment.start, moment.end);
    let first = segments.first().and_then(|s| s.start).ok_or(text(
        "Die Highlight-Szene ist nicht mehr im Puffer.",
        "The highlight is no longer in the buffer.",
    ))?;
    if first > moment.events[0].at {
        return Err(text(
            "Für das Highlight fehlen Videodaten.",
            "Video data for the highlight is missing.",
        )
        .into());
    }
    if segments
        .windows(2)
        .any(|pair| pair[1].start.unwrap_or(0.0) - pair[0].end.unwrap_or(0.0) > 0.15)
    {
        return Err(text(
            "Die Highlight-Aufnahme enthält eine Lücke.",
            "The highlight recording contains a gap.",
        )
        .into());
    }
    let snapshot = Snapshot::new(&source.clips, &segments)?;
    let staged = snapshot.dir.join("highlight.mp4");
    // A fast, cancellable stream copy keeps every audio track. A one-second
    // keyframe margin is deliberate; editor exports can trim more precisely.
    let from = recorder::export_span_controlled(
        &source.ffmpeg,
        None,
        &snapshot.segments,
        moment.start.max(first),
        moment.end,
        1.0,
        &staged,
        Some(control),
    )?;
    control.check()?;
    let path = recorder::clip_path(&source.clips, &format!("VALORANT {}", moment.label()));
    snapshot.publish(&staged, &path)?;
    let offsets = moment
        .events
        .iter()
        .map(|e| (e.at - from).max(0.0))
        .collect();
    Ok((path, offsets))
}

fn run(
    spec: Spec,
    id: u64,
    receiver: &mpsc::Receiver<Command>,
    control: &Control,
    emit: &mut impl FnMut(Event) -> Result<(), String>,
) -> Result<(), String> {
    control.check()?;
    killfeed::validate_name(&spec.player)?;
    let reader = killfeed::Reader::load()?;
    let mut tracker = valorant::Tracker::live();
    let mut rounds = valorant::Rounds::default();
    let mut moments = valorant::Moments::new(spec.preferences.clone());
    let mut archive = Snapshot::new(&spec.source.clips, &[])?;
    let mut cursor = None;
    let mut through = 0.0;
    let mut latest: f64 = 0.0;
    let mut finishing = false;
    let mut tail_error = None;
    let mut detected = 0;
    let mut skipped = 0;
    let mut published_phase = "";
    loop {
        control.check()?;
        if let Ok(Command::Finish(tail)) = receiver.try_recv() {
            finishing = true;
            match tail {
                Ok(tail) => {
                    latest = latest.max(
                        tail.segments
                            .iter()
                            .filter_map(|s| s.end)
                            .fold(0.0, f64::max),
                    );
                    preserve(&mut archive, &tail.segments, latest)?;
                }
                Err(error) => tail_error = Some(error),
            }
        }
        if !finishing {
            let segments = completed(&spec.source)?;
            latest = latest.max(segments.iter().filter_map(|s| s.end).fold(0.0, f64::max));
            preserve(&mut archive, &segments, latest)?;
        }
        // Keep latency and memory bounded even when a PC cannot keep up. Do not
        // count old feed lines as new kills after skipping an unanalyzed range.
        let next = archive
            .segments
            .iter()
            .filter(|s| cursor.is_none_or(|index| s.index > index))
            .find(|s| {
                s.end
                    .is_some_and(|end| end >= latest - valorant::MAX_LAG_SECONDS)
            })
            .cloned();
        let processed = next.is_some();
        if let Some(segment) = next {
            let begin = segment.start.unwrap_or(0.0);
            let end = segment.end.unwrap_or(begin);
            if begin - through > 1.0 {
                skipped += 1;
                tracker.reset_after_gap();
                rounds = valorant::Rounds::default();
                moments.boundary();
            }
            let length = end - begin;
            if !(0.01..=12.0).contains(&length) {
                return Err(text(
                    "Unerwartete Länge eines Aufnahmesegments.",
                    "Unexpected recording segment length.",
                )
                .into());
            }
            let frames = Snapshot::new(&spec.source.clips, &[])?;
            let paths = killfeed::decode_frames(
                &spec.source.ffmpeg,
                &segment.path,
                &frames.dir,
                0.0,
                length,
                control,
            )?;
            if paths.is_empty() {
                return Err(text(
                    "Im Aufnahmesegment sind keine lesbaren Bilder vorhanden.",
                    "The recording segment contains no readable frames.",
                )
                .into());
            }
            for (index, path) in paths.iter().enumerate() {
                control.check()?;
                let at = (begin + index as f64 * 0.5).min(end);
                let pixels = image::open(path).map_err(|e| e.to_string())?.into_rgb8();
                let observation = reader.observe(&pixels, &spec.player, spec.region)?;
                if rounds.frame(at, observation.score, observation.round_banner) {
                    moments.boundary();
                }
                for event in tracker.frame(at, &observation.rows) {
                    if event.confidence == valorant::Confidence::Confirmed {
                        detected += 1;
                        moments.event(event);
                    }
                }
                moments.advance(at);
                fs::remove_file(path).map_err(|e| e.to_string())?;
            }
            through = end;
            cursor = Some(segment.index);
        }
        let no_more = !archive.segments.iter().any(|s| {
            cursor.is_none_or(|index| s.index > index)
                && s.end
                    .is_some_and(|end| end >= latest - valorant::MAX_LAG_SECONDS)
        });
        for moment in moments.take_ready(latest, finishing && no_more) {
            if moment.confirmed_kills() < spec.preferences.min_kills as usize {
                continue;
            }
            emit(Event::Progress(
                id,
                "saving",
                detected,
                (latest - through).max(0.0),
                skipped,
            ))?;
            let (path, offsets) = save(&spec.source, &archive, &moment, control)?;
            emit(Event::Saved(id, path, offsets, moment.events.len()))?;
        }
        if finishing && no_more {
            return tail_error.map_or(Ok(()), Err);
        }
        let phase = if finishing {
            "finishing"
        } else if latest - through > 10.0 {
            "behind"
        } else {
            "watching"
        };
        if next_phase_changed(&mut published_phase, phase) || processed {
            emit(Event::Progress(
                id,
                phase,
                detected,
                (latest - through).max(0.0),
                skipped,
            ))?;
        }
        if no_more {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
fn next_phase_changed<'a>(previous: &mut &'a str, phase: &'a str) -> bool {
    let changed = *previous != phase;
    *previous = phase;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(root: &std::path::Path) -> Source {
        Source {
            directory: root.join("buffer"),
            list: root.join("buffer/segments.csv"),
            clips: root.to_owned(),
            ffmpeg: PathBuf::from("ffmpeg"),
        }
    }
    fn spec(source: Source) -> Spec {
        Spec {
            source,
            preferences: valorant::Preferences {
                enabled: true,
                ..Default::default()
            },
            player: "Elvin".into(),
            region: killfeed::Region::default(),
        }
    }
    #[test]
    fn cancelled_detector_does_not_load_models_or_read_recording_data() {
        let control = Control::new();
        control.cancel();
        let (_, receiver) = mpsc::channel();
        let result = run(
            spec(source(std::path::Path::new("missing"))),
            1,
            &receiver,
            &control,
            &mut |_| panic!("cancelled detector emitted an event"),
        );
        assert_eq!(result.unwrap_err(), crate::jobs::CANCELLED);
    }
    #[test]
    fn completed_segments_exclude_a_partially_written_csv_row() {
        let root = std::env::temp_dir().join(format!("stillclip-auto-csv-{}", std::process::id()));
        let source = source(&root);
        fs::create_dir_all(&source.directory).unwrap();
        for index in 0..2 {
            fs::write(source.directory.join(format!("seg-{index:06}.ts")), b"data").unwrap();
        }
        fs::write(&source.list, "seg-000000.ts,0.0,5.0\nseg-000001.ts,5.0,10.").unwrap();
        assert_eq!(completed(&source).unwrap().len(), 1);
        fs::write(
            &source.list,
            "seg-000000.ts,0.0,5.0\nseg-000001.ts,5.0,10.0\n",
        )
        .unwrap();
        assert_eq!(completed(&source).unwrap().len(), 2);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn archive_is_bounded_and_pins_segments_across_recorder_cleanup() {
        let root =
            std::env::temp_dir().join(format!("stillclip-auto-archive-{}", std::process::id()));
        fs::create_dir_all(root.join("buffer")).unwrap();
        let mut segments = Vec::new();
        for index in 0..40 {
            let path = root.join(format!("buffer/seg-{index:06}.ts"));
            fs::write(&path, b"recording").unwrap();
            segments.push(Segment {
                index,
                path,
                start: Some(index as f64 * 5.0),
                end: Some((index + 1) as f64 * 5.0),
            });
        }
        let mut archive = Snapshot::new(&root, &[]).unwrap();
        preserve(&mut archive, &segments, 200.0).unwrap();
        assert!(archive.segments.len() <= 25);
        fs::remove_dir_all(root.join("buffer")).unwrap();
        assert!(archive
            .segments
            .iter()
            .all(|s| fs::read(&s.path).unwrap() == b"recording"));
        let directory = archive.dir.clone();
        drop(archive);
        assert!(!directory.exists());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    #[ignore = "requires bundled FFmpeg and OCR models; generated footage only"]
    fn synthetic_automatic_multikills_survive_stop_and_keep_audio_and_markers() {
        let root =
            std::env::temp_dir().join(format!("stillclip-auto-video-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("buffer")).unwrap();
        let fixtures: &[(&str, &[u8], f64)] = &[
            (
                "empty",
                include_bytes!("../tests/fixtures/valorant/empty.png"),
                2.0,
            ),
            (
                "one",
                include_bytes!("../tests/fixtures/valorant/one.png"),
                2.0,
            ),
            (
                "two",
                include_bytes!("../tests/fixtures/valorant/two.png"),
                2.0,
            ),
            (
                "three",
                include_bytes!("../tests/fixtures/valorant/three.png"),
                3.0,
            ),
            (
                "round",
                include_bytes!("../tests/fixtures/valorant/round.png"),
                2.0,
            ),
            (
                "empty-next",
                include_bytes!("../tests/fixtures/valorant/empty-next.png"),
                3.0,
            ),
            (
                "next-one",
                include_bytes!("../tests/fixtures/valorant/next-one.png"),
                2.0,
            ),
            (
                "next-two",
                include_bytes!("../tests/fixtures/valorant/next-two.png"),
                5.5,
            ),
        ];
        let mut list = String::from("ffconcat version 1.0\n");
        for (name, bytes, duration) in fixtures {
            image::load_from_memory(bytes)
                .unwrap()
                .into_rgb8()
                .save(root.join(format!("{name}.bmp")))
                .unwrap();
            list.push_str(&format!("file '{name}.bmp'\nduration {duration}\n"));
        }
        list.push_str("file 'next-two.bmp'\n");
        fs::write(root.join("frames.txt"), list).unwrap();
        let ffmpeg = recorder::find_ffmpeg().unwrap();
        let result = crate::capture::command(&ffmpeg)
            .current_dir(&root)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                "frames.txt",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "21.5",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-map",
                "1:a:0",
                "-map",
                "1:a:0",
                "-metadata:s:a:0",
                "language=qaa",
                "-metadata:s:a:1",
                "language=qab",
                "-metadata:s:a:2",
                "language=qac",
                "-r",
                "30",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-g",
                "30",
                "-bf",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-f",
                "segment",
                "-segment_time",
                "5",
                "-reset_timestamps",
                "0",
                "-segment_list_type",
                "csv",
                "-segment_list",
                "buffer/segments.csv",
                "buffer/seg-%06d.ts",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            recorder::diagnostic(&result.stderr)
        );
        let mut source = source(&root);
        source.ffmpeg = ffmpeg.clone();
        let segments = completed(&source).unwrap();
        assert!(segments.len() >= 5);
        let tail = Snapshot::new(&root, &segments).unwrap();
        // Model the recorder's final hand-off: the original ring is already gone.
        fs::remove_dir_all(&source.directory).unwrap();
        let (sender, receiver) = mpsc::channel();
        sender.send(Command::Finish(Ok(tail))).unwrap();
        let control = Control::new();
        let mut clips = Vec::new();
        let started = std::time::Instant::now();
        run(spec(source), 1, &receiver, &control, &mut |event| {
            if let Event::Saved(_, path, markers, kills) = event {
                clips.push((path, markers, kills));
            }
            Ok(())
        })
        .unwrap();
        eprintln!(
            "Automatic 21.5s fixture processed in {:?}",
            started.elapsed()
        );
        assert_eq!(
            clips.iter().map(|c| c.2).collect::<Vec<_>>(),
            vec![3, 2],
            "{clips:?}"
        );
        for (path, markers, kills) in &clips {
            let info = recorder::probe_media(&ffmpeg, path).unwrap();
            assert!(info.audio && info.duration > 5.0);
            assert_eq!(markers.len(), *kills);
            assert!(
                markers.iter().all(|at| *at >= 0.0 && *at < info.duration),
                "{markers:?}, duration {}",
                info.duration
            );
            assert!(path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(&format!("{kills}K")));
            let probe = crate::capture::command(&recorder::ffprobe_for(&ffmpeg).unwrap())
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "a",
                    "-show_entries",
                    "stream_tags=language",
                    "-of",
                    "json",
                ])
                .arg(path)
                .output()
                .unwrap();
            assert!(probe.status.success());
            let streams: Value = serde_json::from_slice(&probe.stdout).unwrap();
            assert_eq!(
                streams["streams"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s["tags"]["language"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                vec!["qaa", "qab", "qac"]
            );
        }
        assert!(
            fs::read_dir(root.join(".stillclip-exports"))
                .unwrap()
                .next()
                .is_none(),
            "private segments must be removed after processing"
        );
        let _ = fs::remove_dir_all(root);
    }
}

impl App {
    pub(super) fn start_auto_highlights(&mut self, proxy: EventLoopProxy<AppEvent>) {
        let settings = self.runtime_settings();
        let preferences = &settings.desktop.highlights;
        let game = self
            .active_game
            .as_ref()
            .map(|g| g.exe.as_str())
            .unwrap_or(&settings.target_exe);
        if !preferences.enabled
            || !game.eq_ignore_ascii_case("valorant-win64-shipping.exe")
            || self.closing
        {
            return;
        }
        let Some(source) = self.recorder.as_ref().and_then(|r| r.highlight_source()) else {
            return;
        };
        let spec = Spec {
            source,
            preferences: preferences.clone(),
            player: settings.desktop.player_name.clone(),
            region: settings.desktop.killfeed_region,
        };
        self.auto_highlights.start(spec, proxy);
    }
    pub(super) fn handle_auto_highlights(&mut self, event: Event, proxy: EventLoopProxy<AppEvent>) {
        match event {
            Event::Saved(id, path, markers, kills) => {
                // A complete clip still belongs in the library if a new session
                // starts before its UI event is handled.
                self.remember_markers(&path, markers);
                self.index.annotations.insert(
                    crate::library::index_key(&path),
                    crate::library::Annotation {
                        game: "VALORANT".into(),
                        tags: vec!["auto-highlight".into(), format!("{kills}k")],
                    },
                );
                self.index.save(&self.config);
                if self
                    .auto_highlights
                    .worker
                    .as_ref()
                    .is_some_and(|w| w.id == id)
                {
                    self.auto_highlights.saved += 1;
                }
                let message = crate::tr_format!(
                    "VALORANT · {kills}K automatisch gespeichert.",
                    "VALORANT · {kills}K saved automatically."
                );
                self.notify(&message, false, Some(path));
                self.refresh();
            }
            Event::Progress(id, phase, detected, lag, skipped) => {
                if self
                    .auto_highlights
                    .worker
                    .as_ref()
                    .is_some_and(|w| w.id == id && !w.control.cancelled())
                {
                    self.auto_highlights.phase = phase;
                    self.auto_highlights.detected = detected;
                    self.auto_highlights.lag = lag;
                    self.auto_highlights.skipped = skipped;
                }
            }
            Event::Tail(id, result) => {
                if let Some(worker) = self
                    .auto_highlights
                    .worker
                    .as_ref()
                    .filter(|w| w.id == id && !w.control.cancelled())
                {
                    self.auto_highlights.phase = "finishing";
                    let _ = worker.sender.send(Command::Finish(result));
                }
            }
            Event::Finished(id, result) => {
                if self
                    .auto_highlights
                    .worker
                    .as_ref()
                    .is_none_or(|w| w.id != id)
                {
                    return;
                }
                self.auto_highlights.worker = None;
                match result {
                    Ok(()) => self.auto_highlights.phase = "complete",
                    Err(error) if error == crate::jobs::CANCELLED => {
                        self.auto_highlights.phase = "paused"
                    }
                    Err(error) => {
                        self.auto_highlights.phase = "error";
                        self.auto_highlights.error = Some(error.clone());
                        self.notify(
                            &crate::tr_format!(
                                "Highlight-Erkennung pausiert: {error}",
                                "Highlight detection paused: {error}"
                            ),
                            true,
                            None,
                        );
                    }
                }
                self.advance_capture(proxy);
            }
        }
    }
}
