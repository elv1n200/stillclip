//! Windows integration: dialogs, Explorer, recycle bin, autostart, global
//! hotkeys and the notification-area icon.
use crate::AppEvent;
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
};
use tao::event_loop::EventLoopProxy;
use windows_sys::Win32::{
    Foundation::HWND,
    System::Com::CoTaskMemFree,
    UI::{Controls::Dialogs::*, Shell::*},
};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Honor Windows' actual Videos folder, including redirected user folders.
pub fn videos_dir() -> Option<PathBuf> {
    unsafe {
        let mut path = std::ptr::null_mut();
        if SHGetKnownFolderPath(&FOLDERID_Videos, 0, std::ptr::null_mut(), &mut path) != 0 || path.is_null() {
            return None;
        }
        let mut len = 0;
        while *path.add(len) != 0 { len += 1; }
        let result = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(path, len)));
        CoTaskMemFree(path.cast());
        Some(result)
    }
}

/// Windows 11 otherwise rounds the native window even when every UI panel is
/// square. Older Windows versions simply ignore unsupported DWM attributes.
pub fn style_window(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_DONOTROUND,
    };
    let corners = DWMWCP_DONOTROUND;
    let dark = 1_i32;
    unsafe {
        DwmSetWindowAttribute(
            hwnd as HWND,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&corners as *const i32).cast(),
            std::mem::size_of_val(&corners) as u32,
        );
        DwmSetWindowAttribute(
            hwnd as HWND,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&dark as *const i32).cast(),
            std::mem::size_of_val(&dark) as u32,
        );
    }
}

/// Hold this handle before reading settings or recovering any buffer. Windows
/// releases it on crashes too, so a stale lock cannot strand the application.
pub struct InstanceGuard(windows_sys::Win32::Foundation::HANDLE);

impl InstanceGuard {
    pub fn acquire() -> Result<Option<Self>, String> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS},
            System::Threading::CreateMutexW,
        };
        let name = wide("Local\\Stillclip.Application");
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err(crate::i18n::text("Die Instanzsperre konnte nicht geöffnet werden.", "Could not open the application instance lock.").into());
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                let title = wide("Stillclip");
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
                };
                let window = FindWindowW(std::ptr::null(), title.as_ptr());
                if !window.is_null() {
                    ShowWindow(window, SW_RESTORE);
                    SetForegroundWindow(window);
                }
                return Ok(None);
            }
            Ok(Some(Self(handle)))
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Free bytes on the volume that holds `path`, for the current user's quota.
/// Walks up to the nearest existing folder, so a clip folder that does not
/// exist yet still reports its drive.
pub fn free_bytes(path: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        if current.is_dir() {
            let wide = wide_path(current);
            let mut free = 0_u64;
            let ok = unsafe {
                GetDiskFreeSpaceExW(
                    wide.as_ptr(),
                    &mut free,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            return (ok != 0).then_some(free);
        }
        candidate = current.parent();
    }
    None
}

pub fn pick_file(owner: isize, executable: bool) -> Option<PathBuf> {
    let mut filename = vec![0u16; 32768];
    let filter = wide(if executable {
        crate::i18n::text("FFmpeg\0ffmpeg.exe\0Programme\0*.exe\0", "FFmpeg\0ffmpeg.exe\0Programs\0*.exe\0")
    } else {
        "Videos\0*.mp4;*.mkv;*.mov;*.webm;*.avi;*.m4v\0"
    });
    let title = wide(if executable {
        crate::i18n::text("FFmpeg auswählen", "Choose FFmpeg")
    } else {
        crate::i18n::text("Video importieren", "Import video")
    });
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner as HWND;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = filename.as_mut_ptr();
    dialog.nMaxFile = filename.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER;
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = filename.iter().position(|v| *v == 0).unwrap_or(0);
    Some(PathBuf::from(OsString::from_wide(&filename[..length])))
}

/// Lets the user choose any program, for the game list.
pub fn pick_program(owner: isize) -> Option<PathBuf> {
    let mut filename = vec![0u16; 32768];
    let filter = wide(crate::i18n::text("Programme\0*.exe\0", "Programs\0*.exe\0"));
    let title = wide(crate::i18n::text("Spiel oder Programm auswählen", "Choose a game or program"));
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner as HWND;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = filename.as_mut_ptr();
    dialog.nMaxFile = filename.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER;
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = filename.iter().position(|v| *v == 0).unwrap_or(0);
    Some(PathBuf::from(OsString::from_wide(&filename[..length])))
}

pub fn pick_folder(owner: isize) -> Option<PathBuf> {
    let title = wide(crate::i18n::text("Ordner für deine lokalen Clips auswählen", "Choose a folder for your local clips"));
    let mut display = vec![0u16; 260];
    let dialog = BROWSEINFOW {
        hwndOwner: owner as HWND,
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: display.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        lpfn: None,
        lParam: 0,
        iImage: 0,
    };
    unsafe {
        let id = SHBrowseForFolderW(&dialog);
        if id.is_null() {
            return None;
        }
        let mut path = vec![0u16; 260];
        let ok = SHGetPathFromIDListW(id, path.as_mut_ptr());
        CoTaskMemFree(id.cast());
        if ok == 0 {
            return None;
        }
        let length = path.iter().position(|v| *v == 0).unwrap_or(0);
        Some(PathBuf::from(OsString::from_wide(&path[..length])))
    }
}

fn shell_open(path: &Path) -> Result<(), String> {
    let operation = wide("open");
    let target = wide_path(path);
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    } as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(crate::tr_format!("Konnte nicht geöffnet werden (Windows {code}).", "Could not open (Windows {code})."))
    }
}

