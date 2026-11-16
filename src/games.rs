//! Game detection: a background thread watches visible windows for known or
//! user-added game executables and reports when one starts or ends.
use crate::AppEvent;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};
use tao::event_loop::EventLoopProxy;

/// One program to watch for. `exe` is the file name, compared case-insensitively.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameEntry {
    pub exe: String,
    pub name: String,
    /// Only matches windows whose title contains this text (for shared hosts like javaw.exe).
    pub title: String,
}

impl Default for GameEntry {
    fn default() -> Self {
        Self {
            exe: String::new(),
            name: String::new(),
            title: String::new(),
        }
    }
}

impl GameEntry {
    pub fn new(exe: &str, name: &str) -> Self {
        Self {
            exe: normalize_exe(exe),
            name: name.trim().to_owned(),
            title: String::new(),
        }
    }
    pub fn normalized(mut self) -> Self {
        self.exe = normalize_exe(&self.exe);
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            self.name = display_name(&self.exe);
        }
        self.title = self.title.trim().to_owned();
        self
    }
    pub fn is_valid(&self) -> bool {
        !self.exe.is_empty() && !self.exe.contains(['/', '\\'])
    }
    pub fn matches(&self, app: &RunningApp) -> bool {
        self.exe == app.exe
            && (self.title.is_empty()
                || app
                    .title
                    .to_lowercase()
                    .contains(&self.title.to_lowercase()))
    }
}

/// Lower-case file name without any directory part.
pub fn normalize_exe(value: &str) -> String {
    let trimmed = value.trim();
    let name = trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed)
        .to_lowercase();
    if name.is_empty() {
        name
    } else if name.ends_with(".exe") {
        name
    } else {
        format!("{name}.exe")
    }
}

