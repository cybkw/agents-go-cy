// Modified for Agents Go by Agents Go contributors, 2026-10-22. See NOTICE.
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crossterm::event::{
    KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::state::Palette;
use crate::input::TerminalKey;
use crate::project_context::{self, journal};

use super::render::put_text;

const PAGE_SIZE: usize = 100;
const WIDE_WIDTH: u16 = 84;

#[derive(Debug)]
struct DisplayRecord {
    seq: u64,
    heading: String,
    time: String,
    metadata: String,
    summary: String,
    text: String,
}

impl From<journal::Record> for DisplayRecord {
    fn from(record: journal::Record) -> Self {
        let role = match record.role.as_str() {
            "user" => "用户",
            "assistant" => "助手",
            _ => "事件",
        };
        let text = visible_text(&record.text);
        let summary = text
            .chars()
            .map(|ch| if ch.is_whitespace() { ' ' } else { ch })
            .take(180)
            .collect();
        let heading = format!(
            "#{:04} {role} · {}",
            record.seq,
            visible_text(&record.agent)
        );
        let time = format!("{} UTC", timestamp(record.timestamp_ms));
        Self {
            seq: record.seq,
            metadata: format!("{heading} · {time}"),
            heading,
            time,
            summary,
            text,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Request {
    Refresh,
    Older(u64),
}

#[derive(Debug)]
struct Page {
    records: Vec<DisplayRecord>,
    has_more: bool,
}

#[derive(Debug)]
struct PendingRead {
    request: Request,
    receiver: Receiver<Result<Page, String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Focus {
    List,
    Body,
}

#[derive(Debug)]
pub(super) struct ConversationHistoryOverlay {
    cwd: String,
    records: Vec<DisplayRecord>,
    selected: usize,
    list_offset: usize,
    body_scroll: usize,
    focus: Focus,
    has_more: bool,
    error: Option<String>,
    pending: Option<PendingRead>,
    body_layout_seq: Option<u64>,
    body_layout_width: u16,
    body_lines: Vec<String>,
}

impl ConversationHistoryOverlay {
    pub(super) fn new(cwd: String) -> Self {
        let mut overlay = Self {
            cwd,
            records: Vec::new(),
            selected: 0,
            list_offset: 0,
            body_scroll: 0,
            focus: Focus::List,
            has_more: false,
            error: None,
            pending: None,
            body_layout_seq: None,
            body_layout_width: 0,
            body_lines: Vec::new(),
        };
        overlay.request(Request::Refresh);
        overlay
    }

    /// Layout may update presentation caches; drawing only reads them. Rewrap
    /// when selection or width changes, not on every terminal-output repaint.
    pub(super) fn prepare_layout(&mut self, cols: u16, rows: u16) {
        let area = window_area(Rect::new(0, 0, cols, rows));
        let inner = if area.width >= 3 && area.height >= 3 {
            Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2)
        } else {
            area
        };
        let width = if inner.width < WIDE_WIDTH {
            inner.width
        } else {
            inner
                .width
                .saturating_sub((inner.width / 5 * 2).clamp(32, 52) + 1)
        };
        let seq = self.records.get(self.selected).map(|record| record.seq);
        if self.body_layout_seq != seq || self.body_layout_width != width {
            self.body_lines = self
                .records
                .get(self.selected)
                .map_or_else(Vec::new, |record| {
                    wrapped_lines(&record.text, usize::from(width))
                });
            self.body_layout_seq = seq;
            self.body_layout_width = width;
        }
    }

    fn request(&mut self, request: Request) {
        if self.pending.is_some() {
            return;
        }
        let cwd = self.cwd.clone();
        let (sender, receiver) = mpsc::channel();
        // Resolving the project marker and waiting for the journal's shared lock
        // both belong off the input/render thread. Dropping the window drops its
        // receiver, so a late result cannot reach a newly opened window.
        match std::thread::Builder::new()
            .name("conversation-history".into())
            .spawn(move || {
                let result = load_page(&cwd, request);
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.pending = Some(PendingRead { request, receiver });
                self.error = None;
            }
            Err(error) => self.error = Some(format!("无法启动读取任务：{error}")),
        }
    }

    pub(super) fn poll(&mut self) -> bool {
        let Some(pending) = self.pending.as_ref() else {
            return false;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                Err("对话记录读取任务意外结束，请按 R 重试。".into())
            }
        };
        let request = pending.request;
        self.pending = None;
        match result {
            Ok(page) => self.apply_page(request, page),
            Err(error) => self.error = Some(visible_text(&error).replace('\n', "␊")),
        }
        true
    }

    fn apply_page(&mut self, request: Request, page: Page) {
        let selected_seq = self.records.get(self.selected).map(|record| record.seq);
        self.has_more = page.has_more;
        self.error = None;
        match request {
            Request::Refresh => {
                // A refresh can replace the text even when the same sequence
                // remains selected (for example, after an external repair).
                self.body_layout_seq = None;
                self.records = page.records;
                self.selected = selected_seq
                    .and_then(|seq| self.records.iter().position(|record| record.seq == seq))
                    .unwrap_or(0);
                if self.records.get(self.selected).map(|record| record.seq) != selected_seq {
                    self.body_scroll = 0;
                }
                self.list_offset = 0;
            }
            Request::Older(_) => {
                let oldest = self.records.last().map_or(u64::MAX, |record| record.seq);
                self.records.extend(
                    page.records
                        .into_iter()
                        .filter(|record| record.seq < oldest),
                );
            }
        }
    }

    fn older(&mut self) {
        if self.has_more {
            if let Some(through) = self
                .records
                .last()
                .and_then(|record| record.seq.checked_sub(1))
            {
                if through > 0 {
                    self.request(Request::Older(through));
                }
            }
        }
    }

    fn select(&mut self, index: usize, hits: &ConversationHistoryHits) {
        let index = index.min(self.records.len().saturating_sub(1));
        if self.selected != index {
            self.selected = index;
            self.body_scroll = 0;
        }
        let rows = hits.visible_rows.max(1);
        self.list_offset = hits.list_offset.min(self.selected);
        if self.selected >= self.list_offset.saturating_add(rows) {
            self.list_offset = self.selected.saturating_add(1).saturating_sub(rows);
        }
    }

    fn scroll_body(&mut self, delta: isize, hits: &ConversationHistoryHits) {
        // Clamp against the geometry most recently drawn, including after resize.
        let current = self.body_scroll.min(hits.max_body_scroll);
        self.body_scroll = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current
                .saturating_add(delta as usize)
                .min(hits.max_body_scroll)
        };
    }

    pub(super) fn handle_key(&mut self, key: &TerminalKey, hits: &ConversationHistoryHits) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return false;
        }
        match key.code {
            KeyCode::Esc if hits.narrow && self.focus == Focus::Body => self.focus = Focus::List,
            KeyCode::Esc => return true,
            KeyCode::Char('r' | 'R') => self.request(Request::Refresh),
            KeyCode::Char('n' | 'N') => self.older(),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.focus == Focus::List {
                    Focus::Body
                } else {
                    Focus::List
                };
            }
            KeyCode::Right | KeyCode::Enter => self.focus = Focus::Body,
            KeyCode::Left => self.focus = Focus::List,
            KeyCode::Up if self.focus == Focus::List => {
                self.select(self.selected.saturating_sub(1), hits)
            }
            KeyCode::Down if self.focus == Focus::List => {
                self.select(self.selected.saturating_add(1), hits)
            }
            KeyCode::Up => self.scroll_body(-1, hits),
            KeyCode::Down => self.scroll_body(1, hits),
            KeyCode::PageUp => {
                self.focus = Focus::Body;
                self.scroll_body(
                    -(isize::try_from(hits.body.height)
                        .unwrap_or(isize::MAX)
                        .max(1)),
                    hits,
                );
            }
            KeyCode::PageDown => {
                self.focus = Focus::Body;
                self.scroll_body(
                    isize::try_from(hits.body.height)
                        .unwrap_or(isize::MAX)
                        .max(1),
                    hits,
                );
            }
            KeyCode::Home if self.focus == Focus::List => self.select(0, hits),
            KeyCode::End if self.focus == Focus::List => {
                self.select(self.records.len().saturating_sub(1), hits)
            }
            KeyCode::Home => self.body_scroll = 0,
            KeyCode::End => self.body_scroll = hits.max_body_scroll,
            _ => {}
        }
        false
    }

    pub(super) fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        hits: &ConversationHistoryHits,
    ) -> bool {
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if hits.close.contains(point) {
                    if hits.narrow && self.focus == Focus::Body {
                        self.focus = Focus::List;
                        return false;
                    }
                    return true;
                }
                if hits.refresh.contains(point) {
                    self.request(Request::Refresh);
                } else if hits.older.contains(point) {
                    self.older();
                } else if let Some((_, index)) =
                    hits.rows.iter().find(|(area, _)| area.contains(point))
                {
                    self.focus = if hits.narrow {
                        Focus::Body
                    } else {
                        Focus::List
                    };
                    self.select(*index, hits);
                } else if hits.body.contains(point) {
                    self.focus = Focus::Body;
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let down = mouse.kind == MouseEventKind::ScrollDown;
                if hits.body.contains(point) {
                    self.focus = Focus::Body;
                    self.scroll_body(if down { 3 } else { -3 }, hits);
                } else if hits.list.contains(point) {
                    self.focus = Focus::List;
                    self.select(
                        if down {
                            self.selected.saturating_add(3)
                        } else {
                            self.selected.saturating_sub(3)
                        },
                        hits,
                    );
                }
            }
            _ => {}
        }
        false
    }
}

