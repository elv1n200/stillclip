# Changelog

## 0.9.0 · 16 November 2026

- Opt-in automatic VALORANT clips from the existing recording buffer. Repeatedly recognized kills are grouped into moments, saved locally with individual timeline markers, and tagged by game and kill count.
- Select a minimum of one to five kills, pre-roll, post-roll and grouping interval. Stable score changes and round banners separate groups; no ace, clutch or headshot claim is inferred from a kill count.
- One background worker with a bounded two-minute segment archive, backlog skipping, visible status and a pause button. Recording and stopping remain available; the final completed segment is preserved before buffer cleanup.
- Fast MP4 saves retain every recorded audio track. Temporary files are cleaned on completion, failure or cancellation.
- Visual killfeed calibration in a recorded frame, with percentage fields for keyboard input. Manual review now groups related kills, labels uncertain readings, and adds every event marker when accepted.
- Shared local OCR reads the killfeed and round context in a single inference image. No game API, game-memory access, client log or cloud processing is required.
- Extend unit, browser and synthetic video checks for grouping, stop hand-off, markers, audio tracks, settings migration and cancellation. Real VALORANT accuracy and gaming performance remain unverified; automatic detection is experimental and disabled by default.

## 0.8.0 · 16 November 2026

- First-run setup in English and German, with an explicitly started ten-second video/audio test and a disposable preview. Existing profiles can reopen the assistant without being forced through it.
- Waveforms, independent game/microphone gain and mute controls for newly recorded isolated tracks, short audio mix previews, and precise exports that preserve the original clip. Legacy mixed audio is clearly identified.
- Configurable recording HUD with four positions, adjustable size, actual microphone-source status, and time since the last saved clip.
- Explicit update checks, release notes, bounded downloads verified against GitHub SHA-256 metadata, and publisher checks before launching setup. Active recordings and exports are preserved. Portable updates retain a manual replacement step.
- Preview and save a diagnostic JSON report with application/Windows/FFmpeg versions, encoder capabilities and redacted errors; no automatic upload.
- Experimental offline VALORANT killfeed analysis with bundled, checksum-pinned OCR models, cancellable processing and manual review. Supports ASCII player names and a configurable region; real-match accuracy still needs evaluation.
- Optional Windows certificate-store signing for the app, installers and uninstallers. Trusted signing requires the owner’s certificate/provider; default builds remain unsigned.
- Extend synthetic media, OCR, interface and installation checks; keep analysis source clips protected and resume requested recording after cancelling analysis.

## 0.7.0 · 16 November 2026

- Switch between English and German instantly in Settings. The choice persists and also applies to native menus, dialogs, and app-owned errors.
- Choose a compact online installer or a complete offline installer, with Start menu integration and normal Windows uninstall.
- Both installers include verified FFmpeg tools and automatically install Microsoft WebView2 when needed. The portable ZIP now includes FFmpeg too.
- Installed settings and recordings live outside the application directory and survive updates and uninstall. Existing portable layouts remain compatible.
- Audio preview and two-pass Discord exports work with recent FFmpeg versions.
- Windows executable icon and version metadata; no separate Visual C++ runtime installation required.
- Pinned media sources, corresponding-source download, component checksums, and automated installation/upgrade/uninstall checks.

## 0.6.0 · 16. November 2026

### Aufnahme

- Eckige Oberfläche mit dunklem Terminal-Design und rosa Akzenten.
- Spieleprofile für Auflösung, Bildrate, Rückblicklänge, Ton und Aufnahmequelle.
- Fensteraufnahme über Windows Graphics Capture und Prozessloopback für Spielton.
- Aufnahmediagnose sowie ein zehnsekündiger Audio-Test mit Pegeln und Wiedergabe.

### Bibliothek und Editor

- Favoriten, Spielzuordnung und Tags mit Filtern nach Spiel, Tag, Datum und Länge.
- Highlight-Listen aus bis zu 20 Ausschnitten; Markierungen werden übernommen.
- Verschiebbarer 9:16-Zuschnitt mit 720×1280 oder 1080×1920.
- Native Dateiübergabe nach Discord oder in den Explorer.

### Export und Zuverlässigkeit

- Exportliste mit Fortschritt, geschätzter Restzeit und Abbrechen.
- Discord-Profile mit geprüften Dateilimits von 20 MB beziehungsweise 1 GB.
- Geschützte Quellclips und temporäre Ordner für unfertige Exportdateien.
- Abbrechen beendet auch Encoder, die über einen Programmstarter ausgeführt werden.
- Aufnahme und Export arbeiten unabhängig; Stoppen und Markieren bleiben verfügbar.
- Favoriten, Tags und Markierungen bleiben beim Umbenennen erhalten.

### Projekt

- Neue Projektseite, aktuelle Vorschauen und getrennte Anleitungen für Nutzung und Entwicklung.
- Portabler Windows-Download mit SHA-256-Prüfsumme.
- Vorlagen für Fehlerberichte, Funktionsvorschläge und Pull Requests.

Praktische Hinweise zur Windows-/FFmpeg-Unterstützung stehen im
[Schnellstart](README.md#quick-start) und in der [Windows-Prüfung](WINDOWS-CHECKS.md).
