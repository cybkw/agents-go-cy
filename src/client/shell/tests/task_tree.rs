// Modified for Agents Go by Agents Go contributors, 2026-10-01. See NOTICE.
use super::*;

fn state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(100, 24).expect("tree frame");
    state
}

fn click(state: &mut ClientShellState, rect: Rect, button: MouseButton) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(button),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::empty(),
    })])
}

fn with_recent_projects(count: usize) -> ClientShellState {
    let mut state = state();
    state.endpoints[0].methods = Some(
        ["project.list", "project.open", "project.forget"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    state.project_catalogs.insert(
        ClientEndpointId::Local,
        (0..count)
            .map(|index| crate::api::schema::ProjectInfo {
                cwd: format!("/project/{index}"),
                label: format!("project {index}"),
            })
            .collect(),
    );
    state.compose(100, 30).unwrap();
    state
}

#[test]
fn recent_projects_and_new_button_have_separate_click_and_scroll_regions() {
    let mut state = with_recent_projects(7);
    assert_eq!(state.hits.recent_projects.len(), 4);
    assert_eq!(state.hits.new_workspace.height, 3);
    assert!(state.hits.recent_body.bottom() < state.hits.new_workspace.y);
    assert!(state.hits.new_workspace.right() <= state.hits.global_launcher.x);
    let recent = state.hits.recent_projects[0].0;
    let result = click(&mut state, recent, MouseButton::Left);
    assert!(
        matches!(&result.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, crate::api::schema::Method::ProjectOpen(target) if target.cwd == "/project/0"))
    );
    click(&mut state, recent, MouseButton::Right);
    let index = match &state.overlay {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .position(|item| item.action == ClientContextMenuAction::ForgetProject)
            .unwrap(),
        _ => panic!("recent menu"),
    };
    let mut result = ClientShellInput::default();
    state.activate_context_menu_item(index, &mut result);
    assert!(
        matches!(&result.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, crate::api::schema::Method::ProjectForget(target) if target.cwd == "/project/0"))
    );
    let body = state.hits.recent_body;
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: body.x + 1,
        row: body.y,
        modifiers: KeyModifiers::empty(),
    })]);
    state.compose(100, 30).unwrap();
    assert_eq!(state.hits.recent_projects[0].1, "/project/1");
    let toggle = state.hits.recent_toggle;
    click(&mut state, toggle, MouseButton::Left);
    state.compose(100, 30).unwrap();
    assert!(state.hits.recent_projects.is_empty());
}

#[test]
fn recent_project_rows_use_full_height_without_overlap() {
    let state = with_recent_projects(7);
    let rows = state.hits.recent_projects.clone();
    assert_eq!(rows.len(), 4);
    for (rect, _) in &rows {
        assert_eq!(rect.height, super::super::task_tree::RECENT_ROW_HEIGHT);
    }
    for pair in rows.windows(2) {
        assert_eq!(pair[0].0.bottom(), pair[1].0.y);
    }
    assert!(rows.last().expect("rows").0.bottom() <= state.hits.recent_body.bottom());
}

#[test]
fn recent_projects_fit_narrow_and_short_terminals() {
    let mut state = with_recent_projects(7);
    for width in [1, 8, 18, 34, 60, 100] {
        for height in [1, 3, 8, 12, 18, 30] {
            state
                .compose(width, height)
                .expect("bounded project catalog layout");
            for (rect, _) in &state.hits.recent_projects {
                assert!(rect.right() <= width && rect.bottom() <= height);
            }
        }
    }
}

#[test]
fn project_catalog_is_optional_coalesced_and_ignores_stale_mutation_results() {
    let mut state = state();
    let now = std::time::Instant::now();
    let mut outcome = ClientShellInput::default();
    state.poll_project_catalog(now, &mut outcome);
    assert!(outcome.actions.is_empty());
    state.endpoints[0].methods = Some(["project.list".to_owned()].into_iter().collect());
    state.poll_project_catalog(now, &mut outcome);
    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!("list request")
    };
    let request_id = request.id.clone();
    let mut coalesced = ClientShellInput::default();
    state.poll_project_catalog(now, &mut coalesced);
    assert!(coalesced.actions.is_empty());
    state.project_catalog_epoch += 1;
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::ProjectList {
            projects: vec![crate::api::schema::ProjectInfo {
                cwd: "/old".into(),
                label: "old".into(),
            }],
        }),
    );
    assert!(state.project_catalogs.is_empty());
    let mut output_only = snapshot();
    output_only.revision += 1;
    state.set_snapshot(Box::new(output_only));
    state.poll_project_catalog(now, &mut coalesced);
    assert!(
        coalesced.actions.is_empty(),
        "terminal output alone does not refetch history"
    );
}

