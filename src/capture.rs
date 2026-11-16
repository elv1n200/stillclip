//! Display enumeration, encoder probing and the FFmpeg capture command line.
//!
//! The command records continuous MPEG-TS segments; `recorder.rs` turns them
//! into clips. Rate control follows the formulas in `encoding.rs`.
use crate::encoding::{self, CaptureEncoding};
use crate::settings::{EncoderChoice, Quality};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub const SEGMENT_SECONDS: u32 = 5;

pub fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command.stdin(Stdio::null());
    command
}

/// A job object that kills its members when Stillclip exits, however it exits.
/// Without it a force-killed app would leave FFmpeg recording forever.
#[cfg(windows)]
fn child_job() -> windows_sys::Win32::Foundation::HANDLE {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    struct Job(isize);
    // The handle lives for the whole process; closing it would kill the children.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}
    static JOB: OnceLock<Job> = OnceLock::new();
    JOB.get_or_init(|| unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !handle.is_null() {
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
        }
        Job(handle as isize)
    })
    .0 as windows_sys::Win32::Foundation::HANDLE
}

/// Ties a spawned helper process to this one. Ignored when the job is unavailable.
pub fn adopt_child(child: &std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        let job = child_job();
        if !job.is_null() {
            unsafe {
                AssignProcessToJobObject(job, child.as_raw_handle() as _);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = child;
}

/// A monitor attached to the desktop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Display {
    /// Windows device name such as `\\.\DISPLAY1`.
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    /// Index of the output on its graphics adapter (Desktop Duplication order).
    pub output_idx: u32,
    pub adapter_idx: u32,
}

impl Display {
    pub fn label(&self) -> String {
        let number = self
            .name
            .trim_start_matches("\\\\.\\DISPLAY")
            .parse::<u32>()
            .ok();
        let base = match number {
            Some(n) => crate::tr_format!("Bildschirm {n}", "Display {n}"),
            None => self.name.clone(),
        };
        if self.primary {
            crate::tr_format!("{base} · {}×{} · Hauptbildschirm", "{base} · {}×{} · Main display", self.width, self.height)
        } else {
            format!("{base} · {}×{}", self.width, self.height)
        }
    }
}

#[cfg(windows)]
pub fn displays() -> Vec<Display> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

    let mut found = Vec::new();
    let factory: Result<IDXGIFactory1, _> = unsafe { CreateDXGIFactory1() };
    if let Ok(factory) = factory {
        let mut adapter_idx = 0;
        while let Ok(adapter) = unsafe { factory.EnumAdapters(adapter_idx) } {
            let mut output_idx = 0;
            while let Ok(output) = unsafe { adapter.EnumOutputs(output_idx) } {
                if let Ok(desc) = unsafe { output.GetDesc() } {
                    if desc.AttachedToDesktop.as_bool() {
                        let end = desc
                            .DeviceName
                            .iter()
                            .position(|c| *c == 0)
                            .unwrap_or(desc.DeviceName.len());
                        let name = String::from_utf16_lossy(&desc.DeviceName[..end]);
                        let rect = desc.DesktopCoordinates;
                        let width = (rect.right - rect.left).max(0) as u32;
                        let height = (rect.bottom - rect.top).max(0) as u32;
                        if width > 0
                            && height > 0
                            && !found.iter().any(|d: &Display| d.name == name)
                        {
                            found.push(Display {
                                name,
                                x: rect.left,
                                y: rect.top,
                                width,
                                height,
                                primary: rect.left == 0 && rect.top == 0,
                                output_idx,
                                adapter_idx,
                            });
                        }
                    }
                }
                output_idx += 1;
            }
            adapter_idx += 1;
        }
    }
    if found.is_empty() {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetSystemMetrics(index: i32) -> i32;
        }
        let (width, height) = unsafe { (GetSystemMetrics(0), GetSystemMetrics(1)) };
        if width > 0 && height > 0 {
            found.push(Display {
                name: "\\\\.\\DISPLAY1".into(),
                x: 0,
                y: 0,
                width: width as u32,
                height: height as u32,
                primary: true,
                output_idx: 0,
                adapter_idx: u32::MAX,
            });
        }
    }
    if !found.iter().any(|d| d.primary) {
        if let Some(first) = found.first_mut() {
            first.primary = true;
        }
    }
    found.sort_by(|a, b| b.primary.cmp(&a.primary).then_with(|| a.name.cmp(&b.name)));
    found
}

