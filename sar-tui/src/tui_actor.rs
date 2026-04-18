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
const BOTTOM_PAGE_SIZE: usize = 1;

#[derive(Debug, Default)]
pub struct TuiActor {
    user_topic: String,
    input_topic: String,
    bottom_panel_topic: String,
    stream_topic: String,
    show_bottom_panel: bool,
}

impl TuiActor {
    pub fn new(user_topic: String, input_topic: String, bottom_panel_topic: String, stream_topic: String, show_bottom_panel: bool) -> Self {
        Self {
            user_topic,
            input_topic,
            bottom_panel_topic,
            stream_topic,
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
            self.input_topic.clone(),
        )));

        let bus_clone = bus.clone();
        let state_clone = state.clone();
        let user_topic_clone = self.user_topic.clone();
        tokio::spawn(async move {
            let mut rx = match bus_clone.subscribe(APP_ID, &user_topic_clone).await {
                Ok(rx) => rx,
                Err(e) => {
                    error!("Failed to subscribe to user topic: {}", e);
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
                        let meta_type = msg.meta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        let text = if meta_type == "UserInput" {
                            format!("> {}", display)
                        } else {
                            format!("[{}] {}", msg.source, display)
                        };
                        let mut state = state_clone.lock().await;
                        state.add_log_entry(text);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("TUI user reader lagged behind, dropped {} messages", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("User topic channel closed");
                        break;
                    }
                }
            }
        });

        if self.show_bottom_panel {
            let bus_clone = bus.clone();
            let state_clone = state.clone();
            let bottom_topic_clone = self.bottom_panel_topic.clone();
            tokio::spawn(async move {
                let mut rx = match bus_clone.subscribe(APP_ID, &bottom_topic_clone).await {
                    Ok(rx) => rx,
                    Err(e) => {
                        error!("Failed to subscribe to bottom panel topic: {}", e);
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
                            state.add_bottom_entry(text);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!("TUI bottom panel reader lagged behind, dropped {} messages", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("Bottom panel topic channel closed");
                            break;
                        }
                    }
                }
            });
        }

        {
            let bus_clone = bus.clone();
            let state_clone = state.clone();
            let stream_topic_clone = self.stream_topic.clone();
            tokio::spawn(async move {
                let mut rx = match bus_clone.subscribe(APP_ID, &stream_topic_clone).await {
                    Ok(rx) => rx,
                    Err(e) => {
                        error!("Failed to subscribe to stream topic: {}", e);
                        return;
                    }
                };
                loop {
                    match rx.recv().await {
                        Ok(msg) => {
                            let mut state = state_clone.lock().await;
                            if let serde_json::Value::String(ref s) = msg.payload {
                                if let Ok(stream_end) = serde_json::from_str::<serde_json::Value>(s) {
                                    if let Some(type_val) = stream_end.get("type").and_then(|t| t.as_str()) {
                                        if type_val == "stream_end" {
                                            state.finalize_stream();
                                            continue;
                                        }
                                    }
                                }
                            }
                            let display = match &msg.payload {
                                serde_json::Value::String(s) => s.clone(),
                                _ => msg.payload.to_string(),
                            };
                            state.append_stream_chunk(&display);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!("TUI stream reader lagged behind, dropped {} messages", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("Stream topic channel closed");
                            break;
                        }
                    }
                }
            });
        }

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
                    let bottom_paragraph = Paragraph::new(snapshot.bottom_text)
                        .block(Block::default().style(Style::default().bg(LIGHT_GRAY)));
                    frame.render_widget(bottom_paragraph, chunks[3]);

                    let bottom_scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                        .begin_symbol(Some("↑"))
                        .end_symbol(Some("↓"));
                    let mut bottom_scrollbar_state = ScrollbarState::new(snapshot.bottom_total_lines)
                        .position(snapshot.bottom_scroll);
                    frame.render_stateful_widget(bottom_scrollbar, chunks[3], &mut bottom_scrollbar_state);
                }
            })?;

            if crossterm::event::poll(Duration::from_millis(100))? {
                let event = event::read()?;
                let mut state = state.lock().await;

                match &event {
                    Event::Key(key) => {
                        let key_str = format!("{:?} {:?}", key.modifiers, key.code);
                        state.last_key = key_str;
                        match key.code {
                            KeyCode::Left if key.modifiers == KeyModifiers::SHIFT => {
                                state.horizontal_scroll = state.horizontal_scroll.saturating_sub(10);
                            }
                            KeyCode::Right if key.modifiers == KeyModifiers::SHIFT => {
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
                                                &self.user_topic,
                                                "system",
                                                format!("Target changed to: {}", new_target),
                                            );
                                            if let Err(e) = bus.publish(APP_ID, info_msg).await {
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
                                    if input.starts_with("/log ") {
                                        let log_msg = input.trim().trim_start_matches("/log ").trim().to_string();
                                        let msg = Message::text(
                                            &self.bottom_panel_topic,
                                            APP_ID,
                                            format!("[manual] {}", log_msg),
                                        );
                                        if let Err(e) = bus.publish(APP_ID, msg).await {
                                            error!("Failed to publish log message: {}", e);
                                        }
                                        state.input_lines.clear();
                                        state.input_lines.push(String::new());
                                        state.active_line = 0;
                                        continue;
                                    }
                                    if input.trim() == "/list actors" {
                                        let actors = bus.list_actors().await;
                                        let mut lines = vec!["=== Actors ===".to_string()];
                                        for actor in &actors {
                                            lines.push(format!("  {} ({} subscriptions, {} publications)", 
                                                actor.id, actor.subscriptions.len(), actor.publications.len()));
                                            for sub in &actor.subscriptions {
                                                lines.push(format!("    - subscribes to: {}", sub));
                                            }
                                            for pub_topic in &actor.publications {
                                                lines.push(format!("    - publishes to: {}", pub_topic));
                                            }
                                        }
                                        if actors.is_empty() {
                                            lines.push("  (no actors registered)".to_string());
                                        }
                                        for line in lines {
                                            let msg = Message::text(
                                                &self.user_topic,
                                                APP_ID,
                                                line,
                                            );
                                            if let Err(e) = bus.publish(APP_ID, msg).await {
                                                error!("Failed to publish list actors message: {}", e);
                                            }
                                        }
                                        state.input_lines.clear();
                                        state.input_lines.push(String::new());
                                        state.active_line = 0;
                                        continue;
                                    }
                                    if input.trim() == "/list topics" {
                                        let topics = bus.list_topic_info().await;
                                        let mut lines = vec!["=== Topics ===".to_string()];
                                        for topic in &topics {
                                            lines.push(format!("  {} (capacity: {})", topic.name, topic.capacity));
                                            if !topic.subscribers.is_empty() {
                                                lines.push("    subscribers:".to_string());
                                                for sub in &topic.subscribers {
                                                    lines.push(format!("      - {}", sub));
                                                }
                                            }
                                            if !topic.publishers.is_empty() {
                                                lines.push("    publishers:".to_string());
                                                for publisher in &topic.publishers {
                                                    lines.push(format!("      - {}", publisher));
                                                }
                                            }
                                        }
                                        if topics.is_empty() {
                                            lines.push("  (no topics)".to_string());
                                        }
                                        for line in lines {
                                            let msg = Message::text(
                                                &self.user_topic,
                                                APP_ID,
                                                line,
                                            );
                                            if let Err(e) = bus.publish(APP_ID, msg).await {
                                                error!("Failed to publish list topics message: {}", e);
                                            }
                                        }
                                        state.input_lines.clear();
                                        state.input_lines.push(String::new());
                                        state.active_line = 0;
                                        continue;
                                    }
                                    if !input.is_empty() {
                                        let msg = Message::text(
                                            &self.input_topic,
                                            APP_ID,
                                            input.clone(),
                                        );
                                        if let Err(e) = bus.publish(APP_ID, msg).await {
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
    bottom_text: Text<'static>,
    bottom_scroll: usize,
    bottom_total_lines: usize,
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
            " {} | PID {} | {} ",
            time_str,
            pid(),
            if state.last_key.is_empty() { "" } else { &state.last_key }
        ));
        Self {
            log_text: state.render_log(),
            scroll: state.scroll,
            horizontal_scroll: state.horizontal_scroll,
            total_lines: state.total_rendered_lines(),
            bottom_text: state.render_bottom(),
            bottom_scroll: state.bottom_scroll,
            bottom_total_lines: state.bottom_total_rendered_lines(),
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

#[derive(Debug, Clone)]
struct LogItem {
    text: String,
    height: usize,
}

struct TuiState {
    log_items: Vec<LogItem>,
    scroll: usize,
    horizontal_scroll: usize,
    at_bottom: bool,
    visible_lines: usize,
    bottom_items: Vec<LogItem>,
    bottom_scroll: usize,
    bottom_at_bottom: bool,
    input_lines: Vec<String>,
    active_line: usize,
    focus_input: bool,
    show_bottom_panel: bool,
    current_target: String,
    last_key: String,
    streaming_active: bool,
}

impl TuiState {
    fn new(show_bottom_panel: bool, input_topic: String) -> Self {
        let mut bottom_items = Vec::new();
        if show_bottom_panel {
            for line in [" Ready ", "", " SAR v0.1.0 ", " Press /quit or Ctrl+C to quit ", ""] {
                let height = line.matches('\n').count() + 1;
                bottom_items.push(LogItem {
                    text: line.to_string(),
                    height,
                });
            }
        }
        Self {
            log_items: Vec::new(),
            scroll: 0,
            horizontal_scroll: 0,
            at_bottom: true,
            visible_lines: 24,
            bottom_items,
            bottom_scroll: 0,
            bottom_at_bottom: true,
            input_lines: vec![String::new()],
            active_line: 0,
            focus_input: true,
            show_bottom_panel,
            current_target: input_topic,
            last_key: String::new(),
            streaming_active: false,
        }
    }

    fn max_scroll(&self, visible_lines: usize) -> usize {
        self.total_rendered_lines().saturating_sub(visible_lines)
    }

    fn total_rendered_lines(&self) -> usize {
        self.log_items.iter().map(|item| item.height).sum()
    }

    fn add_log_entry(&mut self, entry: String) {
        let height = entry.matches('\n').count() + 1;
        self.log_items.push(LogItem {
            text: entry,
            height,
        });
        if self.log_items.len() > 1000 {
            self.log_items.drain(..100);
        }
        if self.at_bottom {
            self.scroll = self.max_scroll(self.visible_lines);
        }
    }

    fn append_stream_chunk(&mut self, chunk: &str) {
        if !self.streaming_active {
            self.log_items.push(LogItem {
                text: String::new(),
                height: 1,
            });
            self.streaming_active = true;
        }
        let parts: Vec<&str> = chunk.split('\n').collect();
        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;
            if let Some(last_item) = self.log_items.last_mut() {
                last_item.text.push_str(part);
                last_item.height = last_item.text.matches('\n').count() + 1;
            } else {
                let height = part.matches('\n').count() + 1;
                self.log_items.push(LogItem {
                    text: part.to_string(),
                    height,
                });
            }
            if !is_last {
                let new_height = 1;
                self.log_items.push(LogItem {
                    text: String::new(),
                    height: new_height,
                });
                if self.at_bottom {
                    self.scroll = self.max_scroll(self.visible_lines);
                }
            }
        }
        if self.at_bottom {
            self.scroll = self.max_scroll(self.visible_lines);
        }
    }

    fn finalize_stream(&mut self) {
        if let Some(last_item) = self.log_items.last_mut() {
            last_item.text.push('\n');
            last_item.height = last_item.text.matches('\n').count() + 1;
        }
        self.streaming_active = false;
        if self.at_bottom {
            self.scroll = self.max_scroll(self.visible_lines);
        }
    }

    fn render_log(&self) -> Text<'static> {
        let lines: Vec<Line<'static>> = self.log_items
            .iter()
            .flat_map(|item| {
                item.text.split('\n').map(|part| Line::from(part.to_string())).collect::<Vec<_>>()
            })
            .collect();
        Text::from(lines)
    }

    fn add_bottom_entry(&mut self, entry: String) {
        let height = entry.matches('\n').count() + 1;
        self.bottom_items.push(LogItem {
            text: entry,
            height,
        });
        if self.bottom_items.len() > 100 {
            self.bottom_items.drain(..10);
        }
        if self.bottom_at_bottom {
            self.bottom_scroll = self.bottom_total_rendered_lines().saturating_sub(5);
        }
    }

    fn bottom_total_rendered_lines(&self) -> usize {
        self.bottom_items.iter().map(|item| item.height).sum()
    }

    fn render_bottom(&self) -> Text<'static> {
        let max_scroll = self.bottom_total_rendered_lines().saturating_sub(5);
        let start = self.bottom_scroll.min(max_scroll);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut current_line = 0;
        for item in &self.bottom_items {
            let item_lines: Vec<Line<'static>> = item.text.split('\n').map(|part| Line::from(part.to_string())).collect();
            for line in item_lines {
                if current_line >= start && current_line < start + 5 {
                    lines.push(line);
                }
                current_line += 1;
                if current_line > start + 5 {
                    break;
                }
            }
            if current_line > start + 5 {
                break;
            }
        }
        Text::from(lines)
    }
}