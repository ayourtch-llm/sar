use std::process::id as pid;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::event::{self, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind};
use crossterm::terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Terminal;
use sar_core::actor::Actor;
use sar_core::bus::SarBus;
use sar_core::message::Message;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

const APP_ID: &str = "sar-tui";
const LIGHT_GRAY: Color = Color::Rgb(211, 211, 211);
const PAGE_SIZE: usize = 10;

#[derive(Debug, Default)]
pub struct TuiActor {
    log_topic: String,
    default_target: String,
    show_bottom_panel: bool,
}

impl TuiActor {
    pub fn new(log_topic: String, default_target: String, show_bottom_panel: bool) -> Self {
        Self {
            log_topic,
            default_target,
            show_bottom_panel,
        }
    }
}

#[async_trait::async_trait]
impl Actor for TuiActor {
    fn id(&self) -> sar_core::ActorId {
        APP_ID.to_string()
    }

    async fn run(&self, bus: &SarBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        terminal::enable_raw_mode()?;
        crossterm::execute!(
            std::io::stderr(),
            terminal::EnterAlternateScreen,
            EnableMouseCapture
        )?;

        let backend = CrosstermBackend::new(std::io::stderr());
        let mut terminal = Terminal::new(backend)?;

        let state = Arc::new(Mutex::new(TuiState::new(
            self.show_bottom_panel,
            self.default_target.clone(),
        )));

        let bus_clone = bus.clone();
        let state_clone = state.clone();
        let log_topic_clone = self.log_topic.clone();
        tokio::spawn(async move {
            let mut rx = match bus_clone.subscribe(&log_topic_clone).await {
                Ok(rx) => rx,
                Err(e) => {
                    error!("Failed to subscribe to log: {}", e);
                    return;
                }
            };
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        let display = match &msg.payload {
                            serde_json::Value::String(s) => s.clone(),
                            _ => msg.payload.to_string(),
                        };
                        let text = format!("[{}] {}", msg.source, display);
                        let mut state = state_clone.lock().await;
                        state.add_log_entry(text);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("TUI log reader lagged behind, dropped {} messages", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("Log topic channel closed");
                        break;
                    }
                }
            }
        });

        loop {
            let show_bottom = {
                let s = state.lock().await;
                s.show_bottom_panel
            };

            let terminal_size = terminal.size()?;
            let log_height = {
                let area = Rect::new(0, 0, terminal_size.width, terminal_size.height);
                let chunks = layout_chunks(area, show_bottom);
                chunks[0].height as usize
            };

            {
                let mut s = state.lock().await;
                s.visible_lines = log_height;
            }

            let snapshot = {
                let s = state.lock().await;
                RenderSnapshot::new(&s)
            };

            terminal.draw(|frame| {
                let chunks = layout_chunks(frame.area(), show_bottom);

                let log_paragraph = Paragraph::new(snapshot.log_text)
                    .block(Block::default())
                    .scroll((snapshot.scroll as u16, snapshot.horizontal_scroll as u16));
                frame.render_widget(log_paragraph, chunks[0]);

                let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(Some("↑"))
                    .end_symbol(Some("↓"));
                let mut scrollbar_state = ScrollbarState::new(snapshot.total_lines)
                    .position(snapshot.scroll);
                frame.render_stateful_widget(scrollbar, chunks[0], &mut scrollbar_state);

                let status_paragraph = Paragraph::new(snapshot.status_text)
                    .block(Block::default().style(Style::default().bg(Color::Black)))
                    .style(Style::default().fg(Color::Green));
                frame.render_widget(status_paragraph, chunks[1]);

                let input_text = Text::from(vec![
                    Line::from(vec![
                        Span::styled("> ", Style::default().fg(Color::Yellow)),
                        Span::raw(snapshot.input_line.as_str()),
                    ]),
                ]);
                let input_paragraph = Paragraph::new(input_text)
                    .block(Block::default());
                frame.render_widget(input_paragraph, chunks[2]);
                frame.set_cursor_position((
                    chunks[2].x + 2 + snapshot.input_line.len() as u16,
                    chunks[2].y,
                ));

                if show_bottom {
                    let info_text = Text::from(vec![
                        Line::from(" Ready "),
                        Line::from(""),
                        Line::from(" SAR v0.1.0 "),
                        Line::from(" Press /quit or Ctrl+C to quit "),
                        Line::from(""),
                    ]);
                    let info_paragraph = Paragraph::new(info_text)
                        .block(Block::default().style(Style::default().bg(LIGHT_GRAY)));
                    frame.render_widget(info_paragraph, chunks[3]);
                }
            })?;

            if crossterm::event::poll(Duration::from_millis(100))? {
                let event = event::read()?;
                let mut state = state.lock().await;

                match &event {
                    Event::Key(key) => {
                        match key.code {
                            KeyCode::Left if key.modifiers == KeyModifiers::CONTROL => {
                                state.horizontal_scroll = state.horizontal_scroll.saturating_sub(10);
                            }
                            KeyCode::Right if key.modifiers == KeyModifiers::CONTROL => {
                                state.horizontal_scroll += 10;
                            }
                            KeyCode::Char('[') if key.modifiers == KeyModifiers::CONTROL => {
                                state.at_bottom = false;
                                let scroll_amount = if state.scroll >= PAGE_SIZE { PAGE_SIZE } else { state.scroll };
                                state.scroll -= scroll_amount;
                            }
                            KeyCode::Char(']') if key.modifiers == KeyModifiers::CONTROL => {
                                let max_scroll = state.max_scroll(snapshot.visible_lines);
                                let scroll_amount = if max_scroll - state.scroll >= PAGE_SIZE { PAGE_SIZE } else { max_scroll.saturating_sub(state.scroll) };
                                state.scroll += scroll_amount;
                                state.at_bottom = state.scroll >= max_scroll;
                            }
                            KeyCode::Char(c) => {
                                if key.modifiers == KeyModifiers::NONE {
                                    let line_idx = state.active_line;
                                    state.input_lines[line_idx].push(c);
                                }
                            }
                            KeyCode::Backspace => {
                                let line_idx = state.active_line;
                                let line = &mut state.input_lines[line_idx];
                                line.pop();
                            }
                            KeyCode::Enter => {
                                if key.modifiers == KeyModifiers::CONTROL {
                                    let line_idx = state.active_line;
                                    state.input_lines.insert(line_idx + 1, String::new());
                                    state.active_line += 1;
                                } else {
                                    let input = state.input_lines.join("\n");
                                    if input.trim() == "/quit" {
                                        drop(state);
                                        break;
                                    }
                                    if input.starts_with("/target ") {
                                        let new_target = input.trim().trim_start_matches("/target ").trim().to_string();
                                        if !new_target.is_empty() {
                                            state.current_target = new_target.clone();
                                            let info_msg = Message::text(
                                                &state.current_target.clone(),
                                                "system",
                                                format!("Target changed to: {}", new_target),
                                            );
                                            if let Err(e) = bus.publish(info_msg).await {
                                                error!("Failed to publish target change: {}", e);
                                            }
                                        }
                                        state.input_lines.clear();
                                        state.input_lines.push(String::new());
                                        state.active_line = 0;
                                        continue;
                                    }
                                    if input.trim() == "/bottom" {
                                        state.at_bottom = true;
                                        state.scroll = state.max_scroll(snapshot.visible_lines);
                                        state.input_lines.clear();
                                        state.input_lines.push(String::new());
                                        state.active_line = 0;
                                        continue;
                                    }
                                    if !input.is_empty() {
                                        let msg = Message::text(
                                            &state.current_target,
                                            APP_ID,
                                            input.clone(),
                                        );
                                        if let Err(e) = bus.publish(msg).await {
                                            error!("Failed to publish input: {}", e);
                                        }
                                    }
                                    state.input_lines.clear();
                                    state.input_lines.push(String::new());
                                    state.active_line = 0;
                                }
                            }
                            KeyCode::Up => {
                                if state.active_line > 0 {
                                    state.active_line -= 1;
                                }
                            }
                            KeyCode::Down => {
                                if state.active_line < state.input_lines.len() - 1 {
                                    state.active_line += 1;
                                }
                            }
                            KeyCode::Esc => {
                                state.focus_input = !state.focus_input;
                            }
                            KeyCode::PageUp => {
                                state.at_bottom = false;
                                let scroll_amount = if state.scroll >= PAGE_SIZE { PAGE_SIZE } else { state.scroll };
                                state.scroll -= scroll_amount;
                            }
                            KeyCode::PageDown => {
                                let max_scroll = state.max_scroll(snapshot.visible_lines);
                                let scroll_amount = if max_scroll - state.scroll >= PAGE_SIZE { PAGE_SIZE } else { max_scroll.saturating_sub(state.scroll) };
                                state.scroll += scroll_amount;
                                state.at_bottom = state.scroll >= max_scroll;
                            }
                            KeyCode::Home => {
                                state.at_bottom = false;
                                state.scroll = 0;
                            }
                            KeyCode::End => {
                                state.at_bottom = true;
                                state.scroll = state.max_scroll(snapshot.visible_lines);
                            }
                            _ => {}
                        }
                    }
                    Event::Mouse(mouse) => {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                state.at_bottom = false;
                                if state.scroll >= PAGE_SIZE {
                                    state.scroll -= PAGE_SIZE;
                                } else {
                                    state.scroll = 0;
                                }
                            }
                            MouseEventKind::ScrollDown => {
                                let max_scroll = state.max_scroll(snapshot.visible_lines);
                                if state.scroll + PAGE_SIZE <= max_scroll {
                                    state.scroll += PAGE_SIZE;
                                } else {
                                    state.scroll = max_scroll;
                                }
                                state.at_bottom = state.scroll >= max_scroll;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }

        crossterm::terminal::disable_raw_mode()?;
        crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        )?;

        Ok(())
    }
}

