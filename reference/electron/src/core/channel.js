'use strict';
// Şifreli kanal. Bir TCP soketini sarar.
//
// El sıkışma (her iki taraf aynı adımları izler):
//   1) HELLO   (açık metin): uygulama, sürüm, cihaz adı, kalıcı Ed25519 kimliği, geçici X25519 anahtarı, rastgele nonce
//   2) AUTH    (açık metin): Ed25519 imzası( transkript || rol )  -> karşı taraf kimliğinin sahibi olduğunu kanıtlar
//   3) Sonrası: AES-256-GCM ile şifreli JSON mesajları. Her yön kendi anahtarına ve sayaç nonce'una sahiptir,
//      bu yüzden tekrar oynatma, sıra değiştirme ve değiştirilmiş paketler reddedilir.
//
// Transkript = SHA-256(etiket || istemci HELLO baytları || sunucu HELLO baytları).
// Eşleştirme kodu (SAS) transkriptten ve ortak sırdan türetilir. Araya giren biri (MITM) olursa
// iki ekranda farklı kodlar çıkar; kullanıcılar kodların aynı olduğunu görüp onaylar.

const crypto = require('crypto');
const { EventEmitter } = require('events');
const { encodeFrame, FrameReader } = require('./framing');
const { rawPublic, publicFromRaw, fingerprint, hkdf } = require('./keys');

const APP = 'kopru';
const VERSION = 1;
const LABEL = Buffer.from('kopru-handshake-v1');
const HANDSHAKE_TIMEOUT = 10000;

function b64(buf) { return buf.toString('base64url'); }
function unb64(s) {
  if (typeof s !== 'string' || !/^[A-Za-z0-9_-]+$/.test(s)) throw new Error('geçersiz base64');
  return Buffer.from(s, 'base64url');
}

class SecureChannel extends EventEmitter {
  // opts: { identity (keys.loadIdentity), name, role: 'client' | 'server' }
  constructor(socket, opts) {
    super();
    this.socket = socket;
    this.identity = opts.identity;
    this.role = opts.role;
    this.name = String(opts.name || 'Bilgisayar').slice(0, 64);
    this.state = 'hello';
    this.closed = false;
    this.sendSeq = 0n;
    this.recvSeq = 0n;
    this.eph = crypto.generateKeyPairSync('x25519');
    this.reader = new FrameReader((f) => this._onFrame(f), (e) => this.close(e));

    socket.setNoDelay(true);
    socket.on('data', (d) => this.reader.push(d));
    socket.on('error', (e) => this.close(e));
    socket.on('close', () => this.close());
    socket.on('drain', () => this.emit('drain'));
    this.timer = setTimeout(() => this.close(new Error('el sıkışma zaman aşımı')), HANDSHAKE_TIMEOUT);

    const hello = {
      t: 'hello', app: APP, v: VERSION, role: this.role, name: this.name,
      id: b64(this.identity.raw), eph: b64(rawPublic(this.eph.publicKey)),
      nonce: b64(crypto.randomBytes(16)),
    };
    this.myHello = Buffer.from(JSON.stringify(hello));
    this._writeRaw(this.myHello);
  }

  _writeRaw(buf) {
    if (this.closed) return false;
    return this.socket.write(encodeFrame(buf));
  }

  _onFrame(frame) {
    if (this.closed) return;
    try {
      if (this.state === 'hello') return this._onHello(frame);
      if (this.state === 'auth') return this._onAuth(frame);
      return this._onSecure(frame);
    } catch (e) {
      this.close(e);
    }
  }

