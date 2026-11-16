# Entwicklung

[Zur Projektübersicht](../README.md) · [Beitragen](../CONTRIBUTING.md)

Windows, Rust/Cargo und die Microsoft C++ Build Tools werden zum Bauen benötigt.
Im Repository-Hauptordner ausführen:

```powershell
cargo build --locked --release
cargo test --locked
```

Weitere Prüfungen:

```powershell
# Synthetische FFmpeg-Tests: Schnitt, Vorschaubild, Segment-Export (kein Bildschirm)
cargo test --locked synthetic_ -- --ignored

# Echte Aufnahme des Hauptbildschirms mit Systemton für etwa zehn Sekunden
cargo test --locked real_capture -- --ignored --nocapture

# Banner einmal anzeigen und als Bild festhalten
$env:STILLCLIP_OVERLAY_SHOT="shots/overlay.png"; cargo test --locked overlay_smoke -- --ignored

# Gerenderte Oberfläche als PNG, ohne Aufnahme
target/release/stillclip.exe --screenshot shots/library.png
target/release/stillclip.exe --screenshot shots/settings.png --page settings
target/release/stillclip.exe --screenshot shots/editor.png --page editor

# Eine bestimmte Stelle der Einstellungen (Element-ID nach dem #)
target/release/stillclip.exe --screenshot shots/ton.png --page "settings#system-volume-row"
```

Optional setzen `STILLCLIP_DATA_DIR` den Einstellungsordner und
`STILLCLIP_FFMPEG` den Pfad zu FFmpeg. `--minimized` startet nur mit dem
Symbol im Infobereich.

### Aufbau

| Datei | Inhalt |
| --- | --- |
| `src/main.rs` | Fenster, WebView, Ereignisschleife, IPC-Aktionen, Hintergrundjobs |
| `src/lifecycle.rs` | Getestete Start-/Stopp-Anforderungen während Aufnahmewechseln |
| `src/export.rs` | Unabhängige Exporte mit gesicherten Segmenten und geschützter Veröffentlichung |
| `src/discord.rs` | Discord-Dateilimits, Bitratenplanung, zweistufiger MP4-Export und Größenprüfung |
| `src/jobs.rs`, `src/process_tree.rs` | Exportliste, Fortschritt und Abbruch einschließlich Windows-Unterprozessen |
| `src/portrait.rs` | Validierter und verschiebbarer 9:16-Zuschnitt |
| `src/audio_test.rs` | Temporäre Audio-Vorschau mit den gewählten Tonquellen |
| `src/highlights.rs` | Mehrere Ausschnitte vereinheitlichen, verbinden und als Highlight exportieren |
| `src/diagnostics.rs` | Begrenztes Lesen und Auswerten vollständiger FFmpeg-Fortschrittsmeldungen |
| `src/games.rs` | Spieleliste, Fenster-/Prozess-Erkennung, Watcher-Thread |
| `src/capture.rs` | Bildschirme (DXGI), Encoder-Erkennung, FFmpeg-Kommandozeile |
| `src/audio.rs` | WASAPI-Aufnahme (Loopback/Mikrofon) in Named Pipes, Gerätewechsel |
| `src/recorder.rs` | Segment-Ring, Rückblick-Export, Sitzungsexport, Schnitt, Vorschaubilder |
| `src/overlay.rs` | Banner-Fenster im Spiel und Bestätigungston |
| `src/library.rs` | Clip-Ordner, Import, Umbenennen, Medienindex |
| `src/settings.rs` | Einstellungen, Tastenkürzel, Qualitätsstufen |
| `src/platform.rs` | Dialoge, Explorer, Papierkorb, Autostart, Hotkeys, Infobereich |
| `src/encoding.rs`, `src/mechanics.rs` | Qualitätsvorgaben, Zeitbereiche und dokumentierte Verhaltensregeln |
| `ui/` | Oberfläche (`index.html`, `app.css`, `app.js`), wird in die EXE eingebettet |

### Automatische Prüfungen und Downloads

`.github/workflows/verify.yml` baut auf Windows, führt Unit-/Regressionstests
und ausschließlich synthetische FFmpeg-Tests aus und stellt ein portables ZIP
als Actions-Artefakt bereit. FFmpeg und WebView2 werden nicht mitgeliefert.
Echte Bildschirm-/Mikrofonprüfungen bleiben ausdrücklich manuelle Tests.

Die Oberfläche wird separat mit simulierten Zustandsdaten geprüft:

```sh
npm ci
npx playwright install chromium
npm test
```

Optional verwendet `CHROME_BIN` ein vorhandenes Chromium. Mit
`STILLCLIP_SHOTS_DIR` werden Vorschauen bei 1280 und 720 Pixel Breite gespeichert.
Die Screenshots zeigen Browser-Vorschauen mit Testdaten.
Die Browserprüfung ersetzt keinen Windows-Test von WASAPI, Grafikkartentreibern
oder exklusiven Vollbildspielen.

Die manuellen Prüfungen für Fensteraufnahme, Spielton und Spieleprofile stehen
in [WINDOWS-CHECKS.md](../WINDOWS-CHECKS.md). Für FFmpeg sind aktuelle Windows-Builds
über die [offizielle Downloadseite](https://ffmpeg.org/download.html) verfügbar;
die Liste in **FFmpeg** und die Meldung unter **Aufnahmequelle**
zeigen, welche Funktionen die ausgewählte Datei unterstützt.

## Screenshots für die Dokumentation

```sh
npm ci
npx playwright install chromium
npm run screenshots
```

Das Skript rendert die unveränderte Oberfläche mit festen Demodaten, erzeugt ein
synthetisches Testvideo über FFmpeg und schreibt die kuratierten Bilder nach
`docs/images/`. Es nimmt weder Bildschirm noch Mikrofon auf. Erforderlich sind
FFmpeg im `PATH` und Chromium; `CHROME_BIN` kann einen vorhandenen Browser wählen.
Die Bilder müssen nach UI-Änderungen bewusst neu erzeugt und geprüft werden.

## Veröffentlichungen

Die Versionsnummer steht in `Cargo.toml` und im Stillclip-Eintrag von `Cargo.lock`.
Für eine Veröffentlichung muss der zugehörige Commit auf `main` alle Prüfungen
bestehen. Das daraus erstellte ZIP wird unverändert an den passenden Release
angehängt. `SHA256SUMS.txt` enthält dessen Prüfsumme. FFmpeg und WebView2 bleiben
externe Voraussetzungen und werden nicht in das Paket aufgenommen.

Der Workflow **Publish verified release** übernimmt diesen Schritt:

1. Einen Release-Entwurf anlegen. `target_commitish` muss die vollständige
   Commit-SHA des erfolgreichen **Build and verify**-Laufs auf `main` sein.
2. Unter **Actions → Publish verified release → Run workflow** die ID dieses
   Prüflaufs und die ID des Release-Entwurfs eintragen. Der Versions-Tag hat die
   Form `v0.6.0`.
3. Der Workflow prüft Quelle und Entwurf, lädt das unveränderte Paket,
   kontrolliert ZIP und Prüfsumme und vergleicht die hochgeladenen Dateien mit
   GitHubs SHA-256-Werten. Erst danach veröffentlicht er den Release.

Die IDs stehen in der Actions-URL beziehungsweise in der GitHub-API-Antwort
für den Release-Entwurf. Bereits veröffentlichte Releases werden abgewiesen.
Nur der manuell gestartete Veröffentlichungsjob hat Schreibrechte; der
normale Build bleibt auf lesenden Zugriff beschränkt.
