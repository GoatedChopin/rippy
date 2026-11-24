// Runs in QuickJS before alphaTab loads (src/songsterr.rs).
(function () {
  'use strict';

  // alphaTab picks its platform at load time and only stays off browser APIs (window, document,
  // canvas, workers) when it thinks it's in Node, which it checks with
  // Object.prototype.toString.call(process).
  if (typeof globalThis.process === 'undefined') {
    globalThis.process = { [Symbol.toStringTag]: 'process', env: {}, versions: {} };
  }

  // alphaTab logs warnings about the score (an unusual bend, say) through console.
  if (typeof globalThis.console === 'undefined') {
    const quiet = () => {};
    globalThis.console = { log: quiet, debug: quiet, info: quiet, warn: quiet, error: quiet, trace: quiet };
  }

  // QuickJS has no TextEncoder or TextDecoder, and alphaTab runs megabytes of XML through them, so
  // Rippy provides UTF-8 in Rust as __rippyUtf8Encode(string) and __rippyUtf8Decode(Uint8Array).
  if (typeof globalThis.TextEncoder === 'undefined') {
    const encode = globalThis.__rippyUtf8Encode;
    globalThis.TextEncoder = class {
      encode(text) { return encode(String(text ?? '')); }
    };
  }
  if (typeof globalThis.TextDecoder === 'undefined') {
    const decode = globalThis.__rippyUtf8Decode;
    globalThis.TextDecoder = class {
      decode(bytes) { return decode(bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes)); }
    };
  }
})();
