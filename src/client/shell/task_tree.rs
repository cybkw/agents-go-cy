// Modified for Agents Go by Agents Go contributors, 2026-10-01. See NOTICE.
//! Client-owned project tree. Nodes refer to existing workspace and pane IDs;
//! expanding a node never changes runtime state or acknowledges a completion.
use super::render::{put_right_text, put_text, ShellRenderState};
use super::*;
use crate::protocol::{ClientShellAgent, ClientShellPane};

/// Height of one "recent projects" row. Larger rows give a bigger click target.
pub(super) const RECENT_ROW_HEIGHT: u16 = 2;

fn subdued_tree_background(color: ratatui::style::Color) -> ratatui::style::Color {
    match color {
        ratatui::style::Color::Rgb(r, g, b) => {
            let value = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
            let shade = if value < 128 { value * 2 / 3 } else { value } as u8;
            ratatui::style::Color::Rgb(shade, shade, shade)
        }
        other => other,
    }
}

enum Row<'a> {
    Machine(&'a ClientShellEndpoint),
    Workspace {
        endpoint: &'a ClientShellEndpoint,
        snapshot: &'a ClientShellSnapshot,
        entry: WorkspaceEntry,
        attention: (usize, usize),
    },
    Pane {
        endpoint: &'a ClientShellEndpoint,
        pane: &'a ClientShellPane,
        agent: Option<&'a ClientShellAgent>,
        depth: u16,
        last: bool,
        duplicate: bool,
    },
}

fn rows<'a>(
    endpoints: &'a [ClientShellEndpoint],
    active_endpoint: &ClientEndpointId,
    active_snapshot: Option<&'a ClientShellSnapshot>,
    collapsed_endpoints: &HashSet<ClientEndpointId>,
    collapsed_groups: &HashSet<String>,
    remote_groups: &HashMap<ClientEndpointId, HashSet<String>>,
    collapsed_tasks: &HashSet<(ClientEndpointId, String)>,
) -> Vec<Row<'a>> {
    let mut rows = Vec::new();
    let empty_groups = HashSet::new();
    let show_machines = endpoints.len() > 1 || active_snapshot.is_none();
    for endpoint in endpoints {
        if show_machines {
            rows.push(Row::Machine(endpoint));
            if collapsed_endpoints.contains(&endpoint.endpoint_id) {
                continue;
            }
        }
        let snapshot = if &endpoint.endpoint_id == active_endpoint {
            active_snapshot.or(endpoint.snapshot.as_deref())
        } else {
            endpoint.snapshot.as_deref()
        };
        let Some(snapshot) = snapshot else { continue };
        let groups = if endpoint.endpoint_id.is_local() {
            collapsed_groups
        } else {
            remote_groups
                .get(&endpoint.endpoint_id)
                .unwrap_or(&empty_groups)
        };
        // Index once per projection rather than searching all panes for each row.
        let mut panes = HashMap::<&str, Vec<&ClientShellPane>>::new();
        for pane in &snapshot.panes {
            panes.entry(&pane.workspace_id).or_default().push(pane);
        }
        let agents = snapshot
            .agents
            .iter()
            .map(|agent| (agent.pane_id.as_str(), agent))
            .collect::<HashMap<_, _>>();
        let mut attention = HashMap::<&str, (usize, usize)>::new();
        let workspace_groups = snapshot
            .workspaces
            .iter()
            .filter_map(|workspace| {
                workspace
                    .worktree
                    .as_ref()
                    .map(|worktree| (workspace.workspace_id.as_str(), worktree.key.as_str()))
            })
            .collect::<HashMap<_, _>>();
        let mut group_attention = HashMap::<&str, (usize, usize)>::new();
        for agent in &snapshot.agents {
            let counts = attention.entry(&agent.workspace_id).or_default();
            match agent.agent_status {
                crate::api::schema::AgentStatus::Blocked => counts.0 += 1,
                crate::api::schema::AgentStatus::Done => counts.1 += 1,
                _ => {}
            }
            if let Some(group) = workspace_groups.get(agent.workspace_id.as_str()) {
                let counts = group_attention.entry(group).or_default();
                match agent.agent_status {
                    crate::api::schema::AgentStatus::Blocked => counts.0 += 1,
                    crate::api::schema::AgentStatus::Done => counts.1 += 1,
                    _ => {}
                }
            }
        }
        for entry in sidebar::workspace_entries(snapshot, groups) {
            let workspace = &snapshot.workspaces[entry.index];
            rows.push(Row::Workspace {
                endpoint,
                snapshot,
                entry,
                attention: workspace
                    .worktree
                    .as_ref()
                    .filter(|worktree| {
                        !worktree.is_linked_worktree && groups.contains(&worktree.key)
                    })
                    .and_then(|worktree| group_attention.get(worktree.key.as_str()))
                    .copied()
                    .unwrap_or_else(|| {
                        attention
                            .get(workspace.workspace_id.as_str())
                            .copied()
                            .unwrap_or_default()
                    }),
            });
            if collapsed_tasks
                .contains(&(endpoint.endpoint_id.clone(), workspace.workspace_id.clone()))
            {
                continue;
            }
            let Some(children) = panes.get(workspace.workspace_id.as_str()) else {
                continue;
            };
            let mut kinds = HashMap::<&str, usize>::new();
            for pane in children {
                *kinds
                    .entry(pane_kind(agents.get(pane.pane_id.as_str()).copied()))
                    .or_default() += 1;
            }
            for (index, pane) in children.iter().enumerate() {
                rows.push(Row::Pane {
                    endpoint,
                    pane,
                    agent: agents.get(pane.pane_id.as_str()).copied(),
                    depth: u16::from(show_machines) * 2 + u16::from(entry.indented) * 2,
                    last: index + 1 == children.len(),
                    duplicate: kinds
                        .get(pane_kind(agents.get(pane.pane_id.as_str()).copied()))
                        .copied()
                        .unwrap_or(0)
                        > 1,
                });
            }
        }
    }
    rows
}

