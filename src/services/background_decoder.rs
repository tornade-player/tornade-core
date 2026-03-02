// Background audio decoder — decouples symphonia decompression from the CoreAudio
// render callback to prevent "HALC_ProxyIOContext::IOWorkLoop: skipping cycle due to
// overload" dropouts when the UI is under CPU pressure (album navigation, artwork
// loading, database queries, etc.).

use rodio::Source;
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

/// Number of samples to pre-decode **synchronously** before handing the source to
/// the sink when starting a new track.
///
/// At 44 100 Hz stereo this equals ~0.5 s.  It guarantees the first batch of
/// CoreAudio render callbacks always find data in the buffer, absorbing the CPU
/// pressure caused by album-artwork fetching and database queries that happen
/// concurrently with track-switching.
pub const PLAY_PREBUFFER_SAMPLES: usize = 44_100;

/// Smaller pre-buffer used after a seek (the seek itself is user-triggered, so
/// CPU contention is lower and we want minimal added latency).
///
/// At 44 100 Hz stereo this equals ~46 ms.
pub const SEEK_PREBUFFER_SAMPLES: usize = 4_096;

/// Samples per chunk sent from the decoder thread to the audio thread.
///
/// 4 096 samples ≈ 46 ms at 44.1 kHz stereo.  Small enough for the sink to stop
/// the decoder promptly when a track changes, large enough to keep channel
/// overhead negligible.
const CHUNK_SIZE: usize = 4_096;

/// Maximum number of chunks buffered in the channel at once.
///
/// 2 048 chunks × 4 096 samples = ~8 M samples ≈ 93 s at 44.1 kHz stereo.
/// The `sync_channel` back-pressure naturally throttles the decoder thread so it
/// stays at most ~93 s ahead, preventing runaway memory use on long tracks.
const CHANNEL_CAPACITY: usize = 2_048;

/// Wraps a rodio [`Source`] and moves decoding to a dedicated OS thread.
///
/// ## Why this exists
///
/// rodio's default behaviour drives `source.next()` from whichever thread cpal
/// uses for the CoreAudio render callback.  If `next()` blocks on disk I/O or
/// CPU-heavy decompression (FLAC via symphonia), CoreAudio logs:
///
/// > `HALC_ProxyIOContext::IOWorkLoop: skipping cycle due to overload`
///
/// …and skips the audio cycle, producing an audible dropout.
///
/// `BackgroundDecoder` moves the heavy work to a thread named `"tornade-decoder"`.
/// The render callback only calls `VecDeque::pop_front()`, which is O(1) and
/// allocation-free — safe for a real-time audio context.
///
/// ## Usage
///
/// ```no_run
/// # use tornade_core::services::background_decoder::{BackgroundDecoder, PLAY_PREBUFFER_SAMPLES};
/// // decoder: rodio::Decoder<_>, sink: rodio::Sink
/// # fn example(decoder: rodio::Decoder<std::io::BufReader<std::fs::File>>, sink: rodio::Sink) {
/// use rodio::Source;
/// let mut bg = BackgroundDecoder::new(decoder.convert_samples::<f32>());
/// bg.prebuffer(PLAY_PREBUFFER_SAMPLES); // fill ~0.5 s before first callback
/// sink.append(bg);
/// # }
/// ```
pub struct BackgroundDecoder {
    /// Receives decoded sample chunks from the background thread.
    receiver: Receiver<Vec<f32>>,
    /// Local sample queue served directly to the CoreAudio render callback.
    /// Refilled from `receiver` whenever it drains.
    pending: VecDeque<f32>,
    channels: u16,
    sample_rate: u32,
    total_duration: Option<Duration>,
}

impl BackgroundDecoder {
    /// Spawns the decoder thread and returns a ready-to-use source.
    ///
    /// Call [`prebuffer`] immediately after to guarantee the audio callback
    /// never underruns on the very first render cycle.
    pub fn new<S>(source: S) -> Self
    where
        S: Source<Item = f32> + Send + 'static,
    {
        let channels = source.channels();
        let sample_rate = source.sample_rate();
        let total_duration = source.total_duration();

        // `sync_channel` applies back-pressure: the sender blocks when the
        // channel is full, throttling the decoder to at most
        // CHANNEL_CAPACITY × CHUNK_SIZE samples ahead of the audio thread.
        let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);

