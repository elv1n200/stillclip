// Browser-level regression checks use explicit fixture state, never real capture.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs/promises');

(async () => {
  const browser = await chromium.launch({ headless: true,
    ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 840 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.route('http://app.localhost/**', async route => {
      const name = new URL(route.request().url()).pathname.slice(1) || 'index.html';
      if (!['index.html', 'app.css', 'desktop.css', 'i18n.js', 'desktop.js', 'app.js'].includes(name)) return route.abort();
      await route.fulfill({ path: path.join(__dirname, '../ui', name),
        contentType: name.endsWith('.html') ? 'text/html' : name.endsWith('.css') ? 'text/css' : 'text/javascript' });
    });
    const preview = await fs.readFile(path.join(__dirname, 'fixtures/preview.webm'));
    await page.route('http://clip.localhost/**', route => {
      const range = route.request().headers().range;
      const match = range && /^bytes=(\d+)-(\d*)$/.exec(range);
      const start = match ? Number(match[1]) : 0;
      const end = match && match[2] ? Math.min(Number(match[2]), preview.length - 1) : preview.length - 1;
      return route.fulfill({ body: preview.subarray(start, end + 1), contentType: 'video/webm',
        status: match ? 206 : 200,
        headers: { 'Access-Control-Allow-Origin': 'http://app.localhost', 'Accept-Ranges': 'bytes',
          ...(match ? { 'Content-Range': `bytes ${start}-${end}/${preview.length}` } : {}) }
      });
    });
    await page.addInitScript(() => { window.messages = []; window.ipc = { postMessage: text => window.messages.push(JSON.parse(text)) }; });
    await page.goto('http://app.localhost/');
    await page.evaluate(() => window.setState({ version: '0.5.0', ffmpeg_available: true,
      ffmpeg_path: 'C:\\Tools\\ffmpeg.exe', clips_dir: 'C:\\Videos\\Stillclip',
      displays: [{ name: 'DISPLAY1', label: 'Hauptbildschirm · 1920×1080', width: 1920, height: 1080, primary: true }],
      capabilities: { probed: true, ddagrab: true, encoders: ['nvenc'] },
      encoders: [{ id: 'nvenc', label: 'NVIDIA NVENC', available: true }], selected_encoder: 'NVIDIA NVENC',
      builtin_games: [{ name: 'Counter-Strike 2', exe: 'cs2.exe' }, { name: 'VALORANT', exe: 'valorant-win64-shipping.exe' }]
    }));
    assert(await page.locator('#record-button').isEnabled());
    assert.equal(await page.locator('.btn-primary').first().evaluate(el => getComputedStyle(el).borderRadius), '0px');
    await page.click('#record-button');
    assert.equal(await page.evaluate(() => window.messages.at(-1).action), 'start');

    // Exports must not disable stopping or marking a live recording.
    await page.evaluate(() => window.setState({ recording: true, busy: true, capture_busy: false,
      diagnostics: { fps: 59.8, dropped: 2, duplicated: 1, frames: 600, lag_seconds: 0.5, age_seconds: 1 } }));
    assert.match(await page.locator('#diagnostic-values').textContent(), /59.8 FPS/);
    assert(await page.locator('#record-button').isEnabled());
    assert(await page.locator('#mark-button').isEnabled());
    assert(await page.locator('#replay-button').isDisabled());
    await page.click('#record-button');
    assert.equal(await page.evaluate(() => window.messages.at(-1).action), 'stop');
    await page.click('#mark-button');
    assert.equal(await page.evaluate(() => window.messages.at(-1).action), 'mark');

    await page.evaluate(() => window.setState({ busy: false }));
    await page.click('#nav-settings');
    for (const id of ['setting-replay', 'setting-replay-long', 'setting-marker-before', 'setting-marker-after', 'setting-precise', 'setting-min-free', 'setting-mic-separate', 'setting-follow-display', 'setting-capture-window', 'setting-target', 'setting-game-audio', 'profile-game']) {
      assert(await page.locator('#' + id).isDisabled(), `${id} must be locked while recording`);
    }
    assert(await page.locator('#hotkey-replay').isEnabled());
    assert(await page.locator('#settings-locked').isVisible());
    await page.evaluate(() => window.setState({ recording: false }));
    assert(await page.locator('#setting-replay').isEnabled());
    assert(await page.locator('#settings-locked').isHidden());
    await page.selectOption('#profile-game', 'valorant-win64-shipping.exe');
    await page.selectOption('#profile-quality', 'p1080');
    await page.selectOption('#profile-fps', '120');
    await page.fill('#profile-replay', '60');
    await page.check('#profile-microphone');
    await page.check('#profile-window');
    await page.check('#profile-audio');
    await page.evaluate(() => window.setState({ elapsed: 1 }));
    assert.equal(await page.inputValue('#profile-fps'), '120', 'live state must not overwrite a profile being edited');
    await page.click('#profile-save');
    await page.click('#save-settings');
    const settingsMessage = await page.evaluate(() => window.messages.at(-1));
    const gameProfile = settingsMessage.settings.game_profiles[0];
    assert.equal(gameProfile.exe, 'valorant-win64-shipping.exe');
    assert.equal(gameProfile.fps, 120); assert.equal(gameProfile.replay_seconds, 60);
    assert(gameProfile.capture_window && gameProfile.game_audio && gameProfile.microphone);
    await page.evaluate(settings => window.setState({ settings }), settingsMessage.settings);
    const sections = await page.locator('.settings-nav a').count();
    assert(sections >= 5);
    await page.locator('.settings-nav a').last().click();
    assert(await page.locator('.settings-card').last().isVisible());

    const shotDir = process.env.STILLCLIP_SHOTS_DIR;
    if (shotDir) await fs.mkdir(shotDir, { recursive: true });
    for (const width of [1280, 720]) {
      await page.setViewportSize({ width, height: 840 });
      await page.evaluate(() => window.scrollTo(0, 0));
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Settings overflow at ${width}`);
      if (shotDir) await page.screenshot({ path: path.join(shotDir, `settings-${width}.png`), animations: 'disabled' });
      await page.click('#nav-library');
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Library overflow at ${width}`);
      if (shotDir) await page.screenshot({ path: path.join(shotDir, `library-${width}.png`), animations: 'disabled' });
      await page.click('#nav-settings');
    }
    await page.setViewportSize({ width: 1280, height: 840 });
    await page.click('#nav-library');
    const clipsFixture = [{
      id: 1, name: 'Testaufnahme', src: 'http://clip.localhost/1', path: 'test.webm',
      duration: 3, width: 320, height: 180, audio: false, bytes: 42000,
      extension: 'webm', created: 1794816000, markers: [1, 2]
    }];
    await page.evaluate(clips => window.setState({ clips }), clipsFixture);
    await page.locator('.card-favorite').click();
    assert.deepEqual(await page.evaluate(() => window.messages.at(-1)), { action: 'favorite', id: 1, favorite: true });
    await page.check('#favorites-only');
    assert.equal(await page.locator('.clip-card').count(), 0);
    clipsFixture[0].favorite = true;
    await page.evaluate(clips => window.setState({ clips }), clipsFixture);
    assert.equal(await page.locator('.clip-card').count(), 1);
    await page.uncheck('#favorites-only');
    await page.fill('#search', 'kein-treffer');
    assert.equal(await page.locator('.clip-card').count(), 0);
    await page.fill('#search', 'Test');
    await page.locator('.clip-card').click();
    await page.waitForFunction(() => document.getElementById('editor-video').readyState >= 2);
    assert(await page.locator('#video-error').isHidden());
    assert.equal(await page.locator('.timeline-mark').count(), 2);
    assert(await page.locator('#trim-clip').isEnabled());
    await page.locator('.timeline-mark').first().click();
    await page.waitForFunction(() => Math.abs(document.getElementById('editor-video').currentTime - 1) < 0.1);
    await page.click('#trim-clip');
    assert.equal(await page.evaluate(() => window.messages.at(-1).action), 'trim');
    for (const [mode, limit] of [['discord_free', '20 MB'], ['discord_nitro', '1 GB']]) {
      await page.selectOption('#trim-mode', mode);
      assert.match(await page.locator('#trim-help').textContent(), new RegExp(limit));
      assert.equal(await page.locator('#trim-label').textContent(), 'Discord-Export einreihen');
      await page.click('#trim-clip');
      const message = await page.evaluate(() => window.messages.at(-1));
      assert.deepEqual(message, { action: 'trim', id: 1, start: 0, end: 3, mode, crop: null });
    }
    // Invalid selections and ordinary file jobs block exports; queued exports do not lock the editor.
    await page.fill('#trim-end', '9');
    assert(await page.locator('#trim-clip').isDisabled());
    await page.fill('#trim-end', '3');
    await page.evaluate(() => window.setState({ busy: true }));
    assert(await page.locator('#trim-clip').isDisabled());
    assert(await page.locator('#trim-mode').isDisabled());
    await page.evaluate(() => window.setState({ busy: false }));
    assert(await page.locator('#trim-clip').isEnabled());
    await page.selectOption('#trim-mode', 'discord_free');
    await page.click('#highlight-add');
    await page.fill('#trim-start', '1');
    await page.click('#highlight-add');
    assert.equal(await page.locator('#highlight-list li').count(), 2);
    await page.locator('#highlight-list li').last().getByRole('button', { name: 'Ausschnitt nach oben' }).click();
    await page.selectOption('#highlight-mode', 'discord_nitro');
    await page.click('#highlight-export');
    assert.deepEqual(await page.evaluate(() => window.messages.at(-1)), {
      action: 'highlights', mode: 'discord_nitro', portrait_width: null, pieces: [{ id: 1, start: 1, end: 3, crop: null }, { id: 1, start: 0, end: 3, crop: null }]
    });
    await page.evaluate(() => window.setState({ busy: true }));
    assert(await page.locator('#highlight-export').isDisabled());
    assert(await page.locator('#highlight-clear').isDisabled());
    await page.evaluate(() => window.setState({ busy: false }));
    await page.locator('#highlight-list li').first().getByRole('button', { name: 'Ausschnitt entfernen' }).click();
    assert.equal(await page.locator('#highlight-list li').count(), 1);
    await page.click('#highlight-clear');
    assert(await page.locator('#highlight-export').isDisabled());
    await page.fill('#trim-start', '0');
    // Crop is movable, keyboard adjustable, and saved independently per highlight piece.
    await page.selectOption('#trim-mode', 'fast');
    await page.selectOption('#crop-format', '720');
    assert.equal(await page.inputValue('#trim-mode'), 'precise');
    await page.locator('#crop-frame').scrollIntoViewIfNeeded();
    const cropBox = await page.locator('#crop-frame').boundingBox();
    assert(cropBox && Math.abs(cropBox.width / cropBox.height - 9 / 16) < 0.01);
    await page.mouse.move(cropBox.x + cropBox.width / 2, cropBox.y + cropBox.height / 2);
    await page.mouse.down(); await page.mouse.move(cropBox.x + cropBox.width / 2 + 60, cropBox.y + cropBox.height / 2); await page.mouse.up();
    assert(Number(await page.inputValue('#crop-x')) > 50);
    await page.locator('#crop-x').fill('100');
    await page.locator('#crop-x').dispatchEvent('input');
    await page.click('#trim-clip');
    assert.deepEqual((await page.evaluate(() => window.messages.at(-1))).crop, { x: 1, y: 0.5, width: 720 });
    await page.click('#highlight-add');
    await page.selectOption('#highlight-format', '1080');
    await page.click('#highlight-export');
    const portrait = await page.evaluate(() => window.messages.at(-1));
    assert.equal(portrait.portrait_width, 1080); assert.equal(portrait.pieces[0].crop.x, 1);
    await page.fill('#clip-game', 'VALORANT'); await page.fill('#clip-tags', 'Ace, Clutch'); await page.click('#save-tags');
    assert.deepEqual(await page.evaluate(() => window.messages.at(-1)), { action: 'annotate', id: 1, game: 'VALORANT', tags: ['Ace', ' Clutch'] });
    // Queue remains operable without stealing focus when percent changes.
    const jobs = [{ id: 8, label: 'Discord-Clip', running: true, percent: 35, phase: 'Kodieren', eta_seconds: 20, cancelling: false }, { id: 9, label: 'Nächster Clip', running: false, percent: 0, phase: 'Wartet', cancelling: false }];
    await page.evaluate(({ jobs, clips }) => window.setState({ exports: jobs, clips: clips.map(c => ({ ...c, locked: true, game: 'VALORANT', tags: ['Ace', 'Clutch'] })) }), { jobs, clips: clipsFixture });
    assert(await page.locator('#trim-clip').isEnabled()); assert(await page.locator('#editor-delete').isDisabled()); assert(await page.locator('#rename-clip').isDisabled());
    await page.locator('#editor-export-list button').first().focus();
    await page.evaluate(jobs => window.setState({ exports: jobs.map(j => ({ ...j, percent: j.percent + 1 })) }), jobs);
    assert(await page.locator('#editor-export-list button').first().evaluate(el => el === document.activeElement));
    await page.locator('#editor-export-list button').last().click();
    assert.deepEqual(await page.evaluate(() => window.messages.at(-1)), { action: 'cancel_export', id: 9 });
    if (shotDir) await page.screenshot({ path: path.join(shotDir, 'queue-editor.png'), animations: 'disabled' });
    await page.evaluate(() => { document.querySelector('.editor').scrollTop = 0; });

    if (shotDir) await page.screenshot({ path: path.join(shotDir, 'editor-1280.png'), animations: 'disabled' });
    await page.locator('#trim-mode').scrollIntoViewIfNeeded();
    if (shotDir) await page.screenshot({ path: path.join(shotDir, 'discord-export-1280.png'), animations: 'disabled' });
    await page.setViewportSize({ width: 720, height: 600 });
    assert(await page.evaluate(() => {
      const box = document.querySelector('.editor').getBoundingClientRect();
      return box.left >= 0 && box.right <= innerWidth;
    }), 'Editor must fit the minimum window size');
    await page.click('#close-editor');
    await page.locator('.library-filters summary').click();
    await page.fill('#search', 'Clutch'); assert.equal(await page.locator('.clip-card').count(), 1);
    await page.selectOption('#filter-game', 'VALORANT'); await page.selectOption('#filter-tag', 'ace');
    await page.fill('#filter-from', '2026-11-16'); await page.locator('#filter-from').dispatchEvent('change');
    assert.equal(await page.locator('.clip-card').count(), 1);
    await page.fill('#filter-to', '2026-11-15'); await page.locator('#filter-to').dispatchEvent('change');
    assert.equal(await page.locator('.clip-card').count(), 0);
    await page.click('#reset-filters'); await page.selectOption('#filter-length', 'long'); assert.equal(await page.locator('.clip-card').count(), 0);
    await page.selectOption('#filter-length', 'short'); assert.equal(await page.locator('.clip-card').count(), 1);
    // Native drag IPC is issued only after a held left pointer crosses the threshold.
    await page.locator('.card-drag').scrollIntoViewIfNeeded();
    const dragBox = await page.locator('.card-drag').boundingBox();
    await page.mouse.move(dragBox.x + 5, dragBox.y + 5); await page.mouse.down(); await page.mouse.move(dragBox.x + 30, dragBox.y + 5); await page.mouse.up();
    assert((await page.evaluate(() => window.messages)).some(m => m.action === 'drag_clip' && m.id === 1));
    await page.click('#nav-settings'); await page.locator('#audio-test-start').scrollIntoViewIfNeeded(); await page.click('#audio-test-start');
    assert.equal((await page.evaluate(() => window.messages.at(-1))).action, 'audio_test');
    await page.evaluate(() => window.setState({ audio_test: { running: true, percent: 30, phase: 'Audio-Test läuft', levels: [['Mikrofon', 1.2], ['Spielton', 0]], src: null } }));
    assert(await page.locator('#audio-test-start').isDisabled()); assert.match(await page.locator('#audio-levels').textContent(), /Übersteuert.*Stille/);
    await page.click('#audio-test-cancel'); assert.equal((await page.evaluate(() => window.messages.at(-1))).action, 'cancel_audio_test');
    await page.evaluate(() => window.setState({ audio_test: { running: false, levels: [], src: 'http://clip.localhost/audio-test/1' } }));
    assert(await page.locator('#audio-preview').isVisible());
    // Language switches are immediate, preserve drafts, and send only language.
    await page.setViewportSize({ width: 1280, height: 840 });
    await page.selectOption('#setting-fps', '30');
    await page.selectOption('#profile-game', 'valorant-win64-shipping.exe');
    await page.selectOption('#profile-fps', '60');
    await page.selectOption('#setting-language', 'en');
    assert.deepEqual(await page.evaluate(() => window.messages.at(-1)), { action: 'set_language', language: 'en' });
    const englishSettings = { ...settingsMessage.settings, language: 'en' };
    await page.evaluate(settings => window.setState({ settings }), englishSettings);
    assert.equal(await page.locator('html').getAttribute('lang'), 'en');
    assert.equal((await page.locator('#nav-settings').textContent()).trim(), 'Settings');
    assert.equal(await page.inputValue('#setting-fps'), '30', 'language change must keep unsaved capture settings');
    assert.equal(await page.inputValue('#profile-fps'), '60', 'language change must keep unsaved game profile');
    assert.equal(await page.locator('#setting-language').getAttribute('aria-label'), 'Language');
    for (const width of [1280, 720]) {
      await page.setViewportSize({ width, height: 840 });
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `English settings overflow at ${width}`);
      if (shotDir) await page.screenshot({ path: path.join(shotDir, `settings-en-${width}.png`), animations: 'disabled' });
    }
    await page.click('#save-settings');
    const savedEnglish = await page.evaluate(() => window.messages.at(-1).settings);
    assert.equal(savedEnglish.language, 'en'); assert.equal(savedEnglish.fps, 30);
    // User content that matches a translated UI label must never be translated.
    const namedClip = { ...clipsFixture[0], name: 'Bibliothek', game: 'Einstellungen', tags: ['Aufnahme'] };
    await page.evaluate(({ settings, clips }) => window.setState({ settings, clips, exports: [] }), { settings: savedEnglish, clips: [namedClip] });
    await page.click('#nav-library'); await page.click('#reset-filters'); await page.fill('#search', '');
    assert.match(await page.locator('.clip-card').textContent(), /Bibliothek/);
    assert.equal(await page.locator('#search').getAttribute('placeholder'), 'Search');
    assert.equal(await page.locator('#record-card').evaluate(el => getComputedStyle(el, '::before').content), '"Recording"');
    assert.equal((await page.locator('.card-drag').textContent()).trim(), '↗ Drag');
    await page.locator('.clip-card').click();
    await page.waitForFunction(() => document.getElementById('editor-video').readyState >= 2);
    assert.equal(await page.locator('#editor-title').textContent(), 'Bibliothek');
    assert.equal(await page.inputValue('#clip-game'), 'Einstellungen');
    assert.equal(await page.inputValue('#clip-tags'), 'Aufnahme');
    assert.match(await page.locator('#editor-subtitle').textContent(), /no audio/);
    await page.selectOption('#trim-mode', 'discord_nitro');
    assert.equal(await page.locator('#trim-label').textContent(), 'Queue Discord export');
    await page.fill('#trim-start', '1'); await page.locator('#trim-start').dispatchEvent('change');
    await page.evaluate(settings => window.setState({ settings }), { ...savedEnglish, language: 'de' });
    assert.equal(Number(await page.inputValue('#trim-start')), 1, 'switching language must preserve the selected clip range');
    assert.equal(await page.locator('#editor-title').textContent(), 'Bibliothek');
    assert.match(await page.locator('#editor-subtitle').textContent(), /ohne Ton/);
    await page.click('#close-editor');
    await page.reload();
    await page.evaluate(settings => window.setState({ settings, ffmpeg_available: true }), savedEnglish);
    assert.equal(await page.locator('html').getAttribute('lang'), 'en', 'the native persisted language must apply after reload');
    assert.match((await page.locator('#nav-library').textContent()).trim(), /^Library\s*0$/);
    assert.equal(await page.evaluate(() => {
      const name = 'Bibliothek <script> {0}';
      return window.i18n.tr`Spiel ${name}`;
    }), 'Spiel Bibliothek <script> {0}', 'unknown templates keep interpolated user text intact');
    const mismatchedPlaceholders = await page.evaluate(() => Object.entries(window.i18n.english).filter(([de, en]) => {
      const slots = value => (value.match(/\{\d+\}/g) || []).sort().join(',');
      return slots(de) !== slots(en);
    }));
    assert.deepEqual(mismatchedPlaceholders, [], 'translations must preserve each interpolation slot');
    // Desktop workflow uses explicit IPC; the wizard never starts capture itself.
    const desktopPrefs = { welcome_complete: false, hud_enabled: true, hud_position: 'top_left', hud_scale: 75, automatic_updates: false, player_name: 'Elvin', killfeed_region: { left: .55, top: .02, width: .44, height: .30 } };
    const desktopState = { task: null, audio: null, review: null, preview: null, update: { checked: false, release: null, portable: true }, models_available: true, error: null, report: null };
    const desktopSettings = { ...savedEnglish, desktop: desktopPrefs, target_exe: '', capture_window: false, game_audio: false };
    await page.evaluate(({ settings, desktop }) => { window.messages=[]; window.setState({ settings, desktop, busy:false, recording:false, exports:[], clips:[], audio_test:{}, dragging:false }); }, { settings: desktopSettings, desktop: desktopState });
    assert(await page.locator('#setup-backdrop').isVisible());
    assert.equal(await page.locator('#setup-title').textContent(), 'Welcome to Stillclip');
    if(shotDir)await page.screenshot({path:path.join(shotDir,'setup-en.png'),animations:'disabled'});
    assert.equal(await page.evaluate(() => window.messages.some(m=>['start','setup_test'].includes(m.action))), false);
    await page.click('#wizard-next'); await page.check('#wizard-mic'); await page.click('#wizard-next');
    await page.click('#wizard-test');
    assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'setup_test');
    assert.equal(await page.evaluate(()=>window.messages.at(-1).settings.microphone),true);
    await page.evaluate(desktop=>window.setState({desktop:{...desktop, task:{id:1,kind:'setup',percent:42,phase:'Test recording',cancelling:false}}}),desktopState);
    assert(await page.locator('#wizard-finish').isDisabled());
    await page.click('#wizard-cancel'); assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'desktop_cancel');
    await page.evaluate(desktop=>window.setState({desktop}),desktopState);
    await page.click('#wizard-finish'); assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'setup_finish');
    desktopPrefs.welcome_complete=true;
    await page.evaluate(settings=>window.setState({settings}),desktopSettings);
    assert(await page.locator('#setup-backdrop').isHidden());
    await page.evaluate(()=>window.setState({capture_pending:true}));
    assert.equal(await page.locator('#record-button-text').textContent(),'Cancel start');
    await page.click('#record-button'); assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'stop');
    await page.evaluate(()=>window.setState({capture_pending:false}));
    await page.click('#nav-settings'); await page.click('#update-check');
    assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'update_check');
    await page.evaluate(()=>window.setState({recording:true})); assert(await page.locator('#update-check').isDisabled());
    await page.evaluate(()=>window.setState({recording:false}));
    // Automatic highlights are opt-in, require models/name and keep capture
    // controls usable while settings and other heavy jobs are locked.
    assert.equal(await page.locator('#auto-enabled').isChecked(),false);
    await page.evaluate(({settings,desktop})=>window.setState({settings:{...settings,desktop:{...settings.desktop,player_name:''}},desktop:{...desktop,models_available:false}}),{settings:desktopSettings,desktop:desktopState});
    assert(await page.locator('#auto-enabled').isDisabled());
    await page.evaluate(({settings,desktop})=>window.setState({settings,desktop}),{settings:desktopSettings,desktop:desktopState});
    await page.check('#auto-enabled');
    const automaticPrefs=await page.evaluate(()=>window.messages.at(-1));
    assert.equal(automaticPrefs.action,'desktop_preferences');
    assert.deepEqual(automaticPrefs.preferences.highlights,{enabled:true,min_kills:2,before:12,after:6,merge_gap:8});
    desktopPrefs.highlights=automaticPrefs.preferences.highlights;
    await page.evaluate(settings=>window.setState({settings,recording:true,auto_highlights:{running:true,phase:'watching',detected:3,saved:1,skipped:0,lag_seconds:2}}),desktopSettings);
    for(const id of ['auto-enabled','auto-player','auto-min-kills','killfeed-player','killfeed-width'])assert(await page.locator('#'+id).isDisabled(),`${id} must stay locked during detection`);
    assert.match(await page.locator('#auto-status').textContent(),/3 detected kills.*1 saved highlights/);
    await page.click('#auto-pause');assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'auto_highlights_pause');
    await page.evaluate(()=>window.setState({recording:false,auto_highlights:{running:true,phase:'finishing',detected:3,saved:1,skipped:1,lag_seconds:12}}));
    assert(await page.locator('#update-check').isDisabled());
    assert(await page.locator('#auto-player').isDisabled());
    assert.match(await page.locator('#auto-status').textContent(),/Processing the final moments.*1 skipped ranges.*12s/);
    await page.evaluate(()=>window.setState({auto_highlights:{running:false,phase:'error',saved:1,error:'<img src=x onerror=alert(1)>'}}));
    assert.equal(await page.locator('#auto-error img').count(),0);
    assert(await page.locator('#auto-pause').isHidden());
    await page.evaluate(()=>window.setState({auto_highlights:{running:false,phase:'complete',saved:1,detected:3}}));
    await page.click('#report-open');
    await page.evaluate(desktop=>window.setState({desktop:{...desktop,report:{version:'0.8.0',status:'<script>private</script>'}}}),desktopState);
    assert.match(await page.inputValue('#report-content'),/<script>private<\/script>/); assert.equal(await page.locator('#report-content script').count(),0);
    await page.click('#report-close');
    await page.evaluate(({clip,desktop})=>window.setState({clips:[{...clip,audio:true}],desktop}),{clip:namedClip,desktop:desktopState});
    await page.click('#nav-library'); await page.locator('.clip-card').click(); await page.locator('#audio-tools summary').click();
    await page.click('#audio-analyze'); assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'audio_analyze');
    const audioData={separated:true,duration:namedClip.duration,tracks:[{index:1,role:'game',peaks:[.2,.7,.3,.5]},{index:2,role:'microphone',peaks:[.1,.3,.1,.2]}]};
    await page.evaluate(({desktop,audio,id})=>window.setState({desktop:{...desktop,audio:{clip:id,result:audio}}}),{desktop:desktopState,audio:audioData,id:namedClip.id});
    assert.equal(await page.locator('.waveform-track').count(),2);
    await page.locator('#mix-game').fill('70'); await page.locator('#mix-game').dispatchEvent('input');
    await page.locator('.waveform-track').nth(1).locator('input[type=checkbox]').check();
    await page.click('#audio-listen-mix');
    assert.deepEqual(await page.evaluate(()=>window.messages.at(-1).mix),{game:70,microphone:0,mixed:100});
    await page.click('#audio-save-mix'); assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'audio_mix');
    await page.locator('#killfeed-tools > summary').click(); await page.click('#killfeed-analyze');
    assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'killfeed_analyze');
    await page.evaluate(({desktop,id})=>window.setState({desktop:{...desktop,review:{clip:id,result:{suggestions:[{at:2,start:0,end:4,text:'Elvin → Opponent <img>'}],samples:10,recognized_rows:2},accepted:[]}}}),{desktop:desktopState,id:namedClip.id});
    assert.equal(await page.locator('#killfeed-suggestions li').count(),1); assert.equal(await page.locator('#killfeed-suggestions img').count(),0);
    await page.locator('#killfeed-suggestions button').first().click();
    assert.equal(Number(await page.inputValue('#trim-start')),0);
    await page.locator('#killfeed-suggestions button').last().click();
    assert.equal(await page.evaluate(()=>window.messages.at(-1).action),'killfeed_accept');
    await page.waitForFunction(()=>document.getElementById('editor-video').readyState>=2);
    await page.click('#killfeed-calibrate');
    assert(await page.locator('#region-backdrop').isVisible());
    const area=await page.locator('#region-canvas').boundingBox();
    await page.mouse.move(area.x+area.width*.57,area.y+area.height*.03);await page.mouse.down();
    await page.mouse.move(area.x+area.width*.99,area.y+area.height*.32,{steps:4});await page.mouse.up();
    await page.click('#region-apply');
    const calibrated=await page.evaluate(()=>window.messages.at(-1));
    assert.equal(calibrated.action,'desktop_preferences');
    for(const [key,value] of Object.entries({left:.57,top:.03,width:.42,height:.29}))assert(Math.abs(calibrated.preferences.killfeed_region[key]-value)<.003,`calibration ${key}`);
    assert(await page.locator('#region-backdrop').isHidden());
    await page.click('#killfeed-calibrate');await page.keyboard.press('Escape');
    assert(await page.locator('#region-backdrop').isHidden());
    assert(await page.locator('#killfeed-calibrate').evaluate(el=>el===document.activeElement));
    for(const width of [1280,720]) {
      await page.setViewportSize({width,height:840});
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`desktop tools overflow at ${width}`);
      if(shotDir)await page.screenshot({path:path.join(shotDir,`desktop-editor-en-${width}.png`),animations:'disabled'});
    }
    await page.click('#close-editor'); await page.click('#nav-settings');
    await page.evaluate(settings=>window.setState({settings:{...settings,language:'de'}}),desktopSettings);
    assert.match(await page.locator('#setup-open').textContent(),/Einrichtungsassistent/);
    assert.match(await page.locator('#report-open').textContent(),/Diagnosebericht/);
    assert.match(await page.locator('#auto-status').textContent(),/Aufnahme ausgewertet/);
    for(const width of [1280,720]) {
      await page.setViewportSize({width,height:840});
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`automatic settings overflow at ${width}`);
      await page.locator('#auto-enabled').scrollIntoViewIfNeeded();
      if(shotDir)await page.screenshot({path:path.join(shotDir,`automatic-settings-de-${width}.png`),animations:'disabled'});
    }
    assert.deepEqual(errors, []);
    console.log('UI checks passed: automatic highlight opt-in/locking/status/pause, visual killfeed calibration, setup, update gating, reports, waveform/mix preview, killfeed review, German/English switching, persisted language, draft/user-content preservation, recording, profiles, favorites, tags/filters, queue progress/cancellation, audio test, native drag IPC, portrait crops/highlights, 1280/720px layout and video playback.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
