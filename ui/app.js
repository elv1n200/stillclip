(() => {
  'use strict';
  const $ = id => document.getElementById(id);

  // ---------------------------------------------------------------- state
  const defaultSettings = {
    clips_dir: '', replay_seconds: 30, fps: 60, quality: 'p720', encoder: 'auto', display: '',
    show_cursor: false, system_audio: true, microphone: false, microphone_device: '',
    keep_full_session: false,
    hotkey_replay: { ctrl: false, alt: false, shift: false, win: false, key: 'F8' },
    hotkey_record: { ctrl: false, alt: false, shift: false, win: false, key: '' },
    close_to_tray: true, record_on_launch: false, start_minimized: false,
    microphone_separate_track: false, precise_clips: false, clip_sound: true, overlay_notifications: true,
    auto_record_games: true, builtin_games: true, custom_games: [],
  };
  let state = {
    clips: [], recording: false, elapsed: 0, busy: false, status: '', error: false,
    ffmpeg_available: false, ffmpeg_path: null, clips_dir: '', settings: { ...defaultSettings }, autostart: false,
    displays: [], microphones: [], default_output: null,
    capabilities: { probed: false, ddagrab: false, encoders: [] }, encoders: [], selected_encoder: '',
    session: null, buffer_bytes: 0, notice: { sequence: 0, text: '', error: false, clip: null },
    replay_presets: [15, 30, 60, 120, 180, 300], replay_min: 15, replay_max: 1200,
    editor_min_seconds: 2, editor_tolerance: 0.000001, fps_choices: [24, 30, 60, 120, 144],
    hotkeys: { replay: 'F8', record: 'Nicht belegt', failed: [] }, tray: false, version: '', recovered: null,
    active_game: null, auto_started: false, apps: [], builtin_games: [],
  };
  let loaded = false;
  let clipSignature = '';
  let lastNotice = 0;
  let highlightId = null;
  let draft = null;           // unsaved settings copy
  let draftAutostart = null;
  let capturing = null;       // 'replay' | 'record' while a hotkey field listens
  let previousFocus = null;
  let menuClip = null;
  let pendingDelete = null;
  let expectReopen = false;

  // ------------------------------------------------------------- helpers
  const ipc = (action, fields = {}) => {
    if (window.ipc && typeof window.ipc.postMessage === 'function') {
      window.ipc.postMessage(JSON.stringify({ action, ...fields }));
    } else {
      $('app-status').textContent = 'Die Verbindung zum Studio ist nicht verfügbar.';
      $('app-status').classList.add('is-error');
    }
  };
  const el = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const icon = name => {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
    use.setAttribute('href', '#i-' + name);
    svg.append(use);
    return svg;
  };
  const pad = n => String(n).padStart(2, '0');
  const clock = seconds => {
    const n = Math.max(0, Math.floor(Number(seconds) || 0));
    return [Math.floor(n / 3600), Math.floor(n % 3600 / 60), n % 60].map(pad).join(':');
  };
  const short = seconds => {
    const n = Math.max(0, Number(seconds) || 0);
    const total = Math.floor(n);
    const h = Math.floor(total / 3600), m = Math.floor(total % 3600 / 60), s = total % 60;
    return h ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
  };
  const precise = seconds => {
    const n = Math.max(0, Number(seconds) || 0);
    const m = Math.floor(n / 60), s = n - m * 60;
    return `${pad(m)}:${s.toFixed(1).padStart(4, '0')}`;
  };
  const seconds = value => (Number(value) || 0).toLocaleString('de-DE', { minimumFractionDigits: 1, maximumFractionDigits: 1 }) + ' s';
  const size = bytes => {
    const n = Number(bytes) || 0;
    if (n >= 1073741824) return (n / 1073741824).toLocaleString('de-DE', { maximumFractionDigits: 2 }) + ' GB';
    if (n >= 1048576) return (n / 1048576).toLocaleString('de-DE', { maximumFractionDigits: 1 }) + ' MB';
    return Math.max(1, Math.round(n / 1024)) + ' KB';
  };
  const dateNumber = value => {
    if (typeof value === 'number') return value < 1e12 ? value * 1000 : value;
    const parsed = Date.parse(value);
    return Number.isFinite(parsed) ? parsed : 0;
  };
  const dateLabel = value => {
    const n = dateNumber(value);
    if (!n) return '';
    const date = new Date(n), now = new Date();
    const sameDay = (a, b) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
    const yesterday = new Date(now); yesterday.setDate(now.getDate() - 1);
    const time = date.toLocaleTimeString('de-DE', { hour: '2-digit', minute: '2-digit' });
    if (sameDay(date, now)) return `Heute, ${time}`;
    if (sameDay(date, yesterday)) return `Gestern, ${time}`;
    return date.toLocaleDateString('de-DE', { day: '2-digit', month: '2-digit', year: 'numeric' });
  };
  const hotkeyLabel = hotkey => {
    if (!hotkey || !hotkey.key) return 'Nicht belegt';
    const parts = [];
    if (hotkey.ctrl) parts.push('Strg');
    if (hotkey.alt) parts.push('Alt');
    if (hotkey.shift) parts.push('Umschalt');
    if (hotkey.win) parts.push('Win');
    parts.push(hotkey.key);
    return parts.join(' + ');
  };
  const qualityLabel = quality => ({ p720: '720p', p1080: '1080p', p1440: '1440p', native: 'Nativ' }[quality] || quality);
  const currentSettings = () => draft || state.settings;
  const displayFor = name => state.displays.find(d => d.name === name) || state.displays.find(d => d.primary) || state.displays[0];
  const outputSize = (display, quality) => {
    if (!display) return null;
    if (quality === 'native') return [display.width, display.height];
    const target = { p720: 720, p1080: 1080, p1440: 1440 }[quality] || 720;
    const ratio = target / display.height;
    let w = Math.floor(display.width * ratio), h = Math.floor(display.height * ratio);
    if (display.width <= w && display.height <= h) { w = display.width; h = display.height; }
    return [Math.max(2, w & ~1), Math.max(2, h & ~1)];
  };

  // ------------------------------------------------------------ navigation
  function view(page) {
    $('page-library').hidden = page !== 'library';
    $('page-settings').hidden = page !== 'settings';
    ['library', 'settings'].forEach(name => {
      const nav = $('nav-' + name);
      nav.classList.toggle('active', page === name);
      if (page === name) nav.setAttribute('aria-current', 'page'); else nav.removeAttribute('aria-current');
    });
    window.scrollTo({ top: 0 });
  }
  $('nav-library').addEventListener('click', () => view('library'));
  $('nav-settings').addEventListener('click', () => { view('settings'); if (loaded && !state.busy) ipc('list_apps'); });

  // --------------------------------------------------------------- toasts
  function toast(text, { error = false, clipId = null } = {}) {
    const node = el('div', 'toast' + (error ? ' error' : ''));
    node.append(el('span', 'toast-text', text));
    const clip = clipId != null ? state.clips.find(c => c.id === clipId) : null;
    if (clip) {
      const button = el('button', 'btn btn-quiet', 'Ansehen');
      button.addEventListener('click', () => { openEditor(clip); dismiss(); });
      node.append(button);
    }
    const close = el('button', 'icon-btn', '');
    close.setAttribute('aria-label', 'Schließen');
    close.append(icon('close'));
    close.addEventListener('click', dismiss);
    node.append(close);
    $('toasts').append(node);
    let timer = setTimeout(dismiss, error ? 9000 : 6000);
    function dismiss() {
      clearTimeout(timer);
      node.classList.add('leaving');
      setTimeout(() => node.remove(), 260);
    }
  }

  // -------------------------------------------------------------- library
  function clipCard(clip) {
    const card = el('div', 'clip-card');
    card.tabIndex = 0;
    card.setAttribute('role', 'button');
    card.setAttribute('aria-label', `${clip.name} öffnen`);
    if (clip.id === highlightId) card.classList.add('is-new');
    const thumbnail = el('div', 'thumbnail');
    if (clip.thumb) {
      const img = document.createElement('img');
      img.src = clip.thumb; img.alt = ''; img.loading = 'lazy'; img.decoding = 'async';
      thumbnail.append(img);
    } else {
      const placeholder = icon(clip.pending ? 'refresh' : 'film');
      placeholder.classList.add('placeholder');
      if (clip.pending) placeholder.classList.add('spin');
      thumbnail.append(placeholder);
    }
    if (clip.duration) thumbnail.append(el('span', 'badge', short(clip.duration)));
    if (clip.width && clip.height) {
      const info = el('span', 'badge left');
      info.append(icon(clip.audio ? 'sound' : 'sound-off'));
      info.append(document.createTextNode(`${clip.height}p`));
      thumbnail.append(info);
    }
    const play = el('span', 'play-circle');
    play.append(icon('play'));
    thumbnail.append(play);
    const menu = el('button', 'card-menu');
    menu.setAttribute('aria-label', 'Weitere Aktionen');
    menu.title = 'Weitere Aktionen';
    menu.append(icon('more'));
    menu.addEventListener('click', event => { event.stopPropagation(); openMenu(clip, event.clientX, event.clientY); });
    const info = el('div', 'clip-info');
    info.append(el('div', 'clip-name', clip.name));
    const meta = el('div', 'clip-meta');
    const extension = typeof clip.extension === 'string' && clip.extension.trim() ? clip.extension.replace(/^\./, '').toUpperCase() : 'VIDEO';
    const details = [extension, size(clip.bytes)];
    if (clip.width && clip.height) details.push(`${clip.width}×${clip.height}`);
    meta.append(el('span', '', details.join(' · ')), el('span', '', dateLabel(clip.created)));
    info.append(meta);
    card.append(thumbnail, menu, info);
    card.addEventListener('click', () => openEditor(clip));
    card.addEventListener('keydown', event => {
      if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); openEditor(clip); }
      if (event.key === 'Delete') { event.preventDefault(); confirmDelete(clip); }
    });
    card.addEventListener('contextmenu', event => { event.preventDefault(); openMenu(clip, event.clientX, event.clientY); });
    return card;
  }

  function renderLibrary() {
    const query = $('search').value.trim().toLocaleLowerCase('de-DE');
    const clips = state.clips.filter(clip => String(clip.name).toLocaleLowerCase('de-DE').includes(query));
    const order = $('sort').value;
    clips.sort((a, b) => {
      if (order === 'name') return String(a.name).localeCompare(String(b.name), 'de');
      if (order === 'size') return Number(b.bytes) - Number(a.bytes);
      if (order === 'longest') return (Number(b.duration) || 0) - (Number(a.duration) || 0);
      if (order === 'oldest') return dateNumber(a.created) - dateNumber(b.created);
      return dateNumber(b.created) - dateNumber(a.created);
    });
    $('nav-count').textContent = state.clips.length;
    $('library-count').textContent = state.clips.length;
    $('empty-state').hidden = state.clips.length !== 0;
    $('first-steps').hidden = state.clips.length !== 0;
    $('no-results').hidden = !(state.clips.length && !clips.length);
    const grid = $('clip-grid');
    grid.replaceChildren(...clips.map(clipCard));
    grid.classList.toggle('is-busy', state.busy);
  }
  $('search').addEventListener('input', renderLibrary);
  $('sort').addEventListener('change', renderLibrary);
  $('reset-search').addEventListener('click', () => { $('search').value = ''; renderLibrary(); $('search').focus(); });

  // ---------------------------------------------------------- context menu
  function openMenu(clip, x, y) {
    menuClip = clip;
    const menu = $('context-menu');
    menu.hidden = false;
    const rect = menu.getBoundingClientRect();
    menu.style.left = Math.min(x, window.innerWidth - rect.width - 8) + 'px';
    menu.style.top = Math.min(y, window.innerHeight - rect.height - 8) + 'px';
    menu.querySelector('button').focus();
  }
  function closeMenu() { $('context-menu').hidden = true; menuClip = null; }
  $('context-menu').querySelectorAll('button').forEach(button => button.addEventListener('click', () => {
    const clip = menuClip; closeMenu();
    if (!clip) return;
    switch (button.dataset.menu) {
      case 'open': openEditor(clip); break;
      case 'external': ipc('open_clip', { id: clip.id }); break;
      case 'reveal': ipc('reveal', { id: clip.id }); break;
      case 'delete': confirmDelete(clip); break;
    }
  }));
  document.addEventListener('pointerdown', event => { if (!$('context-menu').hidden && !$('context-menu').contains(event.target)) closeMenu(); });
  window.addEventListener('blur', closeMenu);

  // --------------------------------------------------------------- confirm
  function confirmDelete(clip) {
    pendingDelete = clip;
    $('confirm-text').textContent = `„${clip.name}“ wird in den Papierkorb verschoben. Du kannst den Clip von dort wiederherstellen.`;
    $('confirm-backdrop').hidden = false;
    $('confirm-cancel').focus();
  }
  function closeConfirm() { $('confirm-backdrop').hidden = true; pendingDelete = null; }
  $('confirm-cancel').addEventListener('click', closeConfirm);
  $('confirm-backdrop').addEventListener('click', event => { if (event.target === $('confirm-backdrop')) closeConfirm(); });
  $('confirm-ok').addEventListener('click', () => {
    const clip = pendingDelete; closeConfirm();
    if (!clip) return;
    if (selected && selected.id === clip.id) closeEditor();
    ipc('delete', { id: clip.id });
  });

  // ---------------------------------------------------------- record card
  function renderRecordCard() {
    const unavailable = !loaded || state.busy;
    const settings = state.settings;
    const recording = state.recording;
    const replayLabel = hotkeyLabel(settings.hotkey_replay);
    $('record-card').classList.toggle('recording', recording);
    $('record-status').textContent = recording ? 'Aufnahme läuft' : 'Bereit für deinen Moment';
    const game = state.active_game;
    $('game-chip').hidden = !game;
    $('game-chip').textContent = game ? `${game.name} erkannt` : '';
    $('side-session-game').hidden = !game;
    $('side-session-game').textContent = game ? game.name : '';
    $('record-title').textContent = recording
      ? (game ? `${game.name} läuft – die letzten ${settings.replay_seconds} Sekunden sind immer griffbereit.` : `Stillclip merkt sich die letzten ${settings.replay_seconds} Sekunden.`)
      : (settings.auto_record_games ? 'Starte ein Spiel – oder die Aufnahme selbst.' : 'Ein guter Moment? Behalte ihn.');
    $('record-description').textContent = recording
      ? (settings.hotkey_replay.key
        ? `Drücke ${replayLabel}, um den Moment als Clip zu sichern. Die Aufnahme läuft dabei ohne Unterbrechung weiter.`
        : 'Klicke auf „Moment sichern“, um die letzten Sekunden als Clip zu behalten. Die Aufnahme läuft dabei weiter.')
      : (settings.auto_record_games
        ? 'Bekannte Spiele werden erkannt und automatisch aufgenommen. Du kannst die Aufnahme jederzeit auch manuell starten.'
        : 'Starte die Aufnahme. Stillclip merkt sich die letzten Sekunden und sichert sie auf Tastendruck – ohne Lücke, ohne Riesendateien.');
    $('record-button').classList.toggle('btn-danger', recording);
    $('record-button').classList.toggle('btn-primary', !recording);
    $('record-button-text').textContent = recording ? 'Aufnahme beenden' : 'Aufnahme starten';
    $('record-button').disabled = unavailable || (!recording && !state.ffmpeg_available);
    $('replay-button').disabled = unavailable || !recording || !state.ffmpeg_available;
    $('replay-button-text').textContent = recording ? `Letzte ${settings.replay_seconds} Sek. sichern` : `Letzte ${settings.replay_seconds} Sek. sichern`;
    $('timer').textContent = clock(state.elapsed);
    $('hotkey-replay-label').textContent = replayLabel;
    $('shortcut-hint').hidden = !settings.hotkey_replay.key && !settings.hotkey_record.key;
    $('hotkey-record-hint').hidden = !settings.hotkey_record.key;
    $('hotkey-record-label').textContent = hotkeyLabel(settings.hotkey_record);

    const meta = $('capture-meta');
    meta.replaceChildren();
    const add = (iconName, text) => {
      if (meta.childElementCount) meta.append(el('span', 'sep', '·'));
      if (iconName) meta.append(icon(iconName));
      meta.append(el('span', '', text));
    };
    const display = displayFor(settings.display);
    if (recording && state.session) {
      add('monitor', `${display ? display.label.split(' · ')[0] : 'Bildschirm'} → ${state.session.width}×${state.session.height}`);
      add(state.session.audio.length ? 'sound' : 'sound-off', state.session.audio.length ? state.session.audio.join(' + ') : 'Ohne Ton');
      add(null, `${settings.fps} FPS`);
      add('chip', state.session.strategy);
      add('clock', `Puffer ${size(state.buffer_bytes)}`);
    } else {
      const output = outputSize(display, settings.quality);
      add('monitor', display ? display.label.split(' · ')[0] + (output ? ` · ${output[0]}×${output[1]}` : '') : 'Hauptbildschirm');
      const audio = [settings.system_audio ? 'Systemton' : null, settings.microphone ? 'Mikrofon' : null].filter(Boolean);
      add(audio.length ? 'sound' : 'sound-off', audio.length ? audio.join(' + ') : 'Ohne Ton');
      add(null, `${settings.fps} FPS`);
      add('chip', state.capabilities.probed ? state.selected_encoder : 'Encoder wird geprüft …');
      if (settings.keep_full_session) add('film', 'Ganze Sitzung wird behalten');
    }
    const warnings = recording && state.session ? state.session.warnings : [];
    $('session-warnings').hidden = !warnings.length;
    if (warnings.length) {
      $('session-warnings').replaceChildren(icon('warning'), el('span', '', warnings.join(' ')));
    }

    const side = $('side-session');
    side.classList.toggle('recording', recording);
    $('side-session-label').textContent = recording ? 'Aufnahme läuft' : (state.ffmpeg_available ? 'Bereit' : 'FFmpeg fehlt');
    $('side-session-timer').textContent = clock(state.elapsed);
    $('side-hotkey').textContent = replayLabel;
    $('side-session-hint').hidden = !settings.hotkey_replay.key;
  }

  // -------------------------------------------------------------- settings
  function settingsDirty() {
    if (!draft) return false;
    return JSON.stringify(draft) !== JSON.stringify(state.settings) || (draftAutostart !== null && draftAutostart !== state.autostart);
  }
  function edit(mutate) {
    if (!draft) { draft = JSON.parse(JSON.stringify(state.settings)); draftAutostart = state.autostart; }
    mutate(draft);
    renderSettings();
  }
  function fillSelect(select, options, value) {
    const signature = JSON.stringify(options);
    if (select.dataset.signature !== signature) {
      select.replaceChildren(...options.map(option => {
        const node = document.createElement('option');
        node.value = option.value; node.textContent = option.label;
        if (option.disabled) node.disabled = true;
        return node;
      }));
      select.dataset.signature = signature;
    }
    if (![...select.options].some(option => option.value === String(value))) {
      const node = document.createElement('option');
      node.value = String(value); node.textContent = `${value} (nicht verfügbar)`;
      select.append(node);
    }
    select.value = String(value);
  }
  function renderSettings() {
    const settings = currentSettings();
    const autostart = draftAutostart === null ? state.autostart : draftAutostart;
    const locked = !loaded || state.busy;
    const recordingLocked = locked || state.recording;

    fillSelect($('setting-display'), [
      { value: '', label: 'Hauptbildschirm (automatisch)' },
      ...state.displays.map(d => ({ value: d.name, label: d.label })),
    ], settings.display);
    $('setting-quality').value = settings.quality;
    const display = displayFor(settings.display);
    const output = outputSize(display, settings.quality);
    $('quality-help').textContent = display && output
      ? (output[0] === display.width && output[1] === display.height
        ? `Bildschirm ${display.width}×${display.height} wird in voller Größe aufgenommen.`
        : `Bildschirm ${display.width}×${display.height} wird auf ${output[0]}×${output[1]} skaliert. Höhere Auflösung bedeutet größere Dateien.`)
      : 'Höhere Auflösung bedeutet größere Dateien.';
    fillSelect($('setting-fps'), state.fps_choices.map(v => ({ value: v, label: `${v} FPS` })), settings.fps);
    fillSelect($('setting-encoder'), [
      { value: 'auto', label: 'Automatisch' },
      ...state.encoders.map(e => ({ value: e.id, label: e.label + (state.capabilities.probed && !e.available ? ' (nicht verfügbar)' : ''), disabled: state.capabilities.probed && !e.available })),
    ], settings.encoder);
    $('encoder-help').textContent = state.capabilities.probed
      ? (settings.encoder === 'auto' ? `Automatisch nutzt derzeit ${state.selected_encoder}.` : 'Fester Encoder. Fällt bei Problemen auf die CPU zurück.')
      : 'Die verfügbaren Encoder werden gerade geprüft …';
    $('setting-cursor').checked = !!settings.show_cursor;
    $('setting-system-audio').checked = !!settings.system_audio;
    $('system-audio-help').textContent = state.default_output
      ? `Alles, was über „${state.default_output}“ läuft.`
      : 'Alles, was über dein Standard-Wiedergabegerät läuft.';
    $('setting-microphone').checked = !!settings.microphone;
    $('microphone-device-row').hidden = !settings.microphone;
    $('microphone-separate-row').hidden = !settings.microphone;
    $('setting-mic-separate').checked = !!settings.microphone_separate_track;
    $('setting-precise').checked = !!settings.precise_clips;
    $('setting-overlay').checked = !!settings.overlay_notifications;
    $('setting-clip-sound').checked = !!settings.clip_sound;
    $('setting-auto-games').checked = !!settings.auto_record_games;
    $('setting-builtin-games').checked = !!settings.builtin_games;
    $('builtin-games-help').textContent = `Eingebaute Liste mit ${state.builtin_games.length} Spielen, z. B. ${state.builtin_games.slice(0, 4).map(g => g.name).join(', ')} …`;
    renderGameList(settings);
    fillSelect($('setting-microphone-device'), [
      { value: '', label: 'Standard-Mikrofon' },
      ...state.microphones.map(m => ({ value: m.id, label: m.name })),
    ], settings.microphone_device);

    const presets = state.replay_presets.map(String);
    fillSelect($('setting-replay'), [
      ...presets.map(v => ({ value: v, label: `${v} Sekunden` })),
      { value: 'custom', label: 'Eigene Länge' },
    ], presets.includes(String(settings.replay_seconds)) ? String(settings.replay_seconds) : 'custom');
    $('setting-replay-custom-row').hidden = $('setting-replay').value !== 'custom';
    if (document.activeElement !== $('setting-replay-custom')) $('setting-replay-custom').value = String(settings.replay_seconds);
    $('setting-replay-custom').min = String(state.replay_min);
    $('setting-replay-custom').max = String(state.replay_max);
    $('setting-replay-help').textContent = `${state.replay_min} bis ${state.replay_max} Sekunden (${state.replay_max / 60} Minuten).`;
    $('setting-full-session').checked = !!settings.keep_full_session;

    if (capturing !== 'replay') $('hotkey-replay').value = settings.hotkey_replay.key ? hotkeyLabel(settings.hotkey_replay) : '';
    if (capturing !== 'record') $('hotkey-record').value = settings.hotkey_record.key ? hotkeyLabel(settings.hotkey_record) : '';
    const failed = state.hotkeys.failed || [];
    $('hotkey-failures').hidden = !failed.length;
    $('hotkey-failures').classList.add('error');
    $('hotkey-failures').textContent = failed.length ? 'Ein Kürzel konnte nicht registriert werden, weil ein anderes Programm es bereits nutzt. Wähle eine andere Taste.' : '';

    $('setting-close-to-tray').checked = !!settings.close_to_tray;
    $('setting-record-on-launch').checked = !!settings.record_on_launch;
    $('setting-start-minimized').checked = !!settings.start_minimized;
    $('setting-autostart').checked = !!autostart;

    ['setting-display', 'setting-quality', 'setting-fps', 'setting-encoder', 'setting-cursor', 'setting-system-audio', 'setting-microphone', 'setting-microphone-device', 'setting-mic-separate', 'setting-full-session']
      .forEach(id => { $(id).disabled = recordingLocked; });
    ['setting-replay', 'setting-replay-custom', 'hotkey-replay', 'hotkey-record', 'setting-close-to-tray', 'setting-record-on-launch', 'setting-start-minimized', 'setting-autostart', 'setting-precise', 'setting-overlay', 'setting-clip-sound', 'setting-auto-games', 'setting-builtin-games', 'app-picker', 'add-running-app', 'refresh-apps', 'pick-game']
      .forEach(id => { $(id).disabled = locked; });

    $('clips-directory').textContent = state.clips_dir || 'Wird geladen …';
    $('ffmpeg-state').textContent = loaded ? (state.ffmpeg_available ? 'FFmpeg ist bereit' : 'FFmpeg noch nicht gefunden') : 'Wird geprüft …';
    $('ffmpeg-state').classList.toggle('unavailable', loaded && !state.ffmpeg_available);
    $('ffmpeg-path').textContent = state.ffmpeg_path || 'Wähle die Datei ffmpeg.exe aus, z. B. aus einem FFmpeg-Download oder einer vorhandenen Installation.';
    const list = $('encoder-list');
    list.replaceChildren(...state.encoders.map(e => {
      const chip = el('span', 'encoder-chip' + (e.available && state.capabilities.probed ? ' available' : ''), e.label + (state.capabilities.probed ? (e.available ? ' · bereit' : ' · nicht verfügbar') : ' · wird geprüft'));
      return chip;
    }));
    if (state.capabilities.probed) {
      list.append(el('span', 'encoder-chip' + (state.capabilities.ddagrab ? ' available' : ''), state.capabilities.ddagrab ? 'Desktop Duplication · bereit' : 'Desktop Duplication · nicht verfügbar (GDI-Fallback)'));
    }

    const dirty = settingsDirty();
    $('save-bar').hidden = !dirty;
    $('save-settings').disabled = locked || !validReplay(settings);
  }
  function renderGameList(settings) {
    const list = $('game-list');
    const games = settings.custom_games || [];
    list.replaceChildren(...games.map(game => {
      const row = el('div', 'game-row');
      row.append(el('strong', '', game.name || game.exe), el('span', '', game.exe));
      const remove = el('button', 'icon-btn danger');
      remove.title = 'Aus der Liste entfernen';
      remove.setAttribute('aria-label', `${game.name} entfernen`);
      remove.append(icon('close'));
      remove.disabled = !loaded || state.busy;
      remove.addEventListener('click', () => ipc('remove_game', { exe: game.exe }));
      row.append(remove);
      return row;
    }));
    if (!games.length) list.append(el('div', 'game-empty', 'Noch keine eigenen Einträge. Spiele aus der eingebauten Liste werden trotzdem erkannt.'));
    const picker = $('app-picker');
    const current = picker.value;
    const known = new Set([...(settings.custom_games || []).map(g => g.exe), ...state.builtin_games.map(g => g.exe)]);
    const options = state.apps.filter(app => !known.has(app.exe));
    picker.replaceChildren(...[{ value: '', label: options.length ? 'Laufendes Programm wählen …' : 'Keine weiteren Programme gefunden – Liste neu einlesen' }, ...options.map(app => ({ value: app.exe, label: `${app.title.length > 48 ? app.title.slice(0, 48) + '…' : app.title}  (${app.exe})` }))].map(option => {
      const node = document.createElement('option');
      node.value = option.value; node.textContent = option.label;
      return node;
    }));
    if ([...picker.options].some(o => o.value === current)) picker.value = current;
    $('add-running-app').disabled = !loaded || state.busy || !picker.value;
  }
  function validReplay(settings) {
    const value = Number(settings.replay_seconds);
    return Number.isInteger(value) && value >= state.replay_min && value <= state.replay_max;
  }
  $('setting-display').addEventListener('change', () => edit(d => { d.display = $('setting-display').value; }));
  $('setting-quality').addEventListener('change', () => edit(d => { d.quality = $('setting-quality').value; }));
  $('setting-fps').addEventListener('change', () => edit(d => { d.fps = Number($('setting-fps').value); }));
  $('setting-encoder').addEventListener('change', () => edit(d => { d.encoder = $('setting-encoder').value; }));
  $('setting-cursor').addEventListener('change', () => edit(d => { d.show_cursor = $('setting-cursor').checked; }));
  $('setting-system-audio').addEventListener('change', () => edit(d => { d.system_audio = $('setting-system-audio').checked; }));
  $('setting-microphone').addEventListener('change', () => edit(d => { d.microphone = $('setting-microphone').checked; }));
  $('setting-microphone-device').addEventListener('change', () => edit(d => { d.microphone_device = $('setting-microphone-device').value; }));
  $('setting-replay').addEventListener('change', () => edit(d => {
    const value = $('setting-replay').value;
    if (value !== 'custom') d.replay_seconds = Number(value);
    else { d.replay_seconds = Number($('setting-replay-custom').value) || d.replay_seconds; if (state.replay_presets.includes(d.replay_seconds)) d.replay_seconds = Math.min(state.replay_max, d.replay_seconds + 15); }
  }));
  $('setting-replay-custom').addEventListener('input', () => edit(d => { d.replay_seconds = Number($('setting-replay-custom').value); }));
  $('setting-full-session').addEventListener('change', () => edit(d => { d.keep_full_session = $('setting-full-session').checked; }));
  $('setting-close-to-tray').addEventListener('change', () => edit(d => { d.close_to_tray = $('setting-close-to-tray').checked; }));
  $('setting-record-on-launch').addEventListener('change', () => edit(d => { d.record_on_launch = $('setting-record-on-launch').checked; }));
  $('setting-start-minimized').addEventListener('change', () => edit(d => { d.start_minimized = $('setting-start-minimized').checked; }));
  $('setting-autostart').addEventListener('change', () => edit(() => { draftAutostart = $('setting-autostart').checked; }));
  $('setting-mic-separate').addEventListener('change', () => edit(d => { d.microphone_separate_track = $('setting-mic-separate').checked; }));
  $('setting-precise').addEventListener('change', () => edit(d => { d.precise_clips = $('setting-precise').checked; }));
  $('setting-overlay').addEventListener('change', () => edit(d => { d.overlay_notifications = $('setting-overlay').checked; }));
  $('setting-clip-sound').addEventListener('change', () => edit(d => { d.clip_sound = $('setting-clip-sound').checked; }));
  $('setting-auto-games').addEventListener('change', () => edit(d => { d.auto_record_games = $('setting-auto-games').checked; }));
  $('setting-builtin-games').addEventListener('change', () => edit(d => { d.builtin_games = $('setting-builtin-games').checked; }));
  $('app-picker').addEventListener('change', () => { $('add-running-app').disabled = !$('app-picker').value; });
  $('add-running-app').addEventListener('click', () => {
    const exe = $('app-picker').value;
    const app = state.apps.find(a => a.exe === exe);
    if (!app) return;
    const title = (app.title || '').split(/ [-–|] /)[0].trim();
    ipc('add_game', { exe, name: title.length > 2 && title.length < 60 ? title : '' });
  });
  $('refresh-apps').addEventListener('click', () => ipc('list_apps'));
  $('pick-game').addEventListener('click', () => ipc('pick_game'));
  $('discard-settings').addEventListener('click', () => { draft = null; draftAutostart = null; renderSettings(); });
  $('save-settings').addEventListener('click', () => {
    const settings = currentSettings();
    if (!validReplay(settings)) { $('setting-replay-custom').reportValidity(); return; }
    const autostart = draftAutostart === null ? state.autostart : draftAutostart;
    ipc('save_settings', { settings, autostart });
    draft = null; draftAutostart = null;
    renderSettings();
  });

  // hotkey capture ------------------------------------------------------
  const keyNameFromEvent = event => {
    const code = event.code || '';
    if (/^[a-zA-Z]$/.test(event.key)) return event.key.toUpperCase();
    if (/^Digit[0-9]$/.test(code)) return code.slice(5);
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
    if (/^Numpad[0-9]$/.test(code)) return 'Num' + code.slice(6);
    const map = {
      Space: 'Space', Pause: 'Pause', Insert: 'Insert', Delete: 'Delete', Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
      ScrollLock: 'ScrollLock', PrintScreen: 'PrintScreen', NumpadMultiply: 'NumMultiply', NumpadAdd: 'NumAdd', NumpadSubtract: 'NumSubtract',
      NumpadDecimal: 'NumDecimal', NumpadDivide: 'NumDivide', Backquote: 'Backquote', Minus: 'Minus', Equal: 'Equal', BracketLeft: 'BracketLeft',
      BracketRight: 'BracketRight', Backslash: 'Backslash', Semicolon: 'Semicolon', Quote: 'Quote', Comma: 'Comma', Period: 'Period', Slash: 'Slash',
      ArrowLeft: 'Left', ArrowUp: 'Up', ArrowRight: 'Right', ArrowDown: 'Down', Enter: 'Enter', NumpadEnter: 'Enter', Tab: 'Tab',
    };
    return map[code] || null;
  };
  function bindHotkeyField(id, field) {
    const input = $(id);
    input.addEventListener('focus', () => { capturing = field; input.classList.add('capturing'); input.value = ''; input.placeholder = 'Taste drücken …'; });
    input.addEventListener('blur', () => { capturing = null; input.classList.remove('capturing'); input.placeholder = 'Nicht belegt'; renderSettings(); });
    input.addEventListener('keydown', event => {
      event.preventDefault();
      const key = event.key;
      if (['Control', 'Shift', 'Alt', 'Meta', 'AltGraph'].includes(key)) return;
      if (key === 'Escape') { input.blur(); return; }
      if (key === 'Backspace' || (key === 'Delete' && !event.ctrlKey && !event.altKey && !event.shiftKey)) {
        edit(d => { d[field === 'replay' ? 'hotkey_replay' : 'hotkey_record'] = { ctrl: false, alt: false, shift: false, win: false, key: '' }; });
        input.blur();
        return;
      }
      const name = keyNameFromEvent(event);
      if (!name) { input.value = 'Diese Taste wird nicht unterstützt'; return; }
      edit(d => { d[field === 'replay' ? 'hotkey_replay' : 'hotkey_record'] = { ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, win: event.metaKey, key: name }; });
      input.blur();
    });
  }
  bindHotkeyField('hotkey-replay', 'replay');
  bindHotkeyField('hotkey-record', 'record');
  $('hotkey-replay-clear').addEventListener('click', () => edit(d => { d.hotkey_replay = { ctrl: false, alt: false, shift: false, win: false, key: '' }; }));
  $('hotkey-record-clear').addEventListener('click', () => edit(d => { d.hotkey_record = { ctrl: false, alt: false, shift: false, win: false, key: '' }; }));

  // ---------------------------------------------------------------- editor
  const video = $('editor-video');
  let selected = null;
  let duration = 0;
  let sel = { start: 0, end: 0 };
  let loop = false;
  let raf = 0;
  const FRAME = 1 / 60;

  const minSelection = () => duration ? Math.min(state.editor_min_seconds, duration) : state.editor_min_seconds;
  const clamp = (value, low, high) => Math.min(Math.max(value, low), high);

  function openEditor(clip) {
    if (state.busy && !expectReopen) return;
    expectReopen = false;
    previousFocus = document.activeElement;
    selected = { id: clip.id, src: clip.src, name: clip.name, path: clip.path };
    duration = Number(clip.duration) || 0;
    sel = { start: 0, end: duration };
    loop = false;
    $('loop-selection').classList.remove('active');
    $('editor-title').textContent = clip.name;
    $('editor-subtitle').textContent = [clip.width && clip.height ? `${clip.width}×${clip.height}` : null, size(clip.bytes), clip.audio === false ? 'ohne Ton' : null, dateLabel(clip.created)].filter(Boolean).join(' · ');
    $('clip-name').value = clip.name;
    $('clip-duration').textContent = duration ? 'Länge ' + precise(duration) : '';
    $('video-error').hidden = true;
    video.src = clip.src;
    $('editor-backdrop').hidden = false;
    document.body.style.overflow = 'hidden';
    renderEditor();
    renderState();
    $('play-pause').focus();
  }
  function closeEditor() {
    cancelAnimationFrame(raf);
    video.pause();
    video.removeAttribute('src');
    video.load();
    $('editor-backdrop').hidden = true;
    document.body.style.overflow = '';
    selected = null;
    if (previousFocus && previousFocus.isConnected) previousFocus.focus();
  }
  function setSelection(start, end, { seek = null } = {}) {
    const min = minSelection();
    if (!duration) { sel = { start: Math.max(0, start), end: Math.max(start, end) }; renderEditor(); return; }
    start = clamp(start, 0, Math.max(0, duration - min));
    end = clamp(end, Math.min(start + min, duration), duration);
    sel = { start, end };
    if (seek !== null && Number.isFinite(seek)) video.currentTime = clamp(seek, 0, duration);
    renderEditor();
  }
  function renderEditor() {
    const total = duration || Math.max(sel.end, video.duration || 0) || 1;
    const pct = value => (clamp(value, 0, total) / total * 100) + '%';
    $('timeline-range').style.left = pct(sel.start);
    $('timeline-range').style.width = (clamp(sel.end, 0, total) - clamp(sel.start, 0, total)) / total * 100 + '%';
    $('handle-start').style.left = pct(sel.start);
    $('handle-end').style.left = pct(sel.end);
    $('timeline-playhead').style.left = pct(video.currentTime || 0);
    $('label-start').textContent = precise(sel.start);
    $('label-end').textContent = precise(sel.end);
    $('label-current').textContent = precise(video.currentTime || 0);
    $('selection-length').textContent = seconds(sel.end - sel.start);
    if (document.activeElement !== $('trim-start')) $('trim-start').value = sel.start.toFixed(2);
    if (document.activeElement !== $('trim-end')) $('trim-end').value = duration || sel.end ? sel.end.toFixed(2) : '';
    $('play-icon').replaceChildren((() => { const use = document.createElementNS('http://www.w3.org/2000/svg', 'use'); use.setAttribute('href', video.paused ? '#i-play' : '#i-pause'); return use; })());
    validateTrim();
  }
  function validateTrim() {
    const start = Number($('trim-start').value), end = Number($('trim-end').value);
    const hasValues = $('trim-start').value !== '' && $('trim-end').value !== '';
    const minimum = minSelection();
    const valid = hasValues && Number.isFinite(start) && Number.isFinite(end) && start >= 0 && end > start
      && end - start + state.editor_tolerance >= minimum && (!duration || end <= duration + state.editor_tolerance);
    $('trim-clip').disabled = !loaded || state.busy || !state.ffmpeg_available || !selected || !valid;
    const invalid = hasValues && !valid;
    $('trim-help').classList.toggle('error', invalid);
    const mode = $('trim-mode').value;
    $('trim-help').textContent = invalid
      ? (duration && end > duration + state.editor_tolerance ? 'Das Ende muss innerhalb der Videolänge liegen.' : duration && duration < state.editor_min_seconds ? 'Dieses kurze Video wird vollständig übernommen.' : 'Der Ausschnitt muss mindestens zwei Sekunden lang sein.')
      : !state.ffmpeg_available ? 'Wähle FFmpeg in den Einstellungen aus, um Clips zuzuschneiden.'
        : mode === 'fast' ? 'Schnell: ohne Neukodierung, der Schnitt beginnt am vorherigen Keyframe (bis zu einer Sekunde früher). Das Original bleibt erhalten.'
          : 'Präzise: bildgenau, dauert je nach Länge ein paar Sekunden. Das Original bleibt erhalten.';
  }

  // timeline interaction
  const track = $('timeline-track');
  const trackTime = clientX => {
    const rect = track.getBoundingClientRect();
    return clamp((clientX - rect.left) / rect.width, 0, 1) * (duration || 1);
  };
  function drag(target, onMove) {
    target.addEventListener('pointerdown', event => {
      if (!duration) return;
      event.preventDefault();
      event.stopPropagation();
      target.setPointerCapture(event.pointerId);
      const origin = { x: event.clientX, sel: { ...sel } };
      const move = e => onMove(trackTime(e.clientX), origin, e);
      const up = () => { target.removeEventListener('pointermove', move); target.removeEventListener('pointerup', up); target.removeEventListener('pointercancel', up); };
      target.addEventListener('pointermove', move);
      target.addEventListener('pointerup', up);
      target.addEventListener('pointercancel', up);
    });
  }
  drag($('handle-start'), time => setSelection(time, sel.end, { seek: time }));
  drag($('handle-end'), time => setSelection(sel.start, time, { seek: time }));
  drag($('timeline-range'), (time, origin) => {
    const delta = time - trackTime(origin.x);
    const length = origin.sel.end - origin.sel.start;
    const start = clamp(origin.sel.start + delta, 0, duration - length);
    setSelection(start, start + length);
  });
  track.addEventListener('pointerdown', event => {
    if (!duration) return;
    video.currentTime = trackTime(event.clientX);
    renderEditor();
  });

  // transport
  function togglePlay() {
    if (video.paused) {
      if (loop && (video.currentTime < sel.start || video.currentTime >= sel.end - 0.05)) video.currentTime = sel.start;
      video.play().catch(() => {});
    } else video.pause();
  }
  const step = delta => { video.pause(); video.currentTime = clamp((video.currentTime || 0) + delta, 0, duration || video.duration || 0); renderEditor(); };
  $('play-pause').addEventListener('click', togglePlay);
  $('step-back').addEventListener('click', () => step(-FRAME));
  $('step-forward').addEventListener('click', () => step(FRAME));
  $('set-start').addEventListener('click', () => setSelection(video.currentTime, Math.max(sel.end, video.currentTime + minSelection())));
  $('set-end').addEventListener('click', () => setSelection(Math.min(sel.start, video.currentTime - minSelection()), video.currentTime));
  $('loop-selection').addEventListener('click', () => { loop = !loop; $('loop-selection').classList.toggle('active', loop); if (loop && video.paused) togglePlay(); });
  video.addEventListener('click', togglePlay);
  video.addEventListener('play', () => {
    const tick = () => { renderEditor(); if (!video.paused) raf = requestAnimationFrame(tick); };
    cancelAnimationFrame(raf); raf = requestAnimationFrame(tick);
  });
  video.addEventListener('pause', () => { cancelAnimationFrame(raf); renderEditor(); });
  video.addEventListener('timeupdate', () => {
    if (loop && !video.paused && video.currentTime >= sel.end - 0.03) video.currentTime = sel.start;
    if (video.paused) renderEditor();
  });
  video.addEventListener('ended', () => { if (loop) { video.currentTime = sel.start; video.play().catch(() => {}); } });
  video.addEventListener('loadedmetadata', () => {
    const known = Number.isFinite(video.duration) && video.duration > 0 ? video.duration : duration;
    const wasWhole = !duration || (sel.start === 0 && Math.abs(sel.end - duration) < 0.01);
    duration = known;
    $('clip-duration').textContent = duration ? 'Länge ' + precise(duration) : '';
    if (wasWhole) sel = { start: 0, end: duration };
    $('trim-start').max = String(Math.max(0, duration - state.editor_min_seconds));
    $('trim-end').min = String(Math.min(state.editor_min_seconds, duration));
    $('trim-end').max = String(duration);
    setSelection(sel.start, sel.end);
  });
  video.addEventListener('error', () => { if (selected) $('video-error').hidden = false; });

  ['trim-start', 'trim-end'].forEach(id => $(id).addEventListener('input', validateTrim));
  $('trim-start').addEventListener('change', () => { const v = Number($('trim-start').value); if (Number.isFinite(v)) setSelection(v, sel.end, { seek: v }); });
  $('trim-end').addEventListener('change', () => { const v = Number($('trim-end').value); if (Number.isFinite(v)) setSelection(sel.start, v, { seek: v }); });
  $('trim-mode').addEventListener('change', validateTrim);
  $('clip-name').addEventListener('input', () => { $('rename-clip').disabled = state.busy || !selected || !$('clip-name').value.trim(); });
  $('clip-name').addEventListener('keydown', event => { if (event.key === 'Enter' && !$('rename-clip').disabled) $('rename-clip').click(); });
  $('rename-clip').addEventListener('click', () => {
    if (!selected) return;
    const name = $('clip-name').value.trim();
    if (name === selected.name) return;
    expectReopen = true;
    ipc('rename', { id: selected.id, name });
  });
  $('trim-clip').addEventListener('click', () => {
    validateTrim();
    if (selected && !$('trim-clip').disabled) ipc('trim', { id: selected.id, start: Number($('trim-start').value), end: Number($('trim-end').value), mode: $('trim-mode').value });
  });
  $('editor-reveal').addEventListener('click', () => { if (selected) ipc('reveal', { id: selected.id }); });
  $('editor-external').addEventListener('click', () => { if (selected) ipc('open_clip', { id: selected.id }); });
  $('editor-delete').addEventListener('click', () => { const clip = selected && state.clips.find(c => c.id === selected.id); if (clip) confirmDelete(clip); });
  $('close-editor').addEventListener('click', closeEditor);
  $('editor-backdrop').addEventListener('click', event => { if (event.target === $('editor-backdrop')) closeEditor(); });

  document.addEventListener('keydown', event => {
    if (!$('confirm-backdrop').hidden) { if (event.key === 'Escape') closeConfirm(); return; }
    if (!$('context-menu').hidden && event.key === 'Escape') { closeMenu(); return; }
    if ($('editor-backdrop').hidden || capturing) return;
    const typing = ['INPUT', 'SELECT', 'TEXTAREA'].includes(document.activeElement && document.activeElement.tagName);
    if (event.key === 'Escape') { event.preventDefault(); if (typing) document.activeElement.blur(); else closeEditor(); return; }
    if (event.key === 'Tab') {
      const focusable = [...$('editor-backdrop').querySelectorAll('button:not(:disabled),input:not(:disabled),select:not(:disabled),video')];
      const first = focusable[0], last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      return;
    }
    if (typing) return;
    const big = event.shiftKey ? 1 : FRAME;
    switch (event.key) {
      case ' ': event.preventDefault(); togglePlay(); break;
      case 'ArrowLeft': event.preventDefault(); step(-big); break;
      case 'ArrowRight': event.preventDefault(); step(big); break;
      case 'Home': event.preventDefault(); video.currentTime = sel.start; renderEditor(); break;
      case 'End': event.preventDefault(); video.currentTime = sel.end; renderEditor(); break;
      case 'i': case 'I': event.preventDefault(); $('set-start').click(); break;
      case 'o': case 'O': event.preventDefault(); $('set-end').click(); break;
      case 'l': case 'L': event.preventDefault(); $('loop-selection').click(); break;
      case 'Delete': event.preventDefault(); $('editor-delete').click(); break;
    }
  });

  // ------------------------------------------------------------- render
  function renderState() {
    const unavailable = !loaded || state.busy;
    document.querySelectorAll('[data-action]').forEach(button => {
      const action = button.dataset.action;
      button.disabled = unavailable || (state.recording && ['choose_folder', 'choose_ffmpeg'].includes(action));
    });
    renderRecordCard();
    $('setup-banner').hidden = !loaded || state.ffmpeg_available;
    ['app-status', 'editor-status'].forEach(id => {
      const status = $(id);
      status.textContent = state.status || '';
      status.classList.toggle('is-error', !!state.error);
      status.classList.toggle('is-busy', !!state.busy);
    });
    $('rename-clip').disabled = unavailable || !selected || !$('clip-name').value.trim();
    ['clip-name', 'trim-start', 'trim-end', 'trim-mode', 'editor-delete'].forEach(id => { $(id).disabled = unavailable; });
    $('version').textContent = state.version ? `Stillclip ${state.version}` : '';
    validateTrim();
  }

  function handleNotice() {
    const notice = state.notice || {};
    if (!notice.sequence || notice.sequence <= lastNotice) return;
    lastNotice = notice.sequence;
    toast(notice.text, { error: !!notice.error, clipId: notice.clip });
    if (notice.clip != null && !notice.error) {
      highlightId = notice.clip;
      renderLibrary();
      setTimeout(() => { if (highlightId === notice.clip) { highlightId = null; renderLibrary(); } }, 6000);
      if (expectReopen && !selected) {
        const clip = state.clips.find(c => c.id === notice.clip);
        if (clip) openEditor(clip);
      }
    }
    expectReopen = false;
  }

  window.setState = next => {
    if (!next || typeof next !== 'object') return;
    state = { ...state, ...next, clips: Array.isArray(next.clips) ? next.clips : state.clips, settings: { ...defaultSettings, ...(next.settings || state.settings) } };
    loaded = true;
    if (draft) draft.custom_games = state.settings.custom_games;
    const signature = JSON.stringify(state.clips);
    if (signature !== clipSignature) { clipSignature = signature; renderLibrary(); }
    if (selected) {
      const clip = state.clips.find(c => c.id === selected.id);
      if (clip) {
        selected.name = clip.name;
        $('editor-title').textContent = clip.name;
        if (document.activeElement !== $('clip-name')) $('clip-name').value = clip.name;
      } else if (!expectReopen) {
        closeEditor();
      } else {
        selected = null;
        $('editor-backdrop').hidden = true;
        document.body.style.overflow = '';
      }
    }
    renderState();
    renderSettings();
    handleNotice();
  };

  document.querySelectorAll('[data-action]').forEach(button => button.addEventListener('click', () => { if (!button.disabled) ipc(button.dataset.action); }));
  $('record-button').addEventListener('click', () => ipc(state.recording ? 'stop' : 'start'));
  $('replay-button').addEventListener('click', () => ipc('replay'));
  window.addEventListener('focus', () => { if (loaded && !state.busy) ipc('refresh'); });

  // Used by the automated screenshot check to render other views.
  window.showPage = page => {
    if (page === 'editor') { view('library'); setTimeout(() => { if (state.clips[0]) openEditor(state.clips[0]); }, 300); return; }
    view(page === 'settings' ? 'settings' : 'library');
    if (page === 'settings') ipc('list_apps');
  };

  renderLibrary();
  renderState();
  renderSettings();
  ipc('ready');
})();