pub fn open_folder(path: &Path) -> Result<(), String> {
    shell_open(path)
}

/// Opens a clip in the user's default video player.
pub fn open_file(path: &Path) -> Result<(), String> {
    shell_open(path)
}

/// Opens Explorer with the file selected.
pub fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    let mut argument = OsString::from("/select,");
    argument.push(path.as_os_str());
    std::process::Command::new("explorer.exe")
        .arg(argument)
        .spawn()
        .map(|_| ())
        .map_err(|error| crate::tr_format!("Der Explorer konnte nicht geöffnet werden: {error}", "Could not open File Explorer: {error}"))
}

/// Moves a file to the recycle bin so a mistaken delete can be undone.
pub fn recycle(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(crate::i18n::text("Die Datei wurde nicht gefunden.", "File not found.").into());
    }
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.push(0);
    from.push(0);
    let mut operation: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    operation.wFunc = FO_DELETE;
    operation.pFrom = from.as_ptr();
    operation.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16;
    let code = unsafe { SHFileOperationW(&mut operation) };
    if code == 0 && operation.fAnyOperationsAborted == 0 {
        Ok(())
    } else {
        Err(crate::tr_format!(
            "Der Clip konnte nicht in den Papierkorb verschoben werden (Code {code}).", "Could not move the clip to the Recycle Bin (code {code})."
        ))
    }
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Stillclip";

pub fn autostart_enabled() -> bool {
    use windows_sys::Win32::System::Registry::*;
    let key_name = wide(RUN_KEY);
    let value_name = wide(RUN_VALUE);
    unsafe {
        let mut key = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key_name.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return false;
        }
        let mut size = 0_u32;
        let found = RegQueryValueExW(
            key,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        ) == 0;
        RegCloseKey(key);
        found
    }
}

