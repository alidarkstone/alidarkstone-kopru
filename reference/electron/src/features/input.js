'use strict';
// Fare/klavye paylaşımı (Windows). Varsayılan olarak KAPALI.
// Yardımcı program (helper/windows/KopruInput.cs) ile stdin/stdout üzerinden JSON satırlarıyla konuşur.
//
//   Bu bilgisayar (kaynak)                     Diğer bilgisayar (hedef)
//   imleç kenara geldi  -> input.enter {edge,pos} ->  imleç karşı kenara konur, kontrol edilir
//   fare/klavye olayları -> input.ev {...}         ->  SendInput ile uygulanır
//   input.leave {pos}   <- imleç giriş kenarından geri çıktı
//   Scroll Lock / bağlantı kopması -> input.release -> hedef basılı tuşları bırakır

const { spawn } = require('child_process');
const readline = require('readline');
const { EventEmitter } = require('events');

const OPPOSITE = { left: 'right', right: 'left', top: 'bottom', bottom: 'top' };
const EVENTS = new Set(['move', 'button', 'wheel', 'key']);

function clampPos(v) { const n = Number(v); return Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0.5; }

class InputShare extends EventEmitter {
  // opts: { helperPath, send(msg), connected(), spawnFn? }
  constructor(opts) {
    super();
    this.helperPath = opts.helperPath;
    this.send = opts.send;
    this.connected = opts.connected;
    this.spawnFn = opts.spawnFn || spawn;
    this.enabled = false;
    this.edge = 'right';
    this.mode = 'local'; // local | remote (bu bilgisayarın faresi diğerinde) | controlled (diğeri bunu kontrol ediyor)
    this.proc = null;
    this.error = '';
  }

  view() { return { enabled: this.enabled, edge: this.edge, mode: this.mode, error: this.error, available: !!this.helperPath }; }
  _changed() { this.emit('change', this.view()); }

  configure({ enabled, edge }) {
    if (edge && OPPOSITE[edge]) this.edge = edge;
    if (typeof enabled === 'boolean') this.enabled = enabled;
    if (this.enabled) this._ensureHelper(); else this._stopHelper();
    this._cmd({ cmd: 'edge', edge: this.enabled && this.connected() ? this.edge : null });
    this._changed();
  }

  // Bağlantı durumu değiştiğinde çağrılır.
  onLink(connected) {
    if (!connected) this._reset();
    this._cmd({ cmd: 'edge', edge: this.enabled && connected ? this.edge : null });
  }

  _ensureHelper() {
    if (this.proc || !this.helperPath) return;
    try {
      this.proc = this.spawnFn(this.helperPath, [], { stdio: ['pipe', 'pipe', 'ignore'], windowsHide: true });
    } catch (e) {
      this.error = 'Yardımcı başlatılamadı: ' + e.message;
      this.proc = null;
      return;
    }
    this.error = '';
    const proc = this.proc;
    proc.on('error', (e) => { this.error = 'Yardımcı hatası: ' + e.message; this._changed(); });
    proc.on('exit', () => {
      if (this.proc !== proc) return;
      this.proc = null;
      this._reset();
      if (this.enabled) { this.error = 'Yardımcı kapandı'; this._changed(); }
    });
    proc.stdin.on('error', () => {});
    readline.createInterface({ input: proc.stdout }).on('line', (line) => {
      let ev;
      try { ev = JSON.parse(line); } catch (_) { return; }
      this._onHelper(ev);
    });
  }

  _stopHelper() {
    if (!this.proc) return;
    this._reset();
    const p = this.proc;
    this.proc = null;
    try { p.stdin.end(JSON.stringify({ cmd: 'quit' }) + '\n'); } catch (_) { /* yok say */ }
    setTimeout(() => { try { p.kill(); } catch (_) { /* yok say */ } }, 1500);
  }

  _cmd(obj) {
    if (this.proc && this.proc.stdin.writable) this.proc.stdin.write(JSON.stringify(obj) + '\n');
  }

  _onHelper(ev) {
    switch (ev.ev) {
      case 'edge':
        if (this.mode !== 'local' || !this.enabled || !this.connected()) return;
        this.mode = 'remote';
        this.send({ t: 'input.enter', edge: OPPOSITE[this.edge], pos: clampPos(ev.pos) });
        this._cmd({ cmd: 'capture', on: true });
        this._changed();
        break;
      case 'escape':
        if (this.mode !== 'remote') return;
        this.send({ t: 'input.release' });
        this._cmd({ cmd: 'capture', on: false });
        this.mode = 'local';
        this._changed();
        break;
      case 'leave':
        if (this.mode !== 'controlled') return;
        this._cmd({ cmd: 'leave' });
        this.mode = 'local';
        this.send({ t: 'input.leave', pos: clampPos(ev.pos) });
        this._changed();
        break;
      default:
        if (EVENTS.has(ev.ev) && this.mode === 'remote') {
          const { ev: kind, ...rest } = ev;
          this.send({ t: 'input.ev', kind, ...rest });
        }
    }
  }

  onMessage(m) {
    if (!m.t.startsWith('input.')) return false;
    switch (m.t) {
      case 'input.enter':
        if (!this.enabled || !this.proc || !OPPOSITE[m.edge]) { this.send({ t: 'input.leave', pos: clampPos(m.pos) }); break; }
        if (this.mode === 'remote') this._cmd({ cmd: 'capture', on: false });
        this.mode = 'controlled';
        this._cmd({ cmd: 'enter', edge: m.edge, pos: clampPos(m.pos) });
        this._changed();
        break;
      case 'input.ev':
        if (this.mode !== 'controlled' || !EVENTS.has(m.kind)) break;
        this._cmd(sanitizeEvent(m));
        break;
      case 'input.leave':
        if (this.mode !== 'remote') break;
        this._cmd({ cmd: 'capture', on: false, pos: clampPos(m.pos) });
        this.mode = 'local';
        this._changed();
        break;
      case 'input.release':
        if (this.mode !== 'controlled') break;
        this._cmd({ cmd: 'leave' });
        this.mode = 'local';
        this._changed();
        break;
    }
    return true;
  }

  _reset() {
    if (this.mode === 'remote') this._cmd({ cmd: 'capture', on: false });
    if (this.mode === 'controlled') this._cmd({ cmd: 'leave' });
    if (this.mode !== 'local') { this.mode = 'local'; this._changed(); }
  }

  stop() { this.enabled = false; this._stopHelper(); }
}

// Diğer bilgisayardan gelen olayı yardımcıya gönderilecek, yalnızca bilinen alanları içeren komuta çevirir.
function sanitizeEvent(m) {
  const n = (v, lim) => { const x = Math.trunc(Number(v)); return Number.isFinite(x) ? Math.max(-lim, Math.min(lim, x)) : 0; };
  switch (m.kind) {
    case 'move': return { cmd: 'move', dx: n(m.dx, 5000), dy: n(m.dy, 5000) };
    case 'button': return { cmd: 'button', b: ['left', 'right', 'middle', 'x1', 'x2'].includes(m.b) ? m.b : 'left', down: m.down === true };
    case 'wheel': return { cmd: 'wheel', d: n(m.d, 12000), h: m.h === true };
    case 'key': return { cmd: 'key', vk: n(m.vk, 254), sc: n(m.sc, 0xffff), ext: m.ext === true, down: m.down === true };
  }
  return { cmd: 'noop' };
}

module.exports = { InputShare, sanitizeEvent, OPPOSITE };
