'use strict';
// Pano paylaşımı (yalnızca metin). Varsayılan olarak KAPALI; kullanıcı açar.
// Panoyu belirli aralıklarla okur, değiştiyse eşe gönderir. Eşten geleni panoya yazar.
// Kendi yazdığımız metni tekrar geri göndermemek için son değeri hatırlar.

const POLL = 600;
const MAX_TEXT = 1024 * 1024; // 1 MB metin üst sınırı

class ClipboardShare {
  // opts: { clipboard: { readText(), writeText(s) }, send(msg), enabled() }
  constructor(opts) {
    this.clipboard = opts.clipboard;
    this.send = opts.send;
    this.enabled = opts.enabled;
    this.last = null;
  }

  start() {
    this.last = this._read();
    this.timer = setInterval(() => this.tick(), POLL);
  }

  stop() { clearInterval(this.timer); }

  _read() {
    try { return this.clipboard.readText() || ''; } catch (_) { return ''; }
  }

  tick() {
    const now = this._read();
    if (now === this.last) return;
    this.last = now;
    if (!this.enabled() || !now || now.length > MAX_TEXT) return;
    this.send({ t: 'clip.text', text: now });
  }

  onMessage(m) {
    if (m.t !== 'clip.text' || !this.enabled()) return false;
    if (typeof m.text !== 'string' || m.text.length > MAX_TEXT) return true;
    this.last = m.text;
    try { this.clipboard.writeText(m.text); } catch (_) { /* yok say */ }
    return true;
  }
}

module.exports = { ClipboardShare };
