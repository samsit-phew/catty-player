use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Represents a music track with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub duration: Option<u64>, // in seconds
}

/// Music database with caching support
pub struct MusicDatabase {
    pub tracks: Vec<Track>,
    cache_path: PathBuf,
}

impl MusicDatabase {
    /// Create a new database instance
    pub fn new() -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("catty");
        
        fs::create_dir_all(&cache_dir)?;
        let cache_path = cache_dir.join("music_cache.json");

        // Try to load from cache
        let tracks = if cache_path.exists() {
            Self::load_cache(&cache_path)?
        } else {
            Vec::new()
        };

        Ok(Self { tracks, cache_path })
    }

    /// Scan XDG Music directory for audio files
    pub fn scan_music_directory(&mut self) -> Result<()> {
        // Get XDG music directory
        let music_dir = dirs::audio_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
            .unwrap_or_else(|| PathBuf::from("."));

        if !music_dir.exists() {
            eprintln!("Music directory not found: {:?}", music_dir);
            return Ok(());
        }

        // Scan for audio files
        let mut tracks = Vec::new();
        for entry in WalkDir::new(music_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                let ext = ext.to_string_lossy().to_lowercase();
                if ["mp3", "flac", "ogg", "wav", "m4a", "opus"].contains(&ext.as_str()) {
                    let title = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    tracks.push(Track {
                        path: path.to_path_buf(),
                        title,
                        artist: None,
                        duration: None,
                    });
                }
            }
        }

        self.tracks = tracks;
        self.save_cache()?;

        Ok(())
    }

    /// Load tracks from cache
    fn load_cache(path: &PathBuf) -> Result<Vec<Track>> {
        let data = fs::read_to_string(path)?;
        let tracks: Vec<Track> = serde_json::from_str(&data)?;
        Ok(tracks)
    }

    /// Save tracks to cache
    fn save_cache(&self) -> Result<()> {
        let data = serde_json::to_string(&self.tracks)?;
        fs::write(&self.cache_path, data)?;
        Ok(())
    }

    /// Get all tracks
    pub fn get_tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Get track count
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }
}