/// Registers or removes the per-user Run entry; nothing system-wide changes.
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::*;
    let key_name = wide(RUN_KEY);
    let value_name = wide(RUN_VALUE);
    unsafe {
        let mut key = std::ptr::null_mut();
        let created = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_SET_VALUE | KEY_READ,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        );
        if created != 0 {
            return Err(crate::tr_format!(
                "Der Autostart-Schlüssel ist nicht erreichbar (Fehler {created}).", "Could not access the startup registry key (error {created})."
            ));
        }
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let command = wide(&format!("\"{}\" --minimized", exe.display()));
            RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        } else {
            let code = RegDeleteValueW(key, value_name.as_ptr());
            if code == 2 {
                0
            } else {
                code
            }
        };
        RegCloseKey(key);
        if result == 0 {
            Ok(())
        } else {
            Err(crate::tr_format!(
                "Der Autostart konnte nicht geändert werden (Fehler {result}).", "Could not change startup settings (error {result})."
            ))
        }
    }
}

// --- Global hotkeys -----------------------------------------------------------

const WM_APP_REBIND: u32 = 0x8000 + 1;
const WM_APP_QUIT: u32 = 0x8000 + 2;

/// A thread owning the hotkey registrations; bindings can change at runtime.
pub struct Hotkeys {
    thread_id: u32,
    bindings: Arc<Mutex<Vec<(i32, u32, u32)>>>,
}

impl Hotkeys {
    pub fn start(proxy: EventLoopProxy<AppEvent>) -> Option<Hotkeys> {
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*};
        let bindings: Arc<Mutex<Vec<(i32, u32, u32)>>> = Arc::new(Mutex::new(Vec::new()));
        let shared = bindings.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("hotkeys".into())
            .spawn(move || unsafe {
                let mut message = std::mem::zeroed();
                // Creating the queue first guarantees posted messages arrive.
                PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                let _ = sender.send(GetCurrentThreadId());
                let mut registered: Vec<i32> = Vec::new();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    match message.message {
                        WM_HOTKEY => {
                            let _ = proxy.send_event(AppEvent::Hotkey(message.wParam as i32));
                        }
                        WM_APP_REBIND => {
                            for id in registered.drain(..) {
                                UnregisterHotKey(std::ptr::null_mut(), id);
                            }
                            let wanted = shared.lock().map(|b| b.clone()).unwrap_or_default();
                            let mut failed = Vec::new();
                            for (id, modifiers, key) in wanted {
                                if RegisterHotKey(
                                    std::ptr::null_mut(),
                                    id,
                                    modifiers | MOD_NOREPEAT,
                                    key,
                                ) == 0
                                {
                                    failed.push(id);
                                } else {
                                    registered.push(id);
                                }
                            }
                            let _ = proxy.send_event(AppEvent::HotkeysApplied(failed));
                        }
                        WM_APP_QUIT => break,
                        _ => {}
                    }
                }
                for id in registered {
                    UnregisterHotKey(std::ptr::null_mut(), id);
                }
            })
            .ok()?;
        let thread_id = receiver.recv().ok()?;
        Some(Hotkeys {
            thread_id,
            bindings,
        })
    }

    /// Replaces all bindings: `(id, modifier flags, virtual key)`.
    pub fn apply(&self, bindings: Vec<(i32, u32, u32)>) {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
        if let Ok(mut shared) = self.bindings.lock() {
            *shared = bindings;
        }
        unsafe {
            PostThreadMessageW(self.thread_id, WM_APP_REBIND, 0, 0);
        }
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
        unsafe {
            PostThreadMessageW(self.thread_id, WM_APP_QUIT, 0, 0);
        }
    }
}

// --- Notification area icon ---------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    ToggleRecording,
    SaveReplay,
    Quit,
}

const WM_TRAY_CALLBACK: u32 = 0x8000 + 20;
const WM_TRAY_UPDATE: u32 = 0x8000 + 21;
const WM_TRAY_NOTIFY: u32 = 0x8000 + 22;
const WM_TRAY_QUIT: u32 = 0x8000 + 23;
const MENU_SHOW: usize = 1;
const MENU_TOGGLE: usize = 2;
const MENU_REPLAY: usize = 3;
const MENU_QUIT: usize = 4;