fn row_geometry(rows: &[Row<'_>]) -> (Vec<u16>, Vec<u16>) {
    let heights = rows
        .iter()
        .map(|row| {
            if matches!(row, Row::Pane { .. }) {
                3
            } else {
                1
            }
        })
        .collect();
    let gaps = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            if index + 1 == rows.len() {
                0
            } else if matches!(rows[index + 1], Row::Workspace { .. } | Row::Machine(_)) {
                2
            } else if matches!(row, Row::Workspace { .. }) {
                1
            } else {
                0
            }
        })
        .collect();
    (heights, gaps)
}

fn pane_kind(agent: Option<&ClientShellAgent>) -> &str {
    agent
        .and_then(|agent| agent.display_agent.as_deref().or(agent.agent.as_deref()))
        .unwrap_or("Shell")
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "codex" => "Codex",
        "opencode" => "OpenCode",
        "shell" => "Shell",
        other => other,
    }
}

const TASK_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn tree_indicator(
    status: crate::api::schema::AgentStatus,
    phase: u8,
    palette: &Palette,
) -> (&'static str, ratatui::style::Color) {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Working => (
            TASK_SPINNER_FRAMES[usize::from(phase) % TASK_SPINNER_FRAMES.len()],
            palette.green,
        ),
        AgentStatus::Blocked => ("!", palette.yellow),
        AgentStatus::Done => ("●", palette.blue),
        AgentStatus::Idle => ("●", palette.green),
        AgentStatus::Unknown => ("—", palette.overlay0),
    }
}

fn put_ellipsis(buffer: &mut Buffer, x: u16, y: u16, right: u16, text: &str, style: Style) -> u16 {
    let width = right.saturating_sub(x);
    if super::render::display_width(text) <= width {
        return super::render::put_segment(buffer, x, y, right, text, style);
    }
    if width == 0 {
        return x;
    }
    let mut clipped = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let size = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if used + size > width - 1 {
            break;
        }
        clipped.push(ch);
        used += size;
    }
    clipped.push('…');
    super::render::put_segment(buffer, x, y, right, &clipped, style)
}

