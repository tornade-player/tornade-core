// Custom audio engine using cpal directly to eliminate CoreAudio buffer overloads.
//
// Root causes of "HALC_ProxyIOContext: skipping cycle due to overload":
//
// 1. Buffer too small — rodio uses BufferSize::Default → CoreAudio picks a tiny
//    buffer (e.g. 128 frames). We compute a buffer targeting ~100 ms based on the
//    device's actual sample rate and set it at the HAL level.
//
// 2. free() on the audio thread — when a ~100 MB Vec<f32> source is exhausted or
//    replaced, dropping it calls free() which can block on the allocator lock.
//    We send exhausted sources through a "garbage" channel for deallocation on the
//    caller's thread, never on the real-time audio thread.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleRate, StreamConfig, SupportedBufferSize};
use log::{info, warn};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed, Ordering::Release};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Target buffer duration in milliseconds. 100 ms gives ample headroom for
/// the real-time audio callback on any device (USB interfaces, Bluetooth, etc.)
/// while remaining imperceptible for music playback (not live monitoring).
const TARGET_BUFFER_MS: f64 = 100.0;

/// Stream sample rate used for the cpal output stream. We decode/resample to
/// this rate in software (cheap: 44100→48000 is a 1.09× ratio) and let CoreAudio
/// handle the final conversion to the device's native rate (e.g. 48000→176400)
/// in optimised native code. This avoids the expensive 4× software upsampling
/// that made song changes slow on high-sample-rate interfaces like the Scarlett Solo.
const STREAM_SAMPLE_RATE: u32 = 48000;

// ---------------------------------------------------------------------------
// macOS HAL-level buffer size control
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
mod hal {
    use std::os::raw::c_void;

    type AudioObjectID = u32;
    type AudioObjectPropertySelector = u32;
    type AudioObjectPropertyScope = u32;
    type AudioObjectPropertyElement = u32;
    type OSStatus = i32;

    #[repr(C)]
    struct AudioObjectPropertyAddress {
        m_selector: AudioObjectPropertySelector,
        m_scope: AudioObjectPropertyScope,
        m_element: AudioObjectPropertyElement,
    }

    const HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE: AudioObjectPropertySelector = 0x646F_7574; // 'dout'
    const DEVICE_PROPERTY_BUFFER_FRAME_SIZE: AudioObjectPropertySelector = 0x6673_697A; // 'fsiz'
    const PROPERTY_SCOPE_GLOBAL: AudioObjectPropertyScope = 0x676C_6F62; // 'glob'
    const PROPERTY_ELEMENT_MAIN: AudioObjectPropertyElement = 0;
    const SYSTEM_OBJECT: AudioObjectID = 1;

    unsafe extern "C" {
        fn AudioObjectGetPropertyData(
            in_object_id: AudioObjectID,
            in_address: *const AudioObjectPropertyAddress,
            in_qualifier_data_size: u32,
            in_qualifier_data: *const c_void,
            io_data_size: *mut u32,
            out_data: *mut c_void,
        ) -> OSStatus;

        fn AudioObjectSetPropertyData(
            in_object_id: AudioObjectID,
            in_address: *const AudioObjectPropertyAddress,
            in_qualifier_data_size: u32,
            in_qualifier_data: *const c_void,
            in_data_size: u32,
            in_data: *const c_void,
        ) -> OSStatus;
    }

    /// Set the HAL-level buffer size on the default output device.
    /// Returns the actual buffer size after the call (may differ from requested).
    pub fn set_default_output_buffer_size(frames: u32) -> Result<u32, String> {
        unsafe {
            let mut device_id: AudioObjectID = 0;
            let mut size = std::mem::size_of::<AudioObjectID>() as u32;
            let addr = AudioObjectPropertyAddress {
                m_selector: HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE,
                m_scope: PROPERTY_SCOPE_GLOBAL,
                m_element: PROPERTY_ELEMENT_MAIN,
            };

            let status = AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &addr,
                0,
                std::ptr::null(),
                &mut size,
                &mut device_id as *mut _ as *mut c_void,
            );
            if status != 0 {
                return Err(format!(
                    "get default output device failed: OSStatus {status}"
                ));
            }

            let buf_addr = AudioObjectPropertyAddress {
                m_selector: DEVICE_PROPERTY_BUFFER_FRAME_SIZE,
                m_scope: PROPERTY_SCOPE_GLOBAL,
                m_element: PROPERTY_ELEMENT_MAIN,
            };

            // Read current value first
            let mut current: u32 = 0;
            let mut sz = std::mem::size_of::<u32>() as u32;
            let _ = AudioObjectGetPropertyData(
                device_id,
                &buf_addr,
                0,
                std::ptr::null(),
                &mut sz,
                &mut current as *mut _ as *mut c_void,
            );

            let status = AudioObjectSetPropertyData(
                device_id,
                &buf_addr,
                0,
                std::ptr::null(),
                std::mem::size_of::<u32>() as u32,
                &frames as *const _ as *const c_void,
            );
            if status != 0 {
                return Err(format!(
                    "set buffer size {frames} failed (was {current}): OSStatus {status}"
                ));
            }

            // Read back to confirm
            let mut actual: u32 = 0;
            let mut sz = std::mem::size_of::<u32>() as u32;
            let status = AudioObjectGetPropertyData(
                device_id,
                &buf_addr,
                0,
                std::ptr::null(),
                &mut sz,
                &mut actual as *mut _ as *mut c_void,
            );
            if status != 0 {
                return Err(format!(
                    "read back buffer size failed: OSStatus {status}"
                ));
            }

            Ok(actual)
        }
    }
}