#[test]
fn offline_project_history_is_read_only_and_scoped_to_endpoint() {
    let mut state = with_recent_projects(1);
    state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Reconnecting);
    state.compose(100, 30).unwrap();
    let recent = state.hits.recent_projects[0].0;
    assert!(click(&mut state, recent, MouseButton::Left)
        .actions
        .is_empty());
    click(&mut state, recent, MouseButton::Right);
    let Some(ClientShellOverlay::ContextMenu(menu)) = &state.overlay else {
        panic!("offline menu")
    };
    assert_eq!(menu.items().len(), 1);
    assert_eq!(menu.items()[0].action, ClientContextMenuAction::ProjectPath);
    state.project_catalogs.insert(
        ClientEndpointId::Ssh(
            crate::client::endpoint::ProfileId::parse("1123456789abcdef0123456789abcdef").unwrap(),
        ),
        vec![crate::api::schema::ProjectInfo {
            cwd: "/other".into(),
            label: "remote".into(),
        }],
    );
    assert_eq!(
        state.project_catalogs[&ClientEndpointId::Local][0].cwd,
        "/project/0"
    );
}

#[test]
fn task_tree_contains_plain_shell_and_collapse_does_not_focus() {
    let mut state = state();
    assert_eq!(state.hits.endpoint_agents.len(), 1);
    assert!(state.hits.agent_body.is_empty());
    let toggle = state.hits.task_toggles[0].0;
    let outcome = click(&mut state, toggle, MouseButton::Left);
    assert!(outcome.actions.is_empty());
    state.compose(100, 24).expect("collapsed");
    assert!(state.hits.endpoint_agents.is_empty());
    // Output updates with unchanged focus must preserve a manual collapse.
    let mut update = snapshot();
    update.revision += 1;
    state.set_snapshot(Box::new(update));
    let mut updated_surface = surface();
    updated_surface.projection_revision += 1;
    state.set_pane_surface(updated_surface);
    state.compose(100, 24).expect("still collapsed");
    assert!(state.hits.endpoint_agents.is_empty());
    click(&mut state, toggle, MouseButton::Left);
    state.compose(100, 24).expect("expanded");
    assert_eq!(state.hits.endpoint_agents.len(), 1);
}

#[test]
fn recent_project_actions_require_their_individual_capabilities() {
    let mut state = with_recent_projects(1);
    state.endpoints[0].methods = Some(["project.list".to_owned()].into_iter().collect());
    let recent = state.hits.recent_projects[0].0;
    assert!(click(&mut state, recent, MouseButton::Left)
        .actions
        .is_empty());
    click(&mut state, recent, MouseButton::Right);
    let Some(ClientShellOverlay::ContextMenu(menu)) = &state.overlay else {
        panic!("project menu")
    };
    assert_eq!(menu.items().len(), 1);
    assert_eq!(menu.items()[0].action, ClientContextMenuAction::ProjectPath);
}

#[test]
fn closing_a_workspace_reveals_history_even_when_the_top_directory_is_unchanged() {
    let mut state = with_recent_projects(7);
    let catalog = state.project_catalogs[&ClientEndpointId::Local].clone();
    let now = std::time::Instant::now();
    let mut outcome = ClientShellInput::default();
    state.poll_project_catalog(now, &mut outcome);
    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!("catalog request")
    };
    let request_id = request.id.clone();
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::ProjectList {
            projects: catalog.clone(),
        }),
    );
    state.recent_collapsed = true;
    state.recent_scroll = 3;
    let mut replacement = snapshot();
    replacement.revision += 1;
    replacement.workspaces[0].workspace_id = "replacement-shell".into();
    replacement.focused_workspace_id = Some("replacement-shell".into());
    replacement.panes[0].workspace_id = "replacement-shell".into();
    state.set_snapshot(Box::new(replacement));
    let mut outcome = ClientShellInput::default();
    state.poll_project_catalog(now, &mut outcome);
    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!("close catalog request")
    };
    let request_id = request.id.clone();
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::ProjectList { projects: catalog }),
    );
    assert!(!state.recent_collapsed);
    assert_eq!(state.recent_scroll, 0);
}

