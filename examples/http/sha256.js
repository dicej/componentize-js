// Source: https://github.com/edwinhayward/incremental-sha256-hasher

// MIT License

// Copyright (c) 2025 Edwin Hayward, Genki Productions Ltd

// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:

// 1. License Notice: The above copyright notice and this permission notice
//    shall be included in all copies or substantial portions of the Software.

// 2. Use at Your Own Risk: The Software is provided "AS IS", WITHOUT WARRANTY
//    OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES
//    OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
//    By using this Software, you acknowledge and accept that any liability for
//    damages, loss, or other issues arising from its use is entirely your own.

// 3. AI Authorship Note: Portions of this Software may have been generated
//    with the assistance of AI tools. Final authorship for the Software rests
//    with the copyright holder.

// ---

// DISCLAIMER OF LIABILITY AND NO WARRANTY

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE, TITLE, AND NON-INFRINGEMENT. IN NO EVENT
// SHALL THE COPYRIGHT HOLDER, CONTRIBUTORS, OR ANYONE DISTRIBUTING THE SOFTWARE
// ON THEIR BEHALF BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
// EXEMPLARY, PUNITIVE OR CONSEQUENTIAL DAMAGES, OR FOR ANY LOSS OF PROFITS,
// DATA, USE, GOODWILL, OR OTHER INTANGIBLE LOSSES, ARISING OUT OF OR IN ANY WAY
// CONNECTED WITH THE SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGES.

// BY USING THE SOFTWARE YOU ACKNOWLEDGE AND AGREE THAT YOU ARE USING IT AT YOUR
// OWN RISK AND THAT YOU, NOT THE COPYRIGHT HOLDER (EDWIN HAYWARD / GENKI
// PRODUCTIONS LTD), ARE SOLELY RESPONSIBLE FOR ANY CONSEQUENCES OF ITS USE.

/**
 * @file Incremental SHA-256 hasher implemented in pure JavaScript.
 * @description
 *   Provides a streaming interface for computing SHA-256 hashes.
 *   Data can be fed incrementally in arbitrary-sized chunks.
 *   Suitable for large files, network streams, or progressive data sources.
 *
 *   Features:
 *   - Streaming incremental updates via `update()`.
 *   - Finalize with `digest()`, supporting multiple output formats.
 *   - Cloneable state for branching hash computations.
 *   - Export/import internal state for persistence or resumption.
 *
 *   Fully adheres to the SHA-256 specification (FIPS 180-4).
 *
 * @version 2.2.0
 */

/** @private */
const MAX_BYTES_FOR_SAFE_BITLENGTH = Math.floor(Number.MAX_SAFE_INTEGER / 8);

export class IncrementalSHA256 {
  constructor() {
    /** @private @type {Uint32Array} SHA-256 round constants */
    const K = new Uint32Array([
      0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
      0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
      0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
      0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
      0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
      0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
      0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
      0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
    ]);
    this.K = K;

    /** @private */ this.W = new Uint32Array(64);
    this.reset();
  }

  /**
   * Resets the hasher to the initial state.
   */
  reset() {
    this.H = new Uint32Array([
      0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
      0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19
    ]);
    this.buffer = new Uint8Array(64);
    this.bufferLength = 0;
    this.bytesHashed = 0;
    this.finalized = false;
  }

  /**
   * Feeds more data into the hash.
   * @param {Uint8Array} chunk - Data to hash.
   * @throws {TypeError} if chunk is not Uint8Array.
   * @throws {Error} if update is called after digest().
   */
  update(chunk) {
    if (this.finalized) throw new Error('Cannot update after digest(); call reset() to reuse');
    if (!(chunk instanceof Uint8Array)) throw new TypeError('IncrementalSHA256.update expects a Uint8Array');
    if (chunk.length === 0) return;

    // If the buffer is already full from a previous operation (e.g. importState),
    // the loop below will be infinite. Process the block BEFORE copying new data.
    if (this.bufferLength === 64) {
      this._processBlock();
      this.bufferLength = 0;
    }

    const newTotal = this.bytesHashed + chunk.length;
    if (!Number.isSafeInteger(newTotal) || newTotal > MAX_BYTES_FOR_SAFE_BITLENGTH) {
      throw new Error('Input too large: would exceed safe bit-length precision');
    }
    this.bytesHashed = newTotal;

    let offset = 0;
    while (offset < chunk.length) {
      const toCopy = Math.min(chunk.length - offset, 64 - this.bufferLength);
      this.buffer.set(chunk.subarray(offset, offset + toCopy), this.bufferLength);
      this.bufferLength += toCopy;
      offset += toCopy;
      if (this.bufferLength === 64) {
        this._processBlock();
        this.bufferLength = 0;
      }
    }
  }

