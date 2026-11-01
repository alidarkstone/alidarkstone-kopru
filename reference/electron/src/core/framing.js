'use strict';
// Uzunluk önekli çerçeveler: [4 bayt büyük-endian uzunluk][gövde].
// TCP akışını tam mesajlara böler. Aşırı büyük çerçeveleri reddeder.

const MAX_FRAME = 4 * 1024 * 1024; // 4 MB (dosya parçaları 256 KB olduğundan bol bol yeter)

function encodeFrame(payload) {
  if (!Buffer.isBuffer(payload)) throw new TypeError('payload Buffer olmalı');
  if (payload.length > MAX_FRAME) throw new RangeError('çerçeve çok büyük');
  const head = Buffer.alloc(4);
  head.writeUInt32BE(payload.length, 0);
  return Buffer.concat([head, payload]);
}

class FrameReader {
  constructor(onFrame, onError) {
    this.onFrame = onFrame;
    this.onError = onError;
    this.chunks = [];
    this.length = 0;
    this.dead = false;
  }

  push(data) {
    if (this.dead) return;
    this.chunks.push(data);
    this.length += data.length;
    while (this.length >= 4) {
      const buf = this.chunks.length === 1 ? this.chunks[0] : Buffer.concat(this.chunks);
      this.chunks = [buf];
      const size = buf.readUInt32BE(0);
      if (size > MAX_FRAME) {
        this.dead = true;
        this.onError(new Error('karşı taraf çok büyük çerçeve gönderdi'));
        return;
      }
      if (buf.length < 4 + size) return;
      const frame = buf.subarray(4, 4 + size);
      const rest = buf.subarray(4 + size);
      this.chunks = rest.length ? [rest] : [];
      this.length = rest.length;
      this.onFrame(Buffer.from(frame));
    }
  }
}

module.exports = { encodeFrame, FrameReader, MAX_FRAME };