#[test]
fn forgetting_the_first_recent_project_keeps_the_list_collapsed_and_preserves_scroll() {
    let mut state = with_recent_projects(7);
    state.recent_collapsed = true;
    state.recent_scroll = 3;
    let projects = state.project_catalogs[&ClientEndpointId::Local][1..].to_vec();
    let mut outcome = ClientShellInput::default();
    state.forget_recent_project("/project/0".into(), &mut outcome);
    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!("forget request")
    };
    let request_id = request.id.clone();
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::ProjectList { projects }),
    );
    assert!(state.recent_collapsed);
    assert_eq!(state.recent_scroll, 2);
}

#[test]
fn task_tree_session_button_targets_its_workspace() {
    let mut state = state();
    state.config.prompt_new_tab_name = true;
    let rect = state.hits.task_add[0].0;
    let outcome = click(&mut state, rect, MouseButton::Left);
    assert!(outcome.actions.is_empty());
    assert!(
        matches!(state.overlay, Some(ClientShellOverlay::Rename(ClientRenameOverlay {
        target: ClientRenameTarget::NewTab { ref workspace_id, .. }, ..
    })) if workspace_id == "ws_1")
    );
}

#[test]
fn task_tree_plain_shell_right_click_uses_existing_pane_menu() {
    let mut state = state();
    let rect = state.hits.endpoint_agents[0].0;
    click(&mut state, rect, MouseButton::Right);
    assert!(
        matches!(state.overlay, Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
        target: ClientContextMenuTarget::Pane { ref pane_id, .. }, ..
    })) if pane_id == "pane_1")
    );
}

#[test]
fn task_tree_reveals_focused_pane_after_keyboard_navigation() {
    let mut projected = snapshot();
    for number in 2..=24 {
        let mut pane = projected.panes[0].clone();
        pane.pane_id = format!("pane_{number}");
        pane.focused = false;
        projected.panes.push(pane);
    }
    let mut state = state();
    state.set_snapshot(Box::new(projected.clone()));
    state.compose(100, 16).expect("overflow");
    state
        .collapsed_tasks
        .insert((state.active_endpoint_id.clone(), "ws_1".into()));
    projected.focused_pane_id = Some("pane_24".into());
    for pane in &mut projected.panes {
        pane.focused = pane.pane_id == "pane_24";
    }
    state.set_snapshot(Box::new(projected));
    state.compose(100, 16).expect("revealed");
    assert!(state
        .hits
        .endpoint_agents
        .iter()
        .any(|(_, _, pane)| pane == "pane_24"));
    assert!(state.collapsed_tasks.is_empty());
}

#[test]
fn task_tree_lists_panes_once_across_tabs() {
    let mut projected = snapshot();
    let mut tab = projected.tabs[0].clone();
    tab.tab_id = "tab_2".into();
    tab.label = "other".into();
    tab.focused = false;
    projected.tabs.push(tab);
    let mut pane = projected.panes[0].clone();
    pane.pane_id = "pane_2".into();
    pane.tab_id = "tab_2".into();
    pane.focused = false;
    projected.panes.push(pane);
    let mut state = state();
    state.set_snapshot(Box::new(projected));
    state.compose(100, 24).expect("multi tab");
    let ids = state
        .hits
        .endpoint_agents
        .iter()
        .map(|(_, _, pane)| pane.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["pane_1", "pane_2"]);
}