  _onHello(frame) {
    const m = JSON.parse(frame.toString('utf8'));
    if (m.t !== 'hello' || m.app !== APP) throw new Error('bu bir Köprü cihazı değil');
    if (m.v !== VERSION) throw new Error('uyumsuz sürüm: ' + m.v);
    const expectRole = this.role === 'client' ? 'server' : 'client';
    if (m.role !== expectRole) throw new Error('rol hatası');
    this.peerRaw = unb64(m.id);
    this.peerKey = publicFromRaw(this.peerRaw, 'Ed25519');
    const peerEph = publicFromRaw(unb64(m.eph), 'X25519');
    unb64(m.nonce);
    if (this.peerRaw.equals(this.identity.raw)) throw new Error('kendine bağlanılamaz');
    this.peerName = String(m.name || 'Bilinmeyen').replace(/[\u0000-\u001f]/g, '').slice(0, 64);

    const clientHello = this.role === 'client' ? this.myHello : frame;
    const serverHello = this.role === 'client' ? frame : this.myHello;
    this.transcript = crypto.createHash('sha256').update(LABEL).update(clientHello).update(serverHello).digest();

    const shared = crypto.diffieHellman({ privateKey: this.eph.privateKey, publicKey: peerEph });
    if (shared.equals(Buffer.alloc(shared.length))) throw new Error('geçersiz anahtar değişimi');
    const c2s = hkdf(shared, this.transcript, 'kopru c2s');
    const s2c = hkdf(shared, this.transcript, 'kopru s2c');
    this.sendKey = this.role === 'client' ? c2s : s2c;
    this.recvKey = this.role === 'client' ? s2c : c2s;
    const sasNum = hkdf(shared, this.transcript, 'kopru sas', 4).readUInt32BE(0) % 1000000;
    this.sas = String(sasNum).padStart(6, '0');
    this.eph = null; // geçici anahtar artık gerekmiyor

    const sig = crypto.sign(null, Buffer.concat([this.transcript, Buffer.from(this.role)]), this.identity.privateKey);
    this._writeRaw(Buffer.from(JSON.stringify({ t: 'auth', sig: b64(sig) })));
    this.state = 'auth';
  }

  _onAuth(frame) {
    const m = JSON.parse(frame.toString('utf8'));
    if (m.t !== 'auth') throw new Error('kimlik doğrulama bekleniyordu');
    const peerRole = this.role === 'client' ? 'server' : 'client';
    const ok = crypto.verify(null, Buffer.concat([this.transcript, Buffer.from(peerRole)]), this.peerKey, unb64(m.sig));
    if (!ok) throw new Error('karşı tarafın imzası geçersiz');
    this.state = 'secure';
    clearTimeout(this.timer);
    this.emit('ready', {
      peerId: fingerprint(this.peerRaw),
      peerRaw: b64(this.peerRaw),
      peerName: this.peerName,
      sas: this.sas,
      address: this.socket.remoteAddress,
    });
  }

  _nonce(seq) {
    const n = Buffer.alloc(12);
    n.writeBigUInt64BE(seq, 4);
    return n;
  }

  _onSecure(frame) {
    if (frame.length < 16) throw new Error('kısa paket');
    const d = crypto.createDecipheriv('aes-256-gcm', this.recvKey, this._nonce(this.recvSeq));
    d.setAuthTag(frame.subarray(frame.length - 16));
    const plain = Buffer.concat([d.update(frame.subarray(0, frame.length - 16)), d.final()]);
    this.recvSeq += 1n;
    const msg = JSON.parse(plain.toString('utf8'));
    if (!msg || typeof msg.t !== 'string') throw new Error('geçersiz mesaj');
    this.emit('message', msg);
  }

  // Şifreli bir JSON mesajı gönderir. Soket tamponu doluysa false döner ('drain' olayını bekleyin).
  send(msg) {
    if (this.state !== 'secure' || this.closed) return false;
    const c = crypto.createCipheriv('aes-256-gcm', this.sendKey, this._nonce(this.sendSeq));
    this.sendSeq += 1n;
    const body = Buffer.concat([c.update(Buffer.from(JSON.stringify(msg))), c.final(), c.getAuthTag()]);
    return this._writeRaw(body);
  }

  get writableNeedDrain() { return this.socket.writableNeedDrain; }

  close(err) {
    if (this.closed) return;
    this.closed = true;
    clearTimeout(this.timer);
    this.sendKey = this.recvKey = null;
    this.socket.destroy();
    this.emit('close', err || null);
  }
}

module.exports = { SecureChannel };
