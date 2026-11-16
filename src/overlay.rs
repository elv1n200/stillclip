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
    pub scale: u32,
    pub anchor_left: bool,
    pub anchor_bottom: bool,
    pub persistent: bool,
}

static BANNER: Mutex<Option<Banner>> = Mutex::new(None);
static HUD: Mutex<Option<Banner>> = Mutex::new(None);
static TOAST_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);

#[cfg(windows)]
mod win {
    use super::{Banner, BANNER, HUD, TOAST_ACTIVE, OVERLAY_HWND};
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::{
        Foundation::{HWND, POINT, SIZE},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };

    pub const WM_OVERLAY_SHOW: u32 = 0x8000 + 40;
    pub const WM_OVERLAY_QUIT: u32 = 0x8000 + 41;
    pub const WM_OVERLAY_HUD: u32 = 0x8000 + 42;
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
            let factor = banner.scale.clamp(60, 160) as f64 / 100.0;
            let padding = (PADDING_X as f64 * factor) as i32;
            let dot = (DOT as f64 * factor) as i32;
            let gap = (12.0 * factor) as i32;
            let height = (HEIGHT as f64 * factor) as i32;
            let screen = GetDC(std::ptr::null_mut());
            let memory = CreateCompatibleDC(screen);
            let font = CreateFontW(
                -(19.0 * factor) as i32,
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
                wide("Consolas").as_ptr(),
            );
            let text = wide(&banner.text);
            let old_font = SelectObject(memory, font);
            let mut measure = windows_sys::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: banner.width_limit.max(200),
                bottom: height,
            };
            DrawTextW(
                memory,
                text.as_ptr(),
                -1,
                &mut measure,
                DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
            );
            let text_width = (measure.right - measure.left).clamp(40, banner.width_limit.max(200));
            let width = padding + dot + gap + text_width + padding;
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                memory,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if bitmap.is_null() || bits.is_null() {
                SelectObject(memory, old_font);
                DeleteObject(font);
                DeleteDC(memory);
                ReleaseDC(std::ptr::null_mut(), screen);
                return;
            }
            let old_bitmap = SelectObject(memory, bitmap);
            let pixels =
                std::slice::from_raw_parts_mut(bits as *mut u8, (width * height * 4) as usize);
            // Background: dark panel; drawn opaque, alpha is applied afterwards.
            let (bg_b, bg_g, bg_r) = (0x0d_u8, 0x0d_u8, 0x0d_u8);
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[0] = bg_b;
                pixel[1] = bg_g;
                pixel[2] = bg_r;
                pixel[3] = 0;
            }
            // Accent dot.
            let dot_colour = if banner.error {
                0x00a9aaff_u32
            } else {
                0x00b7a0e5_u32
            };
            let brush = CreateSolidBrush(dot_colour);
            let pen = CreatePen(PS_NULL, 0, 0);
            let old_brush = SelectObject(memory, brush);
            let old_pen = SelectObject(memory, pen);
            let dot_x = padding;
            let dot_y = (height - dot) / 2;
            Rectangle(memory, dot_x, dot_y, dot_x + dot + 1, dot_y + dot + 1);
            SelectObject(memory, old_brush);
            SelectObject(memory, old_pen);
            DeleteObject(brush);
            DeleteObject(pen);
            // Text.
            SetBkMode(memory, TRANSPARENT as i32);
            SetTextColor(memory, 0x00f4f0ff);
            let mut rect = windows_sys::Win32::Foundation::RECT {
                left: padding + dot + gap,
                top: 0,
                right: width - padding,
                bottom: height,
            };
            DrawTextW(
                memory,
                text.as_ptr(),
                -1,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            // Square frame and premultiplied alpha, matching the main window.
            let alpha = 245_u32;
            for y in 0..height {
                for x in 0..width {
                    let offset = ((y * width + x) * 4) as usize;
                    let border = x == 0 || y == 0 || x == width - 1 || y == height - 1;
                    for channel in 0..3 {
                        let value = if border {
                            130
                        } else {
                            u32::from(pixels[offset + channel])
                        };
                        pixels[offset + channel] = ((value * alpha) / 255) as u8;
                    }
                    pixels[offset + 3] = alpha as u8;
                }
            }
            let position = POINT {
                x: if banner.anchor_left { banner.x } else { banner.x - width },
                y: if banner.anchor_bottom { banner.y - height } else { banner.y },
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
                hwnd, screen, &position, &size, memory, &origin, 0, &blend, ULW_ALPHA,
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
            if !banner.persistent { SetTimer(hwnd, TIMER_HIDE, 2600, None); }
            SelectObject(memory, old_bitmap);
            SelectObject(memory, old_font);
            DeleteObject(bitmap);
            DeleteObject(font);
            DeleteDC(memory);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        unsafe {
            match message {
                WM_OVERLAY_SHOW => {
                    let banner = BANNER.lock().ok().and_then(|b| b.clone());
                    if let Some(banner) = banner {
                        KillTimer(hwnd, TIMER_HIDE);
                        TOAST_ACTIVE.store(true, Ordering::SeqCst);
                        render(hwnd, &banner);
                    }
                    0
                }
                WM_TIMER => {
                    KillTimer(hwnd, TIMER_HIDE);
                    TOAST_ACTIVE.store(false, Ordering::SeqCst);
                    if let Some(hud) = HUD.lock().ok().and_then(|h| h.clone()) { render(hwnd, &hud); }
                    else { ShowWindow(hwnd, SW_HIDE); }
                    0
                }
                WM_OVERLAY_HUD => {
                    if !TOAST_ACTIVE.load(Ordering::SeqCst) {
                        if let Some(hud) = HUD.lock().ok().and_then(|h| h.clone()) { render(hwnd, &hud); }
                        else { ShowWindow(hwnd, SW_HIDE); }
                    }
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
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
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
            // Keep the status HUD out of supported Windows capture APIs.
            SetWindowDisplayAffinity(hwnd, 0x00000011);
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

    pub fn hud(&self, banner: Option<Banner>) {
        if let Ok(mut slot) = HUD.lock() { *slot = banner; }
        #[cfg(windows)]
        win::post(win::WM_OVERLAY_HUD);
    }

    /// Shows the banner anchored with its top-right corner at (`x`, `y`).
    pub fn show(&self, text: &str, error: bool, x: i32, y: i32, width_limit: i32) {
        if let Ok(mut slot) = BANNER.lock() {
            *slot = Some(Banner {
                text: text.to_owned(),
                error,
                x,
                y,
                width_limit, scale: 100, anchor_left: false, anchor_bottom: false, persistent: false,
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
        PlaySoundW(
            bytes.as_ptr().cast(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
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
        // This explicit developer screenshot needs to see the overlay. Normal
        // operation keeps it excluded from supported capture APIs.
        #[cfg(windows)]
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetWindowDisplayAffinity(OVERLAY_HWND.load(Ordering::SeqCst) as _, 0); }
        let display = crate::capture::displays()
            .into_iter()
            .find(|d| d.primary)
            .expect("primary display");
        overlay.show(
            "Moment gesichert · Counter-Strike 2 2026-11-14 07-01-02",
            false,
            display.x + display.width as i32 - 28,
            display.y + 28,
            900,
        );
        play_chime();
        std::thread::sleep(std::time::Duration::from_millis(700));
        let ffmpeg = crate::recorder::find_ffmpeg().expect("ffmpeg");
        let status = crate::capture::command(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg(format!(
                "ddagrab=output_idx={}:framerate=10:draw_mouse=0,hwdownload,format=bgra",
                display.output_idx
            ))
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