#[test]
fn task_tree_two_line_cards_are_clickable_but_gaps_are_not() {
    let mut state = state();
    let mut projected = snapshot();
    let mut second = projected.panes[0].clone();
    second.pane_id = "pane_2".into();
    second.focused = false;
    projected.panes.push(second);
    state.set_snapshot(Box::new(projected));
    state.compose(100, 24).expect("cards");
    let first = state.hits.endpoint_agents[0].0;
    let second = state.hits.endpoint_agents[1].0;
    assert_eq!(first.height, 3);
    assert_eq!(second.y, first.bottom());
    let padding = click(
        &mut state,
        Rect::new(first.x + 5, first.y + 1, 1, 1),
        MouseButton::Left,
    );
    assert!(
        matches!(&padding.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_1"))
    );
    let gap = Rect::new(second.x + 5, second.bottom(), 1, 1);
    assert!(click(&mut state, gap, MouseButton::Left).actions.is_empty());
    let outcome = click(
        &mut state,
        Rect::new(second.x + 5, second.y + 1, 1, 1),
        MouseButton::Left,
    );
    assert!(
        matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"))
    );
}

#[test]
fn task_tree_highlights_the_pane_and_preserves_navigation_preview() {
    let mut state = state();
    let frame = state.compose(100, 24).unwrap().to_ratatui_buffer().unwrap();
    let workspace = state.hits.workspaces[0].rect;
    let pane = state.hits.endpoint_agents[0].0;
    assert_eq!(
        frame[(workspace.x + 2, workspace.y)].bg,
        ratatui::style::Color::Rgb(18, 18, 18)
    );
    for y in pane.y..pane.bottom() {
        assert_eq!(frame[(pane.x + 5, y)].symbol(), "▌");
        assert_eq!(frame[(pane.x + 5, y)].fg, state.config.palette.accent);
        assert_eq!(
            frame[(pane.x + 8, y)].bg,
            state.config.palette.active_row_bg
        );
    }
    let name = &frame[(pane.x + 6, pane.y + 1)];
    assert_eq!(name.fg, state.config.palette.text);
    assert!(name.modifier.contains(ratatui::style::Modifier::BOLD));
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"w");
    let frame = state.compose(100, 24).unwrap().to_ratatui_buffer().unwrap();
    let expected = if state.config.palette.selection_bg == ratatui::style::Color::Reset {
        ratatui::style::Color::Rgb(23, 23, 23)
    } else {
        ratatui::style::Color::Rgb(36, 36, 36)
    };
    assert_eq!(frame[(workspace.x + 2, workspace.y)].bg, expected);
    state.handle_input_bytes(b"\x1b");
    let frame = state.compose(100, 24).unwrap().to_ratatui_buffer().unwrap();
    assert_eq!(
        frame[(workspace.x + 2, workspace.y)].bg,
        ratatui::style::Color::Rgb(18, 18, 18)
    );
}

#[test]
fn task_tree_unfocused_shell_keeps_subdued_name_and_no_focus_marker() {
    let mut state = state();
    let mut projected = snapshot();
    projected.panes[0].focused = false;
    projected.focused_pane_id = None;
    state.set_snapshot(Box::new(projected));
    let frame = state.compose(100, 24).unwrap().to_ratatui_buffer().unwrap();
    let pane = state.hits.endpoint_agents[0].0;
    let name = &frame[(pane.x + 6, pane.y + 1)];
    assert_eq!(name.fg, state.config.palette.subtext0);
    assert!(!name.modifier.contains(ratatui::style::Modifier::BOLD));
    for y in pane.y..pane.bottom() {
        assert_ne!(frame[(pane.x + 5, y)].symbol(), "▌");
        assert_eq!(
            frame[(pane.x + 8, y)].bg,
            ratatui::style::Color::Rgb(18, 18, 18)
        );
    }
}

#[test]
fn task_tree_animation_is_throttled_and_stops_when_not_visible() {
    let mut state = state();
    let now = std::time::Instant::now();
    assert!(!state.tick_task_animation(now));
    state.hits.task_working_visible = true;
    assert!(state.tick_task_animation(now));
    let phase = state.task_animation_phase;
    assert!(!state.tick_task_animation(now + std::time::Duration::from_millis(79)));
    assert_eq!(state.task_animation_phase, phase);
    assert!(state.tick_task_animation(now + std::time::Duration::from_millis(80)));
    for hidden in [
        "collapse",
        "unfocused",
        "overlay",
        "popup",
        "no_work",
        "split",
    ] {
        state.set_pane_surface(if hidden == "popup" {
            surface_with_popup()
        } else {
            surface()
        });
        state.sidebar_collapsed = hidden == "collapse";
        state.outer_focused = Some(hidden != "unfocused");
        state.overlay =
            (hidden == "overlay").then(|| ClientShellOverlay::StatusInfo("info".into()));
        state.hits.task_working_visible = hidden != "no_work";
        state.config.sidebar_layout = if hidden == "split" {
            crate::config::SidebarLayoutConfig::Split
        } else {
            crate::config::SidebarLayoutConfig::Tree
        };
        assert!(!state.tick_task_animation(now + std::time::Duration::from_secs(1)));
        assert!(state.task_animation_deadline.is_none());
    }
}

