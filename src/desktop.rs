//! Desktop workflow orchestration. Only one analysis/test/update task runs at
//! once; file exports continue through the existing queue and source locks.
use crate::{audio_editor, i18n::text, jobs, killfeed, platform, settings, setup, support, updater, App, AppEvent, JobOutcome};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::VecDeque, path::PathBuf, time::Instant};
use tao::event_loop::EventLoopProxy;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub welcome_complete: bool,
    pub hud_enabled: bool,
    pub hud_position: String,
    pub hud_scale: u32,
    pub automatic_updates: bool,
    pub player_name: String,
    pub killfeed_region: killfeed::Region,
}
impl Default for Preferences {
    fn default() -> Self { Self { welcome_complete: false, hud_enabled: true, hud_position: "top_left".into(), hud_scale: 75,
        automatic_updates: false, player_name: String::new(), killfeed_region: killfeed::Region::default() } }
}
impl Preferences {
    pub fn validate(mut self) -> Result<Self, String> {
        if !["top_left", "top_right", "bottom_left", "bottom_right"].contains(&self.hud_position.as_str()) {
            return Err(text("Ungültige HUD-Position.", "Invalid HUD position.").into());
        }
        self.hud_scale = self.hud_scale.clamp(60, 160);
        self.player_name = self.player_name.trim().to_owned();
        if !self.player_name.is_empty() { killfeed::validate_name(&self.player_name)?; }
        if !self.killfeed_region.valid() { return Err(text("Ungültiger Killfeed-Bereich.", "Invalid killfeed region.").into()); }
        Ok(self)
    }
}

pub struct Task { pub id: u64, pub kind: &'static str, pub clip: Option<(u64, PathBuf)>, pub control: jobs::Control }
pub enum Output {
    Audio(u64, audio_editor::Analysis), Review(u64, killfeed::Review), Preview(setup::Preview),
    AudioPreview(u64, audio_editor::Preview),
    Update(Option<updater::Release>), Download(PathBuf), Install(PathBuf),
}
#[derive(Default)]
pub struct Desktop {
    pub task: Option<Task>,
    pub next_task: u64,
    pub audio: Option<(u64, audio_editor::Analysis)>,
    pub review: Option<(u64, killfeed::Review)>,
    pub accepted: Vec<usize>,
    pub preview: Option<setup::Preview>,
    pub audio_preview: Option<(u64, u64, audio_editor::Preview)>,
    pub release: Option<updater::Release>,
    pub downloaded: Option<PathBuf>,
    pub update_checked: bool,
    pub last_error: Option<String>,
    pub errors: VecDeque<String>,
    pub report: Option<Value>,
    pub last_saved: Option<Instant>,
}
impl Desktop {
    pub fn state(&self) -> Value {
        json!({
            "task":self.task.as_ref().map(|t| { let (percent, phase, eta) = t.control.view(); json!({"id":t.id,"kind":t.kind,"clip":t.clip.as_ref().map(|c| c.0),"percent":percent,"phase":phase,"eta":eta,"cancelling":t.control.cancelled()}) }),
            "audio":self.audio.as_ref().map(|(id, result)| json!({"clip":id,"result":result})),
            "review":self.review.as_ref().map(|(id, result)| json!({"clip":id,"result":result,"accepted":self.accepted})),
            "preview":self.preview.as_ref().map(|p| json!({"src":"http://clip.localhost/setup-test","info":p.info})),
            "audio_preview":self.audio_preview.as_ref().map(|(clip, job, _)| json!({"clip":clip,"src":format!("http://clip.localhost/audio-mix/{job}")})),
            "update":{"checked":self.update_checked,"release":self.release,"downloaded":self.downloaded.is_some(),"portable":!settings::application_dir().join("installed.flag").is_file()},
            "models_available":killfeed::models_dir().is_some(),"error":self.last_error,"report":self.report
        })
    }
    pub fn cancel(&self) { if let Some(task) = &self.task { task.control.cancel(); } }
}

impl App {
    pub(super) fn desktop_idle(&self) -> bool {
        !self.recording && !self.busy && !self.capture.transitioning && !self.capture.wants_start() && self.exports.is_empty() && self.audio_test.active.is_none() && self.dragging.is_none() && self.desktop.task.is_none()
    }

