'use strict';
// Köprü: Electron ana süreci. Tüm parçaları birbirine bağlar.

const { app, BrowserWindow, ipcMain, dialog, clipboard, shell, Notification, safeStorage, Menu } = require('electron');
const fs = require('fs');
const os = require('os');
const path = require('path');

const { Store } = require('./store');
const { loadIdentity } = require('./core/keys');
const { Link } = require('./core/link');
const { Discovery } = require('./core/discovery');
const { ClipboardShare } = require('./features/clipboard');
const { FileShare } = require('./features/files');
const { InputShare } = require('./features/input');

if (!app.requestSingleInstanceLock()) { app.quit(); process.exit(0); }

let win = null;
let store, identity, link, discovery, clip, files, input;

function deviceName() { return store.settings.name || os.hostname(); }

function helperPath() {
  if (process.platform !== 'win32') return null;
  const base = app.isPackaged ? process.resourcesPath : path.join(__dirname, '..');
  const p = path.join(base, 'helper', 'windows', 'bin', 'KopruInput.exe');
  return fs.existsSync(p) ? p : null;
}

function downloadDir() {
  return store.settings.downloadDir || path.join(app.getPath('downloads'), 'Köprü');
}

function state() {
  return {
    me: { id: identity.id, name: deviceName(), port: link.port },
    link: link.snapshot(),
    devices: discovery.list().map((d) => ({ ...d, trusted: store.trusted.some((t) => t.id === d.id) })),
    trusted: store.trusted.map(({ id, name, since }) => ({ id, name, since })),
    settings: { ...store.settings, downloadDir: downloadDir() },
    input: input.view(),
    transfers: files.list(),
    platform: process.platform,
  };
}

let pushTimer = null;
function push() {
  if (pushTimer) return;
  pushTimer = setTimeout(() => {
    pushTimer = null;
    if (win && !win.isDestroyed()) win.webContents.send('state', state());
  }, 50);
}

function notify(title, body) {
  if (Notification.isSupported()) new Notification({ title, body, silent: false }).show();
}

function waitDrain() {
  return new Promise((resolve) => {
    if (!link.needDrain) return resolve();
    const done = () => { link.removeListener('drain', done); link.removeListener('state', onState); resolve(); };
    const onState = (s) => { if (s.state !== 'connected') done(); };
    link.on('drain', done);
    link.on('state', onState);
  });
}

function setup() {
  store = new Store(app.getPath('userData'), safeStorage);
  let pem = null;
  try { pem = store.loadIdentityPem(); } catch (_) { pem = null; }
  identity = loadIdentity(pem || undefined);
  if (!pem) store.saveIdentityPem(identity.pem);

  link = new Link({
    identity,
    getName: deviceName,
    trust: { isTrusted: (id, raw) => store.isTrusted(id, raw), add: (p) => store.addTrusted(p) },
  });

  const send = (m) => link.send(m);
  clip = new ClipboardShare({ clipboard, send, enabled: () => store.settings.clipboard && link.state === 'connected' });
  files = new FileShare({ send, waitDrain, downloadDir });
  input = new InputShare({ helperPath: helperPath(), send, connected: () => link.state === 'connected' });

  link.on('state', (s) => {
    if (s.state === 'connected') {
      store.update({ lastPeer: s.peer.id });
    }
    if (s.state === 'pairing' && win) { win.show(); win.focus(); }
    if (s.state === 'idle') files.abortAll();
    input.onLink(s.state === 'connected');
    push();
  });
  link.on('message', (m) => {
    if (clip.onMessage(m) || files.onMessage(m) || input.onMessage(m)) return;
  });
  link.on('error', () => push());

  files.on('change', push);
  files.on('offer', (o) => notify('Dosya geliyor', `${o.name} (${Math.ceil(o.size / 1024)} KB). Kabul etmek için Köprü'yü açın.`));
  files.on('received', (r) => notify('Dosya alındı', r.name));
  input.on('change', push);

  // listen() bitmeden state() çağrılırsa diye geçici boş liste
  discovery = { list: () => [], setName: () => {}, stop: () => {} };
  link.listen().then((port) => {
    discovery = new Discovery({ id: identity.id, name: deviceName(), port });
    discovery.on('change', () => { push(); autoConnect(); });
    discovery.on('error', () => {});
    discovery.start();
    push();
  }, (e) => {
    dialog.showErrorBox('Köprü', 'Bağlantı noktası açılamadı: ' + e.message);
  });
  setInterval(autoConnect, 5000);

  clip.start();
  input.configure(store.settings.input);
}