// ---------------------------------------------------------------------------
// Shared state (atomics only — safe for real-time audio thread)
// ---------------------------------------------------------------------------

pub struct SharedState {
    pub paused: AtomicBool,
    pub stopped: AtomicBool,
    pub finished: AtomicBool,
    pub volume: AtomicU32,            // f32 bits stored as u32
    pub callback_frames: AtomicU32,   // actual frames per callback (set once)
}

impl SharedState {
    fn new(volume: f32) -> Self {
        SharedState {
            paused: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            volume: AtomicU32::new(volume.to_bits()),
            callback_frames: AtomicU32::new(0),
        }
    }
}

type BoxedSource = Box<dyn Iterator<Item = f32> + Send>;

// ---------------------------------------------------------------------------
// AudioControls — remote control (Send + Sync), used by PlayerService
// ---------------------------------------------------------------------------

pub struct AudioControls {
    shared: Arc<SharedState>,
    source_tx: mpsc::Sender<BoxedSource>,
    garbage_rx: Mutex<mpsc::Receiver<BoxedSource>>,
    device_channels: u16,
    device_sample_rate: u32,
}

impl AudioControls {
    fn drain_garbage(&self) {
        let rx = self.garbage_rx.lock().unwrap();
        while rx.try_recv().is_ok() {}
    }

    pub fn play_source(&self, source: BoxedSource) {
        self.drain_garbage();
        self.shared.stopped.store(false, Relaxed);
        self.shared.paused.store(false, Relaxed);
        self.shared.finished.store(false, Release);
        let _ = self.source_tx.send(source);
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Relaxed);
    }

    pub fn stop(&self) {
        self.shared.stopped.store(true, Relaxed);
        self.drain_garbage();
    }

    pub fn set_volume(&self, volume: f32) {
        self.shared.volume.store(volume.to_bits(), Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.drain_garbage();
        self.shared.finished.load(Relaxed)
    }

    pub fn device_config(&self) -> (u16, u32) {
        (self.device_channels, self.device_sample_rate)
    }

    /// Return a lightweight, Send-able handle for use in background threads.
    /// Allows sending sources and setting volume/state without touching the
    /// engine (which holds a !Send cpal::Stream).
    pub fn bg_handle(&self) -> AudioBgHandle {
        AudioBgHandle {
            shared: Arc::clone(&self.shared),
            source_tx: self.source_tx.clone(),
        }
    }
}

/// Lightweight Send handle for background file-read threads.
pub struct AudioBgHandle {
    shared: Arc<SharedState>,
    source_tx: mpsc::Sender<BoxedSource>,
}

impl AudioBgHandle {
    pub fn play_source(&self, source: BoxedSource) {
        self.shared.stopped.store(false, Relaxed);
        self.shared.paused.store(false, Relaxed);
        self.shared.finished.store(false, Release);
        let _ = self.source_tx.send(source);
    }