    fn desktop_task(&mut self, kind: &'static str, clip: Option<(u64, PathBuf)>, proxy: EventLoopProxy<AppEvent>,
        work: impl FnOnce(jobs::Control) -> Result<Output, String> + Send + 'static) {
        self.desktop.next_task += 1;
        let id = self.desktop.next_task;
        let control = jobs::Control::new();
        self.desktop.task = Some(Task { id, kind, clip, control: control.clone() });
        self.desktop.last_error = None;
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(control.clone())))
                .unwrap_or_else(|_| Err(text("Der Vorgang wurde unerwartet beendet.", "The operation stopped unexpectedly.").into()));
            let result = control.check().and(result);
            let _ = proxy.send_event(AppEvent::DesktopFinished(id, result));
        });
    }

    pub(super) fn desktop_finished(&mut self, id: u64, result: Result<Output, String>) {
        if self.desktop.task.as_ref().is_none_or(|t| t.id != id) { return; }
        self.desktop.task = None;
        match result {
            Ok(Output::Audio(id, data)) => self.desktop.audio = Some((id, data)),
            Ok(Output::Review(id, data)) => { self.desktop.review = Some((id, data)); self.desktop.accepted.clear(); },
            Ok(Output::Preview(preview)) => { self.desktop.preview = Some(preview); self.update_media_map(); },
            Ok(Output::AudioPreview(clip, preview)) => { self.desktop.audio_preview = Some((clip, id, preview)); self.update_media_map(); },
            Ok(Output::Update(release)) => { self.desktop.release = release; self.desktop.downloaded = None; self.desktop.update_checked = true; },
            Ok(Output::Download(path)) => self.desktop.downloaded = Some(path),
            Ok(Output::Install(path)) => {
                // Recheck activity after background verification. Never stop a
                // recording or queued export to make an update fit.
                if self.desktop_idle() && !self.closing {
                    match updater::launch_installer(&path) { Ok(()) => { self.closing = true; self.cancel_background(); }, Err(e) => self.message(Err(e)) }
                } else { self.message(Err(text("Das Update ist bereit. Beende laufende Vorgänge und starte die Installation erneut.", "The update is ready. Finish active tasks and start installation again.").into())); }
            }
            Err(error) if error == jobs::CANCELLED => { self.desktop.last_error = None; self.message(Ok(text("Vorgang abgebrochen.", "Operation cancelled.").into())); }
            Err(error) => { self.desktop.last_error = Some(error.clone()); self.message(Err(error)); }
        }
    }

    pub(super) fn desktop_action(&mut self, command: &Value, hwnd: isize, proxy: EventLoopProxy<AppEvent>) -> bool {
        let action = command["action"].as_str().unwrap_or("");
        if !matches!(action, "desktop_preferences" | "setup_finish" | "setup_test" | "setup_dismiss_preview" | "desktop_cancel" |
            "audio_analyze" | "audio_mix" | "audio_mix_preview" | "killfeed_analyze" | "killfeed_accept" | "update_check" | "update_download" | "update_install" |
            "report_preview" | "report_save") { return false; }
        if self.closing { return true; }
        if action == "desktop_cancel" { self.desktop.cancel(); return true; }
        if action == "setup_dismiss_preview" { self.desktop.preview = None; self.update_media_map(); return true; }
        let result: Result<(), String> = (|| {
            match action {
                "desktop_preferences" => {
                    let mut prefs: Preferences = serde_json::from_value(command["preferences"].clone()).map_err(|e| e.to_string())?;
                    prefs.welcome_complete = self.settings.desktop.welcome_complete;
                    let mut settings = self.settings.clone(); settings.desktop = prefs.validate()?;
                    settings::save_settings(&self.config, &settings)?; self.settings = settings;
                    self.message(Ok(text("Einstellungen gespeichert.", "Settings saved.").into()));
                }
                "report_preview" => self.desktop.report = Some(support::create(self)),
                "report_save" => {
                    let report = self.desktop.report.as_ref().ok_or(text("Öffne zuerst die Berichtsvorschau.", "Open the report preview first."))?;
                    if let Some(folder) = platform::pick_folder(hwnd) { let path = support::save(report, &folder)?; platform::reveal_in_explorer(&path)?; }
                }
                "killfeed_accept" => {
                    let index = command["suggestion"].as_u64().ok_or("Missing suggestion.")? as usize;
                    let (id, review) = self.desktop.review.as_ref().ok_or("No review available.")?;
                    if command["id"].as_u64() != Some(*id) { return Err("Suggestion belongs to another clip.".into()); }
                    let at = review.suggestions.get(index).ok_or("Unknown suggestion.")?.at;
                    let path = self.clip_path(command)?;
                    let mut markers = self.markers_of(&path);
                    if !markers.iter().any(|m| (*m - at).abs() < 0.1) { markers.push(at); self.remember_markers(&path, markers); }
                    if !self.desktop.accepted.contains(&index) { self.desktop.accepted.push(index); }
                }
                _ => {
                    if !self.desktop_idle() { return Err(text("Beende zuerst Aufnahme, Tests und laufende Aufträge.", "Finish recording, tests and active jobs first.").into()); }
                    match action {
                        "setup_finish" => {
                            let mut prefs: settings::Settings = serde_json::from_value(command["settings"].clone()).map_err(|e| e.to_string())?;
                            prefs.desktop = self.settings.desktop.clone(); prefs.desktop.welcome_complete = true;
                            let mut save = command.clone(); save["settings"] = serde_json::to_value(prefs).map_err(|e| e.to_string())?;
                            self.save_settings(&save);
                        }
                        "setup_test" => {
                            self.save_settings(command);
                            if self.error { return Ok(()); }
                            let ffmpeg = self.ffmpeg.clone().ok_or(text("FFmpeg fehlt.", "FFmpeg is missing."))?;
                            let config = self.session_config()?; self.session_settings = None;
                            let root = self.config.join("previews");
                            self.desktop.preview = None; self.update_media_map();
                            self.desktop_task("setup", None, proxy, move |control| setup::record(&ffmpeg, &root, config, &control).map(Output::Preview));
                        }
                        "update_check" => {
                            let portable = !settings::application_dir().join("installed.flag").is_file();
                            self.desktop_task("update_check", None, proxy, move |control| { control.check()?; updater::check(portable).map(Output::Update) });
                        }
                        "update_download" => {
                            let release = self.desktop.release.clone().ok_or(text("Prüfe zuerst auf Updates.", "Check for updates first."))?;
                            let root = self.config.join("updates");
                            self.desktop_task("update_download", None, proxy, move |control| updater::download(&release, &root, &control).map(Output::Download));
                        }
                        "update_install" => {
                            let path = self.desktop.downloaded.clone().ok_or("No downloaded update.")?;
                            let release = self.desktop.release.clone().ok_or("No release information.")?;
                            if release.portable { updater::verify_file(&path, &release)?; platform::reveal_in_explorer(&path)?; }
                            else { self.desktop_task("update_verify", None, proxy, move |control| {
                                updater::verify_file(&path, &release)?; control.check()?; updater::verify_publisher(&path)?; Ok(Output::Install(path))
                            }); }
                        }
                        "audio_analyze" | "killfeed_analyze" | "audio_mix" | "audio_mix_preview" => {
                            let path = self.clip_path(command)?;
                            let id = command["id"].as_u64().ok_or("Missing clip ID.")?;
                            let ffmpeg = self.ffmpeg.clone().ok_or(text("FFmpeg fehlt.", "FFmpeg is missing."))?;
                            let root = self.config.join("previews");
                            let start = command["start"].as_f64().unwrap_or(f64::NAN);
                            let end = command["end"].as_f64().unwrap_or(f64::NAN);
                            match action {
                                "audio_analyze" => {
                                    self.desktop.audio = None;
                                    self.desktop_task("audio", Some((id, path.clone())), proxy, move |control| audio_editor::analyze(&ffmpeg, &path, &root, &control).map(|r| Output::Audio(id, r)));
                                }
                                "killfeed_analyze" => {
                                    let prefs = self.settings.desktop.clone(); killfeed::validate_name(&prefs.player_name)?;
                                    let (before, after) = (self.settings.marker_before as f64, self.settings.marker_after as f64);
                                    self.desktop.review = None; self.desktop.accepted.clear();
                                    self.desktop_task("killfeed", Some((id, path.clone())), proxy, move |control| killfeed::analyze(&ffmpeg, &path, &root, &prefs.player_name, prefs.killfeed_region, start, end, before, after, &control).map(|r| Output::Review(id, r)));
                                }
                                "audio_mix_preview" => {
                                    let mix: audio_editor::Mix = serde_json::from_value(command["mix"].clone()).map_err(|e| e.to_string())?;
                                    let mix = mix.validate()?;
                                    self.desktop.audio_preview = None; self.update_media_map();
                                    self.desktop_task("audio_preview", Some((id, path.clone())), proxy, move |control| audio_editor::preview(&ffmpeg, &path, &root, start, end, mix, &control).map(|p| Output::AudioPreview(id, p)));
                                }
                                _ => {
                                    let mix: audio_editor::Mix = serde_json::from_value(command["mix"].clone()).map_err(|e| e.to_string())?;
                                    let mix = mix.validate()?; let crop = crate::parse_crop(&command["crop"])?;
                                    let output = crate::unique_trim_path(&self.settings.clips_dir, path.file_stem().unwrap_or_default().to_string_lossy().as_ref(), "Audio");
                                    self.queue_export(text("Audiomix", "Audio mix").into(), vec![path.clone()], Box::new(move |control| {
                                        audio_editor::export(&ffmpeg, &path, &output, start, end, mix, crop, &control)?;
                                        Ok(JobOutcome { message: text("Audiomix gespeichert. Der Originalclip bleibt erhalten.", "Audio mix saved. The original clip is preserved.").into(), saved: Some(output), session: None, replay: false })
                                    }), proxy);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result { self.desktop.last_error = Some(error.clone()); self.message(Err(error)); }
        true
    }

    pub(super) fn update_hud(&self) {
        let Some(overlay) = &self.overlay else { return; };
        if !self.settings.desktop.hud_enabled || !self.recording || self.closing { overlay.hud(None); return; }
        let Some(display) = self.capture_display() else { return; };
        let prefs = &self.settings.desktop;
        let left = prefs.hud_position.ends_with("left"); let bottom = prefs.hud_position.starts_with("bottom");
        let mic = self.session.as_ref().is_some_and(|s| s.microphone);
        let mut label = format!("REC {:02}:{:02} · MIC {}", self.elapsed / 60, self.elapsed % 60, if mic { "ON" } else { "OFF" });
        if let Some(at) = self.desktop.last_saved { label.push_str(&crate::tr_format!(" · Clip vor {} s", " · Clip {}s ago", at.elapsed().as_secs())); }
        overlay.hud(Some(crate::overlay::Banner { text: label, error: false,
            x: display.x + if left { 20 } else { display.width as i32 - 20 },
            y: display.y + if bottom { display.height as i32 - 24 } else { 20 },
            width_limit: (display.width as i32 - 100).clamp(180, 650), scale: prefs.hud_scale,
            anchor_left: left, anchor_bottom: bottom, persistent: true }));
    }
}