#[test]
fn task_tree_status_info_is_read_only_and_closes_by_key_or_button() {
    let mut state = state();
    state.open_pane_context_menu("pane_1".into(), 5, 5);
    let index = match &state.overlay {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .position(|item| item.action == ClientContextMenuAction::StatusInfo)
            .expect("status item"),
        _ => panic!("menu"),
    };
    let mut outcome = ClientShellInput::default();
    state.activate_context_menu_item(index, &mut outcome);
    assert!(outcome.actions.is_empty());
    assert!(
        matches!(&state.overlay, Some(ClientShellOverlay::StatusInfo(text)) if text.contains("普通 Shell") && text.contains("窗格：pane_1"))
    );
    let input = state.handle_input_bytes(b"ignored");
    assert!(input.actions.is_empty() && input.requests.is_empty());
    state.compose(100, 24).expect("info panel");
    let close = state.hits.overlay_cancel;
    assert!(!close.is_empty());
    click(&mut state, close, MouseButton::Left);
    assert!(state.overlay.is_none());
    for key in [KeyCode::Esc, KeyCode::Enter] {
        state.open_task_status_info("pane_1");
        let input = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(key, KeyModifiers::empty()),
        )]);
        assert!(input.actions.is_empty());
        assert!(state.overlay.is_none());
    }
}

#[test]
fn task_tree_keyboard_visits_visible_shells_without_waiting_for_snapshot() {
    let mut state = state();
    let mut projected = snapshot();
    for number in 2..=3 {
        let mut pane = projected.panes[0].clone();
        pane.pane_id = format!("pane_{number}");
        pane.focused = false;
        projected.panes.push(pane);
    }
    state.set_snapshot(Box::new(projected));
    state.compose(100, 24).expect("three shells");
    state.mode = ClientShellMode::Navigate;
    for id in ["pane_2", "pane_3"] {
        let outcome = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(KeyCode::Tab, KeyModifiers::empty()),
        )]);
        assert!(
            matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == id))
        );
    }
    let previous = state.handle_raw_events(vec![RawInputEvent::Key(
        crate::input::TerminalKey::new(KeyCode::BackTab, KeyModifiers::SHIFT),
    )]);
    assert!(
        matches!(&previous.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"))
    );
    state
        .collapsed_tasks
        .insert((ClientEndpointId::Local, "ws_1".into()));
    let mut outcome = ClientShellInput::default();
    state.move_task_tree_focus(1, &mut outcome);
    assert!(
        matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, crate::api::schema::Method::WorkspaceFocus(target) if target.workspace_id == "ws_1"))
    );
}

#[test]
fn task_tree_workspace_menu_puts_creation_first_and_destructive_actions_last() {
    let mut state = state();
    state.open_workspace_context_menu("ws_1".into(), 4, 4);
    let Some(ClientShellOverlay::ContextMenu(menu)) = &state.overlay else {
        panic!("menu");
    };
    let items = menu.items();
    assert_eq!(items[0].action, ClientContextMenuAction::NewTab);
    assert_eq!(items[1].action, ClientContextMenuAction::RunTaskCommand);
    assert_eq!(
        items.last().expect("items").action,
        ClientContextMenuAction::Close
    );
}

#[test]
fn task_tree_reentering_navigation_uses_current_pane_and_drag_marker_follows_cards() {
    let mut state = state();
    let mut projected = snapshot();
    for number in 2..=3 {
        let mut pane = projected.panes[0].clone();
        pane.pane_id = format!("pane_{number}");
        pane.focused = false;
        projected.panes.push(pane);
    }
    state.set_snapshot(Box::new(projected.clone()));
    state.compose(100, 24).expect("three panes");
    state.mode = ClientShellMode::Navigate;
    state.move_task_tree_focus(1, &mut ClientShellInput::default());
    state.accept_navigate_workspace(&mut ClientShellInput::default());
    assert!(state.task_navigation_cursor.is_none());
    projected.focused_pane_id = Some("pane_3".into());
    for pane in &mut projected.panes {
        pane.focused = pane.pane_id == "pane_3";
    }
    state.set_snapshot(Box::new(projected));
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::WorkspacePicker),
        &mut ClientShellInput::default(),
    );
    let mut outcome = ClientShellInput::default();
    state.move_task_tree_focus(1, &mut outcome);
    assert!(
        matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, crate::api::schema::Method::WorkspaceFocus(target) if target.workspace_id == "ws_1"))
    );
    state.compose(100, 24).expect("visible cards");
    let last = state.hits.endpoint_agents.last().expect("last card").0;
    state.chrome_drag = Some(ClientChromeDrag::Workspace {
        source_workspace_id: "ws_1".into(),
        target: None,
    });
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 5,
        row: last.bottom(),
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(
        matches!(state.chrome_drag, Some(ClientChromeDrag::Workspace { target: Some((None, row)), .. }) if row == last.bottom())
    );
}

