'use strict';
// Anahtar yardımcıları: Ed25519 (kalıcı cihaz kimliği) ve X25519 (oturum başına geçici anahtar).
// Ham 32 baytlık açık anahtarlar base64url olarak taşınır.

const crypto = require('crypto');

function rawPublic(keyObject) {
  return Buffer.from(keyObject.export({ format: 'jwk' }).x, 'base64url');
}

function publicFromRaw(raw, crv) {
  if (!Buffer.isBuffer(raw) || raw.length !== 32) throw new Error('geçersiz açık anahtar');
  return crypto.createPublicKey({ key: { kty: 'OKP', crv, x: raw.toString('base64url') }, format: 'jwk' });
}

function fingerprint(rawPub) {
  return crypto.createHash('sha256').update(rawPub).digest('hex').slice(0, 16);
}

// Kalıcı kimlik. `pem` verilirse ondan yüklenir, yoksa yenisi üretilir.
function loadIdentity(pem) {
  const privateKey = pem
    ? crypto.createPrivateKey(pem)
    : crypto.generateKeyPairSync('ed25519').privateKey;
  if (privateKey.asymmetricKeyType !== 'ed25519') throw new Error('kimlik anahtarı Ed25519 değil');
  const publicKey = crypto.createPublicKey(privateKey);
  const raw = rawPublic(publicKey);
  return {
    privateKey,
    publicKey,
    raw,
    id: fingerprint(raw),
    pem: privateKey.export({ format: 'pem', type: 'pkcs8' }),
  };
}

function hkdf(secret, salt, info, length = 32) {
  return Buffer.from(crypto.hkdfSync('sha256', secret, salt, Buffer.from(info), length));
}

module.exports = { rawPublic, publicFromRaw, fingerprint, loadIdentity, hkdf };
