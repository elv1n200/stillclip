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

7. Vor dem Aufnehmen im Tonbereich den Audio-Test starten. Systemton und Mikrofon
   getrennt und gemeinsam testen, stummschalten sowie die Lautstärke ändern.
   Pegel müssen auf die jeweilige Quelle reagieren; den zehnsekündigen Mix anhören.
   Abbrechen und Abziehen des Geräts dürfen die Bedienung nicht blockieren.
8. Drei Exporte einreihen, währenddessen einen weiteren Clip öffnen. Den zweiten
   wartenden Auftrag entfernen und den aktiven abbrechen. Die anderen Aufträge
   müssen weiterlaufen; keine unfertige MP4 darf in der Bibliothek erscheinen.
   Währenddessen eine Aufnahme stoppen und Markierungen setzen. Schließen in
   den Infobereich lässt Exporte laufen; „Beenden“ bricht offene Exporte ab.
9. Einen Clip am Feld „Ziehen“ nach Discord ziehen. Die Datei muss als Anhang im
   Entwurf erscheinen; noch nicht absenden. Auch Unicode-Dateinamen, Escape zum
   Abbrechen und einen Datei-Explorer als Ziel prüfen. Beide Apps sollten mit
   derselben Windows-Berechtigungsstufe laufen.
10. Spiel und Tags speichern, App neu starten und Clip umbenennen. Angaben und
    Favoriten müssen erhalten bleiben. Datum und Länge kombiniert filtern.
11. Hochkantformat wählen, den Rahmen nach links und rechts verschieben und
    jeweils exportieren. Bildausschnitt und Auflösung prüfen, auch für Highlights
    aus mehreren Clips und beide Discord-Profile. Bei kleineren Dateilimits ist
    eine reduzierte Auflösung vorgesehen.

Die automatischen Tests prüfen zusätzlich Queue-Zustände, Abbruch eines echten
FFmpeg-Prozesses, Hochkantauflösung und Bildinhalt mit synthetischen Farbflächen,
Tagspeicherung, Pegelberechnung und das Windows-Dateiobjekt für Drag-and-drop.
Sie ersetzen keine Aufnahme mit realen Geräten oder das Ablegen in Discord.


## Desktop-Workflow ab 0.8

- Frisches Profil: Assistent in DE/EN, kein Aufnahmebeginn beim Öffnen; Bild-/Tontest nur nach ausdrücklichem Start. Vorschau, Abbruch, Abschluss und erneutes Öffnen prüfen.
- Während eines Tests oder einer Analyse Aufnahme anfordern: Vorgang wird abgebrochen, danach beginnt die gewünschte Aufnahme. Stoppen/Beenden darf keine FFmpeg-Prozesse oder Testaufnahmen zurücklassen.
- HUD in allen vier Ecken und bei 60/100/160 %, DPI-Skalierung und mehreren Bildschirmen prüfen. Mikrofonfehler, Benachrichtigung und Rückkehr zum HUD testen. Keine Zusage für exklusiven Vollbildmodus.
- Neue Aufnahme mit getrennten Quellen: Spielton und Mikrofon einzeln stummschalten und Hörvorschau/Export vergleichen. Alte Clips mit nur gemischtem Ton müssen die Einschränkung anzeigen.
- Echte VALORANT-Aufnahmen: Namensvarianten, eigene Kills/Tode, andere Spieler, mehrfache Kills, Killfeed-Verschiebung, Auflösungen und Seitenverhältnisse prüfen. Fehlalarme und übersehene Ereignisse dokumentieren. Synthetische OCR-Tests ersetzen diese Prüfung nicht.
- Updates ohne Netz, bei laufender Aufnahme/Export, mit manipulierten Downloads und nach heruntergeladener Dateiänderung prüfen. Signierte Updates nur mit tatsächlich vorhandener, vertrauenswürdiger Herausgeberidentität testen.
- Diagnosevorschau auf persönliche Pfade, Kontonamen und Fehlermeldungen prüfen; Bericht lokal speichern und Originalaufnahmen unverändert lassen.