#[cfg(not(windows))]
pub fn displays() -> Vec<Display> {
    Vec::new()
}

/// The configured display, or the primary one when the choice is gone.
pub fn choose_display<'a>(displays: &'a [Display], name: &str) -> Option<&'a Display> {
    displays
        .iter()
        .find(|d| !name.is_empty() && d.name == name)
        .or_else(|| displays.iter().find(|d| d.primary))
        .or_else(|| displays.first())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoder {
    Nvenc,
    Amf,
    Qsv,
    Cpu,
}

impl Encoder {
    pub const ALL: [Encoder; 4] = [Encoder::Nvenc, Encoder::Amf, Encoder::Qsv, Encoder::Cpu];

    pub fn codec(self) -> &'static str {
        match self {
            Encoder::Nvenc => "h264_nvenc",
            Encoder::Amf => "h264_amf",
            Encoder::Qsv => "h264_qsv",
            Encoder::Cpu => "libx264",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Encoder::Nvenc => "nvenc",
            Encoder::Amf => "amf",
            Encoder::Qsv => "qsv",
            Encoder::Cpu => "cpu",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Encoder::Nvenc => "NVIDIA NVENC",
            Encoder::Amf => "AMD AMF",
            Encoder::Qsv => "Intel Quick Sync",
            Encoder::Cpu => "CPU (x264)",
        }
    }
    fn from_choice(choice: EncoderChoice) -> Option<Self> {
        match choice {
            EncoderChoice::Auto => None,
            EncoderChoice::Nvenc => Some(Encoder::Nvenc),
            EncoderChoice::Amf => Some(Encoder::Amf),
            EncoderChoice::Qsv => Some(Encoder::Qsv),
            EncoderChoice::Cpu => Some(Encoder::Cpu),
        }
    }
}

/// What the local FFmpeg build can do. Probed once per FFmpeg file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Capabilities {
    pub encoders: Vec<Encoder>,
    pub ddagrab: bool,
    #[serde(default)]
    pub gfxcapture: bool,
    pub probed: bool,
}

#[derive(Serialize, Deserialize)]
struct CapabilityCache {
    #[serde(default)]
    version: u32,
    ffmpeg: PathBuf,
    size: u64,
    modified: u64,
    capabilities: Capabilities,
}

fn ffmpeg_identity(ffmpeg: &Path) -> (u64, u64) {
    fs::metadata(ffmpeg)
        .map(|m| {
            (
                m.len(),
                m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs()),
            )
        })
        .unwrap_or((0, 0))
}

