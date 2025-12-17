use crate::audio::AudioPlayer;
use crate::config::Config;
use crate::database::{MusicDatabase, Track};
use crate::visualizer::Visualizer;
use rand::seq::SliceRandom;
use rand::thread_rng;

/// Player state management
pub struct PlayerState {
    pub loopC: bool,
    // Search UI state
    pub search_mode: bool,
    pub search_query: String,
    pub search_results: Vec<usize>,
    pub database: MusicDatabase,
    pub audio: AudioPlayer,
    pub queue: Vec<Track>,
    pub current_track_index: Option<usize>,
    pub list_state: usize, // Selected item in the list
    pub scroll_offset: usize,
    pub is_playing: bool,
    pub shuffle: bool,
    pub volume: f32,
    pub visualizer: Visualizer,
    pub config: Config,
    needs_redraw: bool,
    played_indices: Vec<usize>, // Track played songs in shuffle mode
    pub show_help: bool,
}

impl PlayerState {
    /// Create new player state
    pub fn new(database: MusicDatabase, audio: AudioPlayer, config: Config) -> Self {
        let visualizer = Visualizer::new(config.visualizer.bar_count, config.visualizer.smoothing);

        let initial_volume = 0.2; // Start at 20%
        audio.set_volume(initial_volume);

        Self {
            database,
            audio,
            queue: Vec::new(),
            current_track_index: None,
            list_state: 0,
            scroll_offset: 0,
            is_playing: false,
            shuffle: false,
            volume: initial_volume,
            visualizer,
            config,
            needs_redraw: true,
            loopC: false,
            search_mode: false,
            search_query: String::new(),
            search_results: Vec::new(),
            played_indices: Vec::new(),
            show_help: false,
        }
    }

    /// Start search mode (user pressed search keybind)
    pub fn start_search(&mut self) {
        self.search_mode = true;
        self.search_query.clear();
        self.search_results.clear();
    }

    pub fn cancel_search(&mut self) {
        self.search_mode = false;
        self.search_query.clear();
        self.search_results.clear();
    }

    pub fn search_add_char(&mut self, c: char) {
        self.search_query.push(c);
        self.update_search_results();
    }

    pub fn search_backspace(&mut self) {
        self.search_query.pop();
        self.update_search_results();
    }

    pub fn search_submit(&mut self) {
        if let Some(&first) = self.search_results.first() {
            self.list_state = first;
            // Play the selected search result
            self.play_selected();
        }
        self.cancel_search();
    }

    fn update_search_results(&mut self) {
        let q = self.search_query.to_lowercase();
        if q.is_empty() {
            self.search_results.clear();
            return;
        }

        self.search_results = self
            .database
            .get_tracks()
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                if t.title.to_lowercase().contains(&q) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();
    }

    /// Toggle playback
    pub fn toggle_playback(&mut self) {
        if self.is_playing {
            self.audio.pause();
            self.is_playing = false;
        } else {
            if self.current_track_index.is_none() && !self.queue.is_empty() {
                self.play_track(0);
            } else {
                self.audio.resume();
                self.is_playing = true;
            }
        }
    }

    /// Play next track
    pub fn next_track(&mut self) {
        if self.queue.is_empty() {
            return;
        }

        let next_index = if self.shuffle {
            self.get_next_shuffle_index()
        } else {
            self.current_track_index
                .map(|i| (i + 1) % self.queue.len())
                .unwrap_or(0)
        };

        self.play_track(next_index);
    }

    /// Play previous track
    pub fn previous_track(&mut self) {
        if self.queue.is_empty() {
            return;
        }

        let prev_index = self
            .current_track_index
            .map(|i| if i > 0 { i - 1 } else { self.queue.len() - 1 })
            .unwrap_or(0);

        self.play_track(prev_index);
    }

    /// Play track at index
    pub fn play_track(&mut self, index: usize) {
        if let Some(track) = self.queue.get(index) {
            if self.audio.play(&track.path).is_ok() {
                self.current_track_index = Some(index);
                self.is_playing = true;

                // Track played index for shuffle
                if self.shuffle && !self.played_indices.contains(&index) {
                    self.played_indices.push(index);
                }
            }
        }
    }
    #[allow(dead_code)]
    pub fn play_again(&mut self) {
        // Replay the current track if there is one
        if let Some(idx) = self.current_track_index {
            self.play_track(idx);
        }
    }
    /// Toggle shuffle mode
    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.played_indices.clear();

