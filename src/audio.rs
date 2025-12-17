use anyhow::Result;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
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
                if let Some(pct_str) = s
                    .split('%')
                    .next()
                    .and_then(|p| p.split_whitespace().last())
                {
                    if let Ok(pct) = pct_str.parse::<f32>() {
                        return (pct / 100.0).clamp(0.0, 1.0);
                    }
                }
            }
        }
        Err(_) => {}
    }

    1.0 // default
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
    pause_elapsed: Arc<AtomicU64>,
    current_track: Arc<Mutex<Option<PathBuf>>>,
}

impl AudioPlayer {
    /// Create a new player
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
            current_track: Arc::new(Mutex::new(None)),
        })
    }

    /// Play a track
    pub fn play(&self, path: &Path) -> Result<()> {
        let data = std::fs::read(path)?;

        // Store current track path
        *self.current_track.lock().unwrap() = Some(path.to_path_buf());

        // Playback decoder
        let playback_cursor = Cursor::new(data.clone());
        let playback_decoder =
            Decoder::new(BufReader::new(playback_cursor))?.convert_samples::<f32>();

        // Visualization decoder
        let vis_cursor = Cursor::new(data);
        let mut vis_decoder = Decoder::new(BufReader::new(vis_cursor))?.convert_samples::<f32>();

        // Duration
        *self.current_duration.lock().unwrap() = playback_decoder.total_duration();

        self.elapsed_millis.store(0, Ordering::Relaxed);
        *self.start_time.lock().unwrap() = Some(Instant::now());

        // Stop old sink
        self.sink.lock().unwrap().stop();
        let new_sink = Sink::try_new(&self.stream_handle)?;
        new_sink.append(playback_decoder);
        new_sink.play();
        *self.sink.lock().unwrap() = new_sink;

        // Background thread for visualizer
        let sample_buffer = Arc::clone(&self.sample_buffer);
        thread::spawn(move || {
            let channels = vis_decoder.channels() as usize;
            let sample_rate = vis_decoder.sample_rate();
            let chunk_frames = 1024;

            loop {
                let mut tmp = Vec::with_capacity(chunk_frames * channels);
                for _ in 0..(chunk_frames * channels) {
                    if let Some(s) = vis_decoder.next() {
                        tmp.push(s);
                    } else {
                        break;
                    }
                }
                if tmp.is_empty() {
                    break;
                }

                // convert to mono
                let mono = if channels > 1 {
                    let frames = tmp.len() / channels;
                    let mut mono = Vec::with_capacity(frames);
                    for frame_idx in 0..frames {
                        let sum: f32 = (0..channels).map(|ch| tmp[frame_idx * channels + ch]).sum();
                        mono.push(sum / channels as f32);
                    }
                    mono
                } else {
                    tmp
                };

                let mut buf = sample_buffer.lock().unwrap();
                buf.extend_from_slice(&mono);
                if buf.len() > 8192 {
                    buf.drain(..4096);
                }

                let sleep_ms = ((chunk_frames as f32 / sample_rate as f32) * 1000.0) as u64;
                thread::sleep(Duration::from_millis(sleep_ms.max(10)));
            }
        });

        Ok(())
    }

    /// Seek to specific position
    pub fn seek_to(&self, millis: u64) -> Result<()> {
        if let Some(duration) = *self.current_duration.lock().unwrap() {
            let target = millis.min(duration.as_millis() as u64);

            // Stop current sink
            self.sink.lock().unwrap().stop();

            // Recreate decoder
            if let Some(track_path) = &*self.current_track.lock().unwrap() {
                let data = std::fs::read(track_path)?;
                let cursor = Cursor::new(data);
                let mut decoder = Decoder::new(cursor)?.convert_samples::<f32>();

                // skip samples
                let sample_rate = decoder.sample_rate() as u64;
                let channels = decoder.channels() as u64;
                let frames_to_skip = (target * sample_rate) / 1000;
                let samples_to_skip = frames_to_skip * channels;
                for _ in 0..samples_to_skip {
                    if decoder.next().is_none() {
                        break;
                    }
                }

                let new_sink = Sink::try_new(&self.stream_handle)?;
                new_sink.append(decoder);
                new_sink.play();
                *self.sink.lock().unwrap() = new_sink;

                self.elapsed_millis.store(target, Ordering::Relaxed);
                *self.start_time.lock().unwrap() =
                    Some(Instant::now() - Duration::from_millis(target));
            }
        }
        Ok(())
    }

    /// Seek forward/backward
    pub fn seek_forward(&self) -> Result<()> {
        let current = self.get_elapsed_millis();
        self.seek_to(current + 10_000)
    }

    pub fn seek_backward(&self) -> Result<()> {
        let current = self.get_elapsed_millis();
        self.seek_to(current.saturating_sub(10_000))
    }

    /// Pause/resume/stop
    pub fn pause(&self) {
        self.pause_elapsed
            .store(self.get_elapsed_millis(), Ordering::Relaxed);
        self.sink.lock().unwrap().pause();
    }

    pub fn resume(&self) {
        let frozen = self.pause_elapsed.load(Ordering::Relaxed);
        *self.start_time.lock().unwrap() = Some(Instant::now() - Duration::from_millis(frozen));
        self.sink.lock().unwrap().play();
    }

    pub fn stop(&self) {
        self.sink.lock().unwrap().stop();
        self.sample_buffer.lock().unwrap().clear();
        self.elapsed_millis.store(0, Ordering::Relaxed);
        *self.start_time.lock().unwrap() = None;
    }

    /// Utilities
    pub fn get_elapsed_millis(&self) -> u64 {
        if self.sink.lock().unwrap().is_paused() {
            return self.pause_elapsed.load(Ordering::Relaxed);
        }
        if let Some(start) = *self.start_time.lock().unwrap() {
            start.elapsed().as_millis() as u64
        } else {
            self.elapsed_millis.load(Ordering::Relaxed)
        }
    }

    pub fn get_sample_buffer(&self) -> Arc<Mutex<Vec<f32>>> {
        Arc::clone(&self.sample_buffer)
    }

    pub fn set_volume(&self, volume: f32) {
        self.sink.lock().unwrap().set_volume(volume);
    }

    pub fn get_duration(&self) -> Option<Duration> {
        *self.current_duration.lock().unwrap()
    }

    pub fn is_paused(&self) -> bool {
        self.sink.lock().unwrap().is_paused()
    }

    pub fn is_empty(&self) -> bool {
        self.sink.lock().unwrap().empty()
    }
}
