'use strict';
// Yerel ağda cihaz bulma: her birkaç saniyede bir UDP yayını ("ben buradayım").
// Bu bilgi yalnızca listeyi doldurmak içindir, güvenilmez. Kimlik, bağlantıda el sıkışmayla doğrulanır.

const dgram = require('dgram');
const os = require('os');
const { EventEmitter } = require('events');

const DISCOVERY_PORT = 47911;
const EVERY = 3000;
const EXPIRE = 10000;

function broadcastAddresses() {
  const out = new Set(['255.255.255.255']);
  for (const list of Object.values(os.networkInterfaces())) {
    for (const a of list || []) {
      if (a.family !== 'IPv4' || a.internal || !a.netmask) continue;
      const ip = a.address.split('.').map(Number);
      const mask = a.netmask.split('.').map(Number);
      out.add(ip.map((b, i) => (b & mask[i]) | (~mask[i] & 255)).join('.'));
    }
  }
  return [...out];
}

class Discovery extends EventEmitter {
  // opts: { id, name, port, discoveryPort? }
  constructor(opts) {
    super();
    this.me = { app: 'kopru', v: 1, id: opts.id, name: opts.name, port: opts.port };
    this.discoveryPort = opts.discoveryPort || DISCOVERY_PORT;
    this.peers = new Map(); // id -> { id, name, address, port, seen }
  }

  start() {
    this.sock = dgram.createSocket({ type: 'udp4', reuseAddr: true });
    this.sock.on('error', (e) => this.emit('error', e));
    this.sock.on('message', (buf, rinfo) => this._onMessage(buf, rinfo));
    this.sock.bind(this.discoveryPort, () => {
      try { this.sock.setBroadcast(true); } catch (_) { /* yok say */ }
      this._announce();
    });
    this.timer = setInterval(() => { this._announce(); this._expire(); }, EVERY);
  }

  setName(name) { this.me.name = name; }

  _announce() {
    const buf = Buffer.from(JSON.stringify(this.me));
    for (const addr of broadcastAddresses()) this.sock.send(buf, this.discoveryPort, addr, () => {});
  }

  _onMessage(buf, rinfo) {
    if (buf.length > 1024) return;
    let m;
    try { m = JSON.parse(buf.toString('utf8')); } catch (_) { return; }
    if (!m || m.app !== 'kopru' || m.v !== 1 || m.id === this.me.id) return;
    if (typeof m.id !== 'string' || !/^[0-9a-f]{16}$/.test(m.id)) return;
    const port = Number(m.port);
    if (!Number.isInteger(port) || port < 1 || port > 65535) return;
    const name = String(m.name || '').replace(/[\u0000-\u001f]/g, '').slice(0, 64) || rinfo.address;
    const known = this.peers.get(m.id);
    const peer = { id: m.id, name, address: rinfo.address, port, seen: Date.now() };
    this.peers.set(m.id, peer);
    if (!known || known.address !== peer.address || known.name !== peer.name) this.emit('change', this.list());
  }

  _expire() {
    let changed = false;
    for (const [id, p] of this.peers) if (Date.now() - p.seen > EXPIRE) { this.peers.delete(id); changed = true; }
    if (changed) this.emit('change', this.list());
  }

  list() { return [...this.peers.values()].map(({ seen, ...p }) => p); }

  stop() {
    clearInterval(this.timer);
    if (this.sock) this.sock.close();
  }
}

module.exports = { Discovery, DISCOVERY_PORT };
