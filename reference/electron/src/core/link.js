'use strict';
// Bağlantı yöneticisi: tek bir eş cihazla oturum.
//   - Gelen bağlantıları dinler (TCP), istenirse bir cihaza bağlanır.
//   - Güvenilen (daha önce eşleşmiş) cihaz doğrudan bağlanır.
//   - Yeni cihazda iki ekranda 6 haneli kod gösterilir; İKİ taraf da onaylarsa güvenilir listesine eklenir.
//   - Eşleşme tamamlanmadan hiçbir özellik mesajı işlenmez.

const net = require('net');
const { EventEmitter } = require('events');
const { SecureChannel } = require('./channel');

const DEFAULT_PORT = 47910;
const PING_EVERY = 5000;
const DEAD_AFTER = 20000;
const PAIR_TIMEOUT = 90000;

class Link extends EventEmitter {
  // opts: { identity, getName(), trust: { isTrusted(peerId, peerRaw), add(peer) }, port? }
  constructor(opts) {
    super();
    this.identity = opts.identity;
    this.getName = opts.getName;
    this.trust = opts.trust;
    this.port = opts.port === undefined ? DEFAULT_PORT : opts.port;
    this.channel = null;
    this.peer = null;
    this.state = 'idle'; // idle | connecting | checking | pairing | connected
    this.pair = null;    // { sas, mine: bool|null, theirs: bool|null }
  }

  listen() {
    return new Promise((resolve, reject) => {
      this.server = net.createServer((sock) => this._incoming(sock));
      this.server.once('error', reject);
      this.server.listen(this.port, '0.0.0.0', () => {
        this.port = this.server.address().port;
        this.server.removeListener('error', reject);
        this.server.on('error', (e) => this.emit('error', e));
        resolve(this.port);
      });
    });
  }

  _incoming(sock) {
    if (this.channel) { sock.destroy(); return; } // zaten bir oturum var
    this._attach(sock, 'server');
  }

  connect(address, port) {
    if (this.channel) this.disconnect();
    this._setState('connecting');
    const sock = net.connect({ host: address, port: port || DEFAULT_PORT });
    sock.setTimeout(8000, () => sock.destroy(new Error('bağlantı zaman aşımı')));
    sock.once('connect', () => sock.setTimeout(0));
    this._attach(sock, 'client');
  }

  _attach(sock, role) {
    const ch = new SecureChannel(sock, { identity: this.identity, name: this.getName(), role });
    this.channel = ch;
    ch.on('ready', (info) => this._onReady(ch, info));
    ch.on('message', (m) => this._onMessage(ch, m));
    ch.on('drain', () => this.emit('drain'));
    ch.on('close', (err) => this._onClose(ch, err));
  }

  _onReady(ch, info) {
    if (ch !== this.channel) return;
    this.peer = info;
    this.lastSeen = Date.now();
    this.pingTimer = setInterval(() => this._ping(), PING_EVERY);
    // Her iki taraf da birbirini tanıyorsa doğrudan bağlanır; biri bile tanımıyorsa ikisi de kod onayına geçer.
    this.known = this.trust.isTrusted(info.peerId, info.peerRaw);
    this.theyKnow = null;
    ch.send({ t: 'trust', known: this.known });
    this._setState('checking');
  }

  _decide() {
    if (this.known && this.theyKnow) return this._setState('connected');
    this.pair = { sas: this.peer.sas, mine: null, theirs: null };
    this.pairTimer = setTimeout(() => this.disconnect('eşleştirme zaman aşımı'), PAIR_TIMEOUT);
    this._setState('pairing');
  }

  // Kullanıcı ekrandaki kodu onayladı (true) veya reddetti (false).
  confirmPairing(ok) {
    if (this.state !== 'pairing' || !this.pair) return;
    this.pair.mine = !!ok;
    this.channel.send({ t: ok ? 'pair.ok' : 'pair.no' });
    if (!ok) return this.disconnect('eşleştirme reddedildi');
    this._checkPaired();
  }

  _checkPaired() {
    if (this.pair && this.pair.mine && this.pair.theirs) {
      clearTimeout(this.pairTimer);
      this.trust.add({ id: this.peer.peerId, raw: this.peer.peerRaw, name: this.peer.peerName });
      this.pair = null;
      this._setState('connected');
    } else {
      this.emit('state', this.snapshot());
    }
  }

  _onMessage(ch, m) {
    if (ch !== this.channel) return;
    this.lastSeen = Date.now();
    if (m.t === 'ping') return;
    if (this.state === 'checking') {
      if (m.t !== 'trust') return this.disconnect('beklenmeyen mesaj');
      this.theyKnow = m.known === true;
      return this._decide();
    }
    if (this.state === 'pairing') {
      if (m.t === 'pair.ok') { this.pair.theirs = true; this._checkPaired(); }
      else if (m.t === 'pair.no') this.disconnect('karşı taraf eşleştirmeyi reddetti');
      return; // eşleşme bitmeden başka mesaj kabul edilmez
    }
    if (this.state === 'connected') this.emit('message', m);
  }

  _ping() {
    if (Date.now() - this.lastSeen > DEAD_AFTER) return this.disconnect('bağlantı koptu');
    if (this.channel) this.channel.send({ t: 'ping' });
  }

  send(msg) {
    if (this.state !== 'connected' || !this.channel) return false;
    return this.channel.send(msg);
  }

  get needDrain() { return !!(this.channel && this.channel.writableNeedDrain); }

  disconnect(reason) {
    if (this.channel) this.channel.close(reason ? new Error(reason) : null);
  }

  _onClose(ch, err) {
    if (ch !== this.channel) return;
    clearInterval(this.pingTimer);
    clearTimeout(this.pairTimer);
    this.channel = null;
    this.peer = null;
    this.pair = null;
    this.lastError = err ? err.message : '';
    this._setState('idle');
  }

  _setState(s) {
    this.state = s;
    this.emit('state', this.snapshot());
  }

  snapshot() {
    return {
      state: this.state,
      peer: this.peer ? { id: this.peer.peerId, name: this.peer.peerName, address: this.peer.address } : null,
      pairing: this.pair ? { sas: this.pair.sas, mine: this.pair.mine, theirs: this.pair.theirs } : null,
      error: this.lastError || '',
    };
  }

  close() {
    this.disconnect();
    if (this.server) this.server.close();
  }
}

module.exports = { Link, DEFAULT_PORT };
