// Render the real UI with synthetic demo media; never capture a screen or microphone.
const { chromium } = require('playwright');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const { execFileSync } = require('node:child_process');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '..');
const out = path.join(root, 'docs/images');
const palettes = [['#e4a0b7','#594052'], ['#99cabd','#34514d'], ['#b4addc','#45405e'], ['#dab88c','#564535'], ['#8baed2','#33465c'], ['#e4a0b7','#594052']];
const scene = index => {
  const [accent, shadow] = palettes[index];
  const offset = index * 9;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="960" height="540" viewBox="0 0 960 540">
    <defs><linearGradient id="sky" x2="0" y2="1"><stop stop-color="#0b0d16"/><stop offset="1" stop-color="${shadow}"/></linearGradient></defs>
    <path fill="url(#sky)" d="M0 0h960v540H0z"/>
    <g stroke="${accent}" fill="none" opacity=".28">${Array.from({length:13},(_,i)=>`<path d="M${i*120-240} 540 480 218"/>`).join('')}${[240,270,310,360,420,495].map(y=>`<path d="M0 ${y}h960"/>`).join('')}</g>
    <path d="M100 285V130h155v155M705 285V130h155v155" fill="#10131b" stroke="${accent}" opacity=".6"/>
    <path d="m${329+offset} 143 151-74 151 74v172l-151 74-151-74z" fill="${shadow}" stroke="${accent}" stroke-width="2"/>
    <path d="m${329+offset} 143 151 74 151-74m-151 74v172" fill="none" stroke="${accent}" stroke-width="2"/>
    <path d="m${389+offset} 174 91 44 91-44v106l-91 44-91-44z" fill="#15121b" stroke="${accent}"/>
    <path d="M36 36h25m-25 0v25m888-25h-25m25 0v25M36 504h25m-25 0v-25m888 25h-25m25 0v-25" fill="none" stroke="${accent}" stroke-width="2"/>
    <text x="62" y="80" fill="${accent}" font-family="monospace" font-size="18" letter-spacing="4">SYNTHETIC / ${String(index+1).padStart(2,'0')}</text>
    <text x="62" y="467" fill="#d7d2dc" font-family="monospace" font-size="16" letter-spacing="2">STILLCLIP DEMO</text>
  </svg>`;
};

(async () => {
  await fs.mkdir(out, { recursive: true });
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'stillclip-showcase-'));
  const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
  try {
    const art = await browser.newPage({ viewport: { width: 960, height: 540 }, deviceScaleFactor: 1 });
    await art.setContent(`<style>body{margin:0}</style>${scene(0)}`);
    const png = path.join(temp, 'demo.png'); await art.screenshot({ path: png }); await art.close();
    const videoPath = path.join(temp, 'demo.webm');
    execFileSync(process.env.STILLCLIP_FFMPEG || 'ffmpeg', ['-hide_banner','-v','error','-y','-loop','1','-i',png,'-t','12','-r','24','-c:v','libvpx-vp9','-crf','36','-b:v','0','-threads','2','-an',videoPath]);
    const video = await fs.readFile(videoPath);
    const page = await browser.newPage({ viewport: { width: 1140, height: 1040 }, deviceScaleFactor: 1 });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.route('http://app.localhost/**', route => {
      const file = new URL(route.request().url()).pathname.slice(1) || 'index.html';
      if (!['index.html','app.css','i18n.js','app.js'].includes(file)) return route.abort();
      return route.fulfill({ path: path.join(root,'ui',file), contentType: file.endsWith('.html')?'text/html':file.endsWith('.css')?'text/css':'text/javascript' });
    });
    await page.route('http://clip.localhost/**', route => {
      const url = new URL(route.request().url());
      if (url.pathname.startsWith('/thumb/')) return route.fulfill({ body: scene(Number(url.pathname.split('/').pop())-1), contentType: 'image/svg+xml' });
      const match = /^bytes=(\d+)-(\d*)$/.exec(route.request().headers().range || '');
      const start = match ? Number(match[1]) : 0;
      const end = match?.[2] ? Math.min(Number(match[2]),video.length-1) : video.length-1;
      return route.fulfill({ body: video.subarray(start,end+1), contentType:'video/webm', status:match?206:200,
        headers:{'Access-Control-Allow-Origin':'http://app.localhost','Accept-Ranges':'bytes',...(match?{'Content-Range':`bytes ${start}-${end}/${video.length}`}:{})} });
    });
    await page.addInitScript(() => { window.ipc = { postMessage: () => {} }; });
    await page.goto('http://app.localhost/');
    const version = /^version = "([^"]+)"/m.exec(await fs.readFile(path.join(root,'Cargo.toml'),'utf8'))[1];
    const names = ['Neon — Highlight','Orbit — Round 02','Grid — Favorite','Shift — Replay','Afterglow — Selection','Neon — Portrait'];
    const clips = names.map((name,i) => ({ id:i+1,name,src:`http://clip.localhost/${i+1}`,thumb:`http://clip.localhost/thumb/${i+1}`,
      path:`C:\\Stillclip\\clips\\Demo-${i+1}.webm`,duration:12,width:960,height:540,audio:false,bytes:842000+i*124000,
      extension:'webm',created:1794823200-i*3600,game:'Demo',tags:i%2?['Replay']:['Highlight'],favorite:i===0||i===2,markers:[3,8],pending:false }));
    await page.evaluate(({version,clips}) => window.setState({version,clips,ffmpeg_available:true,ffmpeg_path:'C:\\Tools\\ffmpeg.exe',clips_dir:'C:\\Stillclip\\clips',
      settings:{language:'en',replay_seconds:30,quality:'p1080',fps:60,microphone:true},
      displays:[{name:'DISPLAY1',label:'Main display · 1920×1080',width:1920,height:1080,primary:true}],
      capabilities:{probed:true,ddagrab:true,gfxcapture:true,encoders:['nvenc']},encoders:[{id:'nvenc',label:'NVIDIA NVENC',available:true}],selected_encoder:'NVIDIA NVENC',
      microphones:[{id:'demo-mic',name:'Default microphone'}],default_output:'Default output',free_bytes:240*1024**3,clips_bytes:8*1024**2,
      builtin_games:[{name:'VALORANT',exe:'valorant-win64-shipping.exe'},{name:'Counter-Strike 2',exe:'cs2.exe'}]
    }),{version,clips});
    await page.locator('.thumbnail img').last().waitFor();
    await page.evaluate(() => Promise.all([...document.images].map(img=>img.decode().catch(()=>{}))));
    assert.equal(await page.locator('.clip-card').count(),6);
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:path.join(out,'library.png'),animations:'disabled',fullPage:true});
    await page.locator('.clip-card').first().click();
    await page.waitForFunction(()=>document.getElementById('editor-video').readyState>=2);
    await page.selectOption('#crop-format','720');
    await page.fill('#crop-x','60'); await page.locator('#crop-x').dispatchEvent('input');
    await page.fill('#trim-start','2'); await page.locator('#trim-start').dispatchEvent('change');
    await page.fill('#trim-end','10'); await page.locator('#trim-end').dispatchEvent('change');
    await page.evaluate(()=>document.getElementById('editor-video').currentTime=4);
    await page.waitForFunction(()=>Math.abs(document.getElementById('editor-video').currentTime-4)<.1);
    await page.evaluate(()=>document.querySelector('.editor').scrollTop=0);
    const editor = await page.locator('.editor').boundingBox();
    const transport = await page.locator('.transport').boundingBox();
    await page.screenshot({path:path.join(out,'editor.png'),animations:'disabled',
      clip:{x:editor.x,y:editor.y,width:editor.width,height:transport.y+transport.height-editor.y+12}});
    await page.click('#highlight-add');
    await page.fill('#trim-start','5'); await page.locator('#trim-start').dispatchEvent('change');
    await page.click('#highlight-add');
    await page.selectOption('#highlight-mode','discord_nitro'); await page.selectOption('#highlight-format','720');
    await page.evaluate(()=>window.setState({exports:[{id:1,label:'Neon — Highlight · Discord',running:true,percent:62,phase:'Discord: Compressing video',eta_seconds:14,cancelling:false},{id:2,label:'Highlights · 2 excerpts',running:false,percent:0,phase:'Queued',cancelling:false}]}));
    await page.locator('#editor-export-list').scrollIntoViewIfNeeded();
    await page.locator('.editor').screenshot({path:path.join(out,'exports.png'),animations:'disabled'});
    assert.deepEqual(errors,[]);
    console.log('Documentation screenshots rendered from the current UI with synthetic demo media.');
  } finally { await browser.close(); await fs.rm(temp,{recursive:true,force:true}); }
})().catch(error=>{console.error(error);process.exitCode=1;});
