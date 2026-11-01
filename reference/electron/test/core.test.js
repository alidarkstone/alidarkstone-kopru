'use strict';
const test = require('node:test');
const assert = require('node:assert');
const net = require('net');
const { loadIdentity } = require('../src/core/keys');
const { SecureChannel } = require('../src/core/channel');
const { Link } = require('../src/core/link');
const { FrameReader, encodeFrame } = require('../src/core/framing');

function memTrust() {
  const m = new Map();
  return { isTrusted: (id, raw) => m.has(id) && m.get(id).raw === raw, add: (p) => m.set(p.id, p), m };
}

function once(em, ev, pred = () => true) {
  return new Promise((res) => { const f = (x) => { if (pred(x)) { em.removeListener(ev, f); res(x); } }; em.on(ev, f); });
}

test('çerçeveler parça parça gelse de doğru birleşir', () => {
  const got = [];
  const r = new FrameReader((f) => got.push(f.toString()), (e) => { throw e; });
  const all = Buffer.concat([encodeFrame(Buffer.from('merhaba')), encodeFrame(Buffer.from('dünya'))]);
  for (const b of all) r.push(Buffer.from([b]));
  assert.deepStrictEqual(got, ['merhaba', 'dünya']);
});

test('kimlik PEM ile aynı parmak izini verir', () => {
  const a = loadIdentity();
  const b = loadIdentity(a.pem);
  assert.strictEqual(a.id, b.id);
});

async function channelPair() {
  const A = loadIdentity(), B = loadIdentity();
  const server = net.createServer();
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const accepted = once(server, 'connection');
  const cs = net.connect(server.address().port, '127.0.0.1');
  const ss = await accepted;
  const client = new SecureChannel(cs, { identity: A, name: 'A', role: 'client' });
  const srv = new SecureChannel(ss, { identity: B, name: 'B', role: 'server' });
  const [ia, ib] = await Promise.all([once(client, 'ready'), once(srv, 'ready')]);
  return { A, B, client, srv, ia, ib, server, cs, ss };
}

test('el sıkışma: aynı kod, doğru kimlikler, şifreli mesaj gider', async () => {
  const { A, B, client, srv, ia, ib, server } = await channelPair();
  assert.strictEqual(ia.sas, ib.sas);
  assert.match(ia.sas, /^\d{6}$/);
  assert.strictEqual(ia.peerId, B.id);
  assert.strictEqual(ib.peerId, A.id);
  assert.strictEqual(ib.peerName, 'A');
  const got = once(srv, 'message');
  client.send({ t: 'test', s: 'gizli-veri-123' });
  assert.deepStrictEqual(await got, { t: 'test', s: 'gizli-veri-123' });
  client.close(); server.close();
});

test('değiştirilmiş şifreli paket bağlantıyı kapatır', async () => {
  const { client, srv, server, cs } = await channelPair();
  const closed = once(srv, 'close');
  // Geçerli bir çerçeve uzunluğuyla rastgele (sahte) şifreli veri gönder.
  cs.write(encodeFrame(Buffer.alloc(40, 7)));
  const err = await closed;
  assert.ok(err instanceof Error);
  client.close(); server.close();
});

test('ağ üzerinde şifreli içerik açık metin görünmez', async () => {
  const A = loadIdentity(), B = loadIdentity();
  const seen = [];
  const server = net.createServer();
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const accepted = once(server, 'connection');
  const cs = net.connect(server.address().port, '127.0.0.1');
  const ss = await accepted;
  ss.on('data', (d) => seen.push(d));
  const client = new SecureChannel(cs, { identity: A, name: 'A', role: 'client' });
  const srv = new SecureChannel(ss, { identity: B, name: 'B', role: 'server' });
  await Promise.all([once(client, 'ready'), once(srv, 'ready')]);
  const got = once(srv, 'message');
  client.send({ t: 'clip', text: 'COK-GIZLI-PAROLA' });
  await got;
  assert.ok(!Buffer.concat(seen).includes(Buffer.from('COK-GIZLI-PAROLA')));
  client.close(); server.close();
});

async function linkPair() {
  const ta = memTrust(), tb = memTrust();
  const a = new Link({ identity: loadIdentity(), getName: () => 'PC-A', trust: ta, port: 0 });
  const b = new Link({ identity: loadIdentity(), getName: () => 'PC-B', trust: tb, port: 0 });
  await a.listen(); await b.listen();
  return { a, b, ta, tb };
}

test('eşleştirme: iki taraf onaylamadan mesaj geçmez, onaydan sonra geçer; sonra kod sorulmaz', async () => {
  const { a, b, ta, tb } = await linkPair();
  const pa = once(a, 'state', (s) => s.state === 'pairing');
  const pb = once(b, 'state', (s) => s.state === 'pairing');
  a.connect('127.0.0.1', b.port);
  const [sa, sb] = await Promise.all([pa, pb]);
  assert.strictEqual(sa.pairing.sas, sb.pairing.sas);
  assert.strictEqual(a.send({ t: 'x' }), false);

  const ca = once(a, 'state', (s) => s.state === 'connected');
  const cb = once(b, 'state', (s) => s.state === 'connected');
  a.confirmPairing(true); b.confirmPairing(true);
  await Promise.all([ca, cb]);
  assert.strictEqual(ta.m.size, 1); assert.strictEqual(tb.m.size, 1);
  const got = once(b, 'message');
  a.send({ t: 'clip.text', text: 'selam' });
  assert.strictEqual((await got).text, 'selam');

  const ib = once(b, 'state', (s) => s.state === 'idle');
  a.disconnect(); await ib;
  const ra = once(a, 'state', (s) => s.state === 'connected');
  const rb = once(b, 'state', (s) => s.state === 'connected');
  b.connect('127.0.0.1', a.port); // ters yönden yeniden bağlan: kod istenmemeli
  await Promise.all([ra, rb]);
  a.close(); b.close();
});

test('bir taraf cihazı unuttuysa ikisi de yeniden kod onayına düşer', async () => {
  const { a, b, ta, tb } = await linkPair();
  const pa = once(a, 'state', (s) => s.state === 'pairing');
  const pb = once(b, 'state', (s) => s.state === 'pairing');
  a.connect('127.0.0.1', b.port);
  await Promise.all([pa, pb]);
  const ca = once(a, 'state', (s) => s.state === 'connected');
  a.confirmPairing(true); b.confirmPairing(true);
  await ca;
  const ib = once(b, 'state', (s) => s.state === 'idle');
  a.disconnect(); await ib;
  tb.m.clear(); // B, A'yı unuttu
  const pa2 = once(a, 'state', (s) => s.state === 'pairing');
  const pb2 = once(b, 'state', (s) => s.state === 'pairing');
  a.connect('127.0.0.1', b.port);
  await Promise.all([pa2, pb2]);
  a.close(); b.close();
});

test('reddedilen eşleştirme bağlantıyı keser ve güvenilir listesine eklemez', async () => {
  const { a, b, ta, tb } = await linkPair();
  const pb = once(b, 'state', (s) => s.state === 'pairing');
  a.connect('127.0.0.1', b.port);
  await pb;
  const ia = once(a, 'state', (s) => s.state === 'idle');
  b.confirmPairing(false);
  await ia;
  assert.strictEqual(ta.m.size + tb.m.size, 0);
  a.close(); b.close();
});