/// `FortniteClient-Win64-Shipping.exe` → `FortniteClient`.
pub fn display_name(exe: &str) -> String {
    let stem = exe.trim_end_matches(".exe");
    let cut = stem
        .find("-win64")
        .or_else(|| stem.find("_win64"))
        .or_else(|| stem.find("-Win64"))
        .unwrap_or(stem.len());
    let base = &stem[..cut];
    let mut chars = base.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Popular games, keyed by their main executable. Users can add any other program.
pub const BUILTIN: &[(&str, &str)] = &[
    ("cs2.exe", "Counter-Strike 2"),
    ("csgo.exe", "CS:GO"),
    ("VALORANT-Win64-Shipping.exe", "VALORANT"),
    ("FortniteClient-Win64-Shipping.exe", "Fortnite"),
    ("r5apex.exe", "Apex Legends"),
    ("r5apex_dx12.exe", "Apex Legends"),
    ("League of Legends.exe", "League of Legends"),
    ("Overwatch.exe", "Overwatch 2"),
    ("RocketLeague.exe", "Rocket League"),
    ("GTA5.exe", "GTA V"),
    ("GTA5_Enhanced.exe", "GTA V"),
    ("RainbowSix.exe", "Rainbow Six Siege"),
    ("RainbowSix_DX11.exe", "Rainbow Six Siege"),
    ("RainbowSix_Vulkan.exe", "Rainbow Six Siege"),
    ("dota2.exe", "Dota 2"),
    ("TslGame.exe", "PUBG"),
    ("cod.exe", "Call of Duty"),
    ("ModernWarfare.exe", "Call of Duty"),
    ("destiny2.exe", "Destiny 2"),
    ("Warframe.x64.exe", "Warframe"),
    ("RustClient.exe", "Rust"),
    ("EscapeFromTarkov.exe", "Escape from Tarkov"),
    ("eldenring.exe", "Elden Ring"),
    ("nightreign.exe", "Elden Ring Nightreign"),
    ("Cyberpunk2077.exe", "Cyberpunk 2077"),
    ("Minecraft.Windows.exe", "Minecraft"),
    ("HaloInfinite.exe", "Halo Infinite"),
    ("BF2042.exe", "Battlefield 2042"),
    ("bfv.exe", "Battlefield V"),
    ("bf1.exe", "Battlefield 1"),
    ("helldivers2.exe", "Helldivers 2"),
    ("Marvel-Win64-Shipping.exe", "Marvel Rivals"),
    ("Palworld-Win64-Shipping.exe", "Palworld"),
    ("Discovery.exe", "THE FINALS"),
    ("project8.exe", "Deadlock"),
    ("ffxiv_dx11.exe", "Final Fantasy XIV"),
    ("Wow.exe", "World of Warcraft"),
    ("Diablo IV.exe", "Diablo IV"),
    ("HuntGame.exe", "Hunt: Showdown"),
    ("DeadByDaylight-Win64-Shipping.exe", "Dead by Daylight"),
    ("Phasmophobia.exe", "Phasmophobia"),
    ("Among Us.exe", "Among Us"),
    ("FallGuys_client_game.exe", "Fall Guys"),
    ("SoTGame.exe", "Sea of Thieves"),
    ("Terraria.exe", "Terraria"),
    ("Starfield.exe", "Starfield"),
    ("RDR2.exe", "Red Dead Redemption 2"),
    ("witcher3.exe", "The Witcher 3"),
    ("HogwartsLegacy.exe", "Hogwarts Legacy"),
    ("SpaceMarine2.exe", "Space Marine 2"),
    ("FC25.exe", "EA Sports FC 25"),
    ("FC26.exe", "EA Sports FC 26"),
    ("NBA2K25.exe", "NBA 2K25"),
    ("NBA2K26.exe", "NBA 2K26"),
    ("forzahorizon5.exe", "Forza Horizon 5"),
    ("Squad.exe", "Squad"),
    ("ArmaReforger.exe", "Arma Reforger"),
    ("RobloxPlayerBeta.exe", "Roblox"),
    ("GenshinImpact.exe", "Genshin Impact"),
    ("starwarsbattlefrontii.exe", "Star Wars Battlefront II"),
    ("Hearthstone.exe", "Hearthstone"),
    ("Enshrouded.exe", "Enshrouded"),
    ("valheim.exe", "Valheim"),
    ("FactoryGame-Win64-Shipping.exe", "Satisfactory"),
    ("bg3.exe", "Baldur's Gate 3"),
    ("bg3_dx11.exe", "Baldur's Gate 3"),
    ("MonsterHunterWilds.exe", "Monster Hunter Wilds"),
    ("PathOfExile.exe", "Path of Exile"),
    ("PathOfExileSteam.exe", "Path of Exile"),
    ("aces.exe", "War Thunder"),
    ("WorldOfTanks.exe", "World of Tanks"),
    ("Lethal Company.exe", "Lethal Company"),
    ("DeltaForceClient-Win64-Shipping.exe", "Delta Force"),
    ("REPO.exe", "R.E.P.O."),
    ("PEAK.exe", "PEAK"),
    ("Schedule I.exe", "Schedule I"),
    ("Splitgate2.exe", "Splitgate 2"),
    ("smite2.exe", "SMITE 2"),
    ("TheSims4.exe", "Die Sims 4"),
    ("stardew valley.exe", "Stardew Valley"),
    ("Hades2.exe", "Hades II"),
    ("Brawlhalla.exe", "Brawlhalla"),
    ("Ready or Not.exe", "Ready or Not"),
    ("Overwatch Launcher.exe", "Overwatch 2"),
    ("osu!.exe", "osu!"),
    ("Trackmania.exe", "Trackmania"),
    ("GeometryDash.exe", "Geometry Dash"),
    ("DOOMTheDarkAges.exe", "DOOM: The Dark Ages"),
    ("CoDCW.exe", "Call of Duty"),
];

pub fn builtin_games() -> Vec<GameEntry> {
    let mut games: Vec<GameEntry> = BUILTIN
        .iter()
        .map(|(exe, name)| GameEntry::new(exe, name))
        .collect();
    // Minecraft Java runs inside a generic Java host; match it by window title.
    games.push(GameEntry {
        exe: "javaw.exe".into(),
        name: "Minecraft".into(),
        title: "Minecraft".into(),
    });
    games
}

/// A top-level window with its owning executable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunningApp {
    pub hwnd: u64,
    pub pid: u32,
    pub exe: String,
    pub title: String,
    /// Display device name the window sits on, such as `\\.\DISPLAY2`.
    #[serde(default)]
    pub monitor: String,
}

