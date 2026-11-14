//! In-game feedback: a small click-through notification window drawn on top of
//! everything (works with windowed and borderless games) and a short sound.
use std::sync::{
    atomic::{AtomicIsize, Ordering},
    mpsc, Mutex, OnceLock,
};

/// What to draw: text plus an accent colour for the dot.
#[derive(Clone, Debug)]
pub struct Banner {
    pub text: String,
    pub error: bool,
    pub x: i32,
    pub y: i32,
    pub width_limit: i32,
}

static BANNER: Mutex<Option<Banner>> = Mutex::new(None);
static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);

#[cfg(windows)]
mod win {
    use super::{Banner, BANNER, OVERLAY_HWND};
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::{
        Foundation::{HWND, POINT, SIZE},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };

    pub const WM_OVERLAY_SHOW: u32 = 0x8000 + 40;
    pub const WM_OVERLAY_QUIT: u32 = 0x8000 + 41;
    const TIMER_HIDE: usize = 1;
    const PADDING_X: i32 = 22;
    const HEIGHT: i32 = 56;
    const DOT: i32 = 12;

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    /// Draws the banner into a premultiplied BGRA bitmap and shows it.
    unsafe fn render(hwnd: HWND, banner: &Banner) {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let memory = CreateCompatibleDC(screen);
            let font = CreateFontW(
                -19,
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                wide("Segoe UI").as_ptr(),
            );
            let text = wide(&banner.text);
            let old_font = SelectObject(memory, font);
            let mut measure = windows_sys::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: banner.width_limit.max(200),
                bottom: HEIGHT,
            };
            DrawTextW(memory, text.as_ptr(), -1, &mut measure, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
            let text_width = (measure.right - measure.left).clamp(40, banner.width_limit.max(200));
            let width = PADDING_X + DOT + 12 + text_width + PADDING_X;
            let height = HEIGHT;
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                SelectObject(memory, old_font);
                DeleteObject(font);
                DeleteDC(memory);
                ReleaseDC(std::ptr::null_mut(), screen);
                return;
            }
            let old_bitmap = SelectObject(memory, bitmap);
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (width * height * 4) as usize);
            // Background: dark panel; drawn opaque, alpha is applied afterwards.
            let (bg_b, bg_g, bg_r) = (0x24_u8, 0x1b_u8, 0x1d_u8);
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[0] = bg_b;
                pixel[1] = bg_g;
                pixel[2] = bg_r;
                pixel[3] = 0;
            }
            // Accent dot.
            let dot_colour = if banner.error { 0x00a9aaff_u32 } else { 0x00ffb0be_u32 };
            let brush = CreateSolidBrush(dot_colour);
            let pen = CreatePen(PS_NULL, 0, 0);
            let old_brush = SelectObject(memory, brush);
            let old_pen = SelectObject(memory, pen);
            let dot_x = PADDING_X;
            let dot_y = (height - DOT) / 2;
            Ellipse(memory, dot_x, dot_y, dot_x + DOT + 1, dot_y + DOT + 1);
            SelectObject(memory, old_brush);
            SelectObject(memory, old_pen);
            DeleteObject(brush);
            DeleteObject(pen);
            // Text.
            SetBkMode(memory, TRANSPARENT as i32);
            SetTextColor(memory, 0x00f4f0ff);
            let mut rect = windows_sys::Win32::Foundation::RECT {
                left: PADDING_X + DOT + 12,
                top: 0,
                right: width - PADDING_X,
                bottom: height,
            };
            DrawTextW(
                memory,
                text.as_ptr(),
                -1,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            // Rounded corners and premultiplied alpha.
            let radius = 14.0_f32;
            let alpha = 236_u32;
            for y in 0..height {
                for x in 0..width {
                    let offset = ((y * width + x) * 4) as usize;
                    let fx = x as f32 + 0.5;
                    let fy = y as f32 + 0.5;
                    let cx = fx.clamp(radius, width as f32 - radius);
                    let cy = fy.clamp(radius, height as f32 - radius);
                    let distance = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                    let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                    let a = (alpha as f32 * coverage) as u32;
                    for channel in 0..3 {
                        pixels[offset + channel] = ((u32::from(pixels[offset + channel]) * a) / 255) as u8;
                    }
                    pixels[offset + 3] = a as u8;
                }
            }
            let position = POINT {
                x: banner.x - width,
                y: banner.y,
            };
            let size = SIZE {
                cx: width,
                cy: height,
            };
            let origin = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd,
                screen,
                &position,
                &size,
                memory,
                &origin,
                0,
                &blend,
                ULW_ALPHA,
            );
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            SetTimer(hwnd, TIMER_HIDE, 2600, None);
            SelectObject(memory, old_bitmap);
            SelectObject(memory, old_font);
            DeleteObject(bitmap);
            DeleteObject(font);
            DeleteDC(memory);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
        unsafe {
            match message {
                WM_OVERLAY_SHOW => {
                    let banner = BANNER.lock().ok().and_then(|b| b.clone());
                    if let Some(banner) = banner {
                        KillTimer(hwnd, TIMER_HIDE);
                        render(hwnd, &banner);
                    }
                    0
                }
                WM_TIMER => {
                    KillTimer(hwnd, TIMER_HIDE);
                    ShowWindow(hwnd, SW_HIDE);
                    0
                }
                WM_OVERLAY_QUIT => {
                    DestroyWindow(hwnd);
                    PostQuitMessage(0);
                    0
                }
                _ => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
    }

    pub fn run(ready: std::sync::mpsc::Sender<bool>) {
        unsafe {
            let class_name = wide("StillclipOverlay");
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_POPUP,
                0,
                0,
                10,
                10,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                let _ = ready.send(false);
                return;
            }
            OVERLAY_HWND.store(hwnd as isize, Ordering::SeqCst);
            let _ = ready.send(true);
            let mut message = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            OVERLAY_HWND.store(0, Ordering::SeqCst);
        }
    }

    pub fn post(message: u32) {
        let hwnd = OVERLAY_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                PostMessageW(hwnd as HWND, message, 0, 0);
            }
        }
    }
}

