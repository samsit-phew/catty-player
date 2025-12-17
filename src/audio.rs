use anyhow::Result;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::io::{BufReader, Cursor};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Get system volume from PulseAudio using pactl (0.0 to 1.0)
pub fn get_system_volume() -> f32 {
    use std::process::Command;

    match Command::new("pactl")
        .args(&["get-sink-volume", "@DEFAULT_SINK@"])
        .output()
    {
        Ok(output) => {
            if let Ok(s) = String::from_utf8(output.stdout) {
                // Output format: "Volume: front-left: 21870 / 33% / 0.13 dB   front-right: 21870 / 33% / 0.13 dB"
                // Extract percentage value
                if let Some(pct_str) = s.split('%').next().and_then(|p| p.split_whitespace().last()) {
                    if let Ok(pct) = pct_str.parse::<f32>() {
                        return (pct / 100.0).min(1.0).max(0.0);
                    }
                }
            }
        }
        Err(_) => {}
    }

    1.0 // Default to 100% if pactl unavailable
}


/// Audio player using rodio with sample capturing for visualization
pub struct AudioPlayer {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Arc<Mutex<Sink>>,
    current_duration: Arc<Mutex<Option<Duration>>>,
    sample_buffer: Arc<Mutex<Vec<f32>>>,
    elapsed_millis: Arc<AtomicU64>,
    start_time: Arc<Mutex<Option<Instant>>>,
    #[allow(dead_code)]
    pause_elapsed: Arc<AtomicU64>,
}

impl AudioPlayer {
    /// Create a new audio player
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;
        
