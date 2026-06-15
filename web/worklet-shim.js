// AudioWorkletProcessor shim: receives a precompiled WebAssembly.Module from
// the main thread (the worklet scope has no fetch), instantiates the engine,
// and forwards control-message batches. Outputs silence until ready.

import './tc-polyfill.js';
import { initSync, WorkletEngine } from './worklet/rack_worklet.js';

class RackProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.engine = null;
    this.port.onmessageerror = () => {
      this.port.postMessage({ error: 'message failed to deserialize in worklet' });
    };
    this.port.onmessage = (e) => {
      const data = e.data;
      try {
        if (data.wasmBytes) {
          // The main thread sends raw bytes (a plain transferable
          // ArrayBuffer): cloning a compiled WebAssembly.Module to the audio
          // thread fails to deserialize in some Chromium builds, and async
          // instantiate promises are unreliable in AudioWorkletGlobalScope.
          // Sync compile+instantiate is permitted off the main thread and the
          // module is tiny.
          initSync({ module: new WebAssembly.Module(data.wasmBytes) });
          this.engine = new WorkletEngine(sampleRate);
          this.port.postMessage({ ready: true });
        } else if (data.msgs && this.engine) {
          this.engine.on_message(new Uint8Array(data.msgs));
        }
      } catch (err) {
        this.port.postMessage({ error: String(err) });
      }
    };
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    if (this.engine && out.length >= 2) {
      this.engine.process(out[0], out[1]);
      // Meter snapshot back to the UI every 12 quanta (~32 ms at 48 kHz).
      this.quantum = (this.quantum | 0) + 1;
      if (this.quantum % 12 === 0) {
        const meters = this.engine.take_meters();
        this.port.postMessage({ meters: meters.buffer }, [meters.buffer]);
      }
    }
    return true;
  }
}

registerProcessor('rack-processor', RackProcessor);