static TRAY_PROXY: Mutex<Option<EventLoopProxy<AppEvent>>> = Mutex::new(None);
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static TRAY_RECORDING: AtomicBool = AtomicBool::new(false);
static TRAY_ICON: AtomicIsize = AtomicIsize::new(0);
static TRAY_TOOLTIP: Mutex<String> = Mutex::new(String::new());
static TRAY_MESSAGE: Mutex<(String, String)> = Mutex::new((String::new(), String::new()));
static TRAY_READY: AtomicU32 = AtomicU32::new(0);

fn notify_icon_data() -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = TRAY_HWND.load(Ordering::SeqCst) as HWND;
    data.uID = 1;
    data
}

fn copy_wide(target: &mut [u16], text: &str) {
    let mut written = 0;
    for unit in text.encode_utf16() {
        if written + 1 >= target.len() {
            break;
        }
        target[written] = unit;
        written += 1;
    }
    target[written] = 0;
}

unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    match message {
        WM_TRAY_CALLBACK => {
            let event = (lparam as u32) & 0xFFFF;
            match event {
                WM_LBUTTONUP | 0x400 /* NIN_SELECT */ | 0x401 /* NIN_KEYSELECT */ | WM_LBUTTONDBLCLK => {
                    send_tray(TrayAction::Show);
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    let menu = CreatePopupMenu();
                    let recording = TRAY_RECORDING.load(Ordering::SeqCst);
                    let show = wide(crate::i18n::text("Stillclip öffnen", "Open Stillclip"));
                    let toggle = wide(if recording { crate::i18n::text("Aufnahme beenden", "Stop recording") } else { crate::i18n::text("Aufnahme starten", "Start recording") });
                    let replay = wide(crate::i18n::text("Moment sichern", "Save replay"));
                    let quit = wide(crate::i18n::text("Beenden", "Quit"));
                    AppendMenuW(menu, MF_STRING, MENU_SHOW, show.as_ptr());
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                    AppendMenuW(menu, MF_STRING, MENU_TOGGLE, toggle.as_ptr());
                    AppendMenuW(
                        menu,
                        MF_STRING | if recording { 0 } else { MF_GRAYED },
                        MENU_REPLAY,
                        replay.as_ptr(),
                    );
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                    AppendMenuW(menu, MF_STRING, MENU_QUIT, quit.as_ptr());
                    let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
                    GetCursorPos(&mut point);
                    SetForegroundWindow(hwnd);
                    let chosen = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN,
                        point.x,
                        point.y,
                        0,
                        hwnd,
                        std::ptr::null(),
                    );
                    PostMessageW(hwnd, WM_NULL, 0, 0);
                    DestroyMenu(menu);
                    match chosen as usize {
                        MENU_SHOW => send_tray(TrayAction::Show),
                        MENU_TOGGLE => send_tray(TrayAction::ToggleRecording),
                        MENU_REPLAY => send_tray(TrayAction::SaveReplay),
                        MENU_QUIT => send_tray(TrayAction::Quit),
                        _ => {}
                    }
                }
                _ => {}
            }
            0
        }
        WM_TRAY_UPDATE => {
            let mut data = notify_icon_data();
            data.uFlags = NIF_TIP | NIF_ICON;
            data.hIcon = TRAY_ICON.load(Ordering::SeqCst) as _;
            let tooltip = TRAY_TOOLTIP.lock().map(|t| t.clone()).unwrap_or_default();
            copy_wide(&mut data.szTip, &tooltip);
            Shell_NotifyIconW(NIM_MODIFY, &data);
            0
        }
        WM_TRAY_NOTIFY => {
            let mut data = notify_icon_data();
            data.uFlags = NIF_INFO;
            data.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME;
            let (title, text) = TRAY_MESSAGE.lock().map(|m| m.clone()).unwrap_or_default();
            copy_wide(&mut data.szInfoTitle, &title);
            copy_wide(&mut data.szInfo, &text);
            Shell_NotifyIconW(NIM_MODIFY, &data);
            0
        }
        WM_TRAY_QUIT => {
            let data = notify_icon_data();
            Shell_NotifyIconW(NIM_DELETE, &data);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn send_tray(action: TrayAction) {
    if let Ok(proxy) = TRAY_PROXY.lock() {
        if let Some(proxy) = proxy.as_ref() {
            let _ = proxy.send_event(AppEvent::Tray(action));
        }
    }
}

/// The notification-area icon with its own message loop thread.
pub struct Tray;

impl Tray {
    pub fn start(proxy: EventLoopProxy<AppEvent>) -> Option<Tray> {
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        if TRAY_READY.load(Ordering::SeqCst) != 0 {
            return None;
        }
        if let Ok(mut slot) = TRAY_PROXY.lock() {
            *slot = Some(proxy);
        }
        let icon = crate::icon::hicon(crate::icon::SIZE);
        TRAY_ICON.store(icon as isize, Ordering::SeqCst);
        if let Ok(mut tooltip) = TRAY_TOOLTIP.lock() {
            *tooltip = "Stillclip".into();
        }
        let (sender, receiver) = mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || unsafe {
                let class_name = wide("StillclipTrayWindow");
                let instance = GetModuleHandleW(std::ptr::null());
                let class = WNDCLASSW {
                    style: 0,
                    lpfnWndProc: Some(tray_window_proc),
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
                    0,
                    class_name.as_ptr(),
                    class_name.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if hwnd.is_null() {
                    let _ = sender.send(false);
                    return;
                }
                TRAY_HWND.store(hwnd as isize, Ordering::SeqCst);
                let mut data = notify_icon_data();
                data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
                data.uCallbackMessage = WM_TRAY_CALLBACK;
                data.hIcon = TRAY_ICON.load(Ordering::SeqCst) as _;
                copy_wide(&mut data.szTip, "Stillclip");
                if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                    let _ = sender.send(false);
                    return;
                }
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &data);
                TRAY_READY.store(1, Ordering::SeqCst);
                let _ = sender.send(true);
                let mut message = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                TRAY_READY.store(0, Ordering::SeqCst);
            })
            .ok()?;
        match receiver.recv() {
            Ok(true) => Some(Tray),
            _ => None,
        }
    }

    pub fn update(&self, recording: bool, tooltip: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
        TRAY_RECORDING.store(recording, Ordering::SeqCst);
        if let Ok(mut slot) = TRAY_TOOLTIP.lock() {
            *slot = tooltip.to_owned();
        }
        let hwnd = TRAY_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                PostMessageW(hwnd as HWND, WM_TRAY_UPDATE, 0, 0);
            }
        }
    }

    /// A balloon notification, used while the window is hidden.
    pub fn notify(&self, title: &str, text: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
        if let Ok(mut slot) = TRAY_MESSAGE.lock() {
            *slot = (title.to_owned(), text.to_owned());
        }
        let hwnd = TRAY_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                PostMessageW(hwnd as HWND, WM_TRAY_NOTIFY, 0, 0);
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
        let hwnd = TRAY_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                PostMessageW(hwnd as HWND, WM_TRAY_QUIT, 0, 0);
            }
        }
    }
}

/// Applies the generated icon to the main window's title bar and taskbar entry.
pub fn set_window_icon(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageW, ICON_BIG, ICON_SMALL, WM_SETICON,
    };
    let big = crate::icon::hicon(32);
    let small = crate::icon::hicon(16);
    unsafe {
        SendMessageW(hwnd as HWND, WM_SETICON, ICON_BIG as usize, big as isize);
        SendMessageW(
            hwnd as HWND,
            WM_SETICON,
            ICON_SMALL as usize,
            small as isize,
        );
    }
}

#[cfg(test)]
mod instance_tests {
    use super::InstanceGuard;

    #[test]
    fn a_second_instance_cannot_enter_recovery_until_the_first_exits() {
        let first = InstanceGuard::acquire().unwrap().expect("first instance");
        assert!(InstanceGuard::acquire().unwrap().is_none());
        drop(first);
        assert!(InstanceGuard::acquire().unwrap().is_some());
    }
}
