use rustfft::{FftPlanner, num_complex::Complex};
use std::sync::{Arc, Mutex};

/// CAVA-style audio visualizer with real FFT analysis
pub struct Visualizer {
    bars: Vec<f32>,
    bar_count: usize,
    smoothing: f32,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    fft_planner: FftPlanner<f32>,
}

impl Visualizer {
    /// Create a new visualizer
    pub fn new(bar_count: usize, smoothing: f32) -> Self {
        Self {
            bars: vec![0.0; bar_count],
            bar_count,
            smoothing,
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
            fft_planner: FftPlanner::new(),
        }
    }

    /// Get audio buffer reference for audio capture
    pub fn get_audio_buffer(&self) -> Arc<Mutex<Vec<f32>>> {
        Arc::clone(&self.audio_buffer)
    }

    /// Update visualization using FFT of audio samples
    pub fn update(&mut self) {
        let mut buffer = self.audio_buffer.lock().unwrap();
        
        if buffer.is_empty() {
            // Smooth decay when no audio
            for bar in &mut self.bars {
                *bar *= self.smoothing;
            }
            return;
        }

        // Take samples for FFT (power of 2)
        let fft_size = 2048.min(buffer.len().next_power_of_two());
        let samples: Vec<f32> = if buffer.len() >= fft_size {
            buffer.drain(..fft_size).collect()
        } else {
            let mut samples = buffer.drain(..).collect::<Vec<_>>();
            samples.resize(fft_size, 0.0);
            samples
        };

        // Prepare complex input for FFT
        let mut input: Vec<Complex<f32>> = samples
            .iter()
            .map(|&s| Complex::new(s, 0.0))
            .collect();

        // Perform FFT
        let fft = self.fft_planner.plan_fft_forward(fft_size);
        fft.process(&mut input);

        // Calculate magnitude spectrum and map to bars
        let spectrum_size = fft_size / 2;
        let freqs_per_bar = spectrum_size / self.bar_count;

        for (i, bar) in self.bars.iter_mut().enumerate() {
            let start_idx = i * freqs_per_bar;
            let end_idx = ((i + 1) * freqs_per_bar).min(spectrum_size);
            
            if start_idx >= spectrum_size {
                break;
            }

            // Average magnitude for this bar's frequency range
            let mut sum = 0.0;
            for j in start_idx..end_idx {
                let magnitude = input[j].norm();
                sum += magnitude;
            }
            
            let avg_magnitude = sum / (end_idx - start_idx) as f32;
            
            // Normalize and apply logarithmic scaling for better visualization
            let normalized = (avg_magnitude / 100.0).min(1.0);
            let log_scaled = if normalized > 0.0 {
                (normalized.log10() + 2.0) / 2.0 // Scale from -2..0 to 0..1
            } else {
                0.0
            }.max(0.0).min(1.0);

            // Smooth interpolation with previous value
            *bar = *bar * self.smoothing + log_scaled * (1.0 - self.smoothing);
        }
    }

    /// Get current bar heights (0.0 to 1.0)
    pub fn get_bars(&self) -> &[f32] {
        &self.bars
    }

    /// Set bar count
    #[allow(dead_code)]
    pub fn set_bar_count(&mut self, count: usize) {
        self.bar_count = count;
        self.bars.resize(count, 0.0);
    }

    /// Set smoothing factor
    #[allow(dead_code)]
    pub fn set_smoothing(&mut self, smoothing: f32) {
        self.smoothing = smoothing.clamp(0.0, 1.0);
    }
}
