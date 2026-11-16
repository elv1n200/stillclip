# Praktische Windows-Prüfung

Automatische Tests erzeugen nur synthetische Videos und erfassen weder Bildschirm
noch Mikrofon. Diese Prüfungen benötigen einen Windows-PC mit Spiel und Audiogeräten.

1. In den Einstellungen FFmpeg wählen. Für Fensteraufnahme muss unter Aufnahmequelle
   „Fensteraufnahme bereit“ stehen. Ein aktueller Build mit `gfxcapture`/`hwnd` ist nötig.
2. Für Valorant ein Profil mit 1080p, 60 FPS, 30 Sekunden Rückblick, Spielfenster
   und Spielton anlegen. „Profil übernehmen“, dann „Änderungen speichern“ wählen.
   Spiel starten; Aufnahmekarte und Rückblickknopf müssen die Profilwerte zeigen.
   Nach Stop müssen die allgemeinen Vorgaben unverändert sein.
3. Fenster-/randlosen Modus verwenden. Ein anderes Fenster über das Spiel legen;
   der gesicherte Clip darf dieses Fenster und den restlichen Desktop nicht zeigen.
   Fenster verschieben und Größe ändern. Minimieren oder Schließen muss die Aufnahme
   beenden; es darf keine Aufnahme des Desktops als Ersatz beginnen.
4. Während der Aufnahme Musik oder Discord auf einem anderen Programm abspielen.
   Im Clip dürfen bei „Nur Spielton“ nur das Spiel und das aktivierte Mikrofon sein.
   Die optionale zweite Mikrofonspur darf im Discord-Export nicht doppelt erklingen.
   Prozessaufnahme braucht Windows Build 20348+ (z. B. Windows 11).
5. Prüfen, ob Spielton nach Wechsel des Wiedergabegeräts weiter erfasst wird.
   Bei fehlender Unterstützung muss eine Fehlermeldung erscheinen; der Systemmix
   darf bei gewähltem Spielton nicht als Ersatz aufgenommen werden.
6. Zwei Minuten spielen und die Diagnose betrachten. Auflösung/FPS bei Bedarf
   senken. Danach während der Aufnahme einen Discord-/Highlight-Export ausführen
   und Spielleistung sowie Bild-/Tonsynchronität beurteilen. CPU-Last und reale
   Latenz hängen von PC, Spiel und Treibern ab und sind nicht durch CI abgedeckt.

Favoriten lassen sich unabhängig davon mit einem kleinen Testordner prüfen:
alte Clips als Favorit markieren, ein Ordnerlimit setzen und neu einlesen.
Favoriten müssen bleiben; andere alte Clips dürfen gemäß aktivierter Löschoption
entfernt werden. Einen Favoriten umbenennen und die App neu starten.
