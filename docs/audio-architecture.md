# Audio Engine Architecture

This document explains why the audio stack is built the way it is, and the
reasoning behind each key decision. It is intended for contributors who want
to understand or modify the playback pipeline.

---

## Background — the problem we were solving

macOS CoreAudio logs `HALC_ProxyIOContext::IOWorkLoop: skipping cycle due to
overload` when the render callback misses its deadline. This manifests as
audible glitches or complete silence. The error was reproducible under normal
usage (browsing the library while a track plays) and became systematic on
high-sample-rate interfaces (e.g. Focusrite Scarlett Solo at 176 400 Hz).

Three independent root causes were identified:

### 1 — CoreAudio buffer too small (rodio default)

rodio's `OutputStream::try_default()` calls cpal with `BufferSize::Default`.
On macOS, cpal maps `BufferSize::Default` to nothing — it leaves the HAL
buffer at whatever CoreAudio picks, which on most devices is **128 frames**.

At 48 000 Hz, 128 frames = **2.7 ms**. The render callback must fill the
buffer and return within 2.7 ms, with zero margin for OS scheduling jitter.
Any hiccup (context switch, priority inversion, memory allocator contention)
causes a miss.

**Fix:** Set the HAL-level buffer size directly via `AudioObjectSetPropertyData`
before opening the stream, targeting 100 ms. cpal's `BufferSize::Fixed` is
also set, but it only affects the AudioUnit layer; the HAL buffer (which
controls the actual hardware DMA) must be set separately.

### 2 — Memory allocator call on the real-time audio thread

When a rodio `Sink` exhausts a source or receives a new one, the old
`Vec<f32>` holding ~100 MB of decoded PCM is dropped *on the render callback
thread*. `free()` can block on the global allocator lock (especially under
load), which directly causes the deadline miss.

**Fix:** When a source is replaced or exhausted, the old `Box<dyn Iterator>`
is sent through a dedicated `garbage_tx` channel and freed on the calling
thread (inside `drain_garbage()`, called before every `play_source()` and
`stop()`). The render callback never calls `free()`.

### 3 — rodio Sink overhead on the audio thread

rodio's `Sink` wraps each source in six adaptor layers
(`speed → track_position → pausable → amplify → skippable → stoppable →
periodic_access → convert_samples`). `periodic_access` takes a `Mutex` every
5 ms to check stop/skip state and track the playback position. At 48 000 Hz
this is imperceptible in terms of duration, but the Mutex acquisition itself
can stall under lock contention.

`DynamicMixer::next()` also performs an `AtomicBool::load(SeqCst)` **per
sample** (88 200 loads/s). SeqCst loads are cheap individually but add up and
prevent out-of-order execution on the CPU.

**Fix:** Replace the entire rodio Sink with a hand-written render callback
that does, per callback (~100 ms of audio):
- 3 atomic loads (`stopped`, `paused`, `volume`) with `Relaxed` ordering
- 1 non-blocking `try_recv()` to pick up a new source
- Per sample: one iterator `next()` call + one `f32` multiplication (volume)
- Zero Mutex acquisitions, zero allocations, zero I/O

---

## Stream sample rate — why 48 000 Hz, not the device native rate

High-end interfaces like the Scarlett Solo run at 176 400 Hz. Resampling
44 100 Hz FLAC to 176 400 Hz in software is a 4× upsampling pass — expensive
enough (>1 s on a cold decode) to cause noticeable latency on track changes.

CoreAudio already contains a highly optimised sample rate converter in the
HAL. If we open a stream at 48 000 Hz, CoreAudio handles the 48 000 → 176 400
conversion natively, and our software only needs to do 44 100 → 48 000 (a
1.09× ratio, essentially free with `UniformSourceIterator`).

The constant `STREAM_SAMPLE_RATE = 48_000` is the rate used for the cpal
stream config and for `UniformSourceIterator`. The device's native sample rate
is used **only** for the HAL buffer size calculation (because the HAL counts
frames in the device's clock domain).

---

## NAS / network file latency — background thread

`std::fs::read()` of a 26 MB FLAC from a network share takes 3–4 seconds on
a congested network. If this runs on the UI thread (or even on a thread that
holds the engine lock), the app freezes and the previous track either plays
silence or stalls.

**Fix:** `play()` and `seek()` return immediately after:
1. A fast `Path::exists()` check (a single `stat` syscall — fast even on NAS)
2. Updating the player state so the UI reflects the new track right away
3. Spawning a background thread that reads, decodes, and sends the source

The background thread communicates with the render callback via
`AudioBgHandle` — a lightweight `Send` struct containing only
`Arc<SharedState>` and `mpsc::Sender<BoxedSource>`. `AudioEngine` itself is
`!Send` (it holds a `cpal::Stream` which CoreAudio ties to the creating
thread), so it stays behind the `Mutex<Option<AudioEngine>>`.

The `loading: AtomicBool` field on `PlayerService` is set to `true` before
the thread is spawned and back to `false` when the source reaches the
callback. It is polled by the Swift layer (every 250 ms via `get_player_state`)
to show a loading animation on the progress bar.

---

## Component map

```
PlayerService
├── state: Arc<Mutex<PlayerState>>   # track info, position, queue, volume
├── engine: Arc<Mutex<Option<AudioEngine>>>
│   └── AudioEngine
│       ├── _stream: cpal::Stream    # !Send — stays here forever
│       └── controls: AudioControls
│           ├── shared: Arc<SharedState>   # atomics read by render callback
│           ├── source_tx: Sender<BoxedSource>
│           └── garbage_rx: Mutex<Receiver<BoxedSource>>
└── loading: Arc<AtomicBool>         # true while background thread is active

AudioBgHandle  (Send — used from background file-read thread)
├── shared: Arc<SharedState>         # clone from AudioControls
└── source_tx: Sender<BoxedSource>   # clone from AudioControls
```

---

## Render callback invariants

The closure passed to `cpal::Device::build_output_stream` **must never**:
- Allocate or free heap memory (no `Vec`, `Box`, `String`, `Arc::new`, `drop`)
- Block on a `Mutex` or any synchronisation primitive
- Perform I/O (disk, network, system calls beyond atomic reads)

The current callback captures:
- `current_source: Option<BoxedSource>` — owned, no lock required
- `Arc<SharedState>` — read-only atomics (`Relaxed` ordering is sufficient;
  the audio thread does not need to synchronise with other CPU caches)
- `source_rx: Receiver<BoxedSource>` — `try_recv()` is non-blocking
- `garbage_tx: Sender<BoxedSource>` — `send()` on an unbounded channel is
  non-blocking and allocation-free when the channel has capacity

The only allocation that could occur is inside `source_rx.try_recv()` if the
channel implementation allocates on receive. In practice, `std::sync::mpsc`
pre-allocates nodes and `try_recv()` is allocation-free for the common case.
