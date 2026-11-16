(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const tr = window.i18n.tr;

  // ---------------------------------------------------------------- state
  const defaultSettings = {
    language: 'de', clips_dir: '', replay_seconds: 30, fps: 60, quality: 'p720', encoder: 'auto', display: '',
    show_cursor: false, follow_game_display: true,
    system_audio: true, microphone: false, microphone_device: '',
    system_volume: 100, microphone_volume: 100,
    replay_seconds_long: 0, marker_before: 20, marker_after: 10,
    keep_full_session: false, min_free_gb: 5, storage_limit_gb: 0,
    hotkey_replay: { ctrl: false, alt: false, shift: false, win: false, key: 'F8' },
    hotkey_replay_long: { ctrl: false, alt: false, shift: false, win: false, key: '' },
    hotkey_marker: { ctrl: false, alt: false, shift: false, win: false, key: '' },
    hotkey_record: { ctrl: false, alt: false, shift: false, win: false, key: '' },
    close_to_tray: true, record_on_launch: false, start_minimized: false,
    microphone_separate_track: false, precise_clips: false, clip_sound: true, overlay_notifications: true,
    auto_record_games: true, builtin_games: true, custom_games: [],
    game_profiles: [], capture_window: false, game_audio: false, target_exe: '',
  };
  let state = {
    exports: [], export_limit: 12, audio_test: {}, dragging: false,
    clips: [], recording: false, elapsed: 0, busy: false, capture_busy: false, status: '', error: false,
    ffmpeg_available: false, ffmpeg_path: null, clips_dir: '', settings: { ...defaultSettings }, autostart: false,
    displays: [], microphones: [], default_output: null,
    capabilities: { probed: false, ddagrab: false, encoders: [] }, encoders: [], selected_encoder: '',
    session: null, markers: [], buffer_bytes: 0, free_bytes: 0, clips_bytes: 0, max_volume: 200,
    notice: { sequence: 0, text: '', error: false, clip: null },
    replay_presets: [15, 30, 60, 120, 180, 300], replay_min: 15, replay_max: 1200,
    editor_min_seconds: 2, editor_tolerance: 0.000001, fps_choices: [24, 30, 60, 120, 144],
    hotkeys: { replay: 'F8', replay_long: tr('Nicht belegt'), marker: tr('Nicht belegt'), record: tr('Nicht belegt'), failed: [] },
    tray: false, version: '', recovered: null,
    active_game: null, active_game_display: null, capture_display: null,
    auto_started: false, apps: [], builtin_games: [],
  };
  let loaded = false;
  let clipSignature = '';
  let markerSignature = '';
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
      $('app-status').textContent = tr('Die Verbindung zum Studio ist nicht verfügbar.');
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
  const seconds = value => (Number(value) || 0).toLocaleString(window.i18n.locale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 }) + ' s';
  const size = bytes => {
    const n = Number(bytes) || 0;
    if (n >= 1073741824) return (n / 1073741824).toLocaleString(window.i18n.locale(), { maximumFractionDigits: 2 }) + ' GB';
    if (n >= 1048576) return (n / 1048576).toLocaleString(window.i18n.locale(), { maximumFractionDigits: 1 }) + ' MB';
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
    const time = date.toLocaleTimeString(window.i18n.locale(), { hour: '2-digit', minute: '2-digit' });
    if (sameDay(date, now)) return tr`Heute, ${time}`;
    if (sameDay(date, yesterday)) return tr`Gestern, ${time}`;
    return date.toLocaleDateString(window.i18n.locale(), { day: '2-digit', month: '2-digit', year: 'numeric' });
  };
  const hotkeyLabel = hotkey => {
    if (!hotkey || !hotkey.key) return tr('Nicht belegt');
    const parts = [];
    if (hotkey.ctrl) parts.push(tr('Strg'));
    if (hotkey.alt) parts.push('Alt');
    if (hotkey.shift) parts.push(tr('Umschalt'));
    if (hotkey.win) parts.push('Win');
    parts.push(hotkey.key);
    return parts.join(' + ');
  };
  const clipLength = seconds => (seconds >= 120 ? `${Math.round(seconds / 60)} min` : `${seconds} s`);
  const qualityLabel = quality => ({ p720: '720p', p1080: '1080p', p1440: '1440p', native: tr('Nativ') }[quality] || quality);
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

  // Rough size of the segment buffer, the same formula the recorder uses:
  // the preset rate grows with the pixel area, but never shrinks.
  const bufferEstimate = (settings, seconds) => {
    const preset = { p720: [1280, 720, 15], p1080: [1920, 1080, 25], p1440: [2560, 1440, 40], native: [1920, 1080, 25] }[settings.quality]
      || [1280, 720, 15];
    const output = outputSize(displayFor(state.capture_display || settings.display), settings.quality);
    const ratio = output ? (output[0] * output[1]) / (preset[0] * preset[1]) : 1;
    const mbps = preset[2] * Math.max(1, ratio);
    return (seconds * mbps * 1e6) / 8;
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
      const button = el('button', 'btn btn-quiet', tr('Ansehen'));
      button.addEventListener('click', () => { openEditor(clip); dismiss(); });
      node.append(button);
    }
    const close = el('button', 'icon-btn', '');
    close.setAttribute('aria-label', tr('Schließen'));
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
    card.setAttribute('aria-label', tr`${clip.name} öffnen`);
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
    const marks = Array.isArray(clip.markers) ? clip.markers.length : 0;
    if (marks) {
      const badge = el('span', 'badge marks');
      badge.append(icon('bookmark'), document.createTextNode(String(marks)));
      badge.title = marks === 1 ? tr('Ein markierter Moment') : tr`${marks} markierte Momente`;
      thumbnail.append(badge);
    }
    const play = el('span', 'play-circle');
    play.append(icon('play'));
    thumbnail.append(play);
    const menu = el('button', 'card-menu');
    const favorite = el('button', 'card-favorite', clip.favorite ? '★' : '☆');
    favorite.setAttribute('aria-label', clip.favorite ? tr('Favorit entfernen') : tr('Als Favorit behalten'));
    favorite.setAttribute('aria-pressed', String(!!clip.favorite));
    favorite.title = tr('Favoriten sind vor automatischem Löschen geschützt');
    favorite.disabled = state.busy;
    favorite.addEventListener('click', event => { event.stopPropagation(); ipc('favorite', { id: clip.id, favorite: !clip.favorite }); });
    favorite.addEventListener('keydown', event => event.stopPropagation());
    menu.setAttribute('aria-label', tr('Weitere Aktionen'));
    menu.title = tr('Weitere Aktionen');
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
    const annotations = [clip.game, ...(clip.tags || []).map(t => '#' + t)].filter(Boolean);
    if (annotations.length) info.append(el('div', 'clip-tags', annotations.join(' · ')));
    const drag = el('button', 'card-drag', '↗ ' + tr('Ziehen'));
    drag.title = tr('Gedrückt halten und die Datei nach Discord oder in einen Ordner ziehen');
    drag.setAttribute('aria-label', tr('Clip als Datei ziehen'));
    drag.disabled = state.busy || state.dragging;
    let origin = null;
    drag.addEventListener('pointerdown', event => {
      if (event.button !== 0 || drag.disabled) return;
      event.stopPropagation(); event.preventDefault(); origin = { x: event.clientX, y: event.clientY };
      drag.setPointerCapture(event.pointerId);
    });
    drag.addEventListener('pointermove', event => {
      if (!origin || !(event.buttons & 1) || Math.hypot(event.clientX - origin.x, event.clientY - origin.y) < 6) return;
      origin = null; drag.releasePointerCapture(event.pointerId); ipc('drag_clip', { id: clip.id });
    });
    for (const type of ['pointerup', 'pointercancel', 'lostpointercapture']) drag.addEventListener(type, () => { origin = null; });
    drag.addEventListener('click', event => { event.stopPropagation(); toast(tr('Datei am Ziehen-Feld gedrückt halten und nach Discord ziehen. Alternativ: im Explorer anzeigen.')); });
    drag.addEventListener('keydown', event => event.stopPropagation());
    card.append(thumbnail, favorite, menu, drag, info);
    card.addEventListener('click', () => openEditor(clip));
    card.addEventListener('keydown', event => {
      if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); openEditor(clip); }
      if (event.key === 'Delete') { event.preventDefault(); confirmDelete(clip); }
    });
    card.addEventListener('contextmenu', event => { event.preventDefault(); openMenu(clip, event.clientX, event.clientY); });
    return card;
  }

  function updateFilterOptions() {
    for (const [id, values, label] of [
      ['filter-game', [...new Set(state.clips.map(c => c.game).filter(Boolean))], tr('Alle Spiele')],
      ['filter-tag', [...new Set(state.clips.flatMap(c => c.tags || []).map(t => t.toLowerCase()))], tr('Alle Tags')],
    ]) {
      const input = $(id), before = input.value;
      if (before && !values.includes(before)) values.push(before);
      values.sort((a,b) => a.localeCompare(b, 'de'));
      const signature = JSON.stringify(values);
      if (input.dataset.values === signature) continue;
      input.dataset.values = signature;
      input.replaceChildren(new Option(label, ''), ...values.map(v => new Option(v, v))); input.value = before;
    }
  }
  function resetFilters() {
    for (const id of ['search', 'filter-game', 'filter-tag', 'filter-from', 'filter-to', 'filter-length']) $(id).value = '';
    $('favorites-only').checked = false; renderLibrary();
  }
  for (const id of ['filter-game', 'filter-tag', 'filter-from', 'filter-to', 'filter-length']) $(id).addEventListener('change', renderLibrary);
  $('reset-filters').addEventListener('click', resetFilters);

  function renderLibrary() {
    const query = $('search').value.trim().toLocaleLowerCase(window.i18n.locale());
    updateFilterOptions();
    const from = $('filter-from').value ? new Date($('filter-from').value + 'T00:00:00').getTime() / 1000 : -Infinity;
    const until = $('filter-to').value ? new Date($('filter-to').value + 'T00:00:00') : null;
    if (until) until.setDate(until.getDate() + 1);
    const to = until ? until.getTime() / 1000 : Infinity;
    const clips = state.clips.filter(clip => {
      if (![clip.name, clip.game, ...(clip.tags || [])].filter(Boolean).join(' ').toLocaleLowerCase(window.i18n.locale()).includes(query)) return false;
      if ($('favorites-only').checked && !clip.favorite) return false;
      if ($('filter-game').value && clip.game !== $('filter-game').value) return false;
      if ($('filter-tag').value && !(clip.tags || []).some(t => t.toLowerCase() === $('filter-tag').value)) return false;
      if ((from !== -Infinity || to !== Infinity) && (!Number.isFinite(clip.created) || clip.created < from || clip.created >= to)) return false;
      const length = $('filter-length').value;
      if (length && (!Number.isFinite(clip.duration) || (length === 'short' ? clip.duration >= 30 : length === 'medium' ? clip.duration < 30 || clip.duration > 120 : clip.duration <= 120))) return false;
      return true;
    });
    const order = $('sort').value;
    clips.sort((a, b) => {
      if (order === 'name') return String(a.name).localeCompare(String(b.name), window.i18n.locale());
      if (order === 'size') return Number(b.bytes) - Number(a.bytes);
      if (order === 'longest') return (Number(b.duration) || 0) - (Number(a.duration) || 0);
      if (order === 'oldest') return dateNumber(a.created) - dateNumber(b.created);
      return dateNumber(b.created) - dateNumber(a.created);
    });
    $('nav-count').textContent = state.clips.length;
    $('library-count').textContent = state.clips.length;
    $('empty-state').hidden = state.clips.length !== 0;
    $('no-results').hidden = !(state.clips.length && !clips.length);
    const grid = $('clip-grid');
    grid.replaceChildren(...clips.map(clipCard));
    grid.classList.toggle('is-busy', state.busy);
  }
  $('search').addEventListener('input', renderLibrary);
  $('sort').addEventListener('change', renderLibrary);
  $('favorites-only').addEventListener('change', renderLibrary);
  $('reset-search').addEventListener('click', () => { resetFilters(); $('search').focus(); });

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
    if (state.clips.find(c => c.id === clip.id)?.locked) { toast(tr('Dieser Clip wird gerade verwendet. Beende zuerst den Export oder das Ziehen.')); return; }
    pendingDelete = clip;
    $('confirm-text').textContent = tr`„${clip.name}“ wird in den Papierkorb verschoben. Du kannst den Clip von dort wiederherstellen.`;
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
    const settings = { ...state.settings, ...state.runtime_settings };
    const recording = state.recording;
    const replayLabel = hotkeyLabel(settings.hotkey_replay);
    $('record-card').classList.toggle('recording', recording);
    $('record-status').textContent = recording
      ? tr('Aufnahme läuft')
      : (state.ffmpeg_available ? tr('Bereit') : tr('FFmpeg fehlt'));
    const game = state.active_game;
    $('game-chip').hidden = !game;
    $('game-chip').textContent = game ? game.name : '';
    $('side-session-game').hidden = !game;
    $('side-session-game').textContent = game ? game.name : '';
    $('record-button').classList.toggle('btn-danger', recording);
    $('record-button').classList.toggle('btn-primary', !recording);
    $('record-button-text').textContent = recording ? tr('Aufnahme beenden') : tr('Aufnahme starten');
    $('record-button').disabled = !loaded || state.capture_busy || (!recording && !state.ffmpeg_available);
    $('replay-button').disabled = unavailable || !recording || !state.ffmpeg_available;
    $('replay-button-text').textContent = tr`${clipLength(settings.replay_seconds)} sichern`;
    const long = Number(settings.replay_seconds_long) || 0;
    $('replay-long-button').hidden = long <= 0;
    $('replay-long-button').disabled = unavailable || !recording || !state.ffmpeg_available;
    $('replay-long-button-text').textContent = tr`${clipLength(long)} sichern`;
    $('timer').textContent = clock(state.elapsed);
    $('hotkey-replay-label').textContent = replayLabel;
    $('mark-button').disabled = !loaded || state.capture_busy || !recording || !state.ffmpeg_available;
    renderMarkers();
    const longKey = long > 0 && settings.hotkey_replay_long && settings.hotkey_replay_long.key;
    const markKey = settings.hotkey_marker && settings.hotkey_marker.key;
    $('shortcut-hint').hidden = !settings.hotkey_replay.key && !settings.hotkey_record.key && !longKey && !markKey;
    $('hotkey-marker-hint').hidden = !markKey;
    if (markKey) $('hotkey-marker-label').textContent = hotkeyLabel(settings.hotkey_marker);
    $('hotkey-replay-long-hint').hidden = !longKey;
    if (longKey) {
      $('hotkey-replay-long-seconds').textContent = clipLength(long);
      $('hotkey-replay-long-label').textContent = hotkeyLabel(settings.hotkey_replay_long);
    }
    $('hotkey-record-hint').hidden = !settings.hotkey_record.key;
    $('hotkey-record-label').textContent = hotkeyLabel(settings.hotkey_record);

    const meta = $('capture-meta');
    meta.replaceChildren();
    const add = (iconName, text) => {
      if (meta.childElementCount) meta.append(el('span', 'sep', '·'));
      if (iconName) meta.append(icon(iconName));
      meta.append(el('span', '', text));
    };
    const display = displayFor(state.capture_display || settings.display);
    if (recording && state.session) {
      add('monitor', `${settings.capture_window ? tr('Spielfenster') : display ? display.label.split(' · ')[0] : tr('Bildschirm')} → ${state.session.width}×${state.session.height}`);
      add(state.session.audio.length ? 'sound' : 'sound-off', state.session.audio.length ? state.session.audio.join(' + ') : tr('Ohne Ton'));
      add(null, `${settings.fps} FPS`);
      add('chip', state.session.strategy);
      add('clock', tr`Puffer ${size(state.buffer_bytes)}`);
    } else {
      const output = outputSize(display, settings.quality);
      add('monitor', display ? display.label.split(' · ')[0] + (output ? ` · ${output[0]}×${output[1]}` : '') : tr('Hauptbildschirm'));
      const audio = [settings.system_audio ? tr('Systemton') : null, settings.microphone ? tr('Mikrofon') : null].filter(Boolean);
      add(audio.length ? 'sound' : 'sound-off', audio.length ? audio.join(' + ') : tr('Ohne Ton'));
      add(null, `${settings.fps} FPS`);
      add('chip', state.capabilities.probed ? state.selected_encoder : tr('Encoder wird geprüft …'));
      if (settings.keep_full_session) add('film', tr('Ganze Sitzung wird behalten'));
    }
    const warnings = recording && state.session ? state.session.warnings : [];
    $('session-warnings').hidden = !warnings.length;
    if (warnings.length) {
      $('session-warnings').replaceChildren(icon('warning'), el('span', '', warnings.join(' ')));
    }

    const side = $('side-session');
    side.classList.toggle('recording', recording);
    $('side-session-label').textContent = recording ? tr('Aufnahme läuft') : (state.ffmpeg_available ? tr('Bereit') : tr('FFmpeg fehlt'));
    $('side-session-timer').textContent = clock(state.elapsed);
    const stats = state.diagnostics;
    const metric = (value, suffix = '') => value === null || value === undefined ? '–' : `${value}${suffix}`;
    $('diagnostic-values').replaceChildren(...[
      `Encoder: ${state.session?.strategy || state.selected_encoder || '–'}`,
      tr`Kodiert (Ø): ${metric(stats?.fps, ' FPS')}`,
      tr`Verworfen: ${metric(stats?.dropped)}`,
      tr`Dupliziert: ${metric(stats?.duplicated)}`,
      tr`Rückstand: ${metric(stats?.lag_seconds?.toFixed(1), ' s')}`,
      tr`Puffer: ${size(state.buffer_bytes)}`,
      tr`Frei: ${state.free_bytes == null ? '–' : size(state.free_bytes)}`,
    ].map(text => el('span', '', text)));
    $('diagnostic-health').textContent = !state.recording ? tr('Messwerte erscheinen während der Aufnahme. FPS sind Encoderwerte, keine Spiel-FPS.')
      : !stats || stats.frames == null ? tr('Warte auf die ersten Encoderwerte …')
        : stats.age_seconds > 5 || stats.lag_seconds > 5 ? tr('Die Aufnahme liefert verzögert Bilder. Prüfe Fenster, Auflösung, FPS und Encoder.')
          : tr('Die Aufnahme liefert aktuelle Bilder. Verworfen/dupliziert zählt FFmpeg seit Aufnahmestart.');
  }

  // Marked moments waiting for their clip. Rebuilt only when something about
  // them actually changed, so a click never lands on a replaced button.
  function renderMarkers() {
    const markers = state.markers || [];
    const box = $('markers');
    const blocked = !loaded || state.busy || !state.recording || !state.ffmpeg_available;
    const signature = JSON.stringify(markers) + blocked;
    if (signature === markerSignature) return;
    markerSignature = signature;
    box.hidden = !markers.length;
    box.replaceChildren(...markers.map(marker => {
      // A fresh marker is still being recorded; only one whose window the ring
      // has already deleted counts as lost.
      const waiting = !marker.ready;
      const lost = !waiting && marker.seconds < 1;
      const row = el('div', 'marker-row' + (lost ? ' lost' : waiting ? ' waiting' : ''));
      row.append(icon('bookmark'), el('span', 'marker-time', clock(marker.at)));
      let note = tr`${marker.wanted} Sek. bereit`;
      if (lost) note = tr('nicht mehr im Puffer');
      else if (waiting) note = tr('wird noch aufgezeichnet …');
      else if (marker.seconds < marker.wanted) note = tr`noch ${marker.seconds} Sek. verfügbar`;
      row.append(el('span', 'marker-note', note));
      const save = el('button', 'btn btn-quiet', tr('Clip sichern'));
      save.disabled = blocked || lost || waiting;
      save.addEventListener('click', () => ipc('save_marker', { id: marker.id }));
      const drop = el('button', 'icon-btn');
      drop.title = tr('Markierung verwerfen');
      drop.setAttribute('aria-label', tr('Markierung verwerfen'));
      drop.append(icon('close'));
      drop.disabled = !loaded || state.busy;
      drop.addEventListener('click', () => ipc('drop_marker', { id: marker.id }));
      row.append(save, drop);
      return row;
    }));
  }

  // -------------------------------------------------------------- settings
  let profileSignature = '';
  function renderProfiles(settings, locked) {
    const games = [...new Map([...(state.builtin_games || []), ...(settings.custom_games || []), ...(state.apps || []), ...(settings.game_profiles || [])]
      .filter(g => g.exe).map(g => [g.exe, { value: g.exe, label: g.name || g.exe }])).values()];
    fillSelect($('profile-game'), [{ value: '', label: tr('Spiel wählen …') }, ...games], $('profile-game').value || '');
    const exe = $('profile-game').value;
    const stored = (settings.game_profiles || []).find(p => p.exe === exe);
    const data = stored || settings;
    const signature = JSON.stringify([exe, data.quality, data.fps, data.replay_seconds, data.microphone, data.system_audio, data.capture_window, data.game_audio]);
    if (signature !== profileSignature) {
      profileSignature = signature;
      $('profile-quality').value = data.quality;
      fillSelect($('profile-fps'), state.fps_choices.map(v => ({ value: v, label: `${v} FPS` })), data.fps);
      $('profile-replay').value = data.replay_seconds;
      $('profile-microphone').checked = !!data.microphone;
      $('profile-system-audio').checked = !!data.system_audio;
      $('profile-window').checked = !!data.capture_window;
      $('profile-audio').checked = !!data.game_audio;
    }
    $('profile-game').disabled = locked;
    ['profile-quality', 'profile-fps', 'profile-replay', 'profile-microphone', 'profile-system-audio', 'profile-window', 'profile-audio', 'profile-save'].forEach(id => { $(id).disabled = locked || !exe; });
    $('profile-remove').disabled = locked || !stored;
    $('profile-status').textContent = !exe ? tr('Wähle ein Spiel. Ein Profil gilt ab der nächsten Aufnahme.')
      : stored ? tr('Eigenes Profil vorhanden. Änderungen mit „Profil übernehmen“ und unten „Änderungen speichern“ sichern.')
        : tr('Bisher gelten die allgemeinen Einstellungen. Erstelle hier eigene Vorgaben.');
  }
  $('profile-game').addEventListener('change', () => { profileSignature = ''; renderSettings(); });
  $('profile-save').addEventListener('click', () => {
    const exe = $('profile-game').value;
    if (!exe || !$('profile-replay').reportValidity()) return;
    const profile = { exe, quality: $('profile-quality').value, fps: Number($('profile-fps').value), replay_seconds: Number($('profile-replay').value),
      microphone: $('profile-microphone').checked, system_audio: $('profile-system-audio').checked, capture_window: $('profile-window').checked, game_audio: $('profile-audio').checked };
    edit(d => { d.game_profiles = [...(d.game_profiles || []).filter(p => p.exe !== exe), profile]; });
  });
  $('profile-remove').addEventListener('click', () => edit(d => { d.game_profiles = (d.game_profiles || []).filter(p => p.exe !== $('profile-game').value); }));
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
      node.value = String(value); node.textContent = tr`${value} (nicht verfügbar)`;
      select.append(node);
    }
    select.value = String(value);
  }
  function renderSettings() {
    $('setting-language').value = state.settings.language;
    $('setting-language').disabled = !loaded;
    const settings = currentSettings();
    const autostart = draftAutostart === null ? state.autostart : draftAutostart;
    const locked = !loaded || state.busy;
    const recordingLocked = locked || state.recording;
    $('setting-capture-window').value = String(!!settings.capture_window);
    $('setting-game-audio').value = String(!!settings.game_audio);
    const apps = [...new Map((state.apps || []).map(a => [a.exe, a])).values()];
    fillSelect($('setting-target'), [{ value: '', label: tr('Automatisch erkanntes Spiel') }, ...apps.map(a => ({ value: a.exe, label: `${a.exe} · ${a.title}` }))], settings.target_exe || '');
    $('window-support').textContent = state.capabilities.gfxcapture
      ? tr('Fensteraufnahme bereit. Beim Minimieren oder Schließen endet die Aufnahme.')
      : tr('Fensteraufnahme benötigt eine FFmpeg-Version mit gfxcapture. Der Bildschirmmodus funktioniert weiterhin.');
    ['setting-capture-window', 'setting-game-audio', 'setting-target'].forEach(id => { $(id).disabled = recordingLocked; });
    renderProfiles(settings, recordingLocked);

    fillSelect($('setting-display'), [
      { value: '', label: 'Hauptbildschirm (automatisch)' },
      ...state.displays.map(d => ({ value: d.name, label: d.label })),
    ], settings.display);
    $('setting-quality').value = settings.quality;
    const display = displayFor(settings.display);
    const output = outputSize(display, settings.quality);
    $('quality-help').textContent = display && output
      ? (output[0] === display.width && output[1] === display.height
        ? tr`Bildschirm ${display.width}×${display.height} wird in voller Größe aufgenommen.`
        : tr`Bildschirm ${display.width}×${display.height} wird auf ${output[0]}×${output[1]} skaliert. Höhere Auflösung bedeutet größere Dateien.`)
      : tr('Höhere Auflösung bedeutet größere Dateien.');
    fillSelect($('setting-fps'), state.fps_choices.map(v => ({ value: v, label: `${v} FPS` })), settings.fps);
    fillSelect($('setting-encoder'), [
      { value: 'auto', label: tr('Automatisch') },
      ...state.encoders.map(e => ({ value: e.id, label: e.label + (state.capabilities.probed && !e.available ? tr(' (nicht verfügbar)') : ''), disabled: state.capabilities.probed && !e.available })),
    ], settings.encoder);
    $('encoder-help').textContent = state.capabilities.probed
      ? (settings.encoder === 'auto' ? tr`Automatisch nutzt derzeit ${state.selected_encoder}.` : tr('Fester Encoder. Fällt bei Problemen auf die CPU zurück.'))
      : tr('Die verfügbaren Encoder werden gerade geprüft …');
    $('setting-cursor').checked = !!settings.show_cursor;
    $('setting-follow-display').checked = !!settings.follow_game_display;
    const gameDisplay = settings.follow_game_display ? displayFor(state.active_game_display) : null;
    $('display-help').textContent = gameDisplay && state.active_game
      ? tr`${state.active_game.name} läuft auf ${gameDisplay.label.split(' · ')[0]} – dieser wird gerade aufgenommen.`
      : tr('Dieser Bildschirm wird aufgenommen.');
    $('setting-system-audio').checked = !!settings.system_audio;
    $('system-audio-help').textContent = state.default_output
      ? tr`Alles, was über „${state.default_output}“ läuft.`
      : tr('Alles, was über dein Standard-Wiedergabegerät läuft.');
    $('system-volume-row').hidden = !settings.system_audio;
    $('setting-system-volume').max = String(state.max_volume);
    $('setting-system-volume').value = String(settings.system_volume);
    $('system-volume-value').textContent = `${settings.system_volume} %`;
    $('system-volume-help').textContent = settings.system_volume === 100
      ? tr('100 % lässt den Ton unverändert.')
      : (settings.system_volume > 100
        ? tr('Über 100 % kann bei lauten Stellen übersteuern.')
        : (settings.system_volume === 0 ? tr('Stumm: der Spielsound fehlt im Clip.') : tr('Leiser als im Spiel.')));
    $('microphone-volume-row').hidden = !settings.microphone;
    $('setting-microphone-volume').max = String(state.max_volume);
    $('setting-microphone-volume').value = String(settings.microphone_volume);
    $('microphone-volume-value').textContent = `${settings.microphone_volume} %`;
    $('setting-microphone').checked = !!settings.microphone;
    $('microphone-device-row').hidden = !settings.microphone;
    $('microphone-separate-row').hidden = !settings.microphone;
    $('setting-mic-separate').checked = !!settings.microphone_separate_track;
    $('setting-precise').checked = !!settings.precise_clips;
    $('setting-overlay').checked = !!settings.overlay_notifications;
    $('setting-clip-sound').checked = !!settings.clip_sound;
    $('setting-auto-games').checked = !!settings.auto_record_games;
    $('setting-builtin-games').checked = !!settings.builtin_games;
    $('builtin-games-help').textContent = tr`Eingebaute Liste mit ${state.builtin_games.length} Spielen, z. B. ${state.builtin_games.slice(0, 4).map(g => g.name).join(', ')} …`;
    renderGameList(settings);
    fillSelect($('setting-microphone-device'), [
      { value: '', label: tr('Standard-Mikrofon') },
      ...state.microphones.map(m => ({ value: m.id, label: m.name })),
    ], settings.microphone_device);

    const presets = state.replay_presets.map(String);
    fillSelect($('setting-replay'), [
      ...presets.map(v => ({ value: v, label: tr`${v} Sekunden` })),
      { value: 'custom', label: tr('Eigene Länge') },
    ], presets.includes(String(settings.replay_seconds)) ? String(settings.replay_seconds) : 'custom');
    $('setting-replay-custom-row').hidden = $('setting-replay').value !== 'custom';
    if (document.activeElement !== $('setting-replay-custom')) $('setting-replay-custom').value = String(settings.replay_seconds);
    $('setting-replay-custom').min = String(state.replay_min);
    $('setting-replay-custom').max = String(state.replay_max);
    $('setting-replay-help').textContent = tr`${state.replay_min} bis ${state.replay_max} Sekunden (${state.replay_max / 60} Minuten).`;
    $('setting-full-session').checked = !!settings.keep_full_session;
    if (document.activeElement !== $('setting-replay-long')) $('setting-replay-long').value = String(settings.replay_seconds_long);
    const longSeconds = Number(settings.replay_seconds_long) || 0;
    $('setting-replay-long-help').textContent = longSeconds > 0
      ? tr`Eine zweite Taste sichert ${longSeconds} Sekunden. Der Puffer belegt so viel Platz wie diese Länge – etwa ${size(bufferEstimate(settings, longSeconds))}.`
      : tr`Optional: eine zweite Taste sichert eine längere Spanne, zum Beispiel eine ganze Runde. 0 schaltet sie aus. Erlaubt sind ${state.replay_min} bis ${state.replay_max} Sekunden.`;

    if (document.activeElement !== $('setting-marker-before')) $('setting-marker-before').value = String(settings.marker_before);
    if (document.activeElement !== $('setting-marker-after')) $('setting-marker-after').value = String(settings.marker_after);
    const markerKey = settings.hotkey_marker && settings.hotkey_marker.key;
    $('marker-help').textContent = markerKey
      ? tr`${hotkeyLabel(settings.hotkey_marker)} merkt sich die Stelle, der Clip entsteht danach auf Klick: ${settings.marker_before} Sekunden davor, ${settings.marker_after} danach.`
      : tr('Markieren kostet nichts: Stillclip merkt sich nur die Stelle, der Clip entsteht danach auf Klick. Ein Kürzel dafür vergibst du weiter unten.');
    const longInvalid = longSeconds !== 0 && (longSeconds < state.replay_min || longSeconds > state.replay_max);
    $('setting-replay-long').setCustomValidity(longInvalid ? tr`Bitte 0 oder ${state.replay_min} bis ${state.replay_max} Sekunden.` : '');
    $('hotkey-replay-long-row').hidden = longSeconds <= 0;
    $('hotkey-replay-long-help').textContent = longSeconds > 0
      ? tr`Sichert ${longSeconds} Sekunden. Ohne Kürzel bleibt der Knopf im Fenster.`
      : tr('Sichert die zweite Länge aus dem Bereich Rückblick.');
    if (capturing !== 'replay') $('hotkey-replay').value = settings.hotkey_replay.key ? hotkeyLabel(settings.hotkey_replay) : '';
    if (capturing !== 'replay_long') $('hotkey-replay-long').value = settings.hotkey_replay_long.key ? hotkeyLabel(settings.hotkey_replay_long) : '';
    if (capturing !== 'marker') $('hotkey-marker').value = settings.hotkey_marker.key ? hotkeyLabel(settings.hotkey_marker) : '';
    if (capturing !== 'record') $('hotkey-record').value = settings.hotkey_record.key ? hotkeyLabel(settings.hotkey_record) : '';
    const failed = state.hotkeys.failed || [];
    $('hotkey-failures').hidden = !failed.length;
    $('hotkey-failures').classList.add('error');
    $('hotkey-failures').textContent = failed.length ? tr('Ein Kürzel konnte nicht registriert werden, weil ein anderes Programm es bereits nutzt. Wähle eine andere Taste.') : '';

    $('setting-close-to-tray').checked = !!settings.close_to_tray;
    $('setting-record-on-launch').checked = !!settings.record_on_launch;
    $('setting-start-minimized').checked = !!settings.start_minimized;
    $('setting-autostart').checked = !!autostart;

    ['setting-display', 'setting-quality', 'setting-fps', 'setting-encoder', 'setting-cursor', 'setting-system-audio', 'setting-microphone', 'setting-microphone-device', 'setting-mic-separate', 'setting-full-session', 'setting-system-volume', 'setting-microphone-volume', 'setting-follow-display', 'setting-marker-before', 'setting-marker-after', 'setting-replay', 'setting-replay-custom', 'setting-replay-long', 'setting-precise', 'setting-min-free']
      .forEach(id => { $(id).disabled = recordingLocked; });
    ['hotkey-marker', 'hotkey-replay', 'hotkey-replay-long', 'hotkey-record', 'setting-close-to-tray', 'setting-record-on-launch', 'setting-start-minimized', 'setting-autostart', 'setting-overlay', 'setting-clip-sound', 'setting-auto-games', 'setting-builtin-games', 'app-picker', 'add-running-app', 'refresh-apps', 'pick-game', 'setting-storage-limit']
      .forEach(id => { $(id).disabled = locked; });
    $('settings-locked').hidden = !state.recording;


    renderStorage(settings);
    $('clips-directory').textContent = state.clips_dir || tr('Wird geladen …');
    $('ffmpeg-state').textContent = loaded ? (state.ffmpeg_available ? tr('FFmpeg ist bereit') : tr('FFmpeg noch nicht gefunden')) : tr('Wird geprüft …');
    $('ffmpeg-state').classList.toggle('unavailable', loaded && !state.ffmpeg_available);
    $('ffmpeg-path').textContent = state.ffmpeg_path || tr('Wähle die Datei ffmpeg.exe aus, z. B. aus einem FFmpeg-Download oder einer vorhandenen Installation.');
    const list = $('encoder-list');
    list.replaceChildren(...state.encoders.map(e => {
      const chip = el('span', 'encoder-chip' + (e.available && state.capabilities.probed ? ' available' : ''), e.label + (state.capabilities.probed ? (e.available ? ' · bereit' : tr(' · nicht verfügbar')) : tr(' · wird geprüft')));
      return chip;
    }));
    if (state.capabilities.probed) {
      list.append(el('span', 'encoder-chip' + (state.capabilities.ddagrab ? ' available' : ''), state.capabilities.ddagrab ? tr('Desktop Duplication · bereit') : tr('Desktop Duplication · nicht verfügbar (GDI-Fallback)')));
    }

    const dirty = settingsDirty();
    $('save-bar').hidden = !dirty;
    $('save-settings').disabled = locked || !validReplay(settings);
  }
  function renderStorage(settings) {
    if (document.activeElement !== $('setting-min-free')) $('setting-min-free').value = String(settings.min_free_gb);
    if (document.activeElement !== $('setting-storage-limit')) $('setting-storage-limit').value = String(settings.storage_limit_gb);
    const box = $('storage-state');
    if (!loaded) { box.textContent = tr('Wird geladen …'); box.classList.remove('warn'); return; }
    const parts = [state.clips.length
      ? `${state.clips.length} ${state.clips.length === 1 ? tr('Clip belegt') : tr('Clips belegen')} ${size(state.clips_bytes)}.`
      : tr('Noch keine Clips im Ordner.')];
    if (state.free_bytes > 0) parts.push(tr`Auf dem Laufwerk sind ${size(state.free_bytes)} frei.`);
    const perMinute = bufferEstimate(settings, 60);
    if (state.free_bytes > 0 && perMinute > 0) {
      const minutes = Math.floor(state.free_bytes / perMinute);
      const span = minutes >= 120 ? tr`${Math.floor(minutes / 60)} Stunden` : tr`${minutes} Minuten`;
      parts.push(tr`Das reicht für etwa ${span} Aufnahme in ${qualityLabel(settings.quality)}.`);
    }
    const low = state.free_bytes > 0 && settings.min_free_gb > 0
      && state.free_bytes < settings.min_free_gb * 1073741824 * 2;
    box.textContent = parts.join(' ');
    box.classList.toggle('warn', low);
  }
  function renderGameList(settings) {
    const list = $('game-list');
    const games = settings.custom_games || [];
    list.replaceChildren(...games.map(game => {
      const row = el('div', 'game-row');
      row.append(el('strong', '', game.name || game.exe), el('span', '', game.exe));
      const remove = el('button', 'icon-btn danger');
      remove.title = tr('Aus der Liste entfernen');
      remove.setAttribute('aria-label', tr`${game.name} entfernen`);
      remove.append(icon('close'));
      remove.disabled = !loaded || state.busy;
      remove.addEventListener('click', () => ipc('remove_game', { exe: game.exe }));
      row.append(remove);
      return row;
    }));
    if (!games.length) list.append(el('div', 'game-empty', tr('Noch keine eigenen Einträge. Spiele aus der eingebauten Liste werden trotzdem erkannt.')));
    const picker = $('app-picker');
    const current = picker.value;
    const known = new Set([...(settings.custom_games || []).map(g => g.exe), ...state.builtin_games.map(g => g.exe)]);
    const options = state.apps.filter(app => !known.has(app.exe));
    picker.replaceChildren(...[{ value: '', label: options.length ? tr('Laufendes Programm wählen …') : tr('Keine weiteren Programme gefunden – Liste neu einlesen') }, ...options.map(app => ({ value: app.exe, label: `${app.title.length > 48 ? app.title.slice(0, 48) + '…' : app.title}  (${app.exe})` }))].map(option => {
      const node = document.createElement('option');
      node.value = option.value; node.textContent = option.label;
      return node;
    }));
    if ([...picker.options].some(o => o.value === current)) picker.value = current;
    $('add-running-app').disabled = !loaded || state.busy || !picker.value;
  }
  function validReplay(settings) {
    const value = Number(settings.replay_seconds);
    const long = Number(settings.replay_seconds_long);
    const inRange = n => Number.isInteger(n) && n >= state.replay_min && n <= state.replay_max;
    return inRange(value) && (long === 0 || inRange(long));
  }
  $('setting-language').addEventListener('change', () => ipc('set_language', { language: $('setting-language').value }));
  $('setting-display').addEventListener('change', () => edit(d => { d.display = $('setting-display').value; }));
  $('setting-capture-window').addEventListener('change', () => edit(d => { d.capture_window = $('setting-capture-window').value === 'true'; }));
  $('setting-game-audio').addEventListener('change', () => edit(d => { d.game_audio = $('setting-game-audio').value === 'true'; }));
  $('setting-target').addEventListener('change', () => edit(d => { d.target_exe = $('setting-target').value; }));
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
  $('setting-follow-display').addEventListener('change', () => edit(d => { d.follow_game_display = $('setting-follow-display').checked; }));
  $('setting-system-volume').addEventListener('input', () => edit(d => { d.system_volume = Number($('setting-system-volume').value); }));
  $('setting-microphone-volume').addEventListener('input', () => edit(d => { d.microphone_volume = Number($('setting-microphone-volume').value); }));
  $('setting-replay-long').addEventListener('input', () => edit(d => { d.replay_seconds_long = Number($('setting-replay-long').value) || 0; }));
  $('setting-marker-before').addEventListener('input', () => edit(d => { d.marker_before = Number($('setting-marker-before').value) || 0; }));
  $('setting-marker-after').addEventListener('input', () => edit(d => { d.marker_after = Number($('setting-marker-after').value) || 0; }));
  $('setting-min-free').addEventListener('input', () => edit(d => { d.min_free_gb = Number($('setting-min-free').value) || 0; }));
  $('setting-storage-limit').addEventListener('input', () => edit(d => { d.storage_limit_gb = Number($('setting-storage-limit').value) || 0; }));
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
    if (!validReplay(settings)) {
      const long = Number(settings.replay_seconds_long);
      const field = long !== 0 && (long < state.replay_min || long > state.replay_max) ? 'setting-replay-long' : 'setting-replay-custom';
      $(field).reportValidity();
      return;
    }
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
  const HOTKEY_FIELDS = {
    replay: 'hotkey_replay',
    replay_long: 'hotkey_replay_long',
    marker: 'hotkey_marker',
    record: 'hotkey_record',
  };
  function bindHotkeyField(id, field) {
    const key = HOTKEY_FIELDS[field];
    const input = $(id);
    input.addEventListener('focus', () => { capturing = field; input.classList.add('capturing'); input.value = ''; input.placeholder = tr('Taste drücken …'); });
    input.addEventListener('blur', () => { capturing = null; input.classList.remove('capturing'); input.placeholder = tr('Nicht belegt'); renderSettings(); });
    input.addEventListener('keydown', event => {
      event.preventDefault();
      const pressed = event.key;
      if (['Control', 'Shift', 'Alt', 'Meta', 'AltGraph'].includes(pressed)) return;
      if (pressed === 'Escape') { input.blur(); return; }
      if (pressed === 'Backspace' || (pressed === 'Delete' && !event.ctrlKey && !event.altKey && !event.shiftKey)) {
        edit(d => { d[key] = { ctrl: false, alt: false, shift: false, win: false, key: '' }; });
        input.blur();
        return;
      }
      const name = keyNameFromEvent(event);
      if (!name) { input.value = tr('Diese Taste wird nicht unterstützt'); return; }
      edit(d => { d[key] = { ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, win: event.metaKey, key: name }; });
      input.blur();
    });
  }
  bindHotkeyField('hotkey-replay', 'replay');
  bindHotkeyField('hotkey-replay-long', 'replay_long');
  bindHotkeyField('hotkey-marker', 'marker');
  bindHotkeyField('hotkey-record', 'record');
  $('hotkey-replay-clear').addEventListener('click', () => edit(d => { d.hotkey_replay = { ctrl: false, alt: false, shift: false, win: false, key: '' }; }));
  $('hotkey-replay-long-clear').addEventListener('click', () => edit(d => { d.hotkey_replay_long = { ctrl: false, alt: false, shift: false, win: false, key: '' }; }));
  $('hotkey-marker-clear').addEventListener('click', () => edit(d => { d.hotkey_marker = { ctrl: false, alt: false, shift: false, win: false, key: '' }; }));
  $('hotkey-record-clear').addEventListener('click', () => edit(d => { d.hotkey_record = { ctrl: false, alt: false, shift: false, win: false, key: '' }; }));

  // ---------------------------------------------------------------- editor
  const video = $('editor-video');
  let selected = null;
  let clipMarks = [];
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
    selected = { id: clip.id, src: clip.src, name: clip.name, path: clip.path, width: clip.width, height: clip.height };
    $('clip-game').value = clip.game || ''; $('clip-tags').value = (clip.tags || []).join(', ');
    $('crop-format').value = 'original'; $('crop-x').value = '50'; $('crop-y').value = '50';
    clipMarks = (clip.markers || []).filter(Number.isFinite).sort((a, b) => a - b);
    duration = Number(clip.duration) || 0;
    sel = { start: 0, end: duration };
    loop = false;
    $('loop-selection').classList.remove('active');
    $('editor-title').textContent = clip.name;
    $('editor-subtitle').textContent = [clip.width && clip.height ? `${clip.width}×${clip.height}` : null, size(clip.bytes), clip.audio === false ? tr('ohne Ton') : null, dateLabel(clip.created)].filter(Boolean).join(' · ');
    $('clip-name').value = clip.name;
    $('clip-duration').textContent = duration ? tr('Länge ') + precise(duration) : '';
    $('video-error').hidden = true;
    video.src = clip.src;
    $('editor-backdrop').hidden = false;
    document.body.style.overflow = 'hidden';
    renderMarks();
    renderEditor();
    renderState();
    $('play-pause').focus();
    window.StillclipDesktop?.selectClip(clip);
  }
  function closeEditor() {
    cancelAnimationFrame(raf);
    video.pause();
    video.removeAttribute('src');
    video.load();
    $('editor-backdrop').hidden = true;
    document.body.style.overflow = '';
    selected = null;
    window.StillclipDesktop?.selectClip(null);
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
  // Marked moments sit on the timeline as little flags. Drawn when the clip
  // or its length changes, never in the playback loop.
  function renderMarks() {
    const box = $('timeline-marks');
    const total = duration || 0;
    const marks = total ? clipMarks.filter(at => at >= 0 && at <= total + 0.05) : [];
    box.replaceChildren(...marks.map(at => {
      const mark = el('button', 'timeline-mark');
      mark.style.left = (clamp(at, 0, total) / total) * 100 + '%';
      mark.title = tr`Markierter Moment bei ${precise(at)}`;
      mark.setAttribute('aria-label', mark.title);
      mark.addEventListener('click', event => {
        event.stopPropagation();
        video.currentTime = clamp(at, 0, total);
        renderEditor();
      });
      return mark;
    }));
    const count = $('marker-count');
    count.hidden = !marks.length;
    count.title = tr('Mit M zur nächsten Markierung springen, mit Umschalt + M zurück.');
    count.replaceChildren(
      icon('bookmark'),
      el('span', '', marks.length === 1 ? tr('ein markierter Moment') : tr`${marks.length} markierte Momente`),
    );
  }

  // M jumps to the next marked moment, Shift+M to the previous one.
  function jumpToMark(direction) {
    if (!clipMarks.length) return;
    const now = video.currentTime || 0;
    const target = direction > 0
      ? clipMarks.find(at => at > now + 0.05)
      : [...clipMarks].reverse().find(at => at < now - 0.05);
    if (target === undefined) return;
    video.currentTime = clamp(target, 0, duration || target);
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
    renderCrop();
    validateTrim();
  }
  function validateTrim() {
    const start = Number($('trim-start').value), end = Number($('trim-end').value);
    const hasValues = $('trim-start').value !== '' && $('trim-end').value !== '';
    const minimum = minSelection();
    const valid = hasValues && Number.isFinite(start) && Number.isFinite(end) && start >= 0 && end > start
      && end - start + state.editor_tolerance >= minimum && (!duration || end <= duration + state.editor_tolerance);
    $('trim-clip').disabled = !loaded || state.busy || !state.ffmpeg_available || !selected || !valid || state.exports.length >= state.export_limit;
    const invalid = hasValues && !valid;
    $('trim-help').classList.toggle('error', invalid);
    const crop = currentCrop();
    $('trim-mode').querySelector('[value="fast"]').disabled = !!crop;
    if (crop && $('trim-mode').value === 'fast') $('trim-mode').value = 'precise';
    const mode = $('trim-mode').value;
    const discordLimit = mode === 'discord_free' ? '20 MB' : mode === 'discord_nitro' ? '1 GB' : null;
    $('trim-label').textContent = discordLimit ? tr('Discord-Export einreihen') : tr('Export einreihen');
    $('trim-help').textContent = invalid
      ? (duration && end > duration + state.editor_tolerance ? tr('Das Ende muss innerhalb der Videolänge liegen.') : duration && duration < state.editor_min_seconds ? tr('Dieses kurze Video wird vollständig übernommen.') : tr('Der Ausschnitt muss mindestens zwei Sekunden lang sein.'))
      : !state.ffmpeg_available ? tr('Wähle FFmpeg in den Einstellungen aus, um Clips zuzuschneiden.')
        : discordLimit ? tr`MP4 unter ${discordLimit}. Die Qualität passt sich der Cliplänge an; die fertige Dateigröße wird geprüft. Das Komprimieren kann etwas dauern. Das Original bleibt erhalten.`
          : mode === 'fast' ? tr('Schnell: ohne Neukodierung, der Schnitt beginnt am vorherigen Keyframe (bis zu einer Sekunde früher). Das Original bleibt erhalten.')
          : tr('Präzise: bildgenau, dauert je nach Länge ein paar Sekunden. Das Original bleibt erhalten.');
    renderHighlights();
  }

  const highlightPieces = [];
  let highlightSignature = '';
  function renderHighlights() {
    $('highlight-add').disabled = $('trim-clip').disabled || highlightPieces.length >= 20;
    const total = highlightPieces.reduce((sum, piece) => sum + piece.end - piece.start, 0);
    const missing = highlightPieces.some(piece => !state.clips.some(clip => clip.id === piece.id));
    const signature = JSON.stringify([highlightPieces, state.busy, highlightPieces.map(piece => state.clips.find(clip => clip.id === piece.id)?.name)]);
    if (signature !== highlightSignature) {
      highlightSignature = signature;
      $('highlight-list').replaceChildren(...highlightPieces.map((piece, index) => {
      const clip = state.clips.find(c => c.id === piece.id);
      const row = el('li', 'highlight-piece');
      row.append(el('span', '', `${clip?.name || piece.name} · ${piece.start.toFixed(1)}–${piece.end.toFixed(1)} s${clip ? '' : tr(' (nicht mehr verfügbar)')}`));
      for (const [label, delta] of [['↑', -1], ['↓', 1], [tr('Entfernen'), 0]]) {
        const button = el('button', 'btn small', label);
        button.setAttribute('aria-label', delta < 0 ? tr('Ausschnitt nach oben') : delta > 0 ? tr('Ausschnitt nach unten') : tr('Ausschnitt entfernen'));
        button.disabled = state.busy || (delta !== 0 && (index + delta < 0 || index + delta >= highlightPieces.length));
        button.addEventListener('click', () => {
          if (delta) [highlightPieces[index], highlightPieces[index + delta]] = [highlightPieces[index + delta], highlightPieces[index]];
          else highlightPieces.splice(index, 1);
          renderHighlights();
        });
        row.append(button);
      }
      return row;
    }));
    }
    $('highlight-summary').textContent = missing ? tr('Ein Quellclip fehlt. Entferne den betroffenen Ausschnitt.')
      : total > 1800 ? tr('Bitte auf höchstens 30 Minuten kürzen.')
        : highlightPieces.length ? tr`${highlightPieces.length} Ausschnitte · ${short(total)}` : tr('Noch keine Ausschnitte.');
    $('highlight-export').disabled = !loaded || state.busy || !state.ffmpeg_available || !highlightPieces.length || missing || total > 1800 || state.exports.length >= state.export_limit;
    $('highlight-clear').disabled = state.busy || !highlightPieces.length;
    $('highlight-mode').disabled = state.busy;
    $('highlight-format').disabled = state.busy;
  }
  $('highlight-add').addEventListener('click', () => {
    if (!$('highlight-add').disabled && selected) {
      highlightPieces.push({ id: selected.id, name: selected.name, start: Number($('trim-start').value), end: Number($('trim-end').value), crop: currentCrop() });
      renderHighlights();
    }
  });
  $('highlight-clear').addEventListener('click', () => { highlightPieces.length = 0; renderHighlights(); });
  $('highlight-export').addEventListener('click', () => {
    if (!$('highlight-export').disabled) ipc('highlights', { pieces: highlightPieces.map(({ id, start, end, crop }) => ({ id, start, end, crop })), mode: $('highlight-mode').value, portrait_width: $('highlight-format').value === 'original' ? null : Number($('highlight-format').value) });
  });

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
    $('clip-duration').textContent = duration ? tr('Länge ') + precise(duration) : '';
    if (wasWhole) sel = { start: 0, end: duration };
    $('trim-start').max = String(Math.max(0, duration - state.editor_min_seconds));
    $('trim-end').min = String(Math.min(state.editor_min_seconds, duration));
    $('trim-end').max = String(duration);
    setSelection(sel.start, sel.end);
    renderMarks();
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
    if (selected && !$('trim-clip').disabled) ipc('trim', { id: selected.id, start: Number($('trim-start').value), end: Number($('trim-end').value), mode: $('trim-mode').value, crop: currentCrop() });
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
    if (document.activeElement?.tagName === 'BUTTON' && [' ', 'Enter'].includes(event.key)) return;
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
      case 'm': case 'M': event.preventDefault(); jumpToMark(event.shiftKey ? -1 : 1); break;
      case 'Delete': event.preventDefault(); $('editor-delete').click(); break;
    }
  });

  // ------------------------------------------------------------- render
  function renderState() {
    const unavailable = !loaded || state.busy;
    document.querySelectorAll('[data-action]').forEach(button => {
      const action = button.dataset.action;
      button.disabled = unavailable || ((state.recording || state.exports.length || state.audio_test.running || state.dragging) && ['choose_folder', 'choose_ffmpeg'].includes(action));
    });
    renderRecordCard();
    renderQueue(); renderAudioTest();
    if (state.audio_test.running && !state.recording) $('record-button').disabled = true;
    $('setup-banner').hidden = !loaded || state.ffmpeg_available;
    ['app-status', 'editor-status'].forEach(id => {
      const status = $(id);
      status.textContent = state.status || '';
      status.classList.toggle('is-error', !!state.error);
      status.classList.toggle('is-busy', !!state.busy);
    });
    $('rename-clip').disabled = unavailable || !selected || !$('clip-name').value.trim();
    ['clip-name', 'trim-start', 'trim-end', 'trim-mode', 'editor-delete'].forEach(id => { $(id).disabled = unavailable; });
    const locked = selected && state.clips.find(c => c.id === selected.id)?.locked;
    if (locked) { $('rename-clip').disabled = true; $('editor-delete').disabled = true; }
    for (const id of ['clip-game', 'clip-tags', 'save-tags', 'crop-format', 'crop-x', 'crop-y', 'crop-center']) $(id).disabled = unavailable;
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
    if (draft) { draft.custom_games = state.settings.custom_games; draft.language = state.settings.language; draft.desktop = state.settings.desktop; }
    if (window.i18n.setLanguage(state.settings.language)) {
      clipSignature = ''; markerSignature = ''; highlightSignature = '';
      for (const id of ['filter-game', 'filter-tag']) delete $(id).dataset.values;
      if (selected) {
        const clip = state.clips.find(c => c.id === selected.id);
        if (clip) $('editor-subtitle').textContent = [clip.width && clip.height ? `${clip.width}×${clip.height}` : null, size(clip.bytes), clip.audio === false ? tr('ohne Ton') : null, dateLabel(clip.created)].filter(Boolean).join(' · ');
        $('clip-duration').textContent = duration ? tr('Länge ') + precise(duration) : '';
        renderMarks(); renderEditor();
      }
    }
    const signature = JSON.stringify([state.clips, state.busy, state.dragging]);
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
    window.StillclipDesktop?.setState(state);
  };

  document.querySelectorAll('[data-action]').forEach(button => button.addEventListener('click', () => { if (!button.disabled) ipc(button.dataset.action); }));
  $('record-button').addEventListener('click', () => ipc(state.recording ? 'stop' : 'start'));
  $('replay-button').addEventListener('click', () => ipc('replay'));
  $('replay-long-button').addEventListener('click', () => ipc('replay_long'));
  $('mark-button').addEventListener('click', () => ipc('mark'));
  window.addEventListener('focus', () => { if (loaded && !state.busy) ipc('refresh'); });

  document.querySelectorAll('.settings-card').forEach((card, index) => {
    const heading = card.querySelector('h2');
    card.id = `settings-section-${index}`;
    const link = el('a', '', heading.textContent);
    link.href = `#${card.id}`;
    link.addEventListener('click', event => { event.preventDefault(); card.scrollIntoView({ block: 'start' }); });
    document.querySelector('.settings-nav').append(link);
  });
  document.querySelectorAll('.setting-row').forEach(row => {
    const label = row.querySelector('.setting-label');
    if (!label) return;
    row.querySelectorAll('input, select').forEach(input => {
      if (!input.hasAttribute('aria-label')) input.setAttribute('aria-label', label.textContent);
    });
  });

  // Used by the automated screenshot check to render other views. A page may
  // carry an element to scroll to, for example `settings#storage-state`.
  window.showPage = page => {
    const [name, anchor] = String(page).split('#');
    if (name === 'editor') { view('library'); setTimeout(() => { if (state.clips[0]) openEditor(state.clips[0]); }, 300); return; }
    view(name === 'settings' ? 'settings' : 'library');
    if (name === 'settings') ipc('list_apps');
    if (anchor) setTimeout(() => { const target = $(anchor); if (target) target.scrollIntoView({ block: 'center' }); }, 150);
  };

  window.getEditorContext = () => selected ? { id: selected.id, start: sel.start, end: sel.end, crop: currentCrop() } : null;
  window.selectEditorRange = (start, end, seek = start) => setSelection(start, end, { seek });
  function currentCrop() {
    if ($('crop-format').value === 'original') return null;
    return { width: Number($('crop-format').value), x: Number($('crop-x').value) / 100, y: Number($('crop-y').value) / 100 };
  }
  function cropGeometry() {
    const w = video.videoWidth || selected?.width, h = video.videoHeight || selected?.height;
    if (!w || !h) return null;
    const box = video.getBoundingClientRect(), scale = Math.min(box.width / w, box.height / h);
    const width = w * scale, height = h * scale;
    const cropWidth = Math.min(width, height * 9 / 16), cropHeight = Math.min(height, width * 16 / 9);
    return { left: (box.width - width) / 2, top: (box.height - height) / 2, dx: width - cropWidth, dy: height - cropHeight, width: cropWidth, height: cropHeight };
  }
  function renderCrop() {
    const crop = currentCrop(), geometry = cropGeometry();
    $('crop-position').hidden = !crop; $('crop-help').hidden = !crop;
    const frame = $('crop-frame'); frame.hidden = !crop || !geometry || !selected;
    if (frame.hidden) return;
    frame.style.left = geometry.left + geometry.dx * crop.x + 'px'; frame.style.top = geometry.top + geometry.dy * crop.y + 'px';
    frame.style.width = geometry.width + 'px'; frame.style.height = geometry.height + 'px';
    $('crop-x').disabled = geometry.dx < 1; $('crop-y').disabled = geometry.dy < 1;
  }
  $('crop-format').addEventListener('change', () => { renderCrop(); validateTrim(); });
  for (const id of ['crop-x', 'crop-y']) $(id).addEventListener('input', renderCrop);
  $('crop-center').addEventListener('click', () => { $('crop-x').value = '50'; $('crop-y').value = '50'; renderCrop(); });
  new ResizeObserver(renderCrop).observe(video);
  let cropDrag = null;
  $('crop-frame').addEventListener('pointerdown', event => {
    if (event.button !== 0) return;
    event.preventDefault(); event.stopPropagation();
    cropDrag = { x: event.clientX, y: event.clientY, crop: currentCrop(), geometry: cropGeometry() };
    event.currentTarget.setPointerCapture(event.pointerId);
  });
  $('crop-frame').addEventListener('pointermove', event => {
    if (!cropDrag?.geometry) return;
    const { x, y, crop, geometry } = cropDrag;
    if (geometry.dx > 1) $('crop-x').value = String(clamp(crop.x + (event.clientX - x) / geometry.dx, 0, 1) * 100);
    if (geometry.dy > 1) $('crop-y').value = String(clamp(crop.y + (event.clientY - y) / geometry.dy, 0, 1) * 100);
    renderCrop();
  });
  for (const type of ['pointerup', 'pointercancel', 'lostpointercapture']) $('crop-frame').addEventListener(type, () => { cropDrag = null; });
  $('save-tags').addEventListener('click', () => {
    if (selected) ipc('annotate', { id: selected.id, game: $('clip-game').value, tags: $('clip-tags').value.split(',') });
  });

  // Keep queue rows in place while progress changes, including keyboard focus.
  function renderQueue() {
    for (const prefix of ['', 'editor-']) {
      $(prefix + 'export-queue').hidden = !state.exports.length;
      $(prefix + 'export-count').textContent = `${state.exports.length}/${state.export_limit}`;
      const list = $(prefix + 'export-list');
      const ids = new Set(state.exports.map(job => String(job.id)));
      for (const row of [...list.children]) if (!ids.has(row.dataset.id)) row.remove();
      for (const job of state.exports) {
        let row = [...list.children].find(row => row.dataset.id === String(job.id));
        if (!row) {
          row = el('li', 'export-item'); row.dataset.id = String(job.id);
          const progress = el('progress'); progress.max = 100; progress.setAttribute('aria-label', tr('Exportfortschritt'));
          const cancel = el('button', 'btn small', tr('Abbrechen')); cancel.addEventListener('click', () => ipc('cancel_export', { id: job.id }));
          row.append(el('strong'), el('small'), progress, cancel); list.append(row);
        }
        row.querySelector('strong').textContent = job.label;
        row.querySelector('small').textContent = job.cancelling ? tr('Wird abgebrochen …') : job.running ? `${job.phase || tr('Vorbereiten')} · ${job.percent}%${job.eta_seconds != null ? tr` · noch ca. ${short(job.eta_seconds)}` : ''}` : tr('Wartet');
        row.querySelector('progress').setAttribute('aria-label', tr('Exportfortschritt'));
        row.querySelector('progress').value = job.percent || 0;
        row.querySelector('button').disabled = !!job.cancelling;
        row.querySelector('button').textContent = job.running ? tr('Abbrechen') : tr('Entfernen');
      }
    }
  }
  function renderAudioTest() {
    const test = state.audio_test || {};
    $('audio-test-start').disabled = !loaded || state.busy || state.recording || test.running || !state.ffmpeg_available;
    $('audio-test-cancel').hidden = !test.running;
    $('audio-test-status').textContent = test.error || (test.running ? `${test.phase || tr('Audiogeräte öffnen')} · ${test.percent || 0}%` : test.src ? tr('Test bereit. Höre dir den Mix an und passe bei Bedarf die Lautstärke an.') : tr('Bereit für einen Audio-Test.'));
    $('audio-test-status').classList.toggle('is-error', !!test.error);
    const levels = test.levels || [];
    const box = $('audio-levels');
    if (box.children.length !== levels.length) {
      box.replaceChildren(...levels.map(() => { const row = el('div', 'audio-level'); const meter = el('meter'); meter.min = 0; meter.max = 1; meter.low = 0.01; meter.high = 0.9; meter.optimum = 0.4; row.append(el('span'), meter, el('span')); return row; }));
    }
    levels.forEach(([label, peak], index) => {
      const row = box.children[index]; row.children[0].textContent = label;
      row.children[1].value = Math.min(1, Math.max(0, 1 + 20 * Math.log10(Math.max(peak, 0.001)) / 60));
      row.children[1].setAttribute('aria-label', label + tr(' Pegel'));
      row.children[2].textContent = peak >= 1 ? tr('Übersteuert') : peak > 0 ? `${(20 * Math.log10(peak)).toFixed(0)} dBFS` : tr('Stille');
    });
    const preview = $('audio-preview');
    if (test.src && preview.getAttribute('src') !== test.src) preview.src = test.src;
    if (!test.src && preview.hasAttribute('src')) { preview.pause(); preview.removeAttribute('src'); preview.load(); }
    preview.hidden = !test.src;
  }
  $('audio-test-start').addEventListener('click', () => { $('audio-preview').pause(); ipc('audio_test', { settings: currentSettings() }); });
  $('audio-test-cancel').addEventListener('click', () => ipc('cancel_audio_test'));

  window.i18n.bindStatic();
  renderLibrary();
  renderState();
  renderSettings();
  ipc('ready');
})();
