(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const copy = {
    desktop: ['Desktop & Einrichtung', 'Desktop & setup'], intro: ['Einrichten, aufnehmen und auf dem neuesten Stand bleiben.', 'Set up, record and stay up to date.'],
    setup: ['Einrichtungsassistent', 'Setup assistant'], hud: ['Aufnahme-HUD anzeigen', 'Show recording HUD'], position: ['HUD-Position', 'HUD position'], scale: ['HUD-Größe', 'HUD size'],
    tl: ['Oben links', 'Top left'], tr: ['Oben rechts', 'Top right'], bl: ['Unten links', 'Bottom left'], br: ['Unten rechts', 'Bottom right'],
    hudHelp: ['Status und letzter gespeicherter Clip. Für Spiele im Fenster oder randlosen Vollbild.', 'Status and last saved clip. For windowed or borderless games.'],
    updates: ['Updates', 'Updates'], autoUpdate: ['Beim Start auf Updates prüfen', 'Check for updates at startup'], check: ['Auf Updates prüfen', 'Check for updates'],
    download: ['Update herunterladen', 'Download update'], install: ['Installieren & Stillclip schließen', 'Install & close Stillclip'], portable: ['Portable-Paket anzeigen', 'Show portable package'],
    portableHelp: ['Stillclip schließen, das geprüfte ZIP in einen neuen Ordner entpacken und den bisherigen settings-Ordner übernehmen. Den Clip-Ordner beibehalten.', 'Close Stillclip, extract the verified ZIP into a new folder and copy your previous settings folder. Keep the existing clips folder.'],
    updateHelp: ['Aufnahmen und Exporte müssen vor einem Update beendet sein. Die Prüfung verbindet sich mit GitHub.', 'Finish recordings and exports before updating. Checking connects to GitHub.'],
    report: ['Diagnosebericht ansehen', 'Preview diagnostic report'], reportTitle: ['Diagnosebericht', 'Diagnostic report'], reportHelp: ['Persönliche Pfade sind entfernt. Prüfe den Inhalt vor dem Teilen. Es wird nichts hochgeladen.', 'Personal paths are removed. Review the contents before sharing. Nothing is uploaded.'],
    saveReport: ['Bericht speichern …', 'Save report…'], close: ['Schließen', 'Close'], cancel: ['Abbrechen', 'Cancel'],
    welcome: ['Willkommen bei Stillclip', 'Welcome to Stillclip'], welcomeSub: ['Dein erster Clip, in drei Schritten.', 'Your first clip, in three steps.'],
    step1: ['1 / Sprache & Aufnahmequelle', '1 / Language & capture source'], step2: ['2 / Bild & Ton', '2 / Video & audio'], step3: ['3 / Test & Abschluss', '3 / Test & finish'],
    language: ['Sprache', 'Language'], game: ['Spiel', 'Game'], screen: ['Bildschirm', 'Display'], desktopSource: ['Bildschirm / manuell', 'Display / manual'],
    window: ['Nur das Spielfenster aufnehmen', 'Capture only the game window'], gameAudio: ['Nur den Spielton aufnehmen', 'Capture only game audio'],
    sourceHelp: ['Für Fensteraufnahme und Spielton muss das gewählte Spiel laufen. Ohne diese Optionen wird der gewählte Bildschirm mit Systemton aufgenommen.', 'The selected game must be running for window capture and game audio. Otherwise, capture uses the selected display and system audio.'],
    quality: ['Qualität', 'Quality'], fps: ['Bildrate', 'Frame rate'], system: ['System- / Spielton', 'System / game audio'], mic: ['Mikrofon', 'Microphone'],
    micDevice: ['Mikrofon-Gerät', 'Microphone device'], defaultDevice: ['Windows-Standard', 'Windows default'], separate: ['Spielton und Mikrofon zusätzlich getrennt speichern', 'Also save game audio and microphone separately'],
    separateHelp: ['Die normale Wiedergabe enthält beide Quellen. Getrennte Spuren erlauben spätere Lautstärkeänderungen und benötigen etwas mehr Speicher.', 'Normal playback includes both sources. Separate tracks allow later volume edits and use a little more storage.'],
    testHelp: ['Der Test nimmt die gewählte Bildquelle und den aktivierten Ton für zehn Sekunden auf. Er startet erst mit dem Knopf unten. Die Vorschau wird anschließend wieder entfernt.', 'The test records the selected video source and enabled audio for ten seconds. It starts only when you press the button below. The preview is removed afterwards.'],
    test: ['10 Sekunden testen', 'Test for 10 seconds'], finish: ['Einrichtung abschließen', 'Finish setup'], back: ['Zurück', 'Back'], next: ['Weiter', 'Next'],
    finishHelp: ['Du kannst die Einrichtung auch ohne Test abschließen und später erneut öffnen.', 'You can finish without a test and reopen setup later.'],
    listenMix: ['Mix anhören (bis 20 s)', 'Listen to mix (up to 20s)'],
    audioTitle: ['Audio & Wellenform', 'Audio & waveform'], analyzeAudio: ['Wellenform laden', 'Load waveform'], saveMix: ['Bereich mit Audiomix speichern', 'Save selection with audio mix'],
    audioHelp: ['Lade die Audiospuren, um die Lautstärke zu bearbeiten. Das Speichern erstellt einen neuen Clip.', 'Load the audio tracks to edit their volume. Saving creates a new clip.'],
    mixedHelp: ['Dieser Clip enthält bereits gemischten Ton. Spiel und Mikrofon lassen sich darin nicht unabhängig entfernen.', 'This clip already contains mixed audio. Game audio and microphone cannot be removed independently.'],
    separatedHelp: ['Spielton und Mikrofon sind getrennt bearbeitbar. Die Videovorschau spielt den ursprünglichen Mix.', 'Game audio and microphone can be edited separately. Video preview plays the original mix.'],
    noAudio: ['Dieser Clip enthält keinen Ton.', 'This clip has no audio.'], gameTrack: ['Spielton', 'Game audio'], micTrack: ['Mikrofon', 'Microphone'], mixedTrack: ['Gemischter Ton', 'Mixed audio'], mute: ['Stumm', 'Mute'],
    suggestions: ['VALORANT · Highlight-Vorschläge', 'VALORANT · highlight suggestions'], experimental: ['Experimentell · nur lokal', 'Experimental · local only'],
    killfeedHelp: ['Analysiert den markierten Bereich, maximal 30 Minuten. Namen und Killfeed-Layout können die Erkennung beeinflussen. Prüfe jeden Vorschlag; es wird nichts automatisch exportiert.', 'Analyzes the selected range, up to 30 minutes. Names and killfeed layout can affect recognition. Review each suggestion; nothing is exported automatically.'],
    player: ['Dein Name im Killfeed', 'Your name in the killfeed'], playerHelp: ['Ohne #Tag; derzeit lateinische ASCII-Zeichen. Der Name bleibt auf diesem Gerät.', 'Without #tag; currently Latin ASCII characters. The name stays on this device.'],
    region: ['Killfeed-Bereich anpassen (%)', 'Adjust killfeed region (%)'], left: ['Links', 'Left'], top: ['Oben', 'Top'], width: ['Breite', 'Width'], height: ['Höhe', 'Height'],
    scan: ['Bereich analysieren', 'Analyze selection'], preview: ['Bereich ansehen', 'Preview range'], accept: ['Markierung übernehmen', 'Keep marker'], accepted: ['Übernommen', 'Kept'],
    noSuggestions: ['Keine bestätigten Vorschläge. Prüfe den Namen und den Killfeed-Bereich; die Erkennung kann Szenen übersehen.', 'No confirmed suggestions. Check the name and killfeed region; detection can miss scenes.'],
    noModels: ['OCR-Modelle fehlen. Nutze den Installer oder das vollständige Portable-Paket.', 'OCR models are missing. Use the installer or the complete portable package.'],
  };
  const t = key => copy[key]?.[document.documentElement.lang === 'en' ? 1 : 0] || key;
  const label = key => `<span data-copy="${key}"></span>`;
  const button = (id, key, extra = '') => `<button id="${id}" class="btn ${extra}" type="button">${label(key)}</button>`;
  const ipc = (action, fields = {}) => window.ipc?.postMessage(JSON.stringify({ action, ...fields }));
  const create = (html, parent) => { const template = document.createElement('template'); template.innerHTML = html; parent.append(template.content); };
  let state = {}, selected = null, initialized = false, audioKey = '', reviewKey = '', previousFocus = null;
  let wizard = null, step = 0, finishPending = false, autoShown = false;
  const gains = { game: 100, microphone: 100, mixed: 100 };
  const muted = { game: false, microphone: false, mixed: false };

  create(`<div class="settings-card desktop-card" id="desktop-settings">
    <div class="settings-card-heading"><div><h2>${label('desktop')}</h2><p>${label('intro')}</p></div></div>
    <div class="desktop-body">
      <div class="desktop-actions">${button('setup-open','setup')}${button('report-open','report')}</div>
      <label class="desktop-check"><input type="checkbox" id="hud-enabled">${label('hud')}</label>
      <div class="desktop-row"><label for="hud-position">${label('position')}</label><select id="hud-position"><option value="top_left" data-copy="tl"></option><option value="top_right" data-copy="tr"></option><option value="bottom_left" data-copy="bl"></option><option value="bottom_right" data-copy="br"></option></select></div>
      <div class="desktop-row"><label for="hud-scale">${label('scale')}</label><input id="hud-scale" type="range" min="60" max="160" step="5"><output id="hud-scale-value"></output></div>
      <p class="desktop-help">${label('hudHelp')}</p>
      <h3>${label('updates')}</h3><label class="desktop-check"><input type="checkbox" id="automatic-updates">${label('autoUpdate')}</label>
      <p class="desktop-help">${label('updateHelp')}</p><p id="update-summary" role="status"></p>
      <div class="desktop-actions">${button('update-check','check')}${button('update-download','download')}${button('update-install','install')}</div>
      <p class="desktop-help" id="portable-update-help" hidden>${label('portableHelp')}</p><pre class="desktop-notes" id="update-notes" hidden></pre>
    </div></div>`, $('page-settings'));
  create(`<div class="desktop-progress" id="desktop-progress" hidden><div class="desktop-actions"><span id="desktop-phase" role="status"></span>${button('desktop-cancel','cancel')}</div><progress id="desktop-meter" max="100"></progress></div>`, document.querySelector('main'));
  create(`<div class="setup-backdrop" id="setup-backdrop" hidden><section class="setup-dialog" role="dialog" aria-modal="true" aria-labelledby="setup-title">
    <header class="setup-header"><div><p class="setup-kicker">stillclip / setup</p><h2 id="setup-title">${label('welcome')}</h2><p class="desktop-help">${label('welcomeSub')}</p></div>${button('setup-close','close')}</header>
    <div class="setup-step" data-step="0"><h3>${label('step1')}</h3>
      <div class="desktop-row"><label for="wizard-language">${label('language')}</label><select id="wizard-language"><option value="de">Deutsch</option><option value="en">English</option></select></div>
      <div class="desktop-row"><label for="wizard-game">${label('game')}</label><select id="wizard-game"></select></div>
      <div class="desktop-row"><label for="wizard-display">${label('screen')}</label><select id="wizard-display"></select></div>
      <label class="desktop-check"><input type="checkbox" id="wizard-window">${label('window')}</label>
      <label class="desktop-check"><input type="checkbox" id="wizard-game-audio">${label('gameAudio')}</label><p class="desktop-help">${label('sourceHelp')}</p>
    </div>
    <div class="setup-step" data-step="1" hidden><h3>${label('step2')}</h3>
      <div class="desktop-row"><label for="wizard-quality">${label('quality')}</label><select id="wizard-quality"><option value="p720">720p</option><option value="p1080">1080p</option><option value="p1440">1440p</option><option value="native">Native</option></select></div>
      <div class="desktop-row"><label for="wizard-fps">${label('fps')}</label><select id="wizard-fps"><option value="30">30 FPS</option><option value="60">60 FPS</option></select></div>
      <label class="desktop-check"><input type="checkbox" id="wizard-system">${label('system')}</label><label class="desktop-check"><input type="checkbox" id="wizard-mic">${label('mic')}</label>
      <div class="desktop-row"><label for="wizard-mic-device">${label('micDevice')}</label><select id="wizard-mic-device"></select></div>
      <label class="desktop-check"><input type="checkbox" id="wizard-separate">${label('separate')}</label><p class="desktop-help">${label('separateHelp')}</p>
    </div>
    <div class="setup-step" data-step="2" hidden><h3>${label('step3')}</h3><p class="desktop-help">${label('testHelp')}</p>
      <div class="desktop-actions">${button('wizard-test','test')}${button('wizard-cancel','cancel')}</div><p id="wizard-progress" role="status"></p>
      <video id="wizard-preview" class="setup-preview" controls preload="metadata" hidden></video><p id="wizard-warnings" class="desktop-help"></p><p class="desktop-help">${label('finishHelp')}</p>
    </div><p id="wizard-error" class="desktop-error" role="alert"></p>
    <footer class="setup-footer">${button('wizard-back','back')}<div>${button('wizard-next','next','btn-primary')}${button('wizard-finish','finish','btn-primary')}</div></footer>
  </section></div>
  <div class="setup-backdrop" id="report-backdrop" hidden><section class="setup-dialog setup-report" role="dialog" aria-modal="true" aria-labelledby="report-title">
    <header class="setup-header"><h2 id="report-title">${label('reportTitle')}</h2>${button('report-close','close')}</header>
    <p class="desktop-help">${label('reportHelp')}</p><textarea id="report-content" readonly aria-labelledby="report-title"></textarea><div class="desktop-actions">${button('report-save','saveReport')}</div>
  </section></div>`, document.body);
  create(`<details class="desktop-tools" id="audio-tools"><summary>${label('audioTitle')}</summary><p class="desktop-help" id="audio-help"></p>
    <div class="desktop-actions">${button('audio-analyze','analyzeAudio')}</div><div id="audio-waveforms"></div><div class="desktop-actions">${button('audio-listen-mix','listenMix')}${button('audio-save-mix','saveMix')}</div><audio id="mix-preview" controls preload="metadata" hidden></audio>
  </details><details class="desktop-tools" id="killfeed-tools"><summary>${label('suggestions')}</summary><p class="setup-kicker">${label('experimental')}</p><p class="desktop-help">${label('killfeedHelp')}</p>
    <div class="desktop-row"><label for="killfeed-player">${label('player')}</label><input id="killfeed-player" maxlength="32" autocomplete="off" spellcheck="false"></div><p class="desktop-help">${label('playerHelp')}</p>
    <details><summary>${label('region')}</summary><div class="killfeed-region">${['left','top','width','height'].map(k => `<label for="killfeed-${k}">${label(k)}<input id="killfeed-${k}" type="number" min="0" max="100" step="1"></label>`).join('')}</div></details>
    <div class="desktop-actions">${button('killfeed-analyze','scan')}</div><p id="killfeed-summary" class="desktop-help" role="status"></p><ol id="killfeed-suggestions" class="killfeed-suggestions"></ol>
  </details><div class="desktop-progress" id="editor-desktop-progress" hidden><div class="desktop-actions"><span id="editor-desktop-phase" role="status"></span>${button('editor-desktop-cancel','cancel')}</div></div><p id="desktop-editor-error" class="desktop-error" role="alert"></p>`, document.querySelector('.editor-body'));

  function translate() { document.querySelectorAll('[data-copy]').forEach(node => { node.textContent = t(node.dataset.copy); }); }
  function idle() { return !state.recording && !state.busy && !state.capture_busy && !state.exports?.length && !state.audio_test?.running && !state.dragging && !state.desktop?.task; }
  function prefs() { return structuredClone(state.settings?.desktop || {}); }
  function savePreferences() {
    const next = prefs(); next.hud_enabled = $('hud-enabled').checked; next.hud_position = $('hud-position').value;
    next.hud_scale = Number($('hud-scale').value); next.automatic_updates = $('automatic-updates').checked;
    next.player_name = $('killfeed-player').value.trim();
    next.killfeed_region = Object.fromEntries(['left','top','width','height'].map(k => [k, Number($('killfeed-' + k).value) / 100]));
    ipc('desktop_preferences', { preferences: next });
  }
  function options(id, items, selectedValue) {
    $(id).replaceChildren(...items.map(([value, text]) => { const node = document.createElement('option'); node.value = value; node.textContent = text; return node; }));
    $(id).value = selectedValue;
  }
  function readWizard() {
    if (!wizard) return;
    Object.assign(wizard, { language: $('wizard-language').value, target_exe: $('wizard-game').value, display: $('wizard-display').value,
      capture_window: $('wizard-window').checked, game_audio: $('wizard-game-audio').checked, quality: $('wizard-quality').value,
      fps: Number($('wizard-fps').value), system_audio: $('wizard-system').checked, microphone: $('wizard-mic').checked,
      microphone_device: $('wizard-mic-device').value, microphone_separate_track: $('wizard-separate').checked });
  }
  function openWizard() {
    if (!idle()) return;
    wizard = structuredClone(state.settings); step = 0; finishPending = false; previousFocus = document.activeElement;
    $('wizard-language').value = wizard.language;
    const games = [...(state.builtin_games || []), ...(wizard.custom_games || [])];
    options('wizard-game', [['', t('desktopSource')], ...games.map(g => [g.exe, g.name]), ...(wizard.target_exe && !games.some(g => g.exe === wizard.target_exe) ? [[wizard.target_exe,wizard.target_exe]] : [])], wizard.target_exe);
    options('wizard-display', [['', t('defaultDevice')], ...(state.displays || []).map(d => [d.name, d.label])], wizard.display);
    options('wizard-mic-device', [['', t('defaultDevice')], ...(state.microphones || []).map(d => [d.id, d.name])], wizard.microphone_device);
    $('wizard-window').checked = wizard.capture_window; $('wizard-game-audio').checked = wizard.game_audio;
    $('wizard-quality').value = wizard.quality; $('wizard-fps').value = [30,60].includes(wizard.fps) ? wizard.fps : 60;
    $('wizard-system').checked = wizard.system_audio; $('wizard-mic').checked = wizard.microphone; $('wizard-separate').checked = wizard.microphone_separate_track;
    $('setup-backdrop').hidden = false; document.body.style.overflow = 'hidden'; renderWizard(); $('wizard-language').focus();
  }
  function closeWizard() {
    if (state.desktop?.task?.kind === 'setup') ipc('desktop_cancel');
    $('wizard-preview').pause(); $('wizard-preview').removeAttribute('src'); $('wizard-preview').load(); ipc('setup_dismiss_preview');
    $('setup-backdrop').hidden = true; wizard = null; document.body.style.overflow = ''; previousFocus?.focus();
  }
  function renderWizard() {
    if (!wizard) return;
    document.querySelectorAll('[data-step]').forEach(node => { node.hidden = Number(node.dataset.step) !== step; });
    $('wizard-back').disabled = step === 0 || !idle(); $('wizard-next').hidden = step === 2; $('wizard-finish').hidden = step !== 2;
    $('wizard-next').disabled = !idle(); $('wizard-finish').disabled = !idle(); $('wizard-test').disabled = !idle() || !state.ffmpeg_available;
    $('wizard-cancel').hidden = state.desktop?.task?.kind !== 'setup';
    $('wizard-progress').textContent = state.desktop?.task?.kind === 'setup' ? `${state.desktop.task.phase} · ${state.desktop.task.percent}%` : '';
    $('wizard-error').textContent = state.desktop?.error || (state.error ? state.status : '');
    const preview = state.desktop?.preview;
    $('wizard-preview').hidden = !preview;
    if (preview && $('wizard-preview').getAttribute('src') !== preview.src) $('wizard-preview').src = preview.src;
    $('wizard-warnings').textContent = preview?.info?.warnings?.join('\n') || '';
    if (finishPending && state.settings?.desktop?.welcome_complete && !state.error) closeWizard();
  }
  function drawAudio() {
    const data = selected && state.desktop?.audio?.clip === selected.id ? state.desktop.audio.result : null;
    const signature = JSON.stringify([selected?.id, data, document.documentElement.lang]);
    $('audio-analyze').disabled = !selected || !idle(); $('audio-save-mix').disabled = $('audio-listen-mix').disabled = !data?.tracks?.length || !idle();
    const preview = selected && state.desktop?.audio_preview?.clip === selected.id ? state.desktop.audio_preview : null;
    $('mix-preview').hidden = !preview;
    if (preview && $('mix-preview').getAttribute('src') !== preview.src) $('mix-preview').src = preview.src;
    if (!preview) { $('mix-preview').pause(); $('mix-preview').removeAttribute('src'); }
    $('audio-help').textContent = t(!data ? 'audioHelp' : !data.tracks.length ? 'noAudio' : data.separated ? 'separatedHelp' : 'mixedHelp');
    if (signature === audioKey) return;
    audioKey = signature; $('audio-waveforms').replaceChildren();
    for (const track of data?.tracks || []) {
      const row = document.createElement('div'); row.className = 'waveform-track';
      const name = document.createElement('label'); name.htmlFor = 'mix-' + track.role; name.textContent = t(track.role === 'game' ? 'gameTrack' : track.role === 'microphone' ? 'micTrack' : 'mixedTrack');
      const canvas = document.createElement('canvas'); canvas.width = 1200; canvas.height = 70;
      canvas.setAttribute('role','img'); canvas.setAttribute('aria-label', name.textContent + ' · ' + t('audioTitle'));
      const ctx = canvas.getContext('2d'); ctx.fillStyle = '#090909'; ctx.fillRect(0,0,1200,70); ctx.fillStyle = '#e5a0b7';
      (track.peaks || []).forEach((peak, i, all) => { const height = Math.max(1,Math.min(1,peak)*64); ctx.fillRect(i*1200/all.length,(70-height)/2,Math.max(1,1200/all.length-0.4),height); });
      canvas.addEventListener('click', event => { const fraction = (event.clientX-canvas.getBoundingClientRect().left)/canvas.getBoundingClientRect().width; $('editor-video').currentTime = Math.max(0,Math.min(data.duration,fraction*data.duration)); });
      const controls = document.createElement('div'); controls.className = 'waveform-controls';
      const volume = document.createElement('input'); volume.id = name.htmlFor; volume.type = 'range'; volume.min = '0'; volume.max = '200'; volume.value = gains[track.role];
      const out = document.createElement('output'); out.textContent = volume.value + '%';
      volume.addEventListener('input', () => { gains[track.role] = Number(volume.value); out.textContent = volume.value + '%'; });
      const mute = document.createElement('label'); mute.className = 'desktop-check';
      const checkbox = document.createElement('input'); checkbox.type = 'checkbox'; checkbox.checked = muted[track.role]; checkbox.addEventListener('change', () => { muted[track.role] = checkbox.checked; });
      mute.append(checkbox,document.createTextNode(t('mute'))); controls.append(volume,out,mute); row.append(name,canvas,controls); $('audio-waveforms').append(row);
    }
  }
  function renderReview() {
    const review = selected && state.desktop?.review?.clip === selected.id ? state.desktop.review : null;
    const key = JSON.stringify([review, document.documentElement.lang]);
    $('killfeed-analyze').disabled = !selected || !idle() || !state.desktop?.models_available || !$('killfeed-player').value.trim();
    $('killfeed-summary').textContent = state.desktop?.models_available === false ? t('noModels') : review && !review.result.suggestions.length ? t('noSuggestions') : '';
    if (key === reviewKey) return; reviewKey = key; $('killfeed-suggestions').replaceChildren();
    (review?.result?.suggestions || []).forEach((suggestion,index) => {
      const row = document.createElement('li'); const info = document.createElement('span'); info.className = 'suggestion-text';
      info.textContent = `${Math.floor(suggestion.at/60)}:${String(Math.floor(suggestion.at%60)).padStart(2,'0')} · ${suggestion.text}`;
      const preview = document.createElement('button'); preview.className = 'btn small'; preview.textContent = t('preview'); preview.addEventListener('click', () => window.selectEditorRange?.(suggestion.start,suggestion.end,suggestion.at));
      const accept = document.createElement('button'); accept.className = 'btn small'; accept.disabled = review.accepted.includes(index); accept.textContent = t(accept.disabled ? 'accepted' : 'accept');
      accept.addEventListener('click', () => ipc('killfeed_accept',{id:selected.id,suggestion:index})); row.append(info,preview,accept); $('killfeed-suggestions').append(row);
    });
  }
  function render() {
    translate();
    if (!state.desktop || !state.settings?.desktop) return;
    initialized = true; const desktop = state.desktop, preferences = state.settings.desktop;
    const bind = (id,value) => { if (document.activeElement !== $(id)) { if ($(id).type === 'checkbox') $(id).checked = value; else $(id).value = value; } };
    bind('hud-enabled',preferences.hud_enabled); bind('hud-position',preferences.hud_position); bind('hud-scale',preferences.hud_scale); $('hud-scale-value').textContent = preferences.hud_scale + '%';
    bind('automatic-updates',preferences.automatic_updates); bind('killfeed-player',preferences.player_name);
    for (const k of ['left','top','width','height']) bind('killfeed-'+k,Math.round((preferences.killfeed_region?.[k] || 0)*100));
    $('setup-open').disabled = !idle(); $('update-check').disabled = !idle(); $('update-download').disabled = !idle(); $('update-install').disabled = !idle();
    const update = desktop.update || {}, release = update.release;
    $('update-summary').textContent = release ? `Stillclip ${release.version}` : update.checked ? (document.documentElement.lang === 'en' ? 'You are up to date.' : 'Du bist auf dem neuesten Stand.') : '';
    $('update-download').hidden = !release || update.downloaded; $('update-install').hidden = !update.downloaded;
    $('update-install').textContent = t(update.portable ? 'portable' : 'install'); $('portable-update-help').hidden = !update.portable || !update.downloaded;
    $('update-notes').hidden = !release; $('update-notes').textContent = release?.notes || '';
    const task = desktop.task; $('desktop-progress').hidden = !task; $('editor-desktop-progress').hidden = !task;
    $('desktop-phase').textContent = $('editor-desktop-phase').textContent = task ? `${task.phase || task.kind} · ${task.percent}%` : '';
    $('desktop-meter').value = task?.percent || 0;
    $('desktop-cancel').disabled = $('editor-desktop-cancel').disabled = !!task?.cancelling;
    $('desktop-editor-error').textContent = desktop.error || '';
    if (desktop.report) $('report-content').value = JSON.stringify(desktop.report,null,2);
    drawAudio(); renderReview(); renderWizard();
    if (!autoShown && !preferences.welcome_complete && idle()) { autoShown = true; openWizard(); }
  }
  $('setup-open').addEventListener('click',openWizard); $('setup-close').addEventListener('click',closeWizard);
  $('wizard-language').addEventListener('change',()=>{ readWizard(); ipc('set_language',{language:wizard.language}); });
  $('wizard-next').addEventListener('click',()=>{readWizard(); step=Math.min(2,step+1);renderWizard();});
  $('wizard-back').addEventListener('click',()=>{readWizard();step=Math.max(0,step-1);renderWizard();});
  $('wizard-test').addEventListener('click',()=>{readWizard();ipc('setup_test',{settings:wizard});});
  $('wizard-finish').addEventListener('click',()=>{readWizard();finishPending=true;ipc('setup_finish',{settings:wizard});});
  for (const id of ['desktop-cancel','editor-desktop-cancel','wizard-cancel']) $(id).addEventListener('click',()=>ipc('desktop_cancel'));
  for (const id of ['hud-enabled','hud-position','hud-scale','automatic-updates','killfeed-player',...['left','top','width','height'].map(k=>'killfeed-'+k)]) $(id).addEventListener('change',savePreferences);
  $('hud-scale').addEventListener('input',()=>{$('hud-scale-value').textContent=$('hud-scale').value+'%';});
  for (const kind of ['check','download','install']) $('update-'+kind).addEventListener('click',()=>ipc('update_'+kind));
  $('report-open').addEventListener('click',()=>{previousFocus=document.activeElement;ipc('report_preview');$('report-backdrop').hidden=false;$('report-close').focus();});
  const closeReport=()=>{$('report-backdrop').hidden=true;previousFocus?.focus();};
  $('report-close').addEventListener('click',closeReport); $('report-save').addEventListener('click',()=>ipc('report_save'));
  $('audio-analyze').addEventListener('click',()=>{if(selected)ipc('audio_analyze',{id:selected.id});});
  $('audio-save-mix').addEventListener('click',()=>{ const ctx=window.getEditorContext?.();if(ctx)ipc('audio_mix',{id:ctx.id,start:ctx.start,end:ctx.end,crop:ctx.crop,mix:Object.fromEntries(Object.keys(gains).map(k=>[k,muted[k]?0:gains[k]]))}); });
  $('audio-listen-mix').addEventListener('click',()=>{ const ctx=window.getEditorContext?.();if(ctx)ipc('audio_mix_preview',{id:ctx.id,start:ctx.start,end:ctx.end,mix:Object.fromEntries(Object.keys(gains).map(k=>[k,muted[k]?0:gains[k]]))}); });
  $('mix-preview').addEventListener('play',()=> $('editor-video').pause());
  $('editor-video').addEventListener('play',()=> $('mix-preview').pause());
  $('killfeed-analyze').addEventListener('click',()=>{ const ctx=window.getEditorContext?.();if(ctx){savePreferences();ipc('killfeed_analyze',{id:ctx.id,start:ctx.start,end:ctx.end});} });
  document.addEventListener('keydown',event=>{
    const modal=!$('report-backdrop').hidden?$('report-backdrop'):!$('setup-backdrop').hidden?$('setup-backdrop'):null;if(!modal)return;
    if(event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();modal.id==='report-backdrop'?closeReport():closeWizard();}
    if(event.key==='Tab'){const nodes=[...modal.querySelectorAll('button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea,video[controls]')].filter(n=>n.getClientRects().length);const first=nodes[0],last=nodes.at(-1);if(event.shiftKey&&document.activeElement===first){event.preventDefault();last.focus();}else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first.focus();}}
  },true);
  window.StillclipDesktop = {
    setState(next){state=next;render();},
    selectClip(clip){if(selected?.id!==clip?.id){for(const k of Object.keys(gains)){gains[k]=100;muted[k]=false;}}selected=clip;if(!clip)$('mix-preview').pause();audioKey='';reviewKey='';if(initialized){drawAudio();renderReview();}},
  };
  translate();
})();
