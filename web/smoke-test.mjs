// Node smoke test for the worklet wasm: replays exactly what worklet-shim.js
// does — compile the wasm, init the glue with a precompiled module, build the
// engine, render quanta — and asserts on the audio. Run: node smoke-test.mjs
import { readFile } from 'node:fs/promises';
import init, { WorkletEngine } from './worklet/rack_worklet.js';

const wasmBytes = await readFile(new URL('./worklet/rack_worklet_bg.wasm', import.meta.url));
const module = await WebAssembly.compile(wasmBytes);
await init({ module_or_path: module });

const sampleRate = 48000;
const engine = new WorkletEngine(sampleRate);

const left = new Float32Array(128);
const right = new Float32Array(128);

// Let parameter smoothing settle, then measure.
for (let i = 0; i < 100; i++) engine.process(left, right);

let peak = 0;
let crossings = 0;
let last = 0;
const quanta = 200; // ≈ 0.533 s
for (let q = 0; q < quanta; q++) {
  engine.process(left, right);
  for (const s of left) {
    if (!Number.isFinite(s)) throw new Error(`non-finite sample: ${s}`);
    if (Math.abs(s) > 1.0) throw new Error(`sample out of range: ${s}`);
    peak = Math.max(peak, Math.abs(s));
    if (last < 0 && s >= 0) crossings++;
    last = s;
  }
}

const seconds = (quanta * 128) / sampleRate;
const hz = crossings / seconds;
console.log(`peak ${peak.toFixed(3)}, est. frequency ${hz.toFixed(1)} Hz`);
if (peak < 0.1) throw new Error('output near-silent');
if (Math.abs(hz - 440) > 10) throw new Error(`expected ~440 Hz, got ${hz}`);

// Drive pitch up an octave via the message path (SetParam module 0, param 0).
const msg = new ArrayBuffer(32);
const dv = new DataView(msg);
dv.setUint32(0, 1, true); // tag = SetParam
dv.setUint32(4, 0, true); // module 0 (VCO)
dv.setUint32(8, 0, true); // param 0 (pitch)
dv.setFloat32(16, 1.75, true); // 1.75 V/oct = A5
engine.on_message(new Uint8Array(msg));
for (let i = 0; i < 100; i++) engine.process(left, right); // settle

crossings = 0;
last = 0;
for (let q = 0; q < quanta; q++) {
  engine.process(left, right);
  for (const s of left) {
    if (last < 0 && s >= 0) crossings++;
    last = s;
  }
}
const hz2 = crossings / seconds;
console.log(`after SetParam: est. frequency ${hz2.toFixed(1)} Hz`);
if (Math.abs(hz2 - 880) > 20) throw new Error(`expected ~880 Hz, got ${hz2}`);

console.log('worklet wasm smoke test PASSED');