fn load_page(cwd: &str, request: Request) -> Result<Page, String> {
    let root = project_context::project_root(Path::new(cwd)).map_err(|error| {
        if error.starts_with("project context is not enabled") {
            "此项目尚未启用对话记录。请在项目目录运行 herdr context enable。".into()
        } else {
            format!("无法读取项目配置：{error}")
        }
    })?;
    let through = match request {
        Request::Refresh => None,
        Request::Older(through) => Some(through),
    };
    let mut records = journal::read(&root, None, through, PAGE_SIZE + 1)?;
    let has_more = records.len() > PAGE_SIZE;
    records.reverse();
    records.truncate(PAGE_SIZE);
    Ok(Page {
        records: records.into_iter().map(DisplayRecord::from).collect(),
        has_more,
    })
}

/// Render control bytes as visible characters, never as terminal instructions.
/// A journal entry remains plain text, including code fences and ANSI fragments.
fn visible_text(text: &str) -> String {
    let mut visible = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => visible.push('\n'),
            '\t' => visible.push_str("    "),
            '\u{0}'..='\u{1f}' => visible.push(char::from_u32(0x2400 + ch as u32).unwrap()),
            '\u{7f}' => visible.push('␡'),
            '\u{80}'..='\u{9f}' => visible.push('�'),
            _ => visible.push(ch),
        }
    }
    visible
}

