'use strict';
const test = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { EventEmitter } = require('events');
const { PassThrough } = require('stream');
const { FileShare, safeName } = require('../src/features/files');
const { ClipboardShare } = require('../src/features/clipboard');
const { InputShare, sanitizeEvent } = require('../src/features/input');

test('dosya adı temizleme: klasör dışına çıkış ve Windows ayrılmış adları engellenir', () => {
  assert.strictEqual(safeName('../../Windows/System32/evil.dll'), 'evil.dll');
  assert.strictEqual(safeName('..\\..\\a.txt'), 'a.txt');
  assert.strictEqual(safeName('CON'), '_CON');
  assert.strictEqual(safeName('nul.txt'), '_nul.txt');
  assert.strictEqual(safeName('a<b>:c?.txt'), 'a_b__c_.txt');
  assert.strictEqual(safeName('..'), 'dosya');
  assert.strictEqual(safeName('rapor.pdf. . '), 'rapor.pdf');
});

// İki FileShare'i bellekte birbirine bağlar.
function filePair(dlDir) {
  let a, b;
  const q = [];
  const deliver = (to, m) => setImmediate(() => to.onMessage(JSON.parse(JSON.stringify(m))));
  a = new FileShare({ send: (m) => { deliver(b, m); return true; }, waitDrain: async () => {}, downloadDir: () => dlDir });
  b = new FileShare({ send: (m) => { deliver(a, m); return true; }, waitDrain: async () => {}, downloadDir: () => dlDir });
  return { a, b };
}

test('dosya: onay olmadan yazılmaz, onaydan sonra birebir aynı gelir', async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'kopru-'));
  const src = path.join(tmp, 'kaynak.bin');
  const data = crypto.randomBytes(1024 * 1024 + 123);
  fs.writeFileSync(src, data);
  const dl = path.join(tmp, 'gelen');
  const { a, b } = filePair(dl);
  const offered = new Promise((r) => b.once('offer', r));
  await a.offer(src);
  const o = await offered;
  assert.ok(!fs.existsSync(dl), 'onaydan önce klasör bile oluşmamalı');
  const received = new Promise((r) => b.once('received', r));
  b.accept(o.id);
  const r = await received;
  assert.ok(fs.readFileSync(r.path).equals(data));
  assert.strictEqual(path.basename(r.path), 'kaynak.bin');
  // ikinci kez aynı ad: üzerine yazılmaz
  const offered2 = new Promise((res) => b.once('offer', res));
  await a.offer(src);
  const o2 = await offered2;
  const rec2 = new Promise((res) => b.once('received', res));
  b.accept(o2.id);
  assert.strictEqual(path.basename((await rec2).path), 'kaynak (2).bin');
});

test('dosya: bozuk içerik (hash uyuşmazlığı) silinir', async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'kopru-'));
  const dl = path.join(tmp, 'gelen');
  const b = new FileShare({ send: () => true, waitDrain: async () => {}, downloadDir: () => dl });
  const id = 'a'.repeat(16);
  b.onMessage({ t: 'file.offer', id, name: 'x.txt', size: 5 });
  b.accept(id);
  b.onMessage({ t: 'file.chunk', id, data: Buffer.from('hello').toString('base64') });
  b.onMessage({ t: 'file.done', id, sha256: '0'.repeat(64) });
  await new Promise((r) => setTimeout(r, 50));
  assert.strictEqual(b.list()[0].status, 'error');
  assert.deepStrictEqual(fs.readdirSync(dl), []);
});

test('dosya: bildirilenden büyük veri gelirse aktarım iptal edilir', async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'kopru-'));
  const dl = path.join(tmp, 'gelen');
  const b = new FileShare({ send: () => true, waitDrain: async () => {}, downloadDir: () => dl });
  const id = 'b'.repeat(16);
  b.onMessage({ t: 'file.offer', id, name: 'x.txt', size: 2 });
  b.accept(id);
  b.onMessage({ t: 'file.chunk', id, data: Buffer.from('fazla veri').toString('base64') });
  assert.strictEqual(b.list()[0].status, 'cancelled');
});