#[test]
fn task_tree_switching_endpoints_with_same_ids_reveals_only_destination() {
    use crate::client::endpoint::{ClientEndpointStatus, ProfileId, SavedSshEndpoint};
    let mut state = state();
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").expect("profile"),
        label: "remote".into(),
        target: "dev@example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let remote = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    let mut remote_snapshot = snapshot();
    remote_snapshot.boot_id = "remote".into();
    state.set_endpoint_snapshot(&remote, Box::new(remote_snapshot));
    state
        .collapsed_tasks
        .insert((ClientEndpointId::Local, "ws_1".into()));
    state
        .collapsed_tasks
        .insert((remote.clone(), "ws_1".into()));
    state.activate_endpoint_projection(&remote);
    assert!(state
        .collapsed_tasks
        .contains(&(ClientEndpointId::Local, "ws_1".into())));
    assert!(!state
        .collapsed_tasks
        .contains(&(remote.clone(), "ws_1".into())));
    state.compose(100, 24).expect("remote unavailable frame");
    assert!(state
        .hits
        .endpoint_agents
        .iter()
        .any(|(_, endpoint, pane)| endpoint == &remote && pane == "pane_1"));
}

#[test]
fn task_tree_requested_reveal_is_not_overridden_by_previous_focus() {
    let mut state = state();
    let mut projected = snapshot();
    for number in 2..=30 {
        let mut pane = projected.panes[0].clone();
        pane.pane_id = format!("pane_{number}");
        pane.focused = false;
        projected.panes.push(pane);
    }
    state.set_snapshot(Box::new(projected));
    state.compose(100, 16).expect("old focus");
    state.reveal_endpoint_agent(&ClientEndpointId::Local, "pane_30", 0);
    state
        .compose(100, 16)
        .expect("requested pane before focus response");
    assert!(state
        .hits
        .endpoint_agents
        .iter()
        .any(|(_, _, pane)| pane == "pane_30"));
}

#[test]
fn task_tree_command_only_sends_input_to_successful_new_pane() {
    use crate::api::schema::{Method, ResponseResult};
    for success in [true, false] {
        let mut state = state();
        state.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            title: "command",
            input: TextEditor::new("cdx", false),
            target: ClientRenameTarget::NewTaskCommand {
                workspace_id: "ws_1".into(),
            },
        }));
        let mut outcome = ClientShellInput::default();
        state.save_rename_overlay(&mut outcome);
        let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
            panic!("create request")
        };
        assert!(
            matches!(&request.method, Method::TabCreate(params) if params.workspace_id.as_deref() == Some("ws_1"))
        );
        let request_id = request.id.clone();
        let result = if success {
            Ok(serde_json::from_value::<ResponseResult>(serde_json::json!({
                "type": "tab_created", "tab": {"tab_id": "tab_new", "workspace_id": "ws_1", "number": 2, "label": "2", "focused": true, "pane_count": 1, "agent_status": "unknown"},
                "root_pane": {"pane_id": "pane_new", "terminal_id": "term_new", "workspace_id": "ws_1", "tab_id": "tab_new", "focused": true, "agent_status": "unknown", "revision": 0}
            })).expect("create result"))
        } else {
            Err(ClientShellEndpointError {
                code: Some("endpoint_timeout".into()),
                message: "timeout".into(),
            })
        };
        let (_, actions) = state.handle_endpoint_result("boot-1", &request_id, result);
        if success {
            assert!(
                matches!(&actions[..], [ClientShellAction::Endpoint { request, .. }]
                if matches!(&request.method, Method::PaneSendInput(params) if params.pane_id == "pane_new" && params.text == "cdx" && params.keys == ["Enter"]))
            );
        } else {
            assert!(actions.is_empty());
        }
    }
}
