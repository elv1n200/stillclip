# Changelog

## 0.7.0 · 16 November 2026

- Switch between English and German instantly in Settings. The choice persists and also applies to native menus, dialogs, and app-owned errors.
- Choose a compact online installer or a complete offline installer, with Start menu integration and normal Windows uninstall.
- Both installers include verified FFmpeg tools and automatically install Microsoft WebView2 when needed. The portable ZIP now includes FFmpeg too.
- Installed settings and recordings live outside the application directory and survive updates and uninstall. Existing portable layouts remain compatible.
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