const IGNORED_EXES: &[&str] = &[
    "explorer.exe",
    "stillclip.exe",
    "applicationframehost.exe",
    "textinputhost.exe",
    "systemsettings.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "lockapp.exe",
    "nvidia overlay.exe",
    "nvidia share.exe",
    "msedgewebview2.exe",
    "widgets.exe",
    "taskmgr.exe",
    "dwm.exe",
];

#[cfg(windows)]
pub fn window_size(hwnd: u64, pid: u32) -> Option<(u32, u32)> {
    use windows_sys::Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow},
    };
    let handle = hwnd as usize as _;
    let mut owner = 0;
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        if IsWindow(handle) == 0 || IsIconic(handle) != 0 {
            return None;
        }
        GetWindowThreadProcessId(handle, &mut owner);
        if owner != pid || GetWindowRect(handle, &mut rect) == 0 {
            return None;
        }
    }
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    (w > 1 && h > 1).then_some((w as u32, h as u32))
}

#[cfg(not(windows))]
pub fn window_size(_: u64, _: u32) -> Option<(u32, u32)> {
    None
}

/// Device name of the display a window sits on, empty when it cannot be told.
#[cfg(windows)]
fn window_monitor(hwnd: windows_sys::Win32::Foundation::HWND) -> String {
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
    };
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return String::new();
        }
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(monitor, std::ptr::addr_of_mut!(info).cast::<MONITORINFO>()) == 0 {
            return String::new();
        }
        let end = info
            .szDevice
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(info.szDevice.len());
        String::from_utf16_lossy(&info.szDevice[..end])
    }
}

#[cfg(windows)]
pub fn visible_apps() -> Vec<RunningApp> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HWND, LPARAM},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
            IsWindowVisible,
        },
    };

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> i32 {
        let apps = unsafe { &mut *(lparam as *mut Vec<RunningApp>) };
        unsafe {
            if IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let length = GetWindowTextLengthW(hwnd);
            if length <= 0 {
                return 1;
            }
            let mut title = vec![0_u16; length as usize + 1];
            let copied = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            let title = String::from_utf16_lossy(&title[..copied.max(0) as usize]);
            let mut pid = 0_u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 || pid == std::process::id() {
                return 1;
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return 1;
            }
            let mut buffer = vec![0_u16; 1024];
            let mut size = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &mut size,
            );
            CloseHandle(process);
            if ok == 0 {
                return 1;
            }
            let path = String::from_utf16_lossy(&buffer[..size as usize]);
            let exe = normalize_exe(&path);
            if exe.is_empty() || IGNORED_EXES.contains(&exe.as_str()) {
                return 1;
            }
            if !apps.iter().any(|a| a.pid == pid && a.title == title) {
                let monitor = window_monitor(hwnd);
                apps.push(RunningApp {
                    hwnd: hwnd as usize as u64,
                    pid,
                    exe,
                    title,
                    monitor,
                });
            }
        }
        1
    }

    let mut apps: Vec<RunningApp> = Vec::new();
    unsafe {
        EnumWindows(Some(callback), &mut apps as *mut _ as LPARAM);
    }
    apps
}

#[cfg(not(windows))]
pub fn visible_apps() -> Vec<RunningApp> {
    Vec::new()
}

/// Programs the user may add from the settings page: one row per executable.
pub fn candidate_apps() -> Vec<RunningApp> {
    let mut apps = visible_apps();
    apps.sort_by(|a, b| a.exe.cmp(&b.exe).then_with(|| a.title.cmp(&b.title)));
    apps.dedup_by(|a, b| a.exe == b.exe);
    apps
}

/// Watches for a configured game. Emits `GameStarted` once per session and
/// `GameStopped` after the window disappeared for two consecutive checks.
pub struct GameWatcher {
    stop: Arc<AtomicBool>,
    games: Arc<Mutex<Vec<GameEntry>>>,
    thread: Option<JoinHandle<()>>,
}