struct RenderSnapshot {
    log_text: Text<'static>,
    scroll: usize,
    horizontal_scroll: usize,
    total_lines: usize,
    input_line: String,
    status_text: Line<'static>,
    visible_lines: usize,
}

impl RenderSnapshot {
    fn new(state: &TuiState) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let seconds = now.as_secs() % 86400;
        let time_str = format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            (seconds % 3600) / 60,
            seconds % 60
        );
        let status_text = Line::from(format!(
            " {} | PID {} ",
            time_str,
            pid()
        ));
        Self {
            log_text: state.render_log(),
            scroll: state.scroll,
            horizontal_scroll: state.horizontal_scroll,
            total_lines: state.log_entries.len(),
            visible_lines: state.visible_lines,
            input_line: state.input_lines[state.active_line].clone(),
            status_text,
        }
    }
}

fn layout_chunks(area: Rect, show_bottom: bool) -> [Rect; 4] {
    let chunks = if show_bottom {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(5),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(0),
            ])
            .split(area)
    };
    [chunks[0], chunks[1], chunks[2], chunks[3]]
}

struct TuiState {
    log_entries: Vec<String>,
    scroll: usize,
    horizontal_scroll: usize,
    at_bottom: bool,
    visible_lines: usize,
    input_lines: Vec<String>,
    active_line: usize,
    focus_input: bool,
    show_bottom_panel: bool,
    current_target: String,
}

impl TuiState {
    fn new(show_bottom_panel: bool, input_topic: String) -> Self {
        Self {
            log_entries: Vec::new(),
            scroll: 0,
            horizontal_scroll: 0,
            at_bottom: true,
            visible_lines: 24,
            input_lines: vec![String::new()],
            active_line: 0,
            focus_input: true,
            show_bottom_panel,
            current_target: input_topic,
        }
    }

    fn max_scroll(&self, visible_lines: usize) -> usize {
        if self.log_entries.len() <= visible_lines {
            0
        } else {
            self.log_entries.len() - visible_lines
        }
    }

    fn add_log_entry(&mut self, entry: String) {
        self.log_entries.push(entry);
        if self.log_entries.len() > 1000 {
            self.log_entries.drain(..100);
        }
        if self.at_bottom {
            self.scroll = self.max_scroll(self.visible_lines);
        }
    }

    fn render_log(&self) -> Text<'static> {
        let lines: Vec<Line<'static>> = self.log_entries
            .iter()
            .map(|line| Line::from(line.clone()))
            .collect();
        Text::from(lines)
    }
}