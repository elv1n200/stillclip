# Zu Stillclip beitragen

Danke für dein Interesse. Stillclip ist eine lokale Windows-App; Änderungen sollen
den Aufnahmeablauf zuverlässig halten und zur klaren, eckigen Oberfläche passen.

## Einen Fehler melden

Nutze das [Fehlerformular](https://github.com/elv1n200/stillclip/issues/new?template=bug_report.yml).
Hilfreich sind die Stillclip-Version, Windows-Build, FFmpeg-Version, Schritte zum
Nachstellen und das erwartete Ergebnis. Bei Aufnahmeproblemen gehören Encoder,
Aufnahmequelle und Tonquelle dazu. Private Clips oder vollständige Protokolle sind
nicht erforderlich; persönliche Dateipfade vor dem Teilen entfernen.

## Eine Verbesserung vorschlagen

Beschreibe im [Funktionsvorschlag](https://github.com/elv1n200/stillclip/issues/new?template=feature_request.yml),
welches Problem du lösen möchtest und wie der Ablauf aussehen sollte. Ein konkretes
Beispiel ist hilfreicher als eine lange Liste möglicher Einstellungen.

## Code ändern

1. Einen eigenen Branch mit einem sachlichen Namen anlegen.
2. Änderungen auf ein verständliches Thema begrenzen.
3. Relevante Prüfungen aus [Entwicklung & Tests](docs/DEVELOPMENT.md) ausführen.
4. Einen Pull Request mit Problem, Änderung und tatsächlicher Validierung öffnen.

Für Änderungen an Aufnahme und Ton sind echte Windows-Prüfungen zusätzlich zu
synthetischen Tests wichtig. Echte Bildschirm- oder Mikrofonaufnahmen werden
nicht automatisch in CI ausgeführt. Laufende Aufnahmen, Originalclips und
Benutzereinstellungen dürfen durch Editor- oder Exportänderungen nicht beschädigt werden.

Nach sichtbaren UI-Änderungen die Dokumentationsbilder mit `npm run screenshots`
aktualisieren und prüfen. Neue Bedienmöglichkeiten brauchen verständliche Labels
und müssen auch bei kleiner Fenstergröße erreichbar bleiben.
