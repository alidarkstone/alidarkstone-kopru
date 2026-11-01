'use strict';
// Arayüz. Tüm metinler textContent ile yazılır (HTML enjeksiyonu olmaz).
const api = window.kopru;
const $ = (id) => document.getElementById(id);
let S = null;

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function btn(text, onClick, cls) {
  const b = el('button', cls || 'ghost', text);
  b.addEventListener('click', onClick);
  return b;
}

function size(n) {
  if (n < 1024) return n + ' B';
  if (n < 1024 ** 2) return (n / 1024).toFixed(1) + ' KB';
  if (n < 1024 ** 3) return (n / 1024 ** 2).toFixed(1) + ' MB';
  return (n / 1024 ** 3).toFixed(2) + ' GB';
}

const STATUS = {
  idle: ['Bağlı değil', 'off'],
  connecting: ['Bağlanıyor…', 'wait'],
  checking: ['Doğrulanıyor…', 'wait'],
  pairing: ['Eşleştirme onayı bekleniyor', 'wait'],
  connected: ['Bağlı', 'on'],
};

const TRANSFER = {
  waiting: 'Karşı tarafın onayı bekleniyor', sending: 'Gönderiliyor', sent: 'Gönderildi',
  offered: 'Onayınızı bekliyor', receiving: 'Alınıyor', received: 'Alındı',
  rejected: 'Reddedildi', cancelled: 'İptal edildi', error: 'Hata (dosya doğrulanamadı)',
};

function render(s) {
  S = s;
  if (document.activeElement !== $('myName')) $('myName').value = s.me.name;
  $('myId').textContent = 'Kimlik: ' + s.me.id.replace(/(.{4})/g, '$1 ').trim();

  const L = s.link;
  const [title, cls] = STATUS[L.state] || STATUS.idle;
  $('statusTitle').textContent = L.peer ? `${title}: ${L.peer.name}` : title;
  $('statusSub').textContent = L.peer ? `${L.peer.address}` : (L.error ? 'Son durum: ' + L.error : '');
  $('dot').className = 'dot ' + cls;
  $('disconnectBtn').classList.toggle('hidden', L.state === 'idle');

  const pairing = L.state === 'pairing' && L.pairing;
  $('pairBox').classList.toggle('hidden', !pairing);
  if (pairing) {
    $('sas').textContent = L.pairing.sas.slice(0, 3) + ' ' + L.pairing.sas.slice(3);
    const mine = L.pairing.mine === true;
    $('pairYes').disabled = mine;
    $('pairWait').textContent = mine ? 'Diğer bilgisayarda da onaylanması bekleniyor…' : (L.pairing.theirs ? 'Diğer bilgisayar onayladı.' : '');
  }

  // cihazlar
  const dev = $('devices');
  dev.replaceChildren();
  for (const d of s.devices) {
    const li = el('li');
    const info = el('div');
    info.append(el('b', '', d.name), el('div', 'muted small', `${d.address}${d.trusted ? ' · eşleşmiş' : ''}`));
    li.append(info);
    const isPeer = L.peer && L.peer.id === d.id;
    if (!isPeer) li.append(btn('Bağlan', () => api.connect(d.id), ''));
    else li.append(el('span', 'tag', 'bağlı'));
    dev.append(li);
  }
  $('noDevices').classList.toggle('dim', s.devices.length > 0);

  // paylaşım
  $('clipToggle').checked = s.settings.clipboard;
  const inp = s.input;
  $('inputToggle').checked = inp.enabled;
  $('inputToggle').disabled = !inp.available;
  $('edge').value = s.settings.input.edge;
  const modeText = { remote: 'İmleç şu an diğer bilgisayarda (geri almak için Scroll Lock)', controlled: 'Diğer bilgisayar bu bilgisayarı kontrol ediyor' };
  $('inputSub').textContent = !inp.available
    ? (s.platform === 'win32' ? 'Yardımcı derlenmemiş: "npm run build-helper" çalıştırın' : 'Bu sürümde yalnızca Windows destekleniyor')
    : inp.error || modeText[inp.mode] || 'İmleci ekran kenarından diğer bilgisayara geçirin';

  // dosyalar
  $('dlDir').textContent = s.settings.downloadDir;
  $('sendFiles').disabled = L.state !== 'connected';
  const tr = $('transfers');
  tr.replaceChildren();
  for (const t of [...s.transfers].reverse()) {
    const li = el('li');
    const info = el('div', 'grow');
    info.append(el('b', '', (t.dir === 'in' ? '↓ ' : '↑ ') + t.name));
    info.append(el('div', 'muted small', `${size(t.size)} · ${TRANSFER[t.status] || t.status}`));
    if (t.status === 'sending' || t.status === 'receiving') {
      const bar = el('div', 'bar');
      const fill = el('div', 'fill');
      fill.style.width = (t.size ? Math.round((t.done / t.size) * 100) : 100) + '%';
      bar.append(fill);
      info.append(bar);
    }
    li.append(info);
    const acts = el('div', 'row');
    if (t.status === 'offered') acts.append(btn('Kabul et', () => api.fileAccept(t.id), ''), btn('Reddet', () => api.fileReject(t.id)));
    if (['waiting', 'sending', 'receiving'].includes(t.status)) acts.append(btn('İptal', () => api.fileCancel(t.id)));
    if (t.status === 'received') acts.append(btn('Klasörde göster', () => api.fileShow(t.id)));
    li.append(acts);
    tr.append(li);
  }

  // güvenilenler
  const tl = $('trusted');
  tl.replaceChildren();
  for (const t of s.trusted) {
    const li = el('li');
    const info = el('div');
    info.append(el('b', '', t.name), el('div', 'muted small', 'Kimlik: ' + t.id));
    li.append(info, btn('Unut', () => { if (confirm(`${t.name} unutulsun mu? Tekrar bağlanmak için kod onayı gerekir.`)) api.forget(t.id); }));
    tl.append(li);
  }
  $('noTrusted').classList.toggle('hidden', s.trusted.length > 0);
}

// ---- olaylar ----
$('myName').addEventListener('change', (e) => api.settings({ name: e.target.value }));
$('disconnectBtn').addEventListener('click', () => api.disconnect());
$('pairYes').addEventListener('click', () => api.confirmPairing(true));
$('pairNo').addEventListener('click', () => api.confirmPairing(false));
$('manual').addEventListener('submit', (e) => { e.preventDefault(); if ($('addr').value.trim()) api.connectAddress($('addr').value); });
$('clipToggle').addEventListener('change', (e) => api.settings({ clipboard: e.target.checked }));
$('inputToggle').addEventListener('change', (e) => api.settings({ input: { enabled: e.target.checked } }));
$('edge').addEventListener('change', (e) => api.settings({ input: { edge: e.target.value } }));
$('sendFiles').addEventListener('click', () => api.pickFiles());
$('openDownloads').addEventListener('click', () => api.openDownloads());
$('changeDir').addEventListener('click', (e) => { e.preventDefault(); api.chooseDownloadDir(); });

const dz = $('dropZone');
['dragenter', 'dragover'].forEach((ev) => document.addEventListener(ev, (e) => { e.preventDefault(); dz.classList.add('drag'); }));
['dragleave', 'drop'].forEach((ev) => document.addEventListener(ev, (e) => { e.preventDefault(); if (ev === 'drop' || e.target === document.documentElement) dz.classList.remove('drag'); }));
document.addEventListener('drop', (e) => {
  if (S && S.link.state === 'connected' && e.dataTransfer.files.length) api.sendDropped(e.dataTransfer.files);
});

api.onState(render);
api.state().then(render);