pub fn cached_capabilities(ffmpeg: &Path, config_dir: &Path) -> Option<Capabilities> {
    let cache: CapabilityCache =
        crate::settings::read_json(&config_dir.join("ffmpeg-capabilities.json"))?;
    let (size, modified) = ffmpeg_identity(ffmpeg);
    (cache.version == 2
        && cache.ffmpeg == ffmpeg
        && cache.size == size
        && cache.modified == modified
        && cache.capabilities.probed)
        .then_some(cache.capabilities)
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> Option<std::process::Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Runs a tiny synthetic encode per encoder. Never touches the screen.
pub fn probe_capabilities(ffmpeg: &Path, config_dir: &Path) -> Capabilities {
    let mut capabilities = Capabilities {
        encoders: Vec::new(),
        ddagrab: false,
        gfxcapture: false,
        probed: true,
    };
    let mut filters = command(ffmpeg);
    filters.args(["-hide_banner", "-filters"]);
    if let Some(output) = run_with_timeout(filters, Duration::from_secs(20)) {
        capabilities.ddagrab = String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some("ddagrab"));
    }
    let mut window_probe = command(ffmpeg);
    window_probe.args(["-hide_banner", "-h", "filter=gfxcapture"]);
    capabilities.gfxcapture = run_with_timeout(window_probe, Duration::from_secs(5))
        .is_some_and(|out| String::from_utf8_lossy(&out.stdout).contains("hwnd"));
    for encoder in Encoder::ALL {
        let mut probe = command(ffmpeg);
        probe
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=30",
                "-t",
                "0.3",
                "-c:v",
                encoder.codec(),
            ])
            .args(["-f", "null", "-"]);
        if run_with_timeout(probe, Duration::from_secs(25)).is_some_and(|o| o.status.success()) {
            capabilities.encoders.push(encoder);
        }
    }
    let (size, modified) = ffmpeg_identity(ffmpeg);
    let cache = CapabilityCache {
        version: 2,
        ffmpeg: ffmpeg.to_path_buf(),
        size,
        modified,
        capabilities: capabilities.clone(),
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
        let _ = fs::create_dir_all(config_dir);
        let _ = crate::settings::write_atomically(config_dir, "ffmpeg-capabilities.json", &bytes);
    }
    capabilities
}