fn timestamp(milliseconds: u64) -> String {
    let Ok(date) =
        time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000)
    else {
        return "时间未知".into();
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        date.year(),
        u8::from(date.month()),
        date.day(),
        date.hour(),
        date.minute()
    )
}

#[derive(Default)]
pub(super) struct ConversationHistoryHits {
    list: Rect,
    body: Rect,
    refresh: Rect,
    older: Rect,
    close: Rect,
    rows: Vec<(Rect, usize)>,
    visible_rows: usize,
    list_offset: usize,
    max_body_scroll: usize,
    narrow: bool,
}

pub(super) struct ConversationHistoryRender {
    pub(super) area: Rect,
    pub(super) hits: ConversationHistoryHits,
}

fn window_area(screen: Rect) -> Rect {
    let margin_x = u16::from(screen.width >= 30) * 2;
    let margin_y = u16::from(screen.height >= 10);
    Rect::new(
        screen.x + margin_x,
        screen.y + margin_y,
        screen.width.saturating_sub(margin_x * 2),
        screen.height.saturating_sub(margin_y * 2),
    )
}

pub(super) fn render(
    buffer: &mut Buffer,
    overlay: &ConversationHistoryOverlay,
    palette: &Palette,
) -> Option<ConversationHistoryRender> {
    let screen = buffer.area;
    if screen.is_empty() {
        return None;
    }
    let area = window_area(screen);
    let base = Style::default().fg(palette.text).bg(palette.panel_bg);
    let muted = base.fg(palette.overlay0);
    let accent = base.fg(palette.accent);
    buffer.set_style(area, base);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            buffer[(x, y)].set_symbol(" ");
        }
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(accent)
        .style(base);
    let inner = if area.width >= 3 && area.height >= 3 {
        let inner = block.inner(area);
        block.render(area, buffer);
        inner
    } else {
        area
    };
    let mut hits = ConversationHistoryHits {
        narrow: inner.width < WIDE_WIDTH,
        ..Default::default()
    };
    if inner.is_empty() {
        return Some(ConversationHistoryRender { area, hits });
    }
    hits.close = Rect::new(
        inner.right().saturating_sub(6.min(inner.width)),
        inner.y,
        6.min(inner.width),
        1,
    );
    put_text(
        buffer,
        inner.x,
        inner.y,
        inner.width.saturating_sub(hits.close.width),
        "对话记录",
        accent.add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        hits.close.x,
        hits.close.y,
        hits.close.width,
        if hits.narrow && overlay.focus == Focus::Body {
            "[返回]"
        } else {
            "[关闭]"
        },
        muted,
    );
    let header_rows = if inner.height >= 9 {
        3
    } else if inner.height >= 5 {
        2
    } else {
        1
    };
    let footer_rows = if inner.height >= 7 {
        2
    } else {
        1.min(inner.height.saturating_sub(header_rows))
    };
    if header_rows >= 3 {
        put_text(
            buffer,
            inner.x,
            inner.y + 1,
            inner.width,
            &visible_text(&overlay.cwd).replace('\n', "␊"),
            muted,
        );
    }
    if header_rows >= 2 {
        let status = if overlay.pending.is_some() {
            "正在读取…".to_owned()
        } else if let Some(error) = &overlay.error {
            error.clone()
        } else if overlay.records.is_empty() {
            "暂无对话记录，已采集的用户发言和最终回复将在这里显示。".into()
        } else {
            format!(
                "已载入 {} 条 · 最新在前 · 时间为 UTC{}",
                overlay.records.len(),
                if overlay.has_more {
                    " · N 加载更早"
                } else {
                    ""
                }
            )
        };
        put_text(
            buffer,
            inner.x,
            inner.y + header_rows - 1,
            inner.width,
            &status,
            if overlay.error.is_some() {
                base.fg(palette.red)
            } else {
                muted
            },
        );
    }
    let content = Rect::new(
        inner.x,
        inner.y + header_rows,
        inner.width,
        inner.height.saturating_sub(header_rows + footer_rows),
    );
    if !content.is_empty() {
        if hits.narrow {
            if overlay.focus == Focus::List {
                hits.list = content;
            } else {
                hits.body = content;
            }
        } else {
            let list_width = (content.width / 5 * 2).clamp(32, 52);
            hits.list = Rect::new(content.x, content.y, list_width, content.height);
            hits.body = Rect::new(
                content.x + list_width + 1,
                content.y,
                content.width.saturating_sub(list_width + 1),
                content.height,
            );
            for y in content.y..content.bottom() {
                put_text(buffer, content.x + list_width, y, 1, "│", muted);
            }
        }
        render_list(buffer, overlay, palette, &mut hits);
        render_body(buffer, overlay, palette, &mut hits);
    }
    if footer_rows > 0 {
        let y = inner.bottom() - footer_rows;
        let refresh_width = inner.width.min(14);
        hits.refresh = Rect::new(inner.x, y, refresh_width, 1);
        let older_width = inner.width.saturating_sub(refresh_width + 1).min(18);
        if overlay.has_more && older_width > 0 {
            hits.older = Rect::new(inner.x + refresh_width + 1, y, older_width, 1);
        }
        put_text(
            buffer,
            hits.refresh.x,
            y,
            hits.refresh.width,
            "[R 刷新]",
            if overlay.pending.is_some() {
                muted
            } else {
                accent
            },
        );
        if !hits.older.is_empty() {
            put_text(
                buffer,
                hits.older.x,
                y,
                hits.older.width,
                "[N 加载更早]",
                if overlay.pending.is_some() {
                    muted
                } else {
                    accent
                },
            );
        }
        if footer_rows > 1 {
            let help = if hits.narrow {
                if overlay.focus == Focus::List {
                    "↑↓选择  Enter 查看正文  Esc 关闭"
                } else {
                    "↑↓/滚轮/PgUp/PgDn 滚动  Home/End  Esc 返回列表"
                }
            } else {
                "↑↓选择/滚动  Tab/←→切换  PgUp/PgDn/Home/End  Esc关闭"
            };
            put_text(
                buffer,
                inner.x,
                inner.bottom() - 1,
                inner.width,
                help,
                muted,
            );
        }
    }
    Some(ConversationHistoryRender { area, hits })
}