    pub fn set_volume(&self, volume: f32) {
        self.shared.volume.store(volume.to_bits(), Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Diagnostic — write engine config to /tmp for debugging
// ---------------------------------------------------------------------------

fn write_diag(msg: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("tornade-audio-diag.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(f, "[{now}] {msg}");
    }
}

// ---------------------------------------------------------------------------
// AudioEngine — owns the cpal Stream (!Send on macOS)
// ---------------------------------------------------------------------------

pub struct AudioEngine {
    _stream: cpal::Stream,
    controls: AudioControls,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        write_diag("AudioEngine::new() starting");

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("No audio output device found")?;

        let device_name = device.name().unwrap_or_else(|_| "unknown".into());

        let supported = device
            .default_output_config()
            .map_err(|e| format!("Failed to get default output config: {e}"))?;

        let channels = supported.channels();
        let device_sample_rate = supported.sample_rate().0;

        // Use a fixed stream rate — CoreAudio converts to the device's native rate
        let stream_rate = STREAM_SAMPLE_RATE;

        // Compute target buffer frames for ~100 ms at the STREAM rate
        let target_frames = (stream_rate as f64 * TARGET_BUFFER_MS / 1000.0) as u32;

        write_diag(&format!(
            "device={device_name}, {channels}ch, device={device_sample_rate} Hz, \
             stream={stream_rate} Hz, target={target_frames} frames ({TARGET_BUFFER_MS} ms)"
        ));

        // On macOS, set the HAL-level buffer size BEFORE opening the stream.
        // Use the DEVICE sample rate for HAL (it controls the hardware clock).
        let hal_target = (device_sample_rate as f64 * TARGET_BUFFER_MS / 1000.0) as u32;
        #[cfg(target_os = "macos")]
        match hal::set_default_output_buffer_size(hal_target) {
            Ok(actual) => {
                let ms = actual as f64 / device_sample_rate as f64 * 1000.0;
                write_diag(&format!("HAL buffer size: requested={hal_target}, actual={actual} ({ms:.1} ms)"));
                info!("AudioEngine: HAL buffer size set to {actual} frames ({ms:.1} ms)");
            }
            Err(e) => {
                write_diag(&format!("HAL buffer size FAILED: {e}"));
                warn!("AudioEngine: failed to set HAL buffer size: {e}");
            }
        }

        // Clamp to device's supported range for cpal config
        let cpal_buffer_size = match supported.buffer_size() {
            SupportedBufferSize::Range { min, max } => {
                let clamped = target_frames.clamp(*min, *max);
                write_diag(&format!("cpal buffer: {clamped} frames (range {min}–{max})"));
                BufferSize::Fixed(clamped)
            }
            SupportedBufferSize::Unknown => {
                write_diag(&format!("cpal buffer: range unknown, requesting {target_frames}"));
                BufferSize::Fixed(target_frames)
            }
        };

        let config = StreamConfig {
            channels,
            sample_rate: SampleRate(stream_rate),
            buffer_size: cpal_buffer_size,
        };

        let shared = Arc::new(SharedState::new(1.0));
        let (source_tx, source_rx) = mpsc::channel::<BoxedSource>();
        let (garbage_tx, garbage_rx) = mpsc::channel::<BoxedSource>();

        let cb_shared = Arc::clone(&shared);
        let mut current_source: Option<BoxedSource> = None;

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // Record actual callback buffer size (once)
                    if cb_shared.callback_frames.load(Relaxed) == 0 {
                        let frames = data.len() as u32 / channels as u32;
                        cb_shared.callback_frames.store(frames, Relaxed);
                    }

                    if cb_shared.stopped.load(Relaxed) {
                        if let Some(old) = current_source.take() {
                            let _ = garbage_tx.send(old);
                        }
                        data.fill(0.0);
                        return;
                    }

                    while let Ok(src) = source_rx.try_recv() {
                        if let Some(old) = current_source.take() {
                            let _ = garbage_tx.send(old);
                        }
                        current_source = Some(src);
                        cb_shared.finished.store(false, Release);
                    }

                    let vol = f32::from_bits(cb_shared.volume.load(Relaxed));

                    if cb_shared.paused.load(Relaxed) || current_source.is_none() {
                        data.fill(0.0);
                        return;
                    }

                    let src = current_source.as_mut().unwrap();
                    let mut done = false;
                    for sample in data.iter_mut() {
                        if done {
                            *sample = 0.0;
                        } else if let Some(s) = src.next() {
                            *sample = s * vol;
                        } else {
                            *sample = 0.0;
                            done = true;
                        }
                    }
                    if done {
                        if let Some(old) = current_source.take() {
                            let _ = garbage_tx.send(old);
                        }
                        cb_shared.finished.store(true, Release);
                    }
                },
                move |err| {
                    warn!("AudioEngine: stream error: {err}");
                },
                None,
            )
            .map_err(|e| format!("Failed to build output stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start audio stream: {e}"))?;

        // Log callback frames after a short delay (the first callback will have fired)
        let diag_shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let actual = diag_shared.callback_frames.load(Relaxed);
            let ms = if stream_rate > 0 {
                actual as f64 / stream_rate as f64 * 1000.0
            } else {
                0.0
            };
            write_diag(&format!(
                "first callback: {actual} frames ({ms:.1} ms) — \
                 {channels}ch @ {stream_rate} Hz (device native: {device_sample_rate} Hz)"
            ));
        });

        write_diag("AudioEngine::new() complete — stream playing");

        Ok(AudioEngine {
            _stream: stream,
            controls: AudioControls {
                shared,
                source_tx,
                garbage_rx: Mutex::new(garbage_rx),
                device_channels: channels,
                device_sample_rate: stream_rate, // stream rate, not device native
            },
        })
    }

    pub fn controls(&self) -> &AudioControls {
        &self.controls
    }
}
