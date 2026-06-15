// The AudioWorkletGlobalScope lacks TextDecoder/TextEncoder, which the
// wasm-bindgen glue requires (for string passing in error paths). Minimal
// UTF-8 implementations, installed only if missing.

if (typeof globalThis.TextDecoder === 'undefined') {
  globalThis.TextDecoder = class TextDecoder {
    constructor(_label, _options) {}
    decode(input) {
      if (!input) return '';
      const bytes =
        input instanceof Uint8Array
          ? input
          : new Uint8Array(input.buffer || input, input.byteOffset || 0, input.byteLength);
      let s = '';
      let i = 0;
      while (i < bytes.length) {
        const b = bytes[i++];
        if (b < 0x80) {
          s += String.fromCharCode(b);
        } else if (b < 0xe0) {
          s += String.fromCharCode(((b & 0x1f) << 6) | (bytes[i++] & 0x3f));
        } else if (b < 0xf0) {
          s += String.fromCharCode(
            ((b & 0x0f) << 12) | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f)
          );
        } else {
          let cp =
            ((b & 0x07) << 18) |
            ((bytes[i++] & 0x3f) << 12) |
            ((bytes[i++] & 0x3f) << 6) |
            (bytes[i++] & 0x3f);
          cp -= 0x10000;
          s += String.fromCharCode(0xd800 + (cp >> 10), 0xdc00 + (cp & 0x3ff));
        }
      }
      return s;
    }
  };
}

if (typeof globalThis.TextEncoder === 'undefined') {
  globalThis.TextEncoder = class TextEncoder {
    encode(s) {
      const out = [];
      for (let i = 0; i < s.length; i++) {
        const cp = s.codePointAt(i);
        if (cp > 0xffff) i++;
        if (cp < 0x80) out.push(cp);
        else if (cp < 0x800) out.push(0xc0 | (cp >> 6), 0x80 | (cp & 0x3f));
        else if (cp < 0x10000)
          out.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
        else
          out.push(
            0xf0 | (cp >> 18),
            0x80 | ((cp >> 12) & 0x3f),
            0x80 | ((cp >> 6) & 0x3f),
            0x80 | (cp & 0x3f)
          );
      }
      return new Uint8Array(out);
    }
    encodeInto(s, target) {
      const bytes = this.encode(s);
      const written = Math.min(bytes.length, target.length);
      target.set(bytes.subarray(0, written));
      return { read: s.length, written };
    }
  };
}
