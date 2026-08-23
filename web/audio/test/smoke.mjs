// Headless end-to-end test of the WASM audio boundary (backlog 000083).
//
// Verifies the compiled `vb-engine` wasm module through its generated JS bindings — the
// exact Rust → wasm → JS path the browser AudioWorklet uses — without a browser: build the
// synth, feed MIDI, pull render blocks, and assert the audio behaves.
//
// Prerequisite (also what CI runs):
//   wasm-pack build ../../crates/vb-engine --target nodejs --out-dir web/audio/pkg-node --out-name vb_engine
// Then:  node web/audio/test/smoke.mjs
import assert from 'node:assert';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const { WasmSynth } = await import(join(here, '..', 'pkg-node', 'vb_engine.js'));

const SR = 48000;
const block = WasmSynth.blockSize();
assert.strictEqual(block, 128, 'block size must be one WebAudio quantum (128)');

const blockPeak = (b) => {
  assert.strictEqual(b.length, block, 'render returns a full quantum');
  let p = 0;
  for (const s of b) { assert(Number.isFinite(s), 'every sample is finite'); p = Math.max(p, Math.abs(s)); }
  return p;
};
const peakOver = (synth, n) => { let p = 0; for (let i = 0; i < n; i++) p = Math.max(p, blockPeak(synth.render())); return p; };
const lastBlockPeak = (synth, n) => { let p = 0; for (let i = 0; i < n; i++) p = blockPeak(synth.render()); return p; };

// 1. The default patch is silent until a note plays.
const synth = new WasmSynth(SR);
assert(peakOver(synth, 4) < 1e-6, 'default patch is silent at rest');

// 2. A held note makes it audible.
synth.noteOn(69, 127);
assert(peakOver(synth, 8) > 1e-3, 'a held note makes the synth audible');

// 3. After release the voice decays back toward silence (measure the final block).
synth.noteOff(69);
assert(lastBlockPeak(synth, 400) < 1e-3, 'voice decays back toward silence after release');
synth.free();

// 4. A malformed patch is surfaced as a thrown JS error (the path the worklet reports).
assert.throws(() => WasmSynth.fromRon(SR, 'not a patch'), 'malformed RON throws');

console.log('OK: wasm audio boundary verified — silent -> audible -> decay, block=128, fromRon errors cleanly');