fn render_list(
    buffer: &mut Buffer,
    overlay: &ConversationHistoryOverlay,
    palette: &Palette,
    hits: &mut ConversationHistoryHits,
) {
    let area = hits.list;
    if area.is_empty() {
        return;
    }
    let base = Style::default().fg(palette.text).bg(palette.panel_bg);
    let row_height: u16 = if area.height >= 6 {
        3
    } else {
        2.min(area.height)
    };
    hits.visible_rows = usize::from(area.height / row_height).max(1);
    let max_offset = overlay.records.len().saturating_sub(hits.visible_rows);
    hits.list_offset = overlay.list_offset.min(max_offset).min(overlay.selected);
    if overlay.selected >= hits.list_offset + hits.visible_rows {
        hits.list_offset = overlay.selected + 1 - hits.visible_rows;
    }
    if overlay.records.is_empty() {
        put_text(
            buffer,
            area.x,
            area.y,
            area.width,
            if overlay.pending.is_some() {
                "正在载入记录…"
            } else if overlay.error.is_some() {
                "读取失败，按 R 重试。"
            } else {
                "暂无记录"
            },
            base.fg(palette.overlay0),
        );
    }
    for (row, record) in overlay
        .records
        .iter()
        .skip(hits.list_offset)
        .take(hits.visible_rows)
        .enumerate()
    {
        let index = hits.list_offset + row;
        let y = area.y + row as u16 * row_height;
        let rect = Rect::new(area.x, y, area.width, row_height);
        let selected = index == overlay.selected;
        let style = if selected {
            base.bg(palette.selection_bg)
        } else {
            base
        };
        buffer.set_style(rect, style);
        let marker = if selected { "› " } else { "  " };
        put_text(
            buffer,
            area.x,
            y,
            area.width,
            &format!("{marker}{}", record.heading),
            if selected {
                style.fg(palette.accent)
            } else {
                style
            },
        );
        if row_height > 2 {
            put_text(
                buffer,
                area.x + 2.min(area.width),
                y + 1,
                area.width.saturating_sub(2),
                &record.time,
                style.fg(palette.overlay0),
            );
        }
        if row_height > 1 {
            put_text(
                buffer,
                area.x + 2.min(area.width),
                y + row_height - 1,
                area.width.saturating_sub(2),
                &record.summary,
                style.fg(palette.subtext0),
            );
        }
        hits.rows.push((rect, index));
    }
}

