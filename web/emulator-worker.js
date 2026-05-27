// emulator-worker.js — Phase B Step 3
//
// Runs the SNES emulator off the main thread. Audio samples are written
// directly into a SharedArrayBuffer ring that the AudioWorklet processor
// reads from — no postMessage for audio, zero copies across threads.
//
// Framebuffer is still sent via postMessage (transferable ArrayBuffer).
// Phase B Step 2 will move that to SAB as well.
//
// Loaded as an ES-module worker:  new Worker(url, { type: 'module' })

import init, { Emulator } from './pkg/zelda_a_link_to_the_past.js';

let emulator = null;
let wasmMemory = null;
let loopHandle = null;
let frameSeq = 0;

// Audio ring buffer (SharedArrayBuffer)
let audioRingSAB = null;
let audioRingI16 = null;     // Int16Array view of sample data
let audioControlU32 = null;  // Uint32Array view of write_pos / read_pos
let audioRingCapacity = 0;   // total i16 samples in ring

const FRAME_MS = 1000 / 60.0988;

async function handleLoad(romBytes) {
    const wasm = await init();
    wasmMemory = wasm.memory;
    emulator = new Emulator(romBytes);
    self.postMessage({ type: 'ready' });
}

function writeAudioToRing() {
    if (!audioRingSAB || !emulator) return;

    // Zero-copy read from WASM linear memory
    const sampleCount = emulator.audio_samples_len();
    if (sampleCount === 0) return;

    const ptr = emulator.audio_samples_ptr();
    const wasmView = new Int16Array(wasmMemory.buffer, ptr, sampleCount);

    const writePos = Atomics.load(audioControlU32, 0);
    const readPos = Atomics.load(audioControlU32, 1);
    const cap = audioRingCapacity;

    // Available space (leave 2 samples gap to distinguish full from empty)
    const used = (writePos - readPos + cap) % cap;
    const free = cap - used - 2;

    // Write as many samples as we have space for
    const toWrite = Math.min(sampleCount, free);
    let wp = writePos;
    for (let i = 0; i < toWrite; i++) {
        audioRingI16[wp % cap] = wasmView[i];
        wp++;
    }

    Atomics.store(audioControlU32, 0, wp % cap);
    emulator.clear_audio_samples();
}

function tick() {
    if (!emulator) return;

    emulator.run_frame_no_return();
    frameSeq++;

    // Write audio samples to the SAB ring (AudioWorklet reads them)
    writeAudioToRing();

    // Framebuffer: still via postMessage (Step 2 will move to SAB)
    const fbLen = emulator.framebuffer_len();
    const fbPtr = emulator.framebuffer_ptr();
    const fbView = new Uint8Array(wasmMemory.buffer, fbPtr, fbLen);
    const fbCopy = new Uint8Array(fbLen);
    fbCopy.set(fbView);

    self.postMessage(
        {
            type: 'frame',
            seq: frameSeq,
            frameCount: emulator.frame_count(),
            fb: fbCopy,
        },
        [fbCopy.buffer]
    );
}

function startLoop() {
    if (loopHandle !== null || !emulator) return;
    loopHandle = setInterval(tick, FRAME_MS);
}

function stopLoop() {
    if (loopHandle !== null) {
        clearInterval(loopHandle);
        loopHandle = null;
    }
}

self.onmessage = async (ev) => {
    const msg = ev.data;
    switch (msg.type) {
        case 'load':
            await handleLoad(msg.rom);
            break;
        case 'start':
            startLoop();
            break;
        case 'stop':
            stopLoop();
            break;
        case 'input':
            if (emulator) emulator.set_button(msg.button, msg.pressed);
            break;
        case 'audio-ring':
            // Main thread sends the SharedArrayBuffer for the audio ring
            audioRingSAB = msg.sab;
            audioControlU32 = new Uint32Array(audioRingSAB, 0, 2);
            audioRingI16 = new Int16Array(audioRingSAB, 8);
            audioRingCapacity = audioRingI16.length;
            break;
        default:
            break;
    }
};