test('pano: kapalıyken gönderilmez, açıkken gönderilir ve geri yankı yapmaz', () => {
  let text = 'ilk';
  const sent = [];
  let on = false;
  const c = new ClipboardShare({ clipboard: { readText: () => text, writeText: (t) => { text = t; } }, send: (m) => sent.push(m), enabled: () => on });
  c.last = c._read();
  text = 'gizli'; c.tick();
  assert.strictEqual(sent.length, 0);
  on = true;
  text = 'merhaba'; c.tick();
  assert.deepStrictEqual(sent, [{ t: 'clip.text', text: 'merhaba' }]);
  c.onMessage({ t: 'clip.text', text: 'karşıdan' });
  assert.strictEqual(text, 'karşıdan');
  c.tick();
  assert.strictEqual(sent.length, 1, 'gelen metin geri gönderilmemeli');
});

function fakeHelper() {
  const proc = new EventEmitter();
  proc.stdin = new PassThrough();
  proc.stdout = new PassThrough();
  proc.cmds = [];
  proc.stdin.on('data', (d) => d.toString().split('\n').filter(Boolean).forEach((l) => proc.cmds.push(JSON.parse(l))));
  proc.emitEv = (o) => proc.stdout.write(JSON.stringify(o) + '\n');
  proc.kill = () => {};
  return proc;
}
const tick = () => new Promise((r) => setTimeout(r, 20));

test('fare/klavye: kenar -> diğerine geçiş, olaylar iletilir, Scroll Lock geri alır', async () => {
  const proc = fakeHelper();
  const sent = [];
  const inp = new InputShare({ helperPath: 'x.exe', send: (m) => sent.push(m), connected: () => true, spawnFn: () => proc });
  inp.configure({ enabled: true, edge: 'right' });
  await tick();
  assert.deepStrictEqual(proc.cmds.at(-1), { cmd: 'edge', edge: 'right' });
  proc.emitEv({ ev: 'edge', pos: 0.25 }); await tick();
  assert.deepStrictEqual(sent[0], { t: 'input.enter', edge: 'left', pos: 0.25 });
  assert.strictEqual(inp.mode, 'remote');
  proc.emitEv({ ev: 'move', dx: 5, dy: -3 }); await tick();
  assert.deepStrictEqual(sent[1], { t: 'input.ev', kind: 'move', dx: 5, dy: -3 });
  proc.emitEv({ ev: 'escape' }); await tick();
  assert.deepStrictEqual(sent[2], { t: 'input.release' });
  assert.strictEqual(inp.mode, 'local');
  proc.emitEv({ ev: 'move', dx: 1, dy: 1 }); await tick();
  assert.strictEqual(sent.length, 3, 'geri dönünce olay gönderilmemeli');
});

test('fare/klavye: kapalıyken gelen kontrol isteği reddedilir', async () => {
  const sent = [];
  const inp = new InputShare({ helperPath: 'x.exe', send: (m) => sent.push(m), connected: () => true, spawnFn: fakeHelper });
  inp.onMessage({ t: 'input.enter', edge: 'left', pos: 0.5 });
  assert.deepStrictEqual(sent, [{ t: 'input.leave', pos: 0.5 }]);
  assert.strictEqual(inp.mode, 'local');
});

test('fare/klavye: bağlantı koparsa kontrol hemen bırakılır', async () => {
  const proc = fakeHelper();
  const inp = new InputShare({ helperPath: 'x.exe', send: () => true, connected: () => true, spawnFn: () => proc });
  inp.configure({ enabled: true, edge: 'right' });
  inp.onMessage({ t: 'input.enter', edge: 'left', pos: 0.5 });
  assert.strictEqual(inp.mode, 'controlled');
  inp.onLink(false); await tick();
  assert.strictEqual(inp.mode, 'local');
  assert.ok(proc.cmds.some((c) => c.cmd === 'leave'));
});

test('gelen giriş olayları yalnızca bilinen alanlarla ve sınırlar içinde iletilir', () => {
  assert.deepStrictEqual(sanitizeEvent({ kind: 'move', dx: 1e9, dy: 'x', extra: 'kötü' }), { cmd: 'move', dx: 5000, dy: 0 });
  assert.deepStrictEqual(sanitizeEvent({ kind: 'key', vk: 999, sc: 30, down: 'true' }), { cmd: 'key', vk: 254, sc: 30, ext: false, down: false });
  assert.deepStrictEqual(sanitizeEvent({ kind: 'button', b: 'run-cmd', down: true }), { cmd: 'button', b: 'left', down: true });
});