fn render_body(
    buffer: &mut Buffer,
    overlay: &ConversationHistoryOverlay,
    palette: &Palette,
    hits: &mut ConversationHistoryHits,
) {
    let area = hits.body;
    if area.is_empty() {
        return;
    }
    let base = Style::default().fg(palette.text).bg(palette.panel_bg);
    let Some(record) = overlay.records.get(overlay.selected) else {
        put_text(
            buffer,
            area.x,
            area.y,
            area.width,
            "选择一条记录查看完整正文。",
            base.fg(palette.overlay0),
        );
        return;
    };
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &record.metadata,
        base.fg(if overlay.focus == Focus::Body {
            palette.accent
        } else {
            palette.subtext0
        }),
    );
    hits.body = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );
    if hits.body.is_empty() {
        return;
    }
    let lines = &overlay.body_lines;
    hits.max_body_scroll = lines.len().saturating_sub(usize::from(hits.body.height));
    let offset = overlay.body_scroll.min(hits.max_body_scroll);
    for (row, line) in lines
        .iter()
        .skip(offset)
        .take(usize::from(hits.body.height))
        .enumerate()
    {
        put_text(
            buffer,
            hits.body.x,
            hits.body.y + row as u16,
            hits.body.width,
            line,
            base,
        );
    }
}