pub(super) fn render(
    buffer: &mut Buffer,
    area: Rect,
    active_snapshot: Option<&ClientShellSnapshot>,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    if area.is_empty() {
        return;
    }
    let mut tree_palette = config.palette.clone();
    if tree_palette.sidebar_bg == ratatui::style::Color::Reset {
        tree_palette.sidebar_bg = tree_palette.panel_bg;
    }
    tree_palette.sidebar_bg = subdued_tree_background(tree_palette.sidebar_bg);
    tree_palette.active_row_bg = subdued_tree_background(tree_palette.active_row_bg);
    tree_palette.selection_bg = subdued_tree_background(tree_palette.selection_bg);
    tree_palette.surface_dim = subdued_tree_background(tree_palette.surface_dim);
    let palette = &tree_palette;
    let empty_groups = HashSet::new();
    super::render::render_sidebar_background(buffer, area, palette);
    hits.sidebar_divider = Rect::new(area.right().saturating_sub(1), area.y, 1, area.height);
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    put_text(
        buffer,
        content.x,
        content.y,
        content.width,
        " 当前任务",
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let footer_height = if content.height >= 14 { 5 } else { 2 };
    let recent_supported = state
        .endpoints
        .iter()
        .find(|endpoint| &endpoint.endpoint_id == state.active_endpoint_id)
        .and_then(|endpoint| endpoint.methods.as_ref())
        .is_some_and(|methods| methods.contains("project.list"));
    let recent_expanded = !state.recent_collapsed && content.height >= 18;
    let recent_items = if recent_expanded {
        let available = content
            .height
            .saturating_sub(1 + footer_height + 2)
            .saturating_div(RECENT_ROW_HEIGHT)
            .max(1);
        (state.recent_projects.len() as u16).min(4).min(available)
    } else {
        0
    };
    let recent_height = if recent_supported && content.height >= 9 {
        recent_items * RECENT_ROW_HEIGHT + 2
    } else {
        0
    };
    let body = Rect::new(
        content.x,
        content.y.saturating_add(1),
        content.width,
        content
            .height
            .saturating_sub(1 + footer_height + recent_height),
    );
    hits.workspace_body = body;
    let rows = rows(
        state.endpoints,
        state.active_endpoint_id,
        active_snapshot,
        state.collapsed_endpoints,
        state.collapsed_groups,
        state.remote_collapsed_groups,
        state.collapsed_tasks,
    );
    let (heights, gaps) = row_geometry(&rows);
    if !body.is_empty() {
        let navigation = std::mem::take(state.reveal_navigation_workspace);
        let focus = std::mem::take(state.reveal_focused_workspace);
        let target = if navigation {
            rows.iter().position(|row| match row {
                Row::Workspace {
                    endpoint,
                    snapshot,
                    entry,
                    ..
                } => state.selected_workspace_id.is_some_and(|target| {
                    target.matches(
                        &endpoint.endpoint_id,
                        &snapshot.workspaces[entry.index].workspace_id,
                    )
                }),
                _ => false,
            })
        } else if focus {
            rows.iter().position(|row| matches!(row, Row::Pane { endpoint, pane, .. } if &endpoint.endpoint_id == state.active_endpoint_id && active_snapshot.is_some_and(|snapshot| snapshot.focused_pane_id.as_deref() == Some(pane.pane_id.as_str()))))
                .or_else(|| rows.iter().position(|row| matches!(row, Row::Workspace { endpoint, snapshot, entry, .. } if &endpoint.endpoint_id == state.active_endpoint_id && snapshot.workspaces[entry.index].focused)))
        } else {
            None
        };
        if let Some(target) = target {
            *state.workspace_scroll = super::scroll::list_scroll_start_to_reveal(
                &heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
                target,
            );
        }
    }
    let metrics =
        super::scroll::list_scroll_metrics(&heights, &gaps, body.height, *state.workspace_scroll);
    hits.workspace_max_scroll = metrics.max_offset_from_bottom;
    hits.workspace_scroll_metrics = Some(metrics);
    *state.workspace_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let width = body.width.saturating_sub(u16::from(scrollbar));
    let mut y = body.y;
    for (index, row) in rows.iter().enumerate().skip(*state.workspace_scroll) {
        let height = heights[index].min(body.height);
        if height == 0 || y.saturating_add(height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, width, height);
        y = y.saturating_add(height).saturating_add(gaps[index]);
        match row {
            Row::Machine(endpoint) => {
                let collapsed = state.collapsed_endpoints.contains(&endpoint.endpoint_id);
                let badge = super::endpoint_sidebar::render_endpoint_row(
                    buffer,
                    rect,
                    if collapsed { "▸" } else { "▾" },
                    endpoint,
                    collapsed && &endpoint.endpoint_id == state.active_endpoint_id,
                    state.machine_diagnostics,
                    palette,
                );
                hits.machines.push(MachineHit {
                    rect,
                    status_badge: badge,
                    collapse_toggle: Rect::new(
                        rect.x.saturating_add(1),
                        rect.y,
                        u16::from(rect.width > 1),
                        1,
                    ),
                    endpoint_id: endpoint.endpoint_id.clone(),
                });
            }
            Row::Workspace {
                endpoint,
                snapshot,
                entry,
                attention,
            } => {
                let workspace = &snapshot.workspaces[entry.index];
                let depth = u16::from(state.endpoints.len() > 1 || active_snapshot.is_none()) * 2
                    + u16::from(entry.indented) * 2;
                let selected = state.selected_workspace_id.is_some_and(|target| {
                    target.matches(&endpoint.endpoint_id, &workspace.workspace_id)
                });
                let bg = if selected {
                    if palette.selection_bg == ratatui::style::Color::Reset {
                        palette.active_row_bg
                    } else {
                        palette.selection_bg
                    }
                } else {
                    palette.sidebar_bg
                };
                buffer.set_style(rect, Style::default().bg(bg));
                let nested = Rect::new(
                    rect.x.saturating_add(depth.min(rect.width)),
                    rect.y,
                    rect.width.saturating_sub(depth),
                    1,
                );
                if nested.is_empty() {
                    continue;
                }
                let collapsed = state
                    .collapsed_tasks
                    .contains(&(endpoint.endpoint_id.clone(), workspace.workspace_id.clone()));
                let toggle = Rect::new(nested.x, nested.y, nested.width.min(3), 1);
                put_text(
                    buffer,
                    toggle.x,
                    toggle.y,
                    toggle.width,
                    if collapsed { "▸" } else { "▾" },
                    Style::default().fg(palette.overlay0).bg(bg),
                );
                hits.task_toggles.push((
                    toggle,
                    endpoint.endpoint_id.clone(),
                    workspace.workspace_id.clone(),
                ));
                let groups = if endpoint.endpoint_id.is_local() {
                    state.collapsed_groups
                } else {
                    state
                        .remote_collapsed_groups
                        .get(&endpoint.endpoint_id)
                        .unwrap_or(&empty_groups)
                };
                let status = sidebar::displayed_workspace_status(snapshot, workspace, groups);
                let action_width = if &endpoint.endpoint_id == state.active_endpoint_id
                    && endpoint.status == ClientEndpointStatus::Online
                    && nested.width >= 16
                {
                    4
                } else {
                    0
                };
                let status_width = u16::from(
                    collapsed
                        || workspace
                            .worktree
                            .as_ref()
                            .is_some_and(|worktree| groups.contains(&worktree.key)),
                );
                let status_x = nested
                    .right()
                    .saturating_sub(action_width + 2)
                    .max(toggle.right());
                let group_collapsed = workspace.worktree.as_ref().is_some_and(|worktree| {
                    !worktree.is_linked_worktree && groups.contains(&worktree.key)
                });
                let attention_text =
                    if (collapsed || group_collapsed) && attention.0 + attention.1 > 0 {
                        format!(" 待{} 未{}", attention.0, attention.1)
                    } else {
                        String::new()
                    };
                let attention_width = super::render::display_width(&attention_text);
                let attention_width =
                    if attention_width <= nested.width.saturating_sub(action_width + 8) {
                        attention_width
                    } else {
                        0
                    };
                let label_right = if status_width == 0 {
                    nested.right().saturating_sub(action_width + 1)
                } else {
                    status_x.saturating_sub(attention_width + 1)
                };
                let label_x = nested.x.saturating_add(3);
                let branch = workspace
                    .branch
                    .as_deref()
                    .map(|branch| format!(" ▏{branch}"));
                let branch_width = branch
                    .as_deref()
                    .map(|branch| {
                        super::render::display_width(branch)
                            .min(label_right.saturating_sub(label_x).saturating_sub(7))
                    })
                    .unwrap_or(0);
                put_ellipsis(
                    buffer,
                    label_x,
                    nested.y,
                    label_right.saturating_sub(branch_width),
                    &format!("[{}]", workspace.label),
                    Style::default()
                        .fg(palette.green)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                );
                if let Some(branch) = branch {
                    put_ellipsis(
                        buffer,
                        label_right.saturating_sub(branch_width),
                        nested.y,
                        label_right,
                        &branch,
                        Style::default().fg(palette.subtext0).bg(bg),
                    );
                }
                put_text(
                    buffer,
                    label_right,
                    nested.y,
                    attention_width,
                    &attention_text,
                    Style::default().fg(palette.accent).bg(bg),
                );
                let (indicator, indicator_color) =
                    tree_indicator(status, state.task_animation_phase, palette);
                if status_width > 0
                    && status == crate::api::schema::AgentStatus::Working
                    && endpoint.status == ClientEndpointStatus::Online
                {
                    hits.task_working_visible = true;
                }
                put_text(
                    buffer,
                    status_x,
                    nested.y,
                    status_width,
                    indicator,
                    Style::default().fg(indicator_color).bg(bg),
                );
                if action_width > 0 {
                    let add = Rect::new(nested.right().saturating_sub(5), nested.y, 3, 1);
                    put_text(
                        buffer,
                        add.x,
                        add.y,
                        add.width,
                        " + ",
                        Style::default().fg(palette.accent).bg(bg),
                    );
                    hits.task_add.push((
                        add,
                        endpoint.endpoint_id.clone(),
                        workspace.workspace_id.clone(),
                    ));
                }
                // The trailing worktree-group control is separate from the task chevron.
                let group_toggle = if nested.width >= 6 {
                    sidebar::render_parent_group_toggle(
                        buffer,
                        nested,
                        snapshot,
                        entry.index,
                        groups,
                        palette,
                    )
                } else {
                    None
                };
                hits.workspaces.push(WorkspaceHit {
                    rect,
                    endpoint_id: endpoint.endpoint_id.clone(),
                    workspace_id: workspace.workspace_id.clone(),
                    indented: entry.indented,
                    group_toggle,
                });
                if endpoint.status != ClientEndpointStatus::Online {
                    buffer.set_style(
                        rect,
                        Style::default()
                            .fg(palette.overlay0)
                            .add_modifier(Modifier::DIM),
                    );
                }
            }
            Row::Pane {
                endpoint,
                pane,
                agent,
                depth,
                last,
                duplicate,
            } => {
                let focused = &endpoint.endpoint_id == state.active_endpoint_id && pane.focused;
                let bg = if focused {
                    // Preserve the theme's contrast for the current terminal.
                    config.palette.active_row_bg
                } else {
                    palette.sidebar_bg
                };
                let prefix_x = rect.x.saturating_add(*depth).saturating_add(3);
                let content_y = rect.y.saturating_add(u16::from(rect.height > 1));
                let highlight_x = prefix_x.saturating_add(2).min(rect.right());
                buffer.set_style(
                    Rect::new(
                        highlight_x,
                        rect.y,
                        rect.right().saturating_sub(highlight_x),
                        rect.height,
                    ),
                    Style::default().bg(bg),
                );
                for line_y in rect.y..rect.bottom() {
                    if prefix_x < rect.right() && (line_y <= content_y || !last) {
                        put_text(
                            buffer,
                            prefix_x,
                            line_y,
                            1,
                            "│",
                            Style::default().fg(palette.overlay0).bg(palette.sidebar_bg),
                        );
                    }
                    if focused {
                        put_text(
                            buffer,
                            highlight_x,
                            line_y,
                            rect.right().saturating_sub(highlight_x).min(1),
                            "▌",
                            Style::default().fg(palette.accent).bg(bg),
                        );
                    }
                }
                put_text(
                    buffer,
                    prefix_x,
                    content_y,
                    rect.right().saturating_sub(prefix_x).min(2),
                    if *last { "└─" } else { "├─" },
                    Style::default().fg(palette.overlay0).bg(palette.sidebar_bg),
                );
                let kind = kind_label(pane_kind(*agent));
                let x = prefix_x.saturating_add(3);
                let status_x = rect.right().saturating_sub(3).max(rect.x);
                let label = pane.label.clone().unwrap_or_else(|| {
                    if *duplicate {
                        format!(
                            "{kind} · {}",
                            pane.pane_id
                                .rsplit(':')
                                .next()
                                .unwrap_or(&pane.pane_id)
                                .trim_start_matches("pane_")
                                .trim_start_matches('p')
                        )
                    } else {
                        kind.to_owned()
                    }
                });
                let name_end = put_ellipsis(
                    buffer,
                    x,
                    content_y,
                    status_x.saturating_sub(1),
                    &label,
                    Style::default()
                        .fg(if !focused && agent.is_none() {
                            palette.subtext0
                        } else {
                            palette.text
                        })
                        .bg(bg)
                        .add_modifier(if focused {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                );
                if pane.label.is_some() {
                    let hint = format!(" · {kind}");
                    if super::render::display_width(&hint) <= status_x.saturating_sub(name_end + 1)
                    {
                        super::render::put_segment(
                            buffer,
                            name_end,
                            content_y,
                            status_x.saturating_sub(1),
                            &hint,
                            Style::default().fg(palette.overlay0).bg(bg),
                        );
                    }
                }
                if let Some(agent) = agent {
                    let (symbol, color) =
                        tree_indicator(agent.agent_status, state.task_animation_phase, palette);
                    put_text(
                        buffer,
                        status_x,
                        content_y,
                        1,
                        symbol,
                        Style::default().fg(color).bg(bg).add_modifier(
                            if agent.agent_status == crate::api::schema::AgentStatus::Working {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            },
                        ),
                    );
                    hits.task_working_visible |= endpoint.status == ClientEndpointStatus::Online
                        && agent.agent_status == crate::api::schema::AgentStatus::Working;
                }
                if endpoint.status != ClientEndpointStatus::Online {
                    buffer.set_style(
                        rect,
                        Style::default()
                            .fg(palette.overlay0)
                            .add_modifier(Modifier::DIM),
                    );
                }
                hits.endpoint_agents.push((
                    rect,
                    endpoint.endpoint_id.clone(),
                    pane.pane_id.clone(),
                ));
            }
        }
    }
    if scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.workspace_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, palette);
    }
    if rows.is_empty() && !body.is_empty() {
        put_text(
            buffer,
            body.x,
            body.y,
            body.width,
            " No projects",
            Style::default().fg(palette.overlay0),
        );
    }
    if let Some(y) = state
        .workspace_drop_indicator_row
        .filter(|y| *y >= body.y && *y < body.bottom())
    {
        put_text(
            buffer,
            body.x,
            y,
            body.width,
            &"─".repeat(body.width as usize),
            Style::default().fg(palette.accent),
        );
    }
    if recent_height > 0 {
        let header_y = body.bottom().saturating_add(1);
        hits.recent_toggle = Rect::new(content.x, header_y, content.width, 1);
        put_text(
            buffer,
            content.x + 1,
            header_y,
            content.width.saturating_sub(1),
            if recent_expanded {
                "▾ 最近项目"
            } else {
                "▸ 最近项目"
            },
            Style::default().fg(palette.overlay0).bg(palette.sidebar_bg),
        );
        hits.recent_body = Rect::new(
            content.x,
            header_y + 1,
            content.width,
            recent_items * RECENT_ROW_HEIGHT,
        );
        *state.recent_scroll = (*state.recent_scroll).min(
            state
                .recent_projects
                .len()
                .saturating_sub(usize::from(recent_items)),
        );
        for (index, project) in state
            .recent_projects
            .iter()
            .skip(*state.recent_scroll)
            .take(usize::from(recent_items))
            .enumerate()
        {
            let row = Rect::new(
                content.x,
                header_y + 1 + index as u16 * RECENT_ROW_HEIGHT,
                content.width,
                RECENT_ROW_HEIGHT,
            );
            let label_y = row.y + (RECENT_ROW_HEIGHT - 1) / 2;
            let repeated = state
                .recent_projects
                .iter()
                .filter(|other| other.label == project.label)
                .take(2)
                .count()
                > 1;
            let label = if repeated {
                let normalized = project.cwd.replace('\\', "/");
                let parent = normalized
                    .trim_end_matches('/')
                    .rsplit('/')
                    .nth(1)
                    .unwrap_or("");
                format!("[{}] · {}", project.label, parent)
            } else {
                format!("[{}]", project.label)
            };
            put_text(
                buffer,
                row.x + 1,
                label_y,
                row.right().saturating_sub(row.x + 1).min(2),
                "▸ ",
                Style::default().fg(palette.overlay0).bg(palette.sidebar_bg),
            );
            put_ellipsis(
                buffer,
                row.x + 3,
                label_y,
                row.right().saturating_sub(2),
                &label,
                Style::default().fg(palette.green).bg(palette.sidebar_bg),
            );
            hits.recent_projects.push((row, project.cwd.clone()));
            if !state.endpoints.iter().any(|endpoint| {
                &endpoint.endpoint_id == state.active_endpoint_id
                    && endpoint.status == ClientEndpointStatus::Online
            }) {
                buffer.set_style(
                    row,
                    Style::default()
                        .fg(palette.overlay0)
                        .add_modifier(Modifier::DIM),
                );
            }
        }
    }
    let button_height = if footer_height == 5 { 3 } else { 1 };
    let footer_y = content.bottom().saturating_sub(button_height + 1);
    if config.mouse_capture && content.height >= 3 {
        let button_width = content.width.saturating_sub(10);
        hits.new_workspace = Rect::new(content.x + 1, footer_y, button_width, button_height);
        buffer.set_style(hits.new_workspace, Style::default().bg(palette.surface_dim));
        let caption_y = footer_y + u16::from(button_height > 1);
        put_text(
            buffer,
            hits.new_workspace.x + 1,
            caption_y,
            button_width.saturating_sub(2),
            if button_width >= 11 {
                "+ 新建项目"
            } else {
                "+ 新建"
            },
            Style::default()
                .fg(palette.accent)
                .bg(palette.surface_dim)
                .add_modifier(Modifier::BOLD),
        );
        if footer_y > content.y {
            put_text(
                buffer,
                content.x + 1,
                footer_y - 1,
                content.width.saturating_sub(2),
                &"─".repeat(usize::from(content.width.saturating_sub(2))),
                Style::default()
                    .fg(palette.surface_dim)
                    .bg(palette.sidebar_bg),
            );
        }
        let attention = active_snapshot.is_some_and(super::global_menu::global_menu_attention);
        hits.global_launcher = Rect::new(
            content
                .right()
                .saturating_sub(if attention { 8 } else { 6 }.min(content.width)),
            footer_y,
            if attention { 8 } else { 6 }.min(content.width),
            button_height,
        );
        if attention && content.width >= 6 {
            put_text(
                buffer,
                content.right() - 6,
                caption_y,
                2,
                "● ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            );
        }
        put_right_text(
            buffer,
            content,
            caption_y,
            "menu",
            Style::default().fg(palette.overlay0),
        );
    }
    hits.sidebar_toggle = Rect::new(
        content.right().saturating_sub(1),
        content.bottom().saturating_sub(1),
        u16::from(content.width > 0),
        1,
    );
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "«",
        Style::default().fg(palette.overlay0),
    );
}

impl ClientShellState {
    pub(crate) fn tick_task_animation(&mut self, now: std::time::Instant) -> bool {
        if !self.hits.task_working_visible
            || self.sidebar_collapsed
            || self.overlay.is_some()
            || self
                .pane_surface
                .as_ref()
                .is_some_and(|surface| surface.popup.is_some())
            || self.outer_focused == Some(false)
            || self.config.sidebar_layout != crate::config::SidebarLayoutConfig::Tree
        {
            self.task_animation_deadline = None;
            return false;
        }
        if self
            .task_animation_deadline
            .is_some_and(|deadline| now < deadline)
        {
            return false;
        }
        self.task_animation_phase =
            (self.task_animation_phase + 1) % TASK_SPINNER_FRAMES.len() as u8;
        self.task_animation_deadline = Some(now + std::time::Duration::from_millis(80));
        true
    }

    pub(super) fn open_task_status_info(&mut self, pane_id: &str) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(pane) = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id) else {
            return;
        };
        let agent = snapshot
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane_id);
        let status = agent
            .map(|agent| match agent.agent_status {
                crate::api::schema::AgentStatus::Working => {
                    "运行中：Herdr 当前判定智能体正在工作。"
                }
                crate::api::schema::AgentStatus::Blocked => {
                    "需要操作：智能体等待确认、授权或输入。"
                }
                crate::api::schema::AgentStatus::Done => "完成未读：本轮完成，当前客户端尚未查看。",
                crate::api::schema::AgentStatus::Idle => "静息：当前未判定为工作中或待操作。",
                crate::api::schema::AgentStatus::Unknown => "状态未确认：未作运行或就绪判定。",
            })
            .unwrap_or("普通 Shell：不显示智能体运行状态。");
        let workspace = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == pane.workspace_id)
            .map(|workspace| workspace.label.as_str())
            .unwrap_or(&pane.workspace_id);
        let tab = snapshot
            .tabs
            .iter()
            .find(|tab| tab.tab_id == pane.tab_id)
            .map(|tab| tab.label.as_str())
            .unwrap_or(&pane.tab_id);
        let text = format!("名称：{}\n类型：{}\n项目：{workspace}\n标签页：{tab}   窗格：{pane_id}\n\n{status}\n\n● 绿色：静息   旋转符 运行中   ! 需要操作\n● 蓝色：完成未读   — 状态未确认\n查看终端画面后，未读标记按 Herdr 原规则清除。", pane.label.as_deref().unwrap_or(kind_label(pane_kind(agent))), kind_label(pane_kind(agent)));
        self.overlay = Some(ClientShellOverlay::StatusInfo(text));
    }

    pub(super) fn move_task_tree_focus(
        &mut self,
        direction: isize,
        outcome: &mut ClientShellInput,
    ) {
        let projected = rows(
            &self.endpoints,
            &self.active_endpoint_id,
            self.snapshot.as_deref(),
            &self.collapsed_endpoints,
            &self.collapsed_groups,
            &self.remote_collapsed_groups,
            &self.collapsed_tasks,
        );
        let targets = projected
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let (endpoint, target, workspace) = match row {
                    Row::Workspace {
                        endpoint,
                        snapshot,
                        entry,
                        ..
                    } => {
                        let id = &snapshot.workspaces[entry.index].workspace_id;
                        (
                            endpoint,
                            ClientEndpointFocusTarget::Workspace(id.clone()),
                            id.clone(),
                        )
                    }
                    Row::Pane { endpoint, pane, .. } => (
                        endpoint,
                        ClientEndpointFocusTarget::Pane(pane.pane_id.clone()),
                        pane.workspace_id.clone(),
                    ),
                    Row::Machine(_) => return None,
                };
                (endpoint.status == ClientEndpointStatus::Online)
                    .then(|| (index, endpoint.endpoint_id.clone(), target, workspace))
            })
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return;
        }
        let preview = self.navigate_workspace_id.as_ref().filter(|target| {
            !self.snapshot.as_deref().is_some_and(|snapshot| {
                snapshot
                    .focused_workspace_id
                    .as_deref()
                    .is_some_and(|id| target.matches(&self.active_endpoint_id, id))
            })
        });
        let current = targets.iter().position(|(_, endpoint, target, _)| {
            if let Some(cursor) = &self.task_navigation_cursor { return endpoint == &cursor.0 && target == &cursor.1; }
            if let Some(preview) = preview { return matches!(target, ClientEndpointFocusTarget::Workspace(id) if preview.matches(endpoint, id)); }
            endpoint == &self.active_endpoint_id && matches!(target, ClientEndpointFocusTarget::Pane(id) if self.snapshot.as_deref().is_some_and(|snapshot| snapshot.focused_pane_id.as_ref() == Some(id)))
        });
        let next = match current {
            Some(current) => {
                (current as isize + direction).rem_euclid(targets.len() as isize) as usize
            }
            None if direction < 0 => targets.len() - 1,
            None => 0,
        };
        let (index, endpoint, target, workspace) = targets[next].clone();
        let (heights, gaps) = row_geometry(&projected);
        drop(projected);
        if !self.focus_or_activate(endpoint.clone(), target.clone(), outcome) {
            return;
        }
        self.task_navigation_cursor = Some((endpoint.clone(), target));
        self.navigate_workspace_id = self.navigation_target(&endpoint, &workspace);
        self.workspace_scroll = super::scroll::list_scroll_start_to_reveal(
            &heights,
            &gaps,
            self.hits.workspace_body.height,
            self.workspace_scroll,
            index,
        );
        // The requested row is already revealed; an older focused-pane snapshot must not undo it.
        self.reveal_navigation_workspace = false;
        self.reveal_focused_workspace = false;
        outcome.repaint = true;
    }

    pub(super) fn reveal_task_pane(&mut self, endpoint_id: &ClientEndpointId, pane_id: &str) {
        let Some((workspace_id, group)) = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .and_then(|endpoint| endpoint.snapshot.as_deref())
            .and_then(|snapshot| {
                let pane = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id)?;
                let workspace = snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.workspace_id == pane.workspace_id)?;
                Some((
                    workspace.workspace_id.clone(),
                    workspace
                        .worktree
                        .as_ref()
                        .map(|worktree| worktree.key.clone()),
                ))
            })
        else {
            return;
        };
        self.collapsed_endpoints.remove(endpoint_id);
        self.collapsed_tasks
            .remove(&(endpoint_id.clone(), workspace_id));
        if let Some(group) = group {
            if endpoint_id.is_local() {
                self.collapsed_groups.remove(&group);
            } else if let Some(groups) = self.remote_collapsed_groups.get_mut(endpoint_id) {
                groups.remove(&group);
            }
        }
        let rows = rows(
            &self.endpoints,
            &self.active_endpoint_id,
            self.snapshot.as_deref(),
            &self.collapsed_endpoints,
            &self.collapsed_groups,
            &self.remote_collapsed_groups,
            &self.collapsed_tasks,
        );
        if let Some(target) = rows.iter().position(|row| matches!(row, Row::Pane { endpoint, pane, .. } if &endpoint.endpoint_id == endpoint_id && pane.pane_id == pane_id)) {
            let (heights, gaps) = row_geometry(&rows);
            self.workspace_scroll = super::scroll::list_scroll_start_to_reveal(&heights, &gaps, self.hits.workspace_body.height, self.workspace_scroll, target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::AgentStatus;

    fn draw(state: &mut ClientShellState, buffer: &mut Buffer, area: Rect) -> ShellHitMap {
        let mut hits = ShellHitMap::default();
        let mut render_state = ShellRenderState {
            task_animation_phase: state.task_animation_phase,
            recent_projects: state
                .project_catalogs
                .get(&state.active_endpoint_id)
                .map_or(&[], Vec::as_slice),
            recent_scroll: &mut state.recent_scroll,
            recent_collapsed: state.recent_collapsed,
            machine_diagnostics: &state.machine_diagnostics,
            endpoints: &state.endpoints,
            active_endpoint_id: &state.active_endpoint_id,
            collapsed_endpoints: &state.collapsed_endpoints,
            collapsed_groups: &state.collapsed_groups,
            remote_collapsed_groups: &state.remote_collapsed_groups,
            collapsed_tasks: &state.collapsed_tasks,
            workspace_scroll: &mut state.workspace_scroll,
            agent_scroll: &mut state.agent_scroll,
            tab_scroll: &mut state.tab_scroll,
            reveal_focused_workspace: &mut state.reveal_focused_workspace,
            reveal_focused_tab: &mut state.reveal_focused_tab,
            sidebar_collapsed: false,
            sidebar_section_split: state.sidebar_section_split,
            tab_drag_insert_index: None,
            selected_workspace_id: None,
            reveal_navigation_workspace: &mut state.reveal_navigation_workspace,
            dragged_workspace_id: None,
            workspace_drop_indicator_row: None,
        };
        if state.config.sidebar_layout == crate::config::SidebarLayoutConfig::Tree {
            render(
                buffer,
                area,
                state.snapshot.as_deref(),
                &state.config,
                &mut render_state,
                &mut hits,
            );
        } else if let Some(snapshot) = state.snapshot.as_deref() {
            super::super::render::render_sidebar(
                buffer,
                area,
                snapshot,
                &state.config,
                &mut render_state,
                &mut hits,
            );
        }
        hits
    }

    #[test]
    #[ignore = "supporting render profile; no wall-clock assertions"]
    fn task_tree_render_scale_profile() {
        use crate::config::SidebarLayoutConfig;
        for projects in [false, true] {
            for count in [1, 15] {
                for layout in [SidebarLayoutConfig::Split, SidebarLayoutConfig::Tree] {
                    let mut state =
                        ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
                    state.config.sidebar_layout = layout;
                    let mut projected = super::super::tests::snapshot();
                    let template = projected.panes[0].clone();
                    projected.panes.clear();
                    for number in 1..=count {
                        let workspace_id = if projects {
                            format!("ws_{number}")
                        } else {
                            "ws_1".into()
                        };
                        if projects && number > 1 {
                            let mut workspace = projected.workspaces[0].clone();
                            workspace.workspace_id = workspace_id.clone();
                            workspace.focused = false;
                            projected.workspaces.push(workspace);
                        }
                        let mut pane = template.clone();
                        pane.pane_id = format!("pane_{number}");
                        pane.workspace_id = workspace_id.clone();
                        pane.focused = number == 1;
                        projected.agents.push(ClientShellAgent {
                            pane_id: pane.pane_id.clone(),
                            workspace_id,
                            tab_id: pane.tab_id.clone(),
                            name: Some(format!("agent {number}")),
                            display_agent: None,
                            agent: Some("codex".into()),
                            title: Some("populated task".into()),
                            terminal_title: None,
                            terminal_title_stripped: None,
                            agent_status: AgentStatus::Working,
                            state_change_seq: number as u64,
                            state_labels: Vec::new(),
                            tokens: Vec::new(),
                            focused: pane.focused,
                        });
                        projected.panes.push(pane);
                    }
                    state.set_snapshot(Box::new(projected));
                    // Keep all 15 three-row cards and project gaps visible in both layouts.
                    let mut buffer = Buffer::empty(Rect::new(0, 0, 120, 128));
                    let area = Rect::new(0, 0, 36, 128);
                    for _ in 0..20 {
                        std::hint::black_box(draw(&mut state, &mut buffer, area));
                    }
                    let start = std::time::Instant::now();
                    for _ in 0..500 {
                        std::hint::black_box(draw(&mut state, &mut buffer, area));
                    }
                    println!("task_tree_profile layout={layout:?} projects={projects} panes={count} mean_us={:.2}", start.elapsed().as_secs_f64() * 1_000_000.0 / 500.0);
                }
            }
        }
    }

    #[test]
    fn task_tree_shows_status_and_stable_name_without_dynamic_title() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        let mut projected = super::super::tests::snapshot();
        projected.panes[0].label = Some("修复安装流程".into());
        projected.agents.push(ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: None,
            display_agent: None,
            agent: Some("codex".into()),
            title: Some("npm install obsolete fragment".into()),
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Done,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: true,
        });
        state.set_snapshot(Box::new(projected));
        let area = Rect::new(0, 0, 36, 20);
        let mut buffer = Buffer::empty(area);
        let hits = draw(&mut state, &mut buffer, area);
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Codex"));
        assert!(text.replace(' ', "").contains("修复安装流程"));
        assert!(!text.contains("npm install"));
        // Focused completion is already read according to the existing client projection.
        assert!(text.contains('●'));
        assert!(!text.replace(' ', "").contains("就绪"));
        assert!(!text.replace(' ', "").contains("终端#"));
        assert_eq!(hits.endpoint_agents[0].0.height, 3);
    }

    #[test]
    fn task_tree_folded_project_keeps_attention_visible() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        let mut projected = super::super::tests::snapshot();
        projected.agents.push(ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: None,
            display_agent: None,
            agent: Some("codex".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Blocked,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: true,
        });
        state.set_snapshot(Box::new(projected));
        state
            .collapsed_tasks
            .insert((ClientEndpointId::Local, "ws_1".into()));
        let area = Rect::new(0, 0, 36, 20);
        let mut buffer = Buffer::empty(area);
        let hits = draw(&mut state, &mut buffer, area);
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.replace(' ', "").contains("待1未0"));
        assert!(hits.endpoint_agents.is_empty());
        assert_eq!(
            state.snapshot.as_ref().expect("snapshot").agents[0].agent_status,
            AgentStatus::Blocked
        );
    }

    #[test]
    fn solid_status_dots_keep_distinct_idle_and_unread_colors() {
        let palette = Palette::catppuccin();
        assert_eq!(
            tree_indicator(AgentStatus::Idle, 0, &palette),
            ("●", palette.green)
        );
        assert_eq!(
            tree_indicator(AgentStatus::Done, 0, &palette),
            ("●", palette.blue)
        );
    }

    #[test]
    fn task_tree_narrow_unicode_worktree_rows_stay_inside_sidebar() {
        for width in 1..=8 {
            for height in 1..=8 {
                let mut state =
                    ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
                let mut projected = super::super::tests::snapshot();
                projected.workspaces[0].label = "中文项目".into();
                projected.workspaces[0].worktree = Some(crate::protocol::ClientShellWorktree {
                    key: "repo".into(),
                    label: "repo".into(),
                    is_linked_worktree: false,
                });
                let mut child = projected.workspaces[0].clone();
                child.workspace_id = "child".into();
                child.focused = false;
                child
                    .worktree
                    .as_mut()
                    .expect("worktree")
                    .is_linked_worktree = true;
                projected.workspaces.push(child);
                state.set_snapshot(Box::new(projected));
                let mut buffer = Buffer::empty(Rect::new(0, 0, 30, 30));
                for cell in &mut buffer.content {
                    cell.set_symbol("X");
                }
                let area = Rect::new(3, 3, width, height);
                let hits = draw(&mut state, &mut buffer, area);
                for y in 0..30 {
                    for x in 0..30 {
                        if !super::super::contains(area, (x, y)) {
                            assert_eq!(buffer[(x, y)].symbol(), "X", "{width}x{height} at {x},{y}");
                        }
                    }
                }
                for (rect, _, _) in hits
                    .task_toggles
                    .iter()
                    .chain(&hits.task_add)
                    .chain(&hits.endpoint_agents)
                {
                    if !rect.is_empty() {
                        assert!(rect.x >= area.x && rect.right() < area.right());
                        assert!(rect.y >= area.y && rect.bottom() <= area.bottom());
                    }
                }
            }
        }
    }
}
