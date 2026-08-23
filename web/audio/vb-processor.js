// AudioWorkletProcessor for the VST_bomb WASM synth. This file is CONCATENATED after the
// wasm-bindgen `no-modules` glue at load time, so `wasm_bindgen` is in scope here.
class VbSynthProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const { wasmBytes } = options.processorOptions;
    wasm_bindgen.initSync({ module: wasmBytes });
    this.synth = new wasm_bindgen.WasmSynth(sampleRate); // `sampleRate` is a worklet global
    this.port.onmessage = (e) => {
      const m = e.data;
      if (m.type === 'noteOn') this.synth.noteOn(m.note, m.velocity ?? 100);
      else if (m.type === 'noteOff') this.synth.noteOff(m.note);
    };
  }
  process(_inputs, outputs) {
    const out = outputs[0];
    const block = this.synth.render();          // Float32Array, length 128 = one quantum
    for (let ch = 0; ch < out.length; ch++) out[ch].set(block);
    return true;                                 // keep the processor alive
  }
}
registerProcessor('vb-synth', VbSynthProcessor);