/// Keep grapheme clusters intact, including CJK wide characters and emoji. The
/// viewport is indexed with usize rather than Paragraph's u16 scroll offset.
fn wrapped_lines(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut result = Vec::new();
    for logical in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for grapheme in logical.graphemes(true) {
            let columns = UnicodeWidthStr::width(grapheme);
            if columns > width {
                if !line.is_empty() {
                    result.push(std::mem::take(&mut line));
                    used = 0;
                }
                result.push("�".into());
                continue;
            }
            if used + columns > width && !line.is_empty() {
                result.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push_str(grapheme);
            used += columns;
        }
        if !line.is_empty() || logical.is_empty() {
            result.push(line);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    static NEXT_PROJECT: AtomicU64 = AtomicU64::new(0);

    struct Project(PathBuf);

    impl Project {
        fn new(enabled: bool) -> Self {
            let path = std::env::temp_dir().join(format!(
                "agents-go-history-window-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_PROJECT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir_all(path.join(".git")).unwrap();
            if enabled {
                std::fs::create_dir_all(path.join(".agents-go")).unwrap();
                std::fs::write(path.join(".agents-go/project.json"), b"{\"version\":1}").unwrap();
            }
            Self(path)
        }

        fn cwd(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }

        fn history(&self) -> PathBuf {
            self.0.join(".agents-go/history.jsonl")
        }

        fn write_records(&self, count: u64) {
            let mut bytes = Vec::new();
            for seq in 1..=count {
                serde_json::to_writer(&mut bytes, &record(seq, &format!("原文 {seq}\n下一行")))
                    .unwrap();
                bytes.push(b'\n');
            }
            std::fs::write(self.history(), bytes).unwrap();
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn record(seq: u64, text: &str) -> journal::Record {
        serde_json::from_value(serde_json::json!({
            "seq": seq, "timestamp_ms": 0, "agent": "codex", "session_id": "test",
            "turn_id": null, "role": "user", "text": text, "cwd": "test", "reply_to": null,
        }))
        .unwrap()
    }

    fn page(sequences: &[u64], has_more: bool) -> Page {
        Page {
            records: sequences
                .iter()
                .map(|seq| DisplayRecord::from(record(*seq, "正文\n第二行")))
                .collect(),
            has_more,
        }
    }

    fn empty() -> ConversationHistoryOverlay {
        ConversationHistoryOverlay {
            cwd: "test-project".into(),
            records: Vec::new(),
            selected: 0,
            list_offset: 0,
            body_scroll: 0,
            focus: Focus::List,
            has_more: false,
            error: None,
            pending: None,
            body_layout_seq: None,
            body_layout_width: 0,
            body_lines: Vec::new(),
        }
    }

    fn key(code: KeyCode) -> TerminalKey {
        TerminalKey::new(code, KeyModifiers::empty())
    }

    fn wait_for_result(overlay: &mut ConversationHistoryOverlay) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !overlay.poll() {
            assert!(Instant::now() < deadline, "background read did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn draw(
        overlay: &mut ConversationHistoryOverlay,
        width: u16,
        height: u16,
    ) -> (Buffer, ConversationHistoryHits) {
        overlay.prepare_layout(width, height);
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
        let result = render(&mut buffer, overlay, &Palette::catppuccin()).unwrap();
        (buffer, result.hits)
    }

    fn buffer_text(buffer: &Buffer) -> String {
        let mut text = String::new();
        for y in buffer.area.y..buffer.area.bottom() {
            let mut x = buffer.area.x;
            while x < buffer.area.right() {
                let symbol = buffer[(x, y)].symbol();
                text.push_str(symbol);
                // Ratatui stores a blank continuation cell after a wide glyph.
                // Reconstruct displayed text without inserting that cell into CJK.
                x += UnicodeWidthStr::width(symbol).max(1) as u16;
            }
        }
        text
    }

    #[test]
    fn newest_first_pages_load_every_sequence_once_without_writing_history() {
        let project = Project::new(true);
        project.write_records(205);
        let original = std::fs::read(project.history()).unwrap();
        let mut overlay = ConversationHistoryOverlay::new(project.cwd());
        wait_for_result(&mut overlay);
        assert_eq!(overlay.records.len(), PAGE_SIZE);
        assert_eq!(overlay.records.first().unwrap().seq, 205);
        assert_eq!(overlay.records.last().unwrap().seq, 106);
        assert!(overlay.has_more);
        overlay.older();
        wait_for_result(&mut overlay);
        assert_eq!(overlay.records.len(), 200);
        assert_eq!(overlay.records.last().unwrap().seq, 6);
        assert!(overlay.has_more);
        overlay.older();
        wait_for_result(&mut overlay);
        assert_eq!(
            overlay
                .records
                .iter()
                .map(|record| record.seq)
                .collect::<Vec<_>>(),
            (1..=205).rev().collect::<Vec<_>>()
        );
        assert!(!overlay.has_more);
        assert_eq!(std::fs::read(project.history()).unwrap(), original);
    }

    #[test]
    fn absent_journal_and_disabled_project_do_not_create_files() {
        let enabled = Project::new(true);
        let mut overlay = ConversationHistoryOverlay::new(enabled.cwd());
        wait_for_result(&mut overlay);
        assert!(overlay.records.is_empty());
        assert!(overlay.error.is_none());
        assert!(!enabled.history().exists());
        assert!(buffer_text(&draw(&mut overlay, 100, 25).0).contains("暂无对话记录"));

        let disabled = Project::new(false);
        let mut overlay = ConversationHistoryOverlay::new(disabled.cwd());
        wait_for_result(&mut overlay);
        assert!(overlay.error.as_ref().unwrap().contains("尚未启用"));
        assert!(!disabled.0.join(".agents-go").exists());
    }

    #[test]
    fn broken_journal_reports_error_and_refresh_recovers() {
        let project = Project::new(true);
        std::fs::write(project.history(), b"{broken}\n").unwrap();
        let mut overlay = ConversationHistoryOverlay::new(project.cwd());
        wait_for_result(&mut overlay);
        assert!(overlay
            .error
            .as_ref()
            .unwrap()
            .contains("Invalid project history record"));
        assert!(buffer_text(&draw(&mut overlay, 130, 25).0).contains("读取失败"));
        project.write_records(1);
        overlay.handle_key(
            &key(KeyCode::Char('R')),
            &ConversationHistoryHits::default(),
        );
        wait_for_result(&mut overlay);
        assert_eq!(overlay.records.len(), 1);
        assert!(overlay.error.is_none());
    }

    #[test]
    fn refresh_retains_selected_sequence_and_older_pages_do_not_duplicate() {
        let mut overlay = empty();
        overlay.apply_page(Request::Refresh, page(&[5, 4, 3], true));
        overlay.selected = 1;
        overlay.body_scroll = 7;
        overlay.apply_page(Request::Refresh, page(&[6, 5, 4], true));
        assert_eq!(overlay.records[overlay.selected].seq, 4);
        assert_eq!(overlay.body_scroll, 7);
        overlay.apply_page(Request::Older(3), page(&[4, 3, 2, 1], false));
        assert_eq!(
            overlay
                .records
                .iter()
                .map(|record| record.seq)
                .collect::<Vec<_>>(),
            vec![6, 5, 4, 3, 2, 1]
        );
        overlay.apply_page(Request::Refresh, page(&[12, 11], false));
        assert_eq!(overlay.selected, 0);
        assert_eq!(overlay.body_scroll, 0);
    }

    #[test]
    fn pending_request_ignores_refresh_and_closed_windows_discard_old_results() {
        let mut overlay = empty();
        let (old_sender, receiver) = mpsc::channel();
        overlay.pending = Some(PendingRead {
            request: Request::Refresh,
            receiver,
        });
        overlay.request(Request::Refresh);
        old_sender.send(Ok(page(&[1], false))).unwrap();
        assert!(overlay.poll());
        assert_eq!(overlay.records[0].seq, 1);
        assert!(!overlay.poll());

        let (old_sender, receiver) = mpsc::channel();
        overlay.pending = Some(PendingRead {
            request: Request::Refresh,
            receiver,
        });
        drop(overlay);
        assert!(old_sender.send(Ok(page(&[99], false))).is_err());
        let mut reopened = empty();
        let (sender, receiver) = mpsc::channel();
        reopened.pending = Some(PendingRead {
            request: Request::Refresh,
            receiver,
        });
        sender.send(Ok(page(&[2], false))).unwrap();
        assert!(reopened.poll());
        assert_eq!(reopened.records[0].seq, 2);
    }

    #[test]
    fn disconnected_background_reader_reports_error() {
        let mut overlay = empty();
        let (sender, receiver) = mpsc::channel();
        overlay.pending = Some(PendingRead {
            request: Request::Refresh,
            receiver,
        });
        drop(sender);
        assert!(overlay.poll());
        assert!(overlay.error.as_ref().unwrap().contains("意外结束"));
        assert!(overlay.pending.is_none());
    }

    #[test]
    fn file_lock_wait_does_not_block_window_input() {
        let project = Project::new(true);
        project.write_records(1);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(project.history())
            .unwrap();
        file.lock().unwrap();
        let mut overlay = ConversationHistoryOverlay::new(project.cwd());
        assert!(!overlay.poll());
        assert!(overlay.handle_key(&key(KeyCode::Esc), &ConversationHistoryHits::default()));
        assert!(buffer_text(&draw(&mut overlay, 100, 25).0).contains("正在读取"));
        drop(file);
        wait_for_result(&mut overlay);
        assert_eq!(overlay.records.len(), 1);
    }

    #[test]
    fn reader_remains_consistent_during_hook_capture() {
        let project = Project::new(true);
        project.write_records(1);
        let root = project.0.clone();
        let writer = std::thread::spawn(move || {
            for index in 0..20 {
                journal::capture(
                    &root,
                    "codex",
                    &serde_json::json!({
                        "hook_event_name": "UserPromptSubmit", "session_id": "concurrent",
                        "cwd": root.to_string_lossy(), "prompt": format!("并发 {index}"),
                    }),
                )
                .unwrap();
            }
        });
        for _ in 0..10 {
            let page = load_page(&project.cwd(), Request::Refresh).unwrap();
            assert!(page
                .records
                .windows(2)
                .all(|pair| pair[0].seq == pair[1].seq + 1));
        }
        writer.join().unwrap();
        let page = load_page(&project.cwd(), Request::Refresh).unwrap();
        assert_eq!(page.records.len(), 21);
    }

    #[test]
    fn chinese_multiline_graphemes_and_control_bytes_render_safely() {
        assert_eq!(
            wrapped_lines("中文正文\n\n下一行\n", 4),
            vec!["中文", "正文", "", "下一", "行", ""]
        );
        assert_eq!(wrapped_lines("e\u{301}👩‍💻中", 3), vec!["e\u{301}👩‍💻", "中"]);
        assert_eq!(wrapped_lines("中文", 1), vec!["�", "�"]);
        assert!(wrapped_lines("中文", 0).is_empty());
        let mut overlay = empty();
        overlay.records.push(DisplayRecord::from(record(
            1,
            "中文\n\u{1b}[2J\u{7}\u{9b}原文\r\n末尾",
        )));
        let (buffer, hits) = draw(&mut overlay, 120, 25);
        let rendered = buffer_text(&buffer);
        assert!(rendered.contains("␛[2J␇�原文␍"));
        assert!(!rendered.chars().any(char::is_control));
        assert!(!hits.body.is_empty());
    }

    #[test]
    fn keyboard_and_mouse_select_and_scroll_full_body() {
        let mut overlay = empty();
        overlay.records = (1..=20)
            .rev()
            .map(|seq| {
                DisplayRecord::from(record(
                    seq,
                    &(0..100).map(|n| format!("正文 {n}\n")).collect::<String>(),
                ))
            })
            .collect();
        let (_, hits) = draw(&mut overlay, 120, 25);
        overlay.handle_key(&key(KeyCode::Down), &hits);
        assert_eq!(overlay.selected, 1);
        overlay.handle_key(&key(KeyCode::Tab), &hits);
        assert_eq!(overlay.focus, Focus::Body);
        overlay.handle_key(&key(KeyCode::PageDown), &hits);
        assert!(overlay.body_scroll > 0);
        overlay.handle_key(&key(KeyCode::End), &hits);
        assert_eq!(overlay.body_scroll, hits.max_body_scroll);
        overlay.handle_key(&key(KeyCode::Home), &hits);
        assert_eq!(overlay.body_scroll, 0);
        let body_mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: hits.body.x,
            row: hits.body.y,
            modifiers: KeyModifiers::empty(),
        };
        overlay.handle_mouse(body_mouse, &hits);
        assert_eq!(overlay.body_scroll, 3);
        let (row, index) = hits.rows[2];
        overlay.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: row.x,
                row: row.y,
                modifiers: KeyModifiers::empty(),
            },
            &hits,
        );
        assert_eq!(overlay.selected, index);
        assert_eq!(overlay.body_scroll, 0);
        overlay.handle_key(&key(KeyCode::End), &hits);
        let (_, hits) = draw(&mut overlay, 120, 25);
        assert!(hits
            .rows
            .iter()
            .any(|(_, index)| *index == overlay.selected));
        assert!(overlay.handle_key(&key(KeyCode::Esc), &hits));
    }

    #[test]
    fn narrow_window_uses_list_then_body_then_escape_back_and_release_is_ignored() {
        let mut overlay = empty();
        overlay.apply_page(Request::Refresh, page(&[1], false));
        let (_, hits) = draw(&mut overlay, 45, 20);
        assert!(hits.narrow);
        assert!(!hits.list.is_empty());
        assert!(hits.body.is_empty());
        overlay.handle_key(&key(KeyCode::Enter), &hits);
        let (buffer, hits) = draw(&mut overlay, 45, 20);
        assert!(hits.list.is_empty());
        assert!(!hits.body.is_empty());
        assert!(buffer_text(&buffer).contains("正文"));
        assert!(!overlay.handle_key(&key(KeyCode::Esc).with_kind(KeyEventKind::Release), &hits));
        assert_eq!(overlay.focus, Focus::Body);
        assert!(!overlay.handle_key(&key(KeyCode::Esc), &hits));
        assert_eq!(overlay.focus, Focus::List);
        assert!(overlay.handle_key(&key(KeyCode::Esc), &hits));
    }

    #[test]
    fn narrow_window_mouse_opens_record_then_returns_to_list_and_closes() {
        let mut overlay = empty();
        overlay.apply_page(Request::Refresh, page(&[2, 1], false));
        let (_, hits) = draw(&mut overlay, 45, 20);
        let row = hits.rows[1].0;
        let click = |rect: Rect| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        };
        assert!(!overlay.handle_mouse(click(row), &hits));
        assert_eq!(overlay.focus, Focus::Body);
        assert_eq!(overlay.selected, 1);
        let (buffer, hits) = draw(&mut overlay, 45, 20);
        assert!(buffer_text(&buffer).contains("[返回]"));
        assert!(!overlay.handle_mouse(click(hits.close), &hits));
        assert_eq!(overlay.focus, Focus::List);
        let (buffer, hits) = draw(&mut overlay, 45, 20);
        assert!(buffer_text(&buffer).contains("[关闭]"));
        assert!(overlay.handle_mouse(click(hits.close), &hits));
    }

    #[test]
    fn all_small_geometries_render_without_out_of_bounds_or_mutating_state() {
        let mut overlay = empty();
        overlay.apply_page(Request::Refresh, page(&[3, 2, 1], true));
        overlay.selected = 2;
        overlay.body_scroll = usize::MAX;
        overlay.list_offset = usize::MAX;
        for width in 1..=90 {
            for height in 1..=15 {
                let (_, hits) = draw(&mut overlay, width, height);
                for (rect, _) in &hits.rows {
                    assert!(rect.right() <= width && rect.bottom() <= height);
                }
            }
        }
        overlay.focus = Focus::Body;
        for width in 1..=90 {
            let _ = draw(&mut overlay, width, 8);
        }
        assert_eq!(overlay.selected, 2);
        assert_eq!(overlay.body_scroll, usize::MAX);
        assert_eq!(overlay.list_offset, usize::MAX);
        let mut buffer = Buffer::empty(Rect::default());
        assert!(render(&mut buffer, &overlay, &Palette::catppuccin()).is_none());
    }

    #[test]
    fn timestamps_are_explicitly_utc_and_large_values_are_safe() {
        assert_eq!(timestamp(0), "1970-01-01 00:00");
        assert_eq!(timestamp(u64::MAX), "时间未知");
        assert!(DisplayRecord::from(record(1, "test"))
            .metadata
            .ends_with("UTC"));
    }

    #[test]
    fn body_layout_cache_reuses_wrapping_and_rebuilds_for_resize_and_selection() {
        let mut overlay = empty();
        overlay.apply_page(Request::Refresh, page(&[2, 1], false));
        overlay.prepare_layout(120, 25);
        let pointer = overlay.body_lines.as_ptr();
        overlay.prepare_layout(120, 30);
        assert_eq!(overlay.body_lines.as_ptr(), pointer);
        assert_eq!(overlay.body_layout_seq, Some(2));
        let old_width = overlay.body_layout_width;
        overlay.prepare_layout(60, 25);
        assert_ne!(overlay.body_layout_width, old_width);
        overlay.selected = 1;
        overlay.prepare_layout(60, 25);
        assert_eq!(overlay.body_layout_seq, Some(1));
        overlay.apply_page(
            Request::Refresh,
            Page {
                records: vec![DisplayRecord::from(record(1, "替换后的正文"))],
                has_more: false,
            },
        );
        overlay.prepare_layout(60, 25);
        assert_eq!(overlay.body_lines, vec!["替换后的正文"]);
    }
}
