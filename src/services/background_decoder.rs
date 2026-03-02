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
/// At 44 100 Hz stereo this equals ~2 s.  It guarantees the first batch of
/// CoreAudio render callbacks always find data in the buffer, absorbing the CPU
/// pressure caused by album-artwork fetching and database queries that happen
/// concurrently with track-switching, as well as brief OS scheduler starvation.
pub const PLAY_PREBUFFER_SAMPLES: usize = 176_400; // 2 s at 44.1 kHz stereo

/// Smaller pre-buffer used after a seek (the seek itself is user-triggered, so
/// CPU contention is lower and we want minimal added latency).
///
/// At 44 100 Hz stereo this equals ~46 ms.
pub const SEEK_PREBUFFER_SAMPLES: usize = 4_096;

/// Samples per chunk sent from the decoder thread to the audio thread.
///
/// 16 384 samples ≈ 186 ms at 44.1 kHz stereo.  Larger chunks mean fewer
/// channel operations per second, reducing synchronisation overhead.
const CHUNK_SIZE: usize = 16_384;

/// Maximum number of chunks buffered in the channel at once.
///
/// 512 chunks × 16 384 samples = ~8 M samples ≈ 93 s at 44.1 kHz stereo.
/// The `sync_channel` back-pressure naturally throttles the decoder thread so it
/// stays at most ~93 s ahead, preventing runaway memory use on long tracks.
const CHANNEL_CAPACITY: usize = 512;

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
/// The render callback only reads from a pre-filled queue of `Vec<f32>` chunks —
/// no decoding, no disk I/O, no memcpy (chunks are *moved* from the channel,
/// not copied).
///
/// ## Usage
///
/// ```no_run
/// # use tornade_core::services::background_decoder::{BackgroundDecoder, PLAY_PREBUFFER_SAMPLES};
/// # fn example(decoder: rodio::Decoder<std::io::BufReader<std::fs::File>>, sink: rodio::Sink) {
/// use rodio::Source;
/// let mut bg = BackgroundDecoder::new(decoder.convert_samples::<f32>());
/// bg.prebuffer(PLAY_PREBUFFER_SAMPLES); // fill ~2 s before first callback
/// sink.append(bg);
/// # }
/// ```
pub struct BackgroundDecoder {
    /// Receives fully-decoded sample chunks from the background thread.
    receiver: Receiver<Vec<f32>>,

    /// Queue of received chunks waiting to be served to the audio thread.
    ///
    /// Each `Vec<f32>` is *moved* from the channel — no copying.
    /// `VecDeque::pop_front` is O(1); `push_back` is amortised O(1).
    chunk_queue: VecDeque<Vec<f32>>,

    /// Index of the next sample to serve within `chunk_queue.front()`.
    cursor: usize,

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
            chunk_queue: VecDeque::new(),
            cursor: 0,
            channels,
            sample_rate,
            total_duration,
        }
    }

    /// Pre-fills the chunk queue by blocking until at least `target_samples`
    /// are available.
    ///
    /// Must be called before handing `self` to the sink.  Without this, the OS
    /// scheduler might not give the decoder thread any time to run before the
    /// first CoreAudio render callback fires, causing an immediate underrun.
    pub fn prebuffer(&mut self, target_samples: usize) {
        // Count samples already queued (minus already-consumed cursor offset in
        // the front chunk).
        let already_queued: usize = self
            .chunk_queue
            .iter()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.len().saturating_sub(self.cursor) } else { c.len() })
            .sum();

        let mut total = already_queued;

        while total < target_samples {
            match self.receiver.recv() {
                Ok(chunk) => {
                    total += chunk.len();
                    // Chunks are moved into the queue — no copy.
                    self.chunk_queue.push_back(chunk);
                }
                // Short track: decoder finished before we reached the target.
                // That is fine — we will serve whatever we have.
                Err(_) => break,
            }
        }
    }

    /// Number of samples currently queued (available without blocking).
    #[cfg(test)]
    pub fn buffered_samples(&self) -> usize {
        self.chunk_queue
            .iter()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.len().saturating_sub(self.cursor) } else { c.len() })
            .sum()
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
    // `tx` is dropped here → once `rx` drains all chunks, `try_recv` returns
    // `TryRecvError::Disconnected`, which `Iterator::next` maps to `None`.
}

impl Iterator for BackgroundDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            // Fast path: serve the next sample from the front chunk.
            // Just an array index + increment — O(1), no allocation, cache-friendly.
            if let Some(front) = self.chunk_queue.front() {
                if self.cursor < front.len() {
                    let sample = front[self.cursor];
                    self.cursor += 1;
                    return Some(sample);
                }
                // Front chunk fully consumed — drop it and reset cursor.
                // Vec::drop frees its allocation; this is the only point where
                // memory is freed on the audio thread (~every 186 ms at 44.1 kHz).
                self.chunk_queue.pop_front();
                self.cursor = 0;
                // Fall through to try the new front chunk in the next iteration.
                continue;
            }

            // Chunk queue empty — try to refill from the channel (non-blocking).
            match self.receiver.try_recv() {
                Ok(chunk) => {
                    // Chunk is moved in — zero copy.
                    self.chunk_queue.push_back(chunk);
                    // Loop back to serve from the new chunk.
                }

                // Decoder thread is still running but hasn't sent the next chunk
                // yet.  Return a silent sample rather than blocking the CoreAudio
                // render callback (blocking would cause the same "skipping cycle"
                // we are trying to prevent).
                //
                // With a 2 s pre-buffer and a 93 s channel this should never
                // happen under normal load.  If it does, it signals that the
                // decoder thread was starved by the OS for an extended period.
                Err(mpsc::TryRecvError::Empty) => {
                    log::warn!("BackgroundDecoder: audio buffer underrun — decoder thread falling behind");
                    return Some(0.0);
                }

                // Sender was dropped: the decoder thread finished and all chunks
                // have been consumed.  Signal end-of-stream to rodio.
                Err(mpsc::TryRecvError::Disconnected) => return None,
            }
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
