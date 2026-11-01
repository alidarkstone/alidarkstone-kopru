'use strict';
// Dosya gönderme/alma.
//   Gönderen: file.offer {id, name, size}  ->  alıcı kullanıcı onaylarsa file.accept {id}
//   Gönderen: file.chunk {id, data(base64)} ... file.done {id, sha256}
//   Herhangi biri: file.cancel {id}
// Alıcı, kullanıcı onaylamadan diske hiçbir şey yazmaz. Dosya önce ".part" olarak yazılır,
// boyut ve SHA-256 tutarsa asıl adına taşınır. Dosya adı temizlenir (klasör dışına yazma engellenir).

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const { EventEmitter } = require('events');

const CHUNK = 256 * 1024;
const RESERVED = /^(con|prn|aux|nul|com[0-9¹²³]|lpt[0-9¹²³]|conin\$|conout\$) *(\..*)?$/i;

function safeName(name) {
  let n = path.basename(String(name || '').replace(/\\/g, '/'));
  n = n.replace(/[\u0000-\u001f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069<>:"/\\|?*]/g, '_').replace(/[. ]+$/, '').trim();
  if (!n || n === '.' || n === '..') n = 'dosya';
  if (RESERVED.test(n)) n = '_' + n;
  if (n.length > 200) {
    const ext = path.extname(n).slice(0, 20);
    n = n.slice(0, 200 - ext.length) + ext;
  }
  return n;
}

function uniquePath(dir, name) {
  const ext = path.extname(name);
  const base = name.slice(0, name.length - ext.length);
  let p = path.join(dir, name);
  for (let i = 2; fs.existsSync(p) || fs.existsSync(p + '.part'); i++) p = path.join(dir, `${base} (${i})${ext}`);
  return p;
}

class FileShare extends EventEmitter {
  // opts: { send(msg) -> bool, waitDrain() -> Promise, downloadDir() -> string }
  constructor(opts) {
    super();
    this.sendMsg = opts.send;
    this.waitDrain = opts.waitDrain;
    this.downloadDir = opts.downloadDir;
    this.outgoing = new Map(); // id -> { path, name, size, sent, cancelled }
    this.incoming = new Map(); // id -> { name, size, got, hash, stream, part, final, accepted }
  }

  _emit() { this.emit('change', this.list()); }

  list() {
    const out = [...this.outgoing.entries()].map(([id, o]) => ({ id, dir: 'out', name: o.name, size: o.size, done: o.sent, status: o.status }));
    const inc = [...this.incoming.entries()].map(([id, i]) => ({ id, dir: 'in', name: i.name, size: i.size, done: i.got, status: i.status, path: i.final || '' }));
    return out.concat(inc);
  }

  // ---- gönderen ----
  async offer(filePath) {
    const st = await fs.promises.stat(filePath);
    if (!st.isFile()) throw new Error('yalnızca dosya gönderilebilir');
    const id = crypto.randomBytes(8).toString('hex');
    const name = path.basename(filePath);
    this.outgoing.set(id, { path: filePath, name, size: st.size, sent: 0, status: 'waiting' });
    this.sendMsg({ t: 'file.offer', id, name, size: st.size });
    this._emit();
    return id;
  }

  async _stream(id) {
    const o = this.outgoing.get(id);
    if (!o || o.status !== 'waiting') return;
    o.status = 'sending';
    this._emit();
    const hash = crypto.createHash('sha256');
    try {
      const rs = fs.createReadStream(o.path, { highWaterMark: CHUNK });
      for await (const chunk of rs) {
        if (o.status !== 'sending') { rs.destroy(); return; }
        hash.update(chunk);
        o.sent += chunk.length;
        if (!this.sendMsg({ t: 'file.chunk', id, data: chunk.toString('base64') })) await this.waitDrain();
        this._emit();
      }
      if (o.sent !== o.size) throw new Error('dosya gönderim sırasında değişti');
      this.sendMsg({ t: 'file.done', id, sha256: hash.digest('hex') });
      o.status = 'sent';
    } catch (e) {
      o.status = 'error';
      this.sendMsg({ t: 'file.cancel', id });
    }
    this._emit();
  }

  // ---- alıcı ----
  accept(id) {
    const i = this.incoming.get(id);
    if (!i || i.status !== 'offered') return;
    const dir = this.downloadDir();
    fs.mkdirSync(dir, { recursive: true });
    i.final = uniquePath(dir, i.name);
    i.part = i.final + '.part';
    i.stream = fs.createWriteStream(i.part, { flags: 'wx' });
    i.stream.on('error', () => this.cancel(id));
    i.hash = crypto.createHash('sha256');
    i.status = 'receiving';
    this.sendMsg({ t: 'file.accept', id });
    this._emit();
  }

  reject(id) {
    const i = this.incoming.get(id);
    if (!i || i.status !== 'offered') return;
    i.status = 'rejected';
    this.sendMsg({ t: 'file.cancel', id });
    this._emit();
  }

  cancel(id) {
    const o = this.outgoing.get(id);
    if (o && (o.status === 'waiting' || o.status === 'sending')) o.status = 'cancelled';
    const i = this.incoming.get(id);
    if (i && (i.status === 'offered' || i.status === 'receiving')) this._dropIncoming(i, 'cancelled');
    this.sendMsg({ t: 'file.cancel', id });
    this._emit();
  }

  _dropIncoming(i, status) {
    i.status = status;
    if (i.stream) {
      const part = i.part;
      // Dosya henüz açılıyor olabilir: akış tamamen kapanınca sil.
      i.stream.once('close', () => fs.promises.unlink(part).catch(() => {}));
      i.stream.destroy();
      i.stream = null;
    }
    i.final = '';
  }

  // Bağlantı koptuğunda yarım kalanları iptal et.
  abortAll() {
    for (const o of this.outgoing.values()) if (o.status === 'waiting' || o.status === 'sending') o.status = 'cancelled';
    for (const i of this.incoming.values()) if (i.status === 'offered' || i.status === 'receiving') this._dropIncoming(i, 'cancelled');
    this._emit();
  }

  onMessage(m) {
    if (!m.t.startsWith('file.')) return false;
    const id = typeof m.id === 'string' && /^[0-9a-f]{16}$/.test(m.id) ? m.id : null;
    if (!id) return true;
    switch (m.t) {
      case 'file.offer': {
        const size = Number(m.size);
        if (this.incoming.has(id) || !Number.isSafeInteger(size) || size < 0) return true;
        this.incoming.set(id, { name: safeName(m.name), size, got: 0, status: 'offered' });
        this._emit();
        this.emit('offer', { id, name: safeName(m.name), size });
        break;
      }
      case 'file.accept': this._stream(id); break;
      case 'file.chunk': {
        const i = this.incoming.get(id);
        if (!i || i.status !== 'receiving' || typeof m.data !== 'string') return true;
        const buf = Buffer.from(m.data, 'base64');
        i.got += buf.length;
        if (i.got > i.size) { this.cancel(id); return true; }
        i.hash.update(buf);
        i.stream.write(buf);
        this._emitThrottled();
        break;
      }
      case 'file.done': {
        const i = this.incoming.get(id);
        if (!i || i.status !== 'receiving') return true;
        const ok = i.got === i.size && i.hash.digest('hex') === m.sha256;
        if (!ok) { this._dropIncoming(i, 'error'); this._emit(); return true; }
        const { part, final } = i;
        i.stream.end(() => {
          fs.promises.rename(part, final).then(() => {
            i.status = 'received';
            this._emit();
            this.emit('received', { id, name: path.basename(final), path: final });
          }, () => { i.status = 'error'; this._emit(); });
        });
        i.stream = null;
        break;
      }
      case 'file.cancel': {
        const o = this.outgoing.get(id);
        if (o && (o.status === 'waiting' || o.status === 'sending')) o.status = 'cancelled';
        const i = this.incoming.get(id);
        if (i && (i.status === 'offered' || i.status === 'receiving')) this._dropIncoming(i, 'cancelled');
        this._emit();
        break;
      }
    }
    return true;
  }

  _emitThrottled() {
    if (this._tt) return;
    this._tt = setTimeout(() => { this._tt = null; this._emit(); }, 200);
  }
}

module.exports = { FileShare, safeName };
