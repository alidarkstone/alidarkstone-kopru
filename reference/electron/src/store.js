'use strict';
// Ayarların ve güvenilen cihazların saklanması (kullanıcı veri klasöründe JSON dosyaları).
// Cihaz kimliğinin özel anahtarı, mümkünse Windows DPAPI ile (Electron safeStorage) şifreli saklanır.

const fs = require('fs');
const path = require('path');

const DEFAULTS = {
  name: '',
  clipboard: false,
  input: { enabled: false, edge: 'right' },
  downloadDir: '',
  lastPeer: '',
};

function readJson(file, fallback) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch (_) { return fallback; }
}

function writeJson(file, data) {
  const tmp = file + '.tmp';
  fs.writeFileSync(tmp, JSON.stringify(data, null, 2));
  fs.renameSync(tmp, file);
}

class Store {
  constructor(dir, safeStorage) {
    this.dir = dir;
    this.safe = safeStorage && safeStorage.isEncryptionAvailable() ? safeStorage : null;
    fs.mkdirSync(dir, { recursive: true });
    this.settingsFile = path.join(dir, 'settings.json');
    this.trustFile = path.join(dir, 'trusted.json');
    this.idFile = path.join(dir, 'identity.json');
    const s = readJson(this.settingsFile, {});
    this.settings = { ...DEFAULTS, ...s, input: { ...DEFAULTS.input, ...(s.input || {}) } };
    this.trusted = readJson(this.trustFile, []);
    if (!Array.isArray(this.trusted)) this.trusted = [];
  }

  saveSettings() { writeJson(this.settingsFile, this.settings); }

  update(patch) {
    if (typeof patch.name === 'string') this.settings.name = patch.name.trim().slice(0, 40);
    if (typeof patch.clipboard === 'boolean') this.settings.clipboard = patch.clipboard;
    if (patch.input && typeof patch.input === 'object') {
      if (typeof patch.input.enabled === 'boolean') this.settings.input.enabled = patch.input.enabled;
      if (['left', 'right', 'top', 'bottom'].includes(patch.input.edge)) this.settings.input.edge = patch.input.edge;
    }
    if (typeof patch.downloadDir === 'string') this.settings.downloadDir = patch.downloadDir;
    if (typeof patch.lastPeer === 'string') this.settings.lastPeer = patch.lastPeer;
    this.saveSettings();
  }

  // ---- kimlik ----
  loadIdentityPem() {
    const j = readJson(this.idFile, null);
    if (!j) return null;
    if (j.enc === 'safeStorage' && this.safe) return this.safe.decryptString(Buffer.from(j.data, 'base64'));
    if (j.enc === 'none') return j.data;
    return null;
  }

  saveIdentityPem(pem) {
    const data = this.safe
      ? { enc: 'safeStorage', data: this.safe.encryptString(pem).toString('base64') }
      : { enc: 'none', data: pem };
    writeJson(this.idFile, data);
  }

  // ---- güvenilen cihazlar ----
  isTrusted(id, raw) { return this.trusted.some((t) => t.id === id && t.raw === raw); }

  addTrusted(peer) {
    this.trusted = this.trusted.filter((t) => t.id !== peer.id);
    this.trusted.push({ id: peer.id, raw: peer.raw, name: peer.name, since: new Date().toISOString() });
    writeJson(this.trustFile, this.trusted);
  }

  forget(id) {
    this.trusted = this.trusted.filter((t) => t.id !== id);
    writeJson(this.trustFile, this.trusted);
  }
}

module.exports = { Store };
