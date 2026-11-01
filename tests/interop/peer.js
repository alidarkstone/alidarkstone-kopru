'use strict';
// Köprü birlikte çalışma testi için Electron referans çekirdeğini (reference/electron/src) süren uç.
// Rust testi bunu çalıştırır; stdin'den JSON satır komutları alır, stdout'a JSON satır olaylar yazar.
//
//   node peer.js <indirme-klasörü>      -> {"ev":"ready","port":N,"id":"..."}
//   node peer.js --safename '<json dizi>' -> [[girdi, safeName(girdi)], ...]
//
// Komutlar: {cmd:"connect",port} {cmd:"confirm",ok} {cmd:"send",msg} {cmd:"offer",path}
//           {cmd:"accept",id} {cmd:"disconnect"} {cmd:"forget"} {cmd:"quit"}

const path = require('path');
const readline = require('readline');
const SRC = path.resolve(__dirname, '../../reference/electron/src');
const { loadIdentity } = require(path.join(SRC, 'core/keys'));
const { Link } = require(path.join(SRC, 'core/link'));
const { FileShare, safeName } = require(path.join(SRC, 'features/files'));

function out(o) { process.stdout.write(JSON.stringify(o) + '\n'); }

if (process.argv[2] === '--safename') {
  out(JSON.parse(process.argv[3]).map((n) => [n, safeName(n)]));
  process.exit(0);
}

const downloadDir = process.argv[2];
const identity = loadIdentity();
const trusted = new Map();
const trust = {
  isTrusted: (id, raw) => trusted.has(id) && trusted.get(id).raw === raw,
  add: (p) => trusted.set(p.id, p),
};
const link = new Link({ identity, getName: () => 'Node Ucu', trust, port: 0 });

function waitDrain() {
  return new Promise((resolve) => {
    if (!link.needDrain) return resolve();
    const done = () => { link.removeListener('drain', done); link.removeListener('state', onState); resolve(); };
    const onState = (s) => { if (s.state !== 'connected') done(); };
    link.on('drain', done);
    link.on('state', onState);
  });
}

const files = new FileShare({ send: (m) => link.send(m), waitDrain, downloadDir: () => downloadDir });
files.on('offer', (o) => out({ ev: 'offer', ...o }));
files.on('received', (r) => out({ ev: 'received', ...r }));
files.on('change', (list) => {
  for (const t of list) if (t.dir === 'out' && ['sent', 'error', 'cancelled'].includes(t.status)) out({ ev: 'out', id: t.id, status: t.status });
});

link.on('state', (s) => { if (s.state === 'idle') files.abortAll(); out({ ev: 'state', s, trusted: [...trusted.keys()] }); });
link.on('message', (m) => {
  if (files.onMessage(m)) return;
  out({ ev: 'message', m });
});
link.on('error', (e) => out({ ev: 'error', message: String(e && e.message) }));

const rl = readline.createInterface({ input: process.stdin });
rl.on('line', async (line) => {
  let c;
  try { c = JSON.parse(line); } catch (_) { return; }
  try {
    switch (c.cmd) {
      case 'connect': link.connect('127.0.0.1', c.port); break;
      case 'confirm': link.confirmPairing(c.ok); break;
      case 'send': out({ ev: 'sent', ok: link.send(c.msg) }); break;
      case 'offer': out({ ev: 'offered', id: await files.offer(c.path) }); break;
      case 'accept': files.accept(c.id); break;
      case 'disconnect': link.disconnect(); break;
      case 'forget': trusted.clear(); break;
      case 'quit': link.close(); process.exit(0); break;
    }
  } catch (e) {
    out({ ev: 'error', message: String(e && e.message) });
  }
});
rl.on('close', () => { link.close(); process.exit(0); });

link.listen().then((port) => out({ ev: 'ready', port, id: identity.id }));