        std::thread::Builder::new()
            .name("tornade-decoder".into())
            .spawn(move || decode_loop(source, tx))
            .expect("failed to spawn decoder thread");

        Self {
            receiver: rx,
            pending: VecDeque::new(),
            channels,
            sample_rate,
            total_duration,
        }
    }

    /// Pre-fills the local sample buffer by blocking until at least
    /// `target_samples` are available.
    ///
    /// Must be called before handing `self` to the sink.  Without this, the OS
    /// scheduler might not give the decoder thread any time to run before the
    /// first CoreAudio render callback fires, causing an immediate underrun.
    pub fn prebuffer(&mut self, target_samples: usize) {
        while self.pending.len() < target_samples {
            match self.receiver.recv() {
                Ok(chunk) => self.pending.extend(chunk),
                // Short track: decoder finished before we reached the target.
                // That is fine — we will serve whatever we have.
                Err(_) => break,
            }
        }
    }
}

/// Reads samples from `source` in chunks and sends them through `tx`.
///
/// Runs on the `"tornade-decoder"` background thread.  Exits cleanly when:
/// - The source is exhausted (track finished), or
/// - The receiver is dropped (track changed / playback stopped).
fn decode_loop<S>(source: S, tx: SyncSender<Vec<f32>>)
where
    S: Source<Item = f32>,
{
    let mut chunk = Vec::with_capacity(CHUNK_SIZE);

    for sample in source {
        chunk.push(sample);

        if chunk.len() >= CHUNK_SIZE {
            // `sync_channel::send` blocks when the channel is full — this is
            // the intentional back-pressure mechanism.  `is_err()` means the
            // Receiver was dropped (song changed, playback stopped): exit cleanly.
            if tx
                .send(std::mem::replace(
                    &mut chunk,
                    Vec::with_capacity(CHUNK_SIZE),
                ))
                .is_err()
            {
                return;
            }
        }
    }

    // Flush the final partial chunk so the last few samples are not lost.
    if !chunk.is_empty() {
        let _ = tx.send(chunk); // ignore error — receiver may already be gone
    }
    // `tx` is dropped here → once `rx` drains it, `try_recv` returns
    // `TryRecvError::Disconnected`, which `Iterator::next` maps to `None`.
}

impl Iterator for BackgroundDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        // Fast path: serve from the local buffer without touching the channel.
        // This is the common case and runs entirely in the calling thread
        // (the CoreAudio render callback) with no synchronisation overhead.
        if let Some(sample) = self.pending.pop_front() {
            return Some(sample);
        }

        // Slow path: local buffer is empty — try to refill from the channel.
        match self.receiver.try_recv() {
            Ok(chunk) => {
                self.pending.extend(chunk);
                self.pending.pop_front()
            }

            // Decoder thread is still running but has not sent the next chunk
            // yet.  Return a silent sample rather than blocking the CoreAudio
            // render callback (blocking would cause the same "skipping cycle"
            // we are trying to prevent).
            //
            // With proper prebuffering and a 93 s channel this should be
            // extremely rare — if it happens at all it means the decoder
            // thread was starved for an extended period.
            Err(mpsc::TryRecvError::Empty) => {
                log::warn!("BackgroundDecoder: audio buffer underrun — decoder thread falling behind");
                Some(0.0)
            }

            // Sender was dropped: the decoder thread finished and all chunks
            // have been consumed.  Signal end-of-stream to rodio.
            Err(mpsc::TryRecvError::Disconnected) => None,
        }
    }
}

impl Source for BackgroundDecoder {
    fn current_frame_len(&self) -> Option<usize> {
        // We do not guarantee that chunk boundaries align with codec frames,
        // so we report variable-length frames by returning None.
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }
}