// Son bağlanılan güvenilir cihaz ağda görünürse otomatik bağlan.
// Aynı anda iki taraf birden bağlanmaya çalışmasın diye yalnızca kimliği küçük olan taraf bağlanır.
function autoConnect() {
  if (link.state !== 'idle') return;
  const peer = discovery.list().find((d) => d.id === store.settings.lastPeer && store.trusted.some((t) => t.id === d.id));
  if (peer && identity.id < peer.id) link.connect(peer.address, peer.port);
}

function createWindow() {
  win = new BrowserWindow({
    width: 980,
    height: 700,
    minWidth: 760,
    minHeight: 540,
    title: 'Köprü',
    backgroundColor: '#101216',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      spellcheck: false,
    },
  });
  Menu.setApplicationMenu(null);
  win.loadFile(path.join(__dirname, 'ui', 'index.html'));
  // Uygulama içinde başka sayfaya gidilmesin, yeni pencere açılmasın.
  win.webContents.on('will-navigate', (e) => e.preventDefault());
  win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  win.on('closed', () => { win = null; });
}

function handlers() {
  ipcMain.handle('state', () => state());
  ipcMain.handle('connect', (_e, id) => {
    const d = discovery.list().find((x) => x.id === id);
    if (d) link.connect(d.address, d.port);
  });
  ipcMain.handle('connectAddress', (_e, addr) => {
    const m = /^\s*([0-9a-zA-Z.\-:]+?)(?::(\d{1,5}))?\s*$/.exec(String(addr || ''));
    if (m) link.connect(m[1], m[2] ? Number(m[2]) : undefined);
  });
  ipcMain.handle('disconnect', () => link.disconnect());
  ipcMain.handle('confirmPairing', (_e, ok) => link.confirmPairing(ok === true));
  ipcMain.handle('forget', (_e, id) => {
    store.forget(String(id));
    if (link.peer && link.peer.peerId === id) link.disconnect();
    push();
  });
  ipcMain.handle('settings', (_e, patch) => {
    store.update(patch || {});
    if (typeof (patch || {}).name === 'string') discovery.setName(deviceName());
    if (patch && patch.input) input.configure(store.settings.input);
    push();
  });
  ipcMain.handle('chooseDownloadDir', async () => {
    const r = await dialog.showOpenDialog(win, { properties: ['openDirectory', 'createDirectory'], defaultPath: downloadDir() });
    if (!r.canceled && r.filePaths[0]) { store.update({ downloadDir: r.filePaths[0] }); push(); }
  });
  ipcMain.handle('pickFiles', async () => {
    if (link.state !== 'connected') return;
    const r = await dialog.showOpenDialog(win, { properties: ['openFile', 'multiSelections'] });
    if (!r.canceled) for (const f of r.filePaths) await files.offer(f).catch(() => {});
  });
  ipcMain.handle('sendPaths', async (_e, paths) => {
    if (link.state !== 'connected' || !Array.isArray(paths)) return;
    for (const f of paths.slice(0, 50)) if (typeof f === 'string') await files.offer(f).catch(() => {});
  });
  ipcMain.handle('fileAccept', (_e, id) => files.accept(String(id)));
  ipcMain.handle('fileReject', (_e, id) => files.reject(String(id)));
  ipcMain.handle('fileCancel', (_e, id) => files.cancel(String(id)));
  ipcMain.handle('fileShow', (_e, id) => {
    const t = files.list().find((x) => x.id === id && x.dir === 'in' && x.status === 'received');
    if (t && t.path) shell.showItemInFolder(t.path);
  });
  ipcMain.handle('openDownloads', () => {
    fs.mkdirSync(downloadDir(), { recursive: true });
    shell.openPath(downloadDir());
  });
}

app.on('second-instance', () => { if (win) { if (win.isMinimized()) win.restore(); win.show(); win.focus(); } });
app.whenReady().then(() => {
  setup();
  handlers();
  createWindow();
});
app.on('window-all-closed', () => app.quit());
app.on('before-quit', () => {
  try { input.stop(); clip.stop(); discovery.stop(); link.close(); } catch (_) { /* yok say */ }
});
