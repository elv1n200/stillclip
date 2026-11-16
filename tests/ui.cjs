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
      if (!['index.html', 'app.css', 'app.js'].includes(name)) return route.abort();
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
      assert.equal(await page.locator('#trim-label').textContent(), 'Für Discord speichern');
      await page.click('#trim-clip');
      const message = await page.evaluate(() => window.messages.at(-1));
      assert.deepEqual(message, { action: 'trim', id: 1, start: 0, end: 3, mode });
    }
    // Invalid selections and an ongoing export still block duplicate jobs.
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
      action: 'highlights', mode: 'discord_nitro', pieces: [{ id: 1, start: 1, end: 3 }, { id: 1, start: 0, end: 3 }]
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
    assert.deepEqual(errors, []);
    console.log('UI checks passed: recording during export, settings locks, game profiles, capture/audio selection, diagnostics, favorites, highlight ordering/export, 1280/720px layout and real video playback.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
