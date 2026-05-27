// emulator-worker.js — Phase B Step 2
//
// Runs the SNES emulator off the main thread. Both audio and framebuffer
// are written directly into SharedArrayBuffers — no postMessage for
// either hot path, zero copies across threads.
//
// Audio: SAB ring buffer (worker writes, AudioWorklet reads)
// Video: SAB single-slot (worker writes, main thread rAF reads)
//
// Loaded as an ES-module worker:  new Worker(url, { type: 'module' })

import init, { Emulator } from './pkg/rsnes.js';

let emulator = null;
let wasmMemory = null;
let loopHandle = null;
let frameSeq = 0;

// Audio ring buffer (SharedArrayBuffer)
let audioRingSAB = null;
let audioRingI16 = null;     // Int16Array view of sample data
let audioControlU32 = null;  // Uint32Array view of write_pos / read_pos
let audioRingCapacity = 0;   // total i16 samples in ring
let droppedSampleCount = 0;  // samples lost to ring overflow (diagnostic)

// Framebuffer (SharedArrayBuffer — single-slot, not a ring)
let fbSAB = null;
let fbSeqU32 = null;   // Uint32Array view of frame_seq counter (offset 0)
let fbU8 = null;       // Uint8Array view of pixel data (offset 4)

const FRAME_MS = 1000 / 60.0988;

async function handleLoad(romBytes) {
    const wasm = await init();
    wasmMemory = wasm.memory;
    emulator = new Emulator(romBytes);
    frameSeq = 0;
    if (fbSeqU32) Atomics.store(fbSeqU32, 0, 0);
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
    const toWrite = Math.min(sampleCount, free > 0 ? free : 0);
    let wp = writePos;
    for (let i = 0; i < toWrite; i++) {
        audioRingI16[wp % cap] = wasmView[i];
        wp++;
    }

    if (toWrite < sampleCount) {
        droppedSampleCount += sampleCount - toWrite;
    }

    Atomics.store(audioControlU32, 0, wp % cap);
    emulator.clear_audio_samples();
}

function tick() {
    if (!emulator) return;

    emulator.run_frame_no_return();
    frameSeq = (frameSeq + 1) >>> 0;  // keep in Uint32 range to match SAB view

    // Write audio samples to the SAB ring (AudioWorklet reads them)
    writeAudioToRing();

    // Framebuffer → SAB (single-slot, main thread rAF reads)
    const fbLen = emulator.framebuffer_len();
    const fbPtr = emulator.framebuffer_ptr();
    const fbView = new Uint8Array(wasmMemory.buffer, fbPtr, fbLen);

    if (fbSAB) {
        // One copy: WASM linear memory → SAB. Main thread reads directly.
        // Note: the pixel write (set) and the seq signal (Atomics.store)
        // are not jointly atomic — on ARM the main thread could read a
        // partially-written frame. This is the same tradeoff bsnes/higan
        // makes: one torn frame per several million is acceptable for video.
        fbU8.set(fbView);
        Atomics.store(fbSeqU32, 0, frameSeq);
    } else {
        // Fallback: postMessage path (before SAB is wired up).
        // In normal init, fb-sab arrives before the first tick(), so this
        // path is unreachable. If it fires, something broke the send order.
        console.warn('framebuffer SAB not wired — falling back to postMessage');
        const fbCopy = new Uint8Array(fbLen);
        fbCopy.set(fbView);
        self.postMessage(
            { type: 'frame', seq: frameSeq, frameCount: emulator.frame_count(), fb: fbCopy },
            [fbCopy.buffer]
        );
    }

    // Periodic status update (no pixel data — just counters)
    if (frameSeq % 60 === 0) {
        self.postMessage({ type: 'status', seq: frameSeq, frameCount: emulator.frame_count() });
    }
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
        case 'fb-sab':
            // Main thread sends the SharedArrayBuffer for the framebuffer
            fbSAB = msg.sab;
            fbSeqU32 = new Uint32Array(fbSAB, 0, 1);
            fbU8 = new Uint8Array(fbSAB, 4, 256 * 224 * 4);
            break;
        default:
            break;
    }
};
