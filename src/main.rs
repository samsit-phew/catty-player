//! # Catty Player
//!
//! Catty is a terminal music player with customizable colors, keybinds, and a visualizer.
//!
//! ## Configuration
//!
//! Catty reads a `config.toml` file that is `~/.config/catty-player/config.toml`.
//! If the file is missing or invalid, Catty will create one with default values.
//!
//! ### Example `config.toml`
//!
//! ```toml
//! [colors]
//! foreground = "white"
//! background = "black"
//! accent = "cyan"
//! visualizer_foreground = "LightBlue"
//! visualizer_background = "black"
//!
//! [keybinds]
//! quit = "q"
//! play_pause = "space"
//! next = "n"
//! previous = "p"
//! shuffle = "s"
//! volume_up = "+"
//! volume_down = "-"
//! select = "enter"
//! clear = "c"
//! seek_forward = "l"
//! seek_backward = "h"
//! help = "?"
//!
//! [visualizer]
//! bar_count = 50
//! smoothing = 0.7
//! [watermark]
//! water_mark = true /false #toggles samsit-phew mark on help section
//!
//!
//! ```
//!
//! ### Editing the Configuration
//!
//! - **Colors**: Use standard color names or hex codes (e.g., `"red"` or `"#FF0000"`).  
//! - **Keybinds**: Use strings like `"space"`, `"enter"`, `"q"`.  
//! - **Visualizer**: Adjust `bar_count` and `smoothing`.  
//!
//! ### Usage
//!
//! ```bash
//! # Run the player
//! catty-player
//! ```
//! The program will automatically load `config.toml` or generate defaults if missing.

mod audio;
mod config;
mod database;
mod player;
mod ui;
mod visualizer;

use anyhow::Result;
use config::Config;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use audio::AudioPlayer;
use database::MusicDatabase;
use player::PlayerState;
use ui::UI;

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let config = Config::load();

    // Initialize database and scan music
    let mut database = MusicDatabase::new()?;
    database.scan_music_directory()?;

    // Initialize audio player
    let audio_player = AudioPlayer::new()?;

    // Initialize player state
    let mut player_state = PlayerState::new(database, audio_player, config.clone());

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create async event channel
    let (tx, mut rx) = mpsc::channel(100);

    // Spawn event listener
    tokio::spawn(async move {
        loop {
            if event::poll(Duration::from_millis(100)).unwrap() {
                if let Ok(evt) = event::read() {
                    if tx.send(evt).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Main loop
    let mut last_draw = Instant::now();
    let draw_interval = Duration::from_millis(50); // 20 FPS for smooth visualization

    loop {
        // Update visualizer data
        player_state.update_visualizer();

        // Optimized redraw - only when needed
        let needs_redraw = player_state.needs_redraw() || last_draw.elapsed() >= draw_interval;

        if needs_redraw {
            terminal.draw(|f| {
                UI::render(f, &mut player_state);
            })?;
            last_draw = Instant::now();
            player_state.clear_redraw_flag();
        }

        // Handle events
        match tokio::time::timeout(Duration::from_millis(16), rx.recv()).await {
            Ok(Some(Event::Key(key))) => {
                // If search mode is active, route keys to search input
                let handled = if player_state.search_mode {
                    match key.code {
                        KeyCode::Char(c) => {
                            player_state.search_add_char(c);
                            true
                        }
                        KeyCode::Backspace => {
                            player_state.search_backspace();
                            true
                        }
                        KeyCode::Enter => {
                            player_state.search_submit();
                            true
                        }
                        KeyCode::Esc => {
                            player_state.cancel_search();
                            true
                        }
                        _ => false,
                    }
                } else {
                    // Normal key handling
                    match key.code {
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.quit, c, key.modifiers) =>
                        {
                            break;
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.play_pause, c, key.modifiers) =>
                        {
                            player_state.toggle_playback();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.next, c, key.modifiers) =>
                        {
                            player_state.next_track();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.previous, c, key.modifiers) =>
                        {
                            player_state.previous_track();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.shuffle, c, key.modifiers) =>
                        {
                            player_state.toggle_shuffle();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.volume_up, c, key.modifiers) =>
                        {
                            player_state.increase_volume();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.volume_down, c, key.modifiers) =>
                        {
                            player_state.decrease_volume();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.seek_forward, c, key.modifiers) =>
                        {
                            player_state.seek_forward();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(
                                &config.keybinds.seek_backward,
                                c,
                                key.modifiers,
                            ) =>
                        {
                            player_state.seek_backward();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.help, c, key.modifiers) =>
                        {
                            player_state.toggle_help();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.search, c, key.modifiers) =>
                        {
                            player_state.start_search();
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.LoopC, c, key.modifiers) =>
                        {
                            player_state.loopC = !player_state.loopC;
                            true
                        }
                        KeyCode::Char(c)
                            if matches_keybind(&config.keybinds.clear, c, key.modifiers) =>
                        {
                            player_state.clear_queue();
                            true
                        }
                        KeyCode::Char('-') => {
                            player_state.decrease_volume();
                            true
                        }
                        KeyCode::Up => {
                            player_state.scroll_up();
                            true
                        }
                        KeyCode::Down => {
                            player_state.scroll_down();
                            true
                        }
                        KeyCode::Enter if config.keybinds.select == "enter" => {
                            player_state.play_selected();
                            true
                        }
                        _ => false,
                    }
                };

                if handled {
                    player_state.mark_needs_redraw();
                }
            }
            _ => {}
        }

        // Auto-advance or loop when current finishes
        if player_state.should_advance() {
            if player_state.loopC {
                // Replay same track
                if let Some(idx) = player_state.current_track_index {
                    player_state.play_track(idx);
                }
            } else {
                player_state.next_track();
            }
            player_state.mark_needs_redraw();
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn matches_keybind(keybind: &str, c: char, _modifiers: KeyModifiers) -> bool {
    keybind.to_lowercase() == c.to_string().to_lowercase() || (keybind == "space" && c == ' ')
}