        if let Some(current) = self.current_track_index {
            self.played_indices.push(current);
        }
    }

    /// Get next shuffle index (without repetition until all played)
    fn get_next_shuffle_index(&mut self) -> usize {
        let queue_len = self.queue.len();

        // Reset if all tracks have been played
        if self.played_indices.len() >= queue_len {
            self.played_indices.clear();
            if let Some(current) = self.current_track_index {
                self.played_indices.push(current);
            }
        }

        // Find unplayed tracks
        let mut unplayed: Vec<usize> = (0..queue_len)
            .filter(|i| !self.played_indices.contains(i))
            .collect();

        if unplayed.is_empty() {
            return 0;
        }

        // Pick random unplayed track
        let mut rng = thread_rng();
        unplayed.shuffle(&mut rng);
        unplayed[0]
    }

    /// Increase volume
    pub fn increase_volume(&mut self) {
        self.volume = (self.volume + 0.05).min(1.0);
        self.audio.set_volume(self.volume);
    }

    /// Decrease volume
    pub fn decrease_volume(&mut self) {
        self.volume = (self.volume - 0.05).max(0.0);
        self.audio.set_volume(self.volume);
    }

    /// Scroll up in list
    pub fn scroll_up(&mut self) {
        if self.list_state > 0 {
            self.list_state -= 1;
        }
    }

    /// Scroll down in list
    pub fn scroll_down(&mut self) {
        let max = self.database.track_count().saturating_sub(1);
        if self.list_state < max {
            self.list_state += 1;
        }
    }

    /// Play selected track
    pub fn play_selected(&mut self) {
        let tracks = self.database.get_tracks();
        if let Some(_track) = tracks.get(self.list_state) {
            self.queue.clear();
            self.queue.extend_from_slice(tracks);
            self.played_indices.clear();
            self.play_track(self.list_state);
        }
    }

    /// Clear queue
    pub fn clear_queue(&mut self) {
        self.queue.clear();
        self.current_track_index = None;
        self.audio.stop();
        self.is_playing = false;
        self.played_indices.clear();
    }

    /// Check if should advance to next track
    pub fn should_advance(&self) -> bool {
        self.is_playing && self.audio.is_empty() && !self.queue.is_empty()
    }

    /// Update visualizer data
    pub fn update_visualizer(&mut self) {
        if self.is_playing {
            // Get audio samples and pass to visualizer
            let sample_buffer = self.audio.get_sample_buffer();
            let samples = sample_buffer.lock().unwrap();

            // Copy samples to visualizer's buffer
            let viz_buffer = self.visualizer.get_audio_buffer();
            let mut viz_buf = viz_buffer.lock().unwrap();
            viz_buf.extend_from_slice(&samples);
            drop(viz_buf);

            // Clear audio player's buffer after copying
            drop(samples);
            sample_buffer.lock().unwrap().clear();

            // Update visualizer with FFT
            self.visualizer.update();
        }
    }

    /// Check if needs redraw
    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    /// Mark that redraw is needed
    pub fn mark_needs_redraw(&mut self) {
        self.needs_redraw = true;
    }

    /// Clear redraw flag
    pub fn clear_redraw_flag(&mut self) {
        self.needs_redraw = false;
    }

    /// Get current track
    pub fn get_current_track(&self) -> Option<&Track> {
        self.current_track_index.and_then(|i| self.queue.get(i))
    }

    /// Seek forward by 10 seconds
    pub fn seek_forward(&mut self) {
        let current = self.audio.get_elapsed_millis();
        let new_pos = current + 10_000; // 10 seconds in milliseconds
        self.audio.set_elapsed_millis(new_pos);
    }

    /// Seek backward by 10 seconds
    pub fn seek_backward(&mut self) {
        let current = self.audio.get_elapsed_millis();
        let new_pos = current.saturating_sub(10_000); // 10 seconds in milliseconds
        self.audio.set_elapsed_millis(new_pos);
    }

    /// Toggle help menu visibility
    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    /// Get elapsed time in seconds
    pub fn get_elapsed_seconds(&self) -> f32 {
        (self.audio.get_elapsed_millis() as f32) / 1000.0
    }

    /// Get total duration in seconds
    pub fn get_duration_seconds(&self) -> f32 {
        self.audio
            .get_duration()
            .map(|d| d.as_secs() as f32 + d.subsec_millis() as f32 / 1000.0)
            .unwrap_or(0.0)
    }
}