impl GameWatcher {
    pub fn start(proxy: EventLoopProxy<AppEvent>) -> GameWatcher {
        let stop = Arc::new(AtomicBool::new(false));
        let games: Arc<Mutex<Vec<GameEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = stop.clone();
        let thread_games = games.clone();
        let thread = std::thread::Builder::new()
            .name("game-watcher".into())
            .spawn(move || {
                let mut active: Option<(GameEntry, u32)> = None;
                let mut missing = 0;
                while !thread_stop.load(Ordering::SeqCst) {
                    let watched = thread_games.lock().map(|g| g.clone()).unwrap_or_default();
                    if !watched.is_empty() || active.is_some() {
                        let apps = visible_apps();
                        match &active {
                            None => {
                                let found = apps.iter().find_map(|app| {
                                    watched
                                        .iter()
                                        .find(|g| g.matches(app))
                                        .map(|g| (g.clone(), app.pid, app.monitor.clone()))
                                });
                                if let Some((entry, pid, monitor)) = found {
                                    active = Some((entry.clone(), pid));
                                    missing = 0;
                                    let _ = proxy.send_event(AppEvent::GameStarted(entry, monitor));
                                }
                            }
                            Some((entry, pid)) => {
                                let alive = apps.iter().any(|app| app.pid == *pid)
                                    || apps.iter().any(|app| entry.matches(app));
                                if alive {
                                    missing = 0;
                                } else {
                                    missing += 1;
                                    if missing >= 2 {
                                        let entry = entry.clone();
                                        active = None;
                                        let _ = proxy.send_event(AppEvent::GameStopped(entry));
                                    }
                                }
                            }
                        }
                    }
                    for _ in 0..20 {
                        if thread_stop.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            })
            .ok();
        GameWatcher {
            stop,
            games,
            thread,
        }
    }

    pub fn set_games(&self, games: Vec<GameEntry>) {
        if let Ok(mut slot) = self.games.lock() {
            *slot = games;
        }
    }
}

impl Drop for GameWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executables_are_normalized_and_matched() {
        assert_eq!(normalize_exe("C:/Games/CS2/cs2.exe"), "cs2.exe");
        assert_eq!(normalize_exe(" Fortnite "), "fortnite.exe");
        assert_eq!(normalize_exe(""), "");
        assert_eq!(
            display_name("FortniteClient-Win64-Shipping.exe"),
            "FortniteClient"
        );
        assert_eq!(display_name("cs2.exe"), "Cs2");
        let entry = GameEntry::new("CS2.EXE", " Counter-Strike 2 ");
        assert_eq!(entry.exe, "cs2.exe");
        assert_eq!(entry.name, "Counter-Strike 2");
        let app = RunningApp {
            hwnd: 0,
            pid: 7,
            exe: "cs2.exe".into(),
            title: "Counter-Strike 2".into(),
            monitor: "\\\\.\\DISPLAY1".into(),
        };
        assert!(entry.matches(&app));
        let minecraft = builtin_games()
            .into_iter()
            .find(|g| g.exe == "javaw.exe")
            .unwrap();
        assert!(minecraft.matches(&RunningApp {
            hwnd: 0,
            pid: 1,
            exe: "javaw.exe".into(),
            title: "Minecraft 1.21".into(),
            monitor: String::new(),
        }));
        assert!(!minecraft.matches(&RunningApp {
            hwnd: 0,
            pid: 1,
            exe: "javaw.exe".into(),
            title: "IntelliJ".into(),
            monitor: String::new(),
        }));
        let unnamed = GameEntry {
            exe: "Game-Win64-Shipping".into(),
            name: String::new(),
            title: String::new(),
        }
        .normalized();
        assert_eq!(unnamed.exe, "game-win64-shipping.exe");
        assert_eq!(unnamed.name, "Game");
        assert!(!GameEntry::new("", "x").is_valid());
    }

    #[test]
    fn builtin_list_has_unique_executables() {
        let games = builtin_games();
        let mut exes: Vec<&str> = games.iter().map(|g| g.exe.as_str()).collect();
        exes.sort();
        exes.dedup();
        assert_eq!(exes.len(), games.len());
        assert!(games.iter().all(|g| g.is_valid() && !g.name.is_empty()));
    }
}