  /** @private */
  _processBlock() {
    const H = this.H;
    const W = this.W;
    const blockBuffer = this.buffer;

    for (let i = 0; i < 16; i++) {
      const j = i * 4;
      W[i] = ((blockBuffer[j] << 24) | (blockBuffer[j + 1] << 16) | (blockBuffer[j + 2] << 8) | (blockBuffer[j + 3])) >>> 0;
    }

    for (let i = 16; i < 64; i++) {
      const s0 = (W[i - 15] >>> 7 | W[i - 15] << 25) ^ (W[i - 15] >>> 18 | W[i - 15] << 14) ^ (W[i - 15] >>> 3);
      const s1 = (W[i - 2] >>> 17 | W[i - 2] << 15) ^ (W[i - 2] >>> 19 | W[i - 2] << 13) ^ (W[i - 2] >>> 10);
      W[i] = (W[i - 16] + s0 + W[i - 7] + s1) | 0;
    }

    let a = H[0], b = H[1], c = H[2], d = H[3];
    let e = H[4], f = H[5], g = H[6], h = H[7];

    for (let i = 0; i < 64; i++) {
      const S1 = (e >>> 6 | e << 26) ^ (e >>> 11 | e << 21) ^ (e >>> 25 | e << 7);
      const ch = (e & f) ^ (~e & g);
      const temp1 = (h + S1 + ch + this.K[i] + W[i]) | 0;
      const S0 = (a >>> 2 | a << 30) ^ (a >>> 13 | a << 19) ^ (a >>> 22 | a << 10);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (S0 + maj) | 0;

      h = g; g = f; f = e; e = (d + temp1) | 0;
      d = c; c = b; b = a; a = (temp1 + temp2) | 0;
    }

    H[0] = (H[0] + a) | 0; H[1] = (H[1] + b) | 0; H[2] = (H[2] + c) | 0; H[3] = (H[3] + d) | 0;
    H[4] = (H[4] + e) | 0; H[5] = (H[5] + f) | 0; H[6] = (H[6] + g) | 0; H[7] = (H[7] + h) | 0;
  }

  /**
   * Finalizes the hash and returns the digest.
   * @param {'hex'|'bytes'} [format='hex'] Output format.
   * @returns {string|Uint8Array} Hex string or byte array.
   * @throws {Error} if called after a previous digest without reset.
   */
  digest(format = 'hex') {
    if (this.finalized) throw new Error('digest() has already been called; call reset() to reuse');

    this.buffer[this.bufferLength++] = 0x80;
    if (this.bufferLength > 56) {
      while (this.bufferLength < 64) this.buffer[this.bufferLength++] = 0;
      this._processBlock();
      this.bufferLength = 0;
    }
    while (this.bufferLength < 56) this.buffer[this.bufferLength++] = 0;

    const bitLength = this.bytesHashed * 8;
    const high = Math.floor(bitLength / 0x100000000);
    const low = bitLength >>> 0;

    this.buffer[56] = (high >>> 24) & 0xff;
    this.buffer[57] = (high >>> 16) & 0xff;
    this.buffer[58] = (high >>> 8) & 0xff;
    this.buffer[59] = high & 0xff;
    this.buffer[60] = (low >>> 24) & 0xff;
    this.buffer[61] = (low >>> 16) & 0xff;
    this.buffer[62] = (low >>> 8) & 0xff;
    this.buffer[63] = low & 0xff;

    this._processBlock();

    let out;
    if (format === 'hex') {
      out = '';
      for (let i = 0; i < 8; i++) {
        out += ('00000000' + (this.H[i] >>> 0).toString(16)).slice(-8);
      }
    } else if (format === 'bytes') {
      out = new Uint8Array(32);
      for (let i = 0; i < 8; i++) {
        out[i*4] = (this.H[i] >>> 24) & 0xff;
        out[i*4+1] = (this.H[i] >>> 16) & 0xff;
        out[i*4+2] = (this.H[i] >>> 8) & 0xff;
        out[i*4+3] = this.H[i] & 0xff;
      }
    } else {
      throw new TypeError(`Unknown digest format: ${format}`);
    }

    this.finalized = true;
    return out;
  }

  /**
   * Creates a deep copy of the current hasher state.
   * Useful for branching hashes without re-hashing.
   * @returns {IncrementalSHA256} Cloned hasher.
   */
  clone() {
    const copy = new IncrementalSHA256();
    copy.H.set(this.H);
    copy.buffer.set(this.buffer);
    copy.bufferLength = this.bufferLength;
    copy.bytesHashed = this.bytesHashed;
    copy.finalized = this.finalized;
    return copy;
  }

  /**
   * Exports the internal state for persistence or resumption.
   * @returns {object} State object.
   */
  exportState() {
    return {
      H: Array.from(this.H, v => v >>> 0),
      buffer: Array.from(this.buffer.subarray(0, this.bufferLength)),
      bufferLength: this.bufferLength >>> 0,
      bytesHashed: this.bytesHashed,
      finalized: !!this.finalized
    };
  }

  /**
   * Imports a previously exported state.
   * @param {object} state - State object from `exportState()`.
   * @param {boolean} [strict=false] - If true, validates all numbers are proper 32-bit integers.
   * @throws {TypeError} on invalid state.
   */
  importState(state, strict = false) {
    if (!state || !Array.isArray(state.H) || state.H.length !== 8) throw new TypeError('Invalid SHA-256 state: H must be array of 8 integers');
    if (!Array.isArray(state.buffer) || state.buffer.length > 64) throw new TypeError('Invalid SHA-256 state: buffer must be array of <=64 bytes');
    const bl = Number(state.bufferLength);
    if (!Number.isInteger(bl) || bl < 0 || bl > 64) throw new TypeError('Invalid SHA-256 state: bufferLength must be integer 0..64');
    const bytesHashed = Number(state.bytesHashed);
    if (!Number.isFinite(bytesHashed) || bytesHashed < 0 || !Number.isSafeInteger(bytesHashed) || bytesHashed > MAX_BYTES_FOR_SAFE_BITLENGTH) {
      throw new TypeError('Invalid SHA-256 state: bytesHashed too large to represent bit-length safely');
    }

    if (strict) {
      for (const v of state.H) {
        if (!Number.isInteger(v) || v < 0 || v > 0xFFFFFFFF) throw new TypeError('Invalid SHA-256 state: H values must be 32-bit unsigned integers');
      }
    }

    this.H.set(state.H.map(v => v >>> 0));
    this.buffer.fill(0);
    this.buffer.set(Uint8Array.from(state.buffer).slice(0, bl));
    this.bufferLength = bl;
    this.bytesHashed = bytesHashed;
    this.finalized = !!state.finalized;
  }
}