        Ok(Self {
            _stream: stream,
            stream_handle,
            sink: Arc::new(Mutex::new(sink)),
            current_duration: Arc::new(Mutex::new(None)),
            sample_buffer: Arc::new(Mutex::new(Vec::new())),
            elapsed_millis: Arc::new(AtomicU64::new(0)),
            start_time: Arc::new(Mutex::new(None)),
            pause_elapsed: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Play a track from file path
    pub fn play(&self, path: &Path) -> Result<()> {
        // Read file bytes into memory so we can create two independent decoders:
        // one for playback and one for extracting samples for the visualizer.
        let data = std::fs::read(path)?;

        // Playback decoder (convert to f32 samples)
        let playback_cursor = Cursor::new(data.clone());
        let playback_decoder = Decoder::new(BufReader::new(playback_cursor))?.convert_samples::<f32>();

        // Visualization decoder (separate reader so we don't consume playback samples)
        let vis_cursor = Cursor::new(data);
        let mut vis_decoder = Decoder::new(BufReader::new(vis_cursor))?.convert_samples::<f32>();

        // Store duration if available (from playback decoder)
        // Note: convert_samples() returns an adapter that still exposes total_duration()
        let duration = playback_decoder.total_duration();
        *self.current_duration.lock().unwrap() = duration;

        // Reset elapsed time and set start time
        self.elapsed_millis.store(0, Ordering::Relaxed);
        *self.start_time.lock().unwrap() = Some(Instant::now());

        // Stop previous sink and replace with a new one for playback
        let sink = self.sink.lock().unwrap();
        sink.stop();
        drop(sink);

        let new_sink = Sink::try_new(&self.stream_handle)?;
        new_sink.append(playback_decoder);
        new_sink.play();
        *self.sink.lock().unwrap() = new_sink;

        // Spawn a background thread to consume the visualization decoder at roughly
        // the audio playback rate and push mono f32 samples into sample_buffer.
        let sample_buffer = Arc::clone(&self.sample_buffer);
        thread::spawn(move || {
            let channels = vis_decoder.channels() as usize;
            let sample_rate = vis_decoder.sample_rate();

            // We'll read in small chunks and sleep to approximate real-time
            let chunk_frames = 1024usize; // frames per chunk (per-channel frames)
            loop {
                // Collect up to chunk_frames * channels samples
                let mut tmp = Vec::with_capacity(chunk_frames * channels);
                for _ in 0..(chunk_frames * channels) {
                    if let Some(s) = vis_decoder.next() {
                        tmp.push(s);
                    } else {
                        break;
                    }
                }

                if tmp.is_empty() {
                    break; // finished
                }

                // Convert to mono by averaging channels if necessary
                if channels > 1 {
                    let frames = tmp.len() / channels;
                    let mut mono = Vec::with_capacity(frames);
                    for frame_idx in 0..frames {
                        let mut sum = 0.0f32;
                        for ch in 0..channels {
                            sum += tmp[frame_idx * channels + ch];
                        }
                        mono.push(sum / channels as f32);
                    }

                    let mut buf = sample_buffer.lock().unwrap();
                    buf.extend_from_slice(&mono);
                    if buf.len() > 8192 {
                        buf.drain(..4096);
                    }
                } else {
                    let mut buf = sample_buffer.lock().unwrap();
                    buf.extend_from_slice(&tmp);
                    if buf.len() > 8192 {
                        buf.drain(..4096);
                    }
                }

                // Sleep for approximately chunk_frames / sample_rate seconds
                if sample_rate > 0 {
                    let secs = (chunk_frames as f32) / (sample_rate as f32);
                    let millis = (secs * 1000.0) as u64;
                    thread::sleep(Duration::from_millis(millis));
                } else {
                    // fallback small sleep
                    thread::sleep(Duration::from_millis(10));
                }
            }
        });

        Ok(())
    }

    /// Get sample buffer for visualization
    pub fn get_sample_buffer(&self) -> Arc<Mutex<Vec<f32>>> {
        Arc::clone(&self.sample_buffer)
    }

    /// Pause playback
    pub fn pause(&self) {
        // Capture the current elapsed time before pausing
        let elapsed = self.get_elapsed_millis();
        self.pause_elapsed.store(elapsed, Ordering::Relaxed);
        self.sink.lock().unwrap().pause();
    }

    /// Resume playback
    pub fn resume(&self) {
        // Resume from the frozen position
        let frozen_elapsed = self.pause_elapsed.load(Ordering::Relaxed);
        let now = Instant::now();
        let adjusted_start = now - Duration::from_millis(frozen_elapsed);
        *self.start_time.lock().unwrap() = Some(adjusted_start);
        self.sink.lock().unwrap().play();
    }

    /// Stop playback
    pub fn stop(&self) {
        self.sink.lock().unwrap().stop();
        self.sample_buffer.lock().unwrap().clear();
        self.elapsed_millis.store(0, Ordering::Relaxed);
        *self.start_time.lock().unwrap() = None;
    }

    /// Get elapsed time in milliseconds (wall-clock based, stops when paused)
    pub fn get_elapsed_millis(&self) -> u64 {
        // If paused, return the frozen elapsed time
        if self.sink.lock().unwrap().is_paused() {
            return self.pause_elapsed.load(Ordering::Relaxed);
        }
        
        // Otherwise calculate from start time
        if let Some(start) = *self.start_time.lock().unwrap() {
            start.elapsed().as_millis() as u64
        } else {
            self.elapsed_millis.load(Ordering::Relaxed)
        }
    }

    /// Set elapsed time in milliseconds (for seeking)
    pub fn set_elapsed_millis(&self, millis: u64) {
        // Reset start time to now minus the desired elapsed time
        let now = Instant::now();
        let adjusted_start = now - Duration::from_millis(millis);
        *self.start_time.lock().unwrap() = Some(adjusted_start);
    }

    /// Check if player is paused
    #[allow(dead_code)]
    pub fn is_paused(&self) -> bool {
        self.sink.lock().unwrap().is_paused()
    }

    /// Check if player is empty (finished playing)
    pub fn is_empty(&self) -> bool {
        self.sink.lock().unwrap().empty()
    }

    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) {
        self.sink.lock().unwrap().set_volume(volume);
    }

    /// Get current volume
    #[allow(dead_code)]
    pub fn get_volume(&self) -> f32 {
        self.sink.lock().unwrap().volume()
    }

    /// Get current track duration
    #[allow(dead_code)]
    pub fn get_duration(&self) -> Option<Duration> {
        *self.current_duration.lock().unwrap()
    }
}