/// Picks the encoder for a recording: the explicit choice when available, else
/// the best hardware encoder, else the CPU.
pub fn select_encoder(choice: EncoderChoice, available: &[Encoder]) -> Encoder {
    if let Some(wanted) = Encoder::from_choice(choice) {
        if available.contains(&wanted) || available.is_empty() {
            return wanted;
        }
    }
    Encoder::ALL
        .into_iter()
        .find(|e| available.contains(e))
        .unwrap_or(Encoder::Cpu)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoSource {
    /// Desktop Duplication through FFmpeg's `ddagrab` filter.
    Ddagrab {
        output_idx: u32,
    },
    /// GDI screen grabbing by desktop coordinates.
    Gdigrab,
    Window {
        hwnd: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Strategy {
    pub source: VideoSource,
    pub encoder: Encoder,
}

impl Strategy {
    pub fn label(&self) -> String {
        let source = match self.source {
            VideoSource::Ddagrab { .. } => "Desktop Duplication",
            VideoSource::Gdigrab => "GDI",
            VideoSource::Window { .. } => "Windows-Fensteraufnahme",
        };
        format!("{source} · {}", self.encoder.label())
    }
}

/// Capture attempts in order of preference. A later entry only runs when the
/// previous FFmpeg process died right after starting.
pub fn strategies(display: &Display, encoder: Encoder, ddagrab: bool) -> Vec<Strategy> {
    let mut list = Vec::new();
    let dda_possible = ddagrab && display.adapter_idx == 0;
    for candidate in [encoder, Encoder::Cpu] {
        if dda_possible {
            list.push(Strategy {
                source: VideoSource::Ddagrab {
                    output_idx: display.output_idx,
                },
                encoder: candidate,
            });
        }
        list.push(Strategy {
            source: VideoSource::Gdigrab,
            encoder: candidate,
        });
    }
    list.dedup();
    list
}

/// A raw PCM input that a capture thread feeds through a named pipe.
#[derive(Clone, Debug)]
pub struct AudioInput {
    pub path: String,
    pub sample_format: &'static str,
    pub rate: u32,
    pub channels: u16,
    /// Loudness factor; 1.0 passes the samples through unchanged.
    pub gain: f32,
}

pub struct CaptureRequest<'a> {
    pub display: &'a Display,
    pub strategy: &'a Strategy,
    pub fps: u32,
    pub quality: Quality,
    pub show_cursor: bool,
    pub audio: &'a [AudioInput],
    /// Also keep the last audio input (the microphone) as its own track.
    pub separate_microphone: bool,
    pub segment_dir: &'a Path,
    pub list_path: &'a Path,
}

/// Output size for a display at the chosen quality.
pub fn output_size(display: &Display, quality: Quality) -> (u32, u32) {
    match quality {
        Quality::Native => (display.width.max(2) & !1, display.height.max(2) & !1),
        other => {
            let (_, height, _) = other.configured();
            encoding::capture_size(display.width, display.height, height)
        }
    }
}

pub fn encoder_args(
    command: &mut Command,
    encoder: Encoder,
    rates: &CaptureEncoding,
    hardware_frames: bool,
) {
    command.args(["-c:v", encoder.codec()]);
    match encoder {
        Encoder::Cpu => {
            command.args(["-preset", "veryfast", "-pix_fmt", "yuv420p"]);
        }
        Encoder::Nvenc => {
            command.args([
                "-preset",
                "p4",
                "-tune",
                "hq",
                "-rc",
                "vbr",
                "-spatial-aq",
                "1",
            ]);
            if !hardware_frames {
                command.args(["-pix_fmt", "yuv420p"]);
            }
        }
        Encoder::Amf => {
            command.args([
                "-usage",
                "transcoding",
                "-quality",
                "speed",
                "-rc",
                "vbr_peak",
                "-pix_fmt",
                "yuv420p",
            ]);
        }
        Encoder::Qsv => {
            command.args(["-preset", "veryfast", "-pix_fmt", "nv12"]);
        }
    }
    command
        .arg("-b:v")
        .arg(rates.bitrate.to_string())
        .arg("-maxrate")
        .arg(rates.max_bitrate.to_string())
        .arg("-bufsize")
        .arg(rates.buffer_size.to_string())
        .args(["-bf", "0"])
        .arg("-g")
        .arg(rates.gop_frames.to_string());
}

/// A gain that differs enough from unity to be worth a filter.
fn adjusted(gain: f32) -> bool {
    (gain - 1.0).abs() > 0.005
}

/// Maps the audio inputs: loudness per source, one mixed track, and the
/// microphone as a second track when that is switched on.
fn audio_mapping(command: &mut Command, request: &CaptureRequest) {
    let count = request.audio.len();
    if count == 0 {
        command.arg("-an");
        return;
    }
    if count == 1 {
        command.args(["-map", "1:a:0"]);
        let gain = request.audio[0].gain;
        if adjusted(gain) {
            command.arg("-af").arg(format!("volume={gain:.3}"));
        }
        return;
    }
    let mut chains: Vec<String> = Vec::new();
    let mut mix: Vec<String> = Vec::new();
    let mut microphone = None;
    for (position, input) in request.audio.iter().enumerate() {
        let index = position + 1;
        let mut filters: Vec<String> = Vec::new();
        if adjusted(input.gain) {
            filters.push(format!("volume={:.3}", input.gain));
        }
        // The last input is the microphone; splitting it keeps the mix intact
        // while the same samples also become their own track.
        if request.separate_microphone && index == count {
            filters.push("asplit=2".into());
            chains.push(format!("[{index}:a]{}[m{index}][mic]", filters.join(",")));
            mix.push(format!("[m{index}]"));
            microphone = Some("[mic]".to_owned());
        } else if filters.is_empty() {
            mix.push(format!("[{index}:a]"));
        } else {
            chains.push(format!("[{index}:a]{}[g{index}]", filters.join(",")));
            mix.push(format!("[g{index}]"));
        }
    }
    chains.push(format!(
        "{}amix=inputs={count}:duration=longest:dropout_transition=0:normalize=0[a]",
        mix.join("")
    ));
    command
        .arg("-filter_complex")
        .arg(chains.join(";"))
        .args(["-map", "[a]"]);
    if let Some(track) = microphone {
        command.arg("-map").arg(track);
    }
}

/// Builds the complete FFmpeg command for one capture attempt.
pub fn build_command(ffmpeg: &Path, request: &CaptureRequest) -> Command {
    let display = request.display;
    let (width, height) = output_size(display, request.quality);
    let (configured_width, configured_height, mbps) = request.quality.configured();
    let rates = CaptureEncoding::for_output(
        width,
        height,
        request.fps,
        configured_width,
        configured_height,
        mbps,
    );
    let native = (width, height) == (display.width, display.height);
    let mut command = command(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "warning",
            "-nostats",
            "-y",
            "-stats_period",
            "1",
            "-progress",
        ])
        .arg(request.segment_dir.join("progress.txt"));
    let cursor = if request.show_cursor { "1" } else { "0" };
    let mut hardware_frames = false;
    match request.strategy.source {
        VideoSource::Window { hwnd } => {
            let graph = format!("gfxcapture=hwnd={hwnd}:max_framerate={}:capture_cursor={cursor},hwdownload,format=bgra,scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,fps={}", request.fps, request.fps);
            command
                .args(["-init_hw_device", "d3d11va", "-f", "lavfi", "-i"])
                .arg(graph);
        }
        VideoSource::Ddagrab { output_idx } => {
            let mut graph = format!(
                "ddagrab=output_idx={output_idx}:framerate={}:draw_mouse={cursor}",
                request.fps
            );
            hardware_frames = native && request.strategy.encoder == Encoder::Nvenc;
            if !hardware_frames {
                graph.push_str(",hwdownload,format=bgra");
                if !native {
                    graph.push_str(&format!(",scale={width}:{height}"));
                }
            }
            command
                .args(["-init_hw_device", "d3d11va", "-f", "lavfi", "-i"])
                .arg(graph);
        }
        VideoSource::Gdigrab => {
            command
                .args(["-f", "gdigrab", "-framerate"])
                .arg(request.fps.to_string())
                .args(["-draw_mouse", cursor, "-offset_x"])
                .arg(display.x.to_string())
                .arg("-offset_y")
                .arg(display.y.to_string())
                .arg("-video_size")
                .arg(format!("{}x{}", display.width, display.height))
                .args(["-i", "desktop"]);
        }
    }
    for input in request.audio {
        command
            .args([
                "-thread_queue_size",
                "4096",
                "-f",
                input.sample_format,
                "-ar",
            ])
            .arg(input.rate.to_string())
            .arg("-ac")
            .arg(input.channels.to_string())
            .arg("-i")
            .arg(&input.path);
    }
    if request.strategy.source == VideoSource::Gdigrab && !native {
        command.arg("-vf").arg(format!("scale={width}:{height}"));
    }
    command.args(["-map", "0:v:0"]);
    audio_mapping(&mut command, request);
    encoder_args(
        &mut command,
        request.strategy.encoder,
        &rates,
        hardware_frames,
    );
    command.args(["-fps_mode", "cfr"]);
    if !request.audio.is_empty() {
        command.args(["-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-ac", "2"]);
    }
    command
        .args(["-flush_packets", "1", "-f", "segment", "-segment_time"])
        .arg(SEGMENT_SECONDS.to_string())
        .args([
            "-segment_format",
            "mpegts",
            "-reset_timestamps",
            "0",
            "-segment_list_type",
            "csv",
            "-segment_list_flags",
            "live",
            "-segment_list",
        ])
        .arg(request.list_path)
        .arg(request.segment_dir.join("seg-%06d.ts"));
    command
}

/// Renders a command line for logs; arguments with spaces are quoted.
pub fn describe(command: &Command) -> String {
    std::iter::once(command.get_program().to_string_lossy().into_owned())
        .chain(command.get_args().map(|a| {
            let a = a.to_string_lossy();
            if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                a.into_owned()
            }
        }))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> Display {
        Display {
            name: "\\\\.\\DISPLAY1".into(),
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
            primary: true,
            output_idx: 0,
            adapter_idx: 0,
        }
    }

    #[test]
    fn selects_requested_or_best_available_encoder() {
        let available = [Encoder::Amf, Encoder::Cpu];
        assert_eq!(
            select_encoder(EncoderChoice::Auto, &available),
            Encoder::Amf
        );
        assert_eq!(
            select_encoder(EncoderChoice::Nvenc, &available),
            Encoder::Amf
        );
        assert_eq!(select_encoder(EncoderChoice::Cpu, &available), Encoder::Cpu);
        assert_eq!(select_encoder(EncoderChoice::Auto, &[]), Encoder::Cpu);
        assert_eq!(select_encoder(EncoderChoice::Nvenc, &[]), Encoder::Nvenc);
    }

    #[test]
    fn strategies_fall_back_to_gdi_and_cpu() {
        let list = strategies(&display(), Encoder::Nvenc, true);
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].source, VideoSource::Ddagrab { output_idx: 0 });
        assert_eq!(list[0].encoder, Encoder::Nvenc);
        assert_eq!(list[3].source, VideoSource::Gdigrab);
        assert_eq!(list[3].encoder, Encoder::Cpu);
        let cpu_only = strategies(&display(), Encoder::Cpu, false);
        assert_eq!(cpu_only.len(), 1);
        let mut other_adapter = display();
        other_adapter.adapter_idx = 1;
        assert!(strategies(&other_adapter, Encoder::Nvenc, true)
            .iter()
            .all(|s| s.source == VideoSource::Gdigrab));
    }

    fn pipe(name: &str, gain: f32) -> AudioInput {
        AudioInput {
            path: format!("\\\\.\\pipe\\{name}"),
            sample_format: "f32le",
            rate: 48000,
            channels: 2,
            gain,
        }
    }

    fn audio_line(audio: &[AudioInput], separate_microphone: bool) -> String {
        let display = display();
        let strategy = Strategy {
            source: VideoSource::Ddagrab { output_idx: 0 },
            encoder: Encoder::Nvenc,
        };
        let command = build_command(
            Path::new("ffmpeg"),
            &CaptureRequest {
                display: &display,
                strategy: &strategy,
                fps: 60,
                quality: Quality::P720,
                show_cursor: false,
                audio,
                separate_microphone,
                segment_dir: Path::new("buffer"),
                list_path: Path::new("buffer/segments.csv"),
            },
        );
        describe(&command)
    }

    #[test]
    fn window_capture_uses_only_the_selected_handle_and_keeps_progress() {
        for encoder in [Encoder::Nvenc, Encoder::Cpu] {
            let display = display();
            let strategy = Strategy {
                source: VideoSource::Window { hwnd: 123456 },
                encoder,
            };
            let cmd = build_command(
                Path::new("ffmpeg"),
                &CaptureRequest {
                    display: &display,
                    strategy: &strategy,
                    fps: 60,
                    quality: Quality::P1080,
                    show_cursor: false,
                    audio: &[],
                    separate_microphone: false,
                    segment_dir: Path::new("buffer"),
                    list_path: Path::new("buffer/segments.csv"),
                },
            );
            let args = describe(&cmd);
            assert!(args.contains("gfxcapture=hwnd=123456"));
            assert!(!args.contains("-i desktop") && !args.contains("ddagrab="));
            assert!(args.contains("progress.txt") && args.contains("capture_cursor=0"));
        }
    }

    #[test]
    fn loudness_is_applied_per_source() {
        // One source: a plain audio filter is enough.
        let text = audio_line(&[pipe("a", 0.5)], false);
        assert!(text.contains("-map 1:a:0 -af volume=0.500"), "{text}");
        assert!(!text.contains("filter_complex"), "{text}");
        // Unity gain stays out of the command line entirely.
        let text = audio_line(&[pipe("a", 1.0), pipe("b", 1.0)], false);
        assert!(
            text.contains("-filter_complex [1:a][2:a]amix=inputs=2"),
            "{text}"
        );
        assert!(!text.contains("volume="), "{text}");
        // System quieter, microphone louder, both mixed into one track.
        let text = audio_line(&[pipe("a", 0.8), pipe("b", 1.5)], false);
        assert!(
            text.contains(
                "-filter_complex [1:a]volume=0.800[g1];[2:a]volume=1.500[g2];[g1][g2]amix=inputs=2"
            ),
            "{text}"
        );
        // The microphone keeps its loudness on its own track as well.
        let text = audio_line(&[pipe("a", 1.0), pipe("b", 1.5)], true);
        assert!(
            text.contains("[2:a]volume=1.500,asplit=2[m2][mic]"),
            "{text}"
        );
        assert!(text.contains("[1:a][m2]amix=inputs=2"), "{text}");
        assert!(text.contains("-map [a] -map [mic]"), "{text}");
    }

    #[test]
    fn output_size_follows_quality() {
        assert_eq!(output_size(&display(), Quality::P720), (1280, 720));
        assert_eq!(output_size(&display(), Quality::P1080), (1920, 1080));
        assert_eq!(output_size(&display(), Quality::P1440), (2560, 1440));
        assert_eq!(output_size(&display(), Quality::Native), (2560, 1440));
        let mut small = display();
        small.width = 1366;
        small.height = 768;
        assert_eq!(output_size(&small, Quality::P1080), (1366, 768));
    }

    #[test]
    fn command_line_uses_segments_and_audio_mixing() {
        let display = display();
        let strategy = Strategy {
            source: VideoSource::Ddagrab { output_idx: 0 },
            encoder: Encoder::Nvenc,
        };
        let audio = [
            AudioInput {
                path: "\\\\.\\pipe\\a".into(),
                sample_format: "f32le",
                rate: 48000,
                channels: 2,
                gain: 1.0,
            },
            AudioInput {
                path: "\\\\.\\pipe\\b".into(),
                sample_format: "f32le",
                rate: 48000,
                channels: 2,
                gain: 1.0,
            },
        ];
        let command = build_command(
            Path::new("ffmpeg"),
            &CaptureRequest {
                display: &display,
                strategy: &strategy,
                fps: 60,
                quality: Quality::P720,
                show_cursor: false,
                audio: &audio,
                separate_microphone: true,
                segment_dir: Path::new("buffer"),
                list_path: Path::new("buffer/segments.csv"),
            },
        );
        let text = describe(&command);
        assert!(text.contains("-map [a] -map [mic]"), "{text}");
        assert!(text.contains("[2:a]asplit=2[m2][mic]"), "{text}");
        assert!(text.contains(
            "ddagrab=output_idx=0:framerate=60:draw_mouse=0,hwdownload,format=bgra,scale=1280:720"
        ));
        assert!(text.contains("amix=inputs=2"));
        assert!(text.contains("-c:v h264_nvenc"));
        assert!(text.contains("-b:v 15000000 -maxrate 30000000 -bufsize 30000000 -bf 0 -g 24"));
        assert!(text.contains("-segment_format mpegts"));
        assert!(text.contains("seg-%06d.ts"));
        let native = build_command(
            Path::new("ffmpeg"),
            &CaptureRequest {
                display: &display,
                strategy: &strategy,
                fps: 60,
                quality: Quality::Native,
                show_cursor: true,
                audio: &[],
                separate_microphone: false,
                segment_dir: Path::new("buffer"),
                list_path: Path::new("buffer/segments.csv"),
            },
        );
        let text = describe(&native);
        assert!(text.contains("draw_mouse=1 "), "{text}");
        assert!(!text.contains("hwdownload"), "{text}");
        assert!(text.contains(" -an "), "{text}");
        let gdi = build_command(
            Path::new("ffmpeg"),
            &CaptureRequest {
                display: &display,
                strategy: &Strategy {
                    source: VideoSource::Gdigrab,
                    encoder: Encoder::Cpu,
                },
                fps: 30,
                quality: Quality::P1080,
                show_cursor: false,
                audio: &audio[..1],
                separate_microphone: false,
                segment_dir: Path::new("buffer"),
                list_path: Path::new("buffer/segments.csv"),
            },
        );
        let text = describe(&gdi);
        assert!(text.contains("-f gdigrab -framerate 30 -draw_mouse 0 -offset_x 0 -offset_y 0 -video_size 2560x1440 -i desktop"));
        assert!(text.contains("-vf scale=1920:1080"));
        assert!(text.contains("-map 1:a:0"));
        assert!(text.contains("-c:v libx264"));
    }
}