/// The overlay window with its own thread.
pub struct Overlay;

impl Overlay {
    #[cfg(windows)]
    pub fn start() -> Option<Overlay> {
        if OVERLAY_HWND.load(Ordering::SeqCst) != 0 {
            return Some(Overlay);
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("overlay".into())
            .spawn(move || win::run(sender))
            .ok()?;
        match receiver.recv() {
            Ok(true) => Some(Overlay),
            _ => None,
        }
    }

    #[cfg(not(windows))]
    pub fn start() -> Option<Overlay> {
        None
    }

    /// Shows the banner anchored with its top-right corner at (`x`, `y`).
    pub fn show(&self, text: &str, error: bool, x: i32, y: i32, width_limit: i32) {
        if let Ok(mut slot) = BANNER.lock() {
            *slot = Some(Banner {
                text: text.to_owned(),
                error,
                x,
                y,
                width_limit,
            });
        }
        #[cfg(windows)]
        win::post(win::WM_OVERLAY_SHOW);
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        #[cfg(windows)]
        win::post(win::WM_OVERLAY_QUIT);
    }
}

// --- Sound cue ----------------------------------------------------------------

/// A short two-note chime, generated once (48 kHz, mono, 16-bit WAV).
fn chime() -> &'static Vec<u8> {
    static CHIME: OnceLock<Vec<u8>> = OnceLock::new();
    CHIME.get_or_init(|| {
        let rate = 48_000_u32;
        let notes: [(f32, f32); 2] = [(880.0, 0.09), (1318.5, 0.16)];
        let mut samples: Vec<i16> = Vec::new();
        for (frequency, seconds) in notes {
            let count = (rate as f32 * seconds) as usize;
            for i in 0..count {
                let t = i as f32 / rate as f32;
                let envelope = {
                    let attack = (t / 0.008).min(1.0);
                    let release = ((seconds - t) / 0.05).clamp(0.0, 1.0);
                    attack * release
                };
                let value = (t * frequency * std::f32::consts::TAU).sin() * 0.35 * envelope;
                samples.push((value * i16::MAX as f32) as i16);
            }
        }
        let data_len = (samples.len() * 2) as u32;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    })
}

/// Plays the chime asynchronously on the default output device.
#[cfg(windows)]
pub fn play_chime() {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    let bytes = chime();
    unsafe {
        PlaySoundW(bytes.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

#[cfg(not(windows))]
pub fn play_chime() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shows a banner on the primary display and grabs that screen with FFmpeg
    /// into `STILLCLIP_OVERLAY_SHOT` so the rendering can be inspected.
    #[test]
    #[ignore = "draws on the developer's screen"]
    fn overlay_smoke() {
        let Some(shot) = std::env::var_os("STILLCLIP_OVERLAY_SHOT") else {
            return;
        };
        let overlay = Overlay::start().expect("overlay window");
        let display = crate::capture::displays().into_iter().find(|d| d.primary).expect("primary display");
        overlay.show("Moment gesichert · Counter-Strike 2 2026-11-14 07-01-02", false, display.x + display.width as i32 - 28, display.y + 28, 900);
        play_chime();
        std::thread::sleep(std::time::Duration::from_millis(700));
        let ffmpeg = crate::recorder::find_ffmpeg().expect("ffmpeg");
        let status = crate::capture::command(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("ddagrab=output_idx={}:framerate=10:draw_mouse=0,hwdownload,format=bgra", display.output_idx))
            .args(["-frames:v", "1"])
            .arg(shot)
            .status()
            .expect("ffmpeg runs");
        assert!(status.success());
        std::thread::sleep(std::time::Duration::from_millis(2500));
    }

    #[test]
    fn chime_is_a_valid_wav() {
        let wav = chime();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let data_len = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
        assert_eq!(wav.len(), 44 + data_len);
        assert!(data_len > 20_000);
    }
}
