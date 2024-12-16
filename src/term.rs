use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute, queue,
    style::{self, Color, Stylize},
    terminal::{self, ClearType},
    QueueableCommand,
};
use std::io::{stdout, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use ctrlc;
use std::time::{Duration, SystemTime};

#[derive(Debug)]
pub struct LogEntry {
    timestamp: SystemTime,
    message: String,
}

#[derive(Debug)]
pub struct TerminalUI {
    logs: Arc<Mutex<Vec<LogEntry>>>,
    active_peers: Arc<Mutex<usize>>,
    progress: Arc<Mutex<f64>>,
    running: Arc<Mutex<bool>>,
}

impl TerminalUI {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(Mutex::new(Vec::new())),
            active_peers: Arc::new(Mutex::new(0)),
            progress: Arc::new(Mutex::new(0.0)),
            running: Arc::new(Mutex::new(true)),
        }
    }

    pub fn start(&self) -> std::io::Result<()> {
        // Save current terminal settings
        terminal::enable_raw_mode()?;
        execute!(stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;

        let logs = Arc::clone(&self.logs);
        let active_peers = Arc::clone(&self.active_peers);
        let progress = Arc::clone(&self.progress);
        let running = Arc::clone(&self.running);

        let running_handler = Arc::clone(&running);
        ctrlc::set_handler(move || {
            if let Ok(mut running) = running_handler.lock() {
                *running = false;
            }
            // Restore terminal immediately on Ctrl+C
            let _ = terminal::disable_raw_mode();
            let mut stdout = stdout();
            let _ = execute!(
                stdout,
                style::ResetColor,
                cursor::Show,
                terminal::LeaveAlternateScreen
            );
        })
        .expect("Error setting Ctrl-C handler");

        // Spawn refresh thread
        thread::spawn(move || {
            let mut stdout = stdout();
            while *running.lock().unwrap() {
                if let Err(e) = Self::draw_screen(&mut stdout, &logs, &active_peers, &progress) {
                    eprintln!("Error drawing screen: {:?}", e);
                    break;
                }
                thread::sleep(Duration::from_millis(100));
                std::thread::yield_now();
            }
        });

        Ok(())
    }

    pub fn stop(&self) -> std::io::Result<()> {
        // Set running to false to stop the refresh thread
        if let Ok(mut running) = self.running.lock() {
            *running = false;
        }

        // Restore terminal
        let mut stdout = stdout();
        execute!(
            stdout,
            style::ResetColor,
            cursor::Show,
            terminal::LeaveAlternateScreen
        )?;
        terminal::disable_raw_mode()?;

        Ok(())
    }

    pub fn add_log(&self, message: String) {
        if let Ok(mut logs) = self.logs.lock() {
            // Calculate the drain range before mutating
            let drain_start = if logs.len() > 1000 { 0 } else { logs.len() };
            let drain_end = logs.len().saturating_sub(1000);

            // Push new message
            logs.push(LogEntry {
                timestamp: SystemTime::now(),
                message,
            });

            // Drain old messages if needed
            if drain_start < drain_end {
                logs.drain(drain_start..drain_end);
            }
        }
    }

    pub fn update_peers(&self, count: usize) {
        if let Ok(mut peers) = self.active_peers.lock() {
            *peers = count;
        }
    }

    pub fn update_progress(&self, new_progress: f64) {
        if let Ok(mut progress) = self.progress.lock() {
            *progress = new_progress;
        }
    }

    fn draw_screen(
        stdout: &mut std::io::Stdout,
        logs: &Arc<Mutex<Vec<LogEntry>>>,
        active_peers: &Arc<Mutex<usize>>,
        progress: &Arc<Mutex<f64>>,
    ) -> std::io::Result<()> {
        // Get terminal size
        let (width, height) = terminal::size()?;
        let split = width / 3; // Left panel takes 1/3 of screen

        // Clear screen
        queue!(stdout, terminal::Clear(ClearType::All))?;

        // Draw left panel (peer info)
        let peers = active_peers.lock().map(|guard| *guard).unwrap_or(0);
        let progress_val = progress.lock().map(|guard| *guard).unwrap_or(0.0);

        // Draw title for left panel
        queue!(
            stdout,
            cursor::MoveTo(2, 1),
            style::SetForegroundColor(Color::Blue)
        )?;
        write!(stdout, "Active Peers: {}", peers)?;

        queue!(stdout, cursor::MoveTo(2, 3))?;
        write!(stdout, "Download Progress: {:.1}%", progress_val * 100.0)?;

        // Draw progress bar
        queue!(stdout, cursor::MoveTo(2, 4))?;
        let bar_width = (split as usize - 4).max(0);
        let filled = ((progress_val * bar_width as f64) as usize).min(bar_width);
        let bar = format!("[{}{}]", "=".repeat(filled), " ".repeat(bar_width - filled));
        write!(stdout, "{}", bar)?;

        // Draw separator
        queue!(stdout, style::SetForegroundColor(Color::DarkGrey))?;
        for y in 0..height {
            queue!(stdout, cursor::MoveTo(split, y))?;
            write!(stdout, "│")?;
        }

        // Draw right panel (logs)
        queue!(stdout, style::SetForegroundColor(Color::Reset))?;
        if let Ok(logs) = logs.lock() {
            let visible_height = height.saturating_sub(2) as usize;
            let start_idx = logs.len().saturating_sub(visible_height);

            for (i, log) in logs.iter().skip(start_idx).enumerate() {
                if i >= visible_height {
                    break;
                }

                queue!(stdout, cursor::MoveTo(split + 2, i as u16 + 1))?;

                // Format timestamp
                let timestamp = log
                    .timestamp
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let time = chrono::DateTime::from_timestamp(timestamp as i64, 0)
                    .map(|t| t.format("%H:%M:%S").to_string())
                    .unwrap_or_default();

                // Calculate available width for log message
                let max_log_width = width.saturating_sub(split + 3) as usize;
                let timestamp_width = 9; // HH:MM:SS
                let message_width = max_log_width.saturating_sub(timestamp_width + 3);

                // Truncate message if needed
                let truncated_message = if log.message.len() > message_width {
                    format!("{}...", &log.message[..message_width.saturating_sub(3)])
                } else {
                    log.message.clone()
                };

                queue!(stdout, style::SetForegroundColor(Color::DarkGrey))?;
                write!(stdout, "[{}] ", time)?;
                queue!(stdout, style::SetForegroundColor(Color::Reset))?;
                write!(stdout, "{}", truncated_message)?;
            }
        }

        stdout.flush()?;
        Ok(())
    }
}

impl Drop for TerminalUI {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

