// Main-thread glue: load the WASM synth into an AudioWorklet and play it.
//
// The synth runs entirely inside the AudioWorklet (the real-time audio thread). This file
// only fetches the wasm, assembles the worklet module, wires a QWERTY keyboard to MIDI
// note messages, and exposes a headless-verifiable self-test.
const GLUE = 'pkg-nomod/seclab_engine.js';
const WASM = 'pkg-nomod/vb_engine_bg.wasm';

// QWERTY → MIDI (roughly one octave from 'a', with the black keys on the upper row).
const KEYMAP = { a: 60, w: 61, s: 62, e: 63, d: 64, f: 65, t: 66, g: 67, y: 68, h: 69, u: 70, j: 71, k: 72 };

let ctx, node, analyser, started = false;
const held = new Set();
const $ = (id) => document.getElementById(id);
const setStatus = (t) => { $('status').textContent = t; };

async function start() {
  if (started) return;
  const [glueText, wasmBytes] = await Promise.all([
    fetch(GLUE).then((r) => r.text()),
    fetch(WASM).then((r) => r.arrayBuffer()),
  ]);
  ctx = new AudioContext();
  await ctx.resume();
  // The worklet module is assembled from three scripts, in order:
  //   1. text-polyfill.js — AudioWorkletGlobalScope has no TextEncoder/TextDecoder,
  //      which the wasm-bindgen glue needs.
  //   2. the wasm-bindgen `no-modules` glue (defines the `wasm_bindgen` global).
  //   3. seclab-processor.js — our AudioWorkletProcessor, which uses `wasm_bindgen`.
  // They are concatenated into one Blob because AudioWorkletGlobalScope supports neither
  // ES `import` nor `importScripts`.
  const [polyfillText, processorText] = await Promise.all([
    fetch('text-polyfill.js').then((r) => r.text()),
    fetch('seclab-processor.js').then((r) => r.text()),
  ]);
  const blob = new Blob([polyfillText, '\n', glueText, '\n', processorText], { type: 'application/javascript' });
  await ctx.audioWorklet.addModule(URL.createObjectURL(blob));
  // The wasm bytes are handed to the processor's constructor; it instantiates the module
  // inside the worklet via `wasm_bindgen.initSync`.
  node = new AudioWorkletNode(ctx, 'vb-synth', { processorOptions: { wasmBytes } });
  analyser = ctx.createAnalyser();
  analyser.fftSize = 2048;
  node.connect(analyser);
  analyser.connect(ctx.destination);
  started = true;
  setStatus(`running @ ${ctx.sampleRate} Hz`);
  $('selftest').disabled = false;
}

function noteOn(note) {
  if (node && !held.has(note)) { held.add(note); node.port.postMessage({ type: 'noteOn', note, velocity: 110 }); }
}
function noteOff(note) {
  if (node && held.has(note)) { held.delete(note); node.port.postMessage({ type: 'noteOff', note }); }
}

function measurePeak() {
  const buf = new Float32Array(analyser.fftSize);
  analyser.getFloatTimeDomainData(buf);
  let p = 0;
  for (const s of buf) p = Math.max(p, Math.abs(s));
  return p;
}

// Headless-verifiable self-test: play A4 briefly, sample the analyser, report the peak.
// Lets an automated browser confirm the whole path makes sound without needing to listen.
async function selfTest() {
  $('peak').textContent = 'testing…';
  noteOn(69);
  await new Promise((r) => setTimeout(r, 250));
  const peak = measurePeak();
  noteOff(69);
  const verdict = peak > 0.001 ? 'PASS' : 'FAIL';
  $('peak').textContent = `peak=${peak.toFixed(4)} ${verdict}`;
  $('peak').dataset.verdict = verdict;
  $('peak').dataset.peak = peak.toFixed(4);
}

window.addEventListener('keydown', (e) => { const n = KEYMAP[e.key]; if (n && !e.repeat) noteOn(n); });
window.addEventListener('keyup', (e) => { const n = KEYMAP[e.key]; if (n) noteOff(n); });
$('start').addEventListener('click', () => start().catch((err) => setStatus('error: ' + err.message)));
$('selftest').addEventListener('click', () => selfTest().catch((err) => { $('peak').textContent = 'error: ' + err.message; }));
