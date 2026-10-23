// Modified for Agents Go by Agents Go contributors, 2026-10-22. See NOTICE.
use super::*;

struct HistoryProject(std::path::PathBuf);

impl HistoryProject {
    fn new(text: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "agents-go-history-window-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join(".agents-go")).unwrap();
        std::fs::write(root.join(".agents-go/project.json"), br#"{"version":1}"#).unwrap();
        crate::project_context::journal::capture(
            &root,
            "codex",
            &serde_json::json!({
                "hook_event_name": "UserPromptSubmit",
                "session_id": "history-window-test",
                "turn_id": "test-turn",
                "cwd": root.to_str().unwrap(),
                "prompt": text,
            }),
        )
        .unwrap();
        Self(root)
    }

    fn cwd(&self) -> String {
        self.0.to_str().unwrap().to_owned()
    }
}

impl Drop for HistoryProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state
}

fn activate_history(state: &mut ClientShellState) {
    let Some(ClientShellOverlay::ContextMenu(menu)) = state.overlay.as_ref() else {
        panic!("expected project menu");
    };
    let index = menu
        .items()
        .iter()
        .position(|item| item.action == ClientContextMenuAction::ConversationHistory)
        .expect("conversation history entry");
    let outcome = &mut ClientShellInput::default();
    state.activate_context_menu_item(index, outcome);
    assert!(outcome.requests.is_empty());
    assert!(outcome.actions.is_empty());
}

fn loaded_text(state: &mut ClientShellState) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !state.poll_conversation_history() {
        assert!(
            std::time::Instant::now() < deadline,
            "history worker timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let frame = state.compose(140, 35).expect("history window");
    frame
        .frame
        .cells
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect()
}

fn key(code: KeyCode) -> RawInputEvent {
    RawInputEvent::Key(crate::input::TerminalKey::new(code, KeyModifiers::empty()))
}

#[test]
fn conversation_history_workspace_menu_reads_clicked_project_without_changing_focus() {
    let project = HistoryProject::new("clicked-project-record");
    let other = HistoryProject::new("other-project-record");
    let mut state = state();
    let mut projected = snapshot();
    // A Home/Path launch policy must not redirect history to another project.
    projected.workspaces[0].new_workspace_cwd = other.cwd();
    projected.panes[0].foreground_cwd = Some(project.cwd());
    projected.panes[0].cwd = Some(other.cwd());
    let mut second = projected.workspaces[0].clone();
    second.workspace_id = "ws_2".into();
    second.new_workspace_cwd = other.cwd();
    projected.workspaces.push(second);
    projected.focused_workspace_id = Some("ws_2".into());
    state.set_snapshot(Box::new(projected));
    state.open_workspace_context_menu("ws_1".into(), 2, 3);
    activate_history(&mut state);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConversationHistory(_))
    ));
    let text = loaded_text(&mut state);
    assert!(text.contains("clicked-project-record"));
    assert!(!text.contains("other-project-record"));
    assert_eq!(
        state
            .snapshot
            .as_ref()
            .unwrap()
            .focused_workspace_id
            .as_deref(),
        Some("ws_2")
    );
}

#[test]
fn conversation_history_uses_active_tab_and_falls_back_to_shell_cwd() {
    let project = HistoryProject::new("active-tab-record");
    let other = HistoryProject::new("inactive-tab-record");
    let mut state = state();
    let mut projected = snapshot();
    projected.panes[0].foreground_cwd = Some(other.cwd());
    let mut pane = projected.panes[0].clone();
    pane.pane_id = "pane_2".into();
    pane.tab_id = "tab_2".into();
    pane.foreground_cwd = None;
    pane.cwd = Some(project.cwd());
    pane.focused = false;
    projected.panes.push(pane);
    let mut tab = projected.tabs[0].clone();
    tab.tab_id = "tab_2".into();
    projected.tabs.push(tab);
    projected.workspaces[0].active_tab_id = "tab_2".into();
    state.set_snapshot(Box::new(projected));
    state.open_workspace_context_menu("ws_1".into(), 2, 3);
    activate_history(&mut state);
    let text = loaded_text(&mut state);
    assert!(text.contains("active-tab-record"));
    assert!(!text.contains("inactive-tab-record"));
}

#[test]
fn conversation_history_missing_pane_directory_does_not_use_launch_policy() {
    let project = HistoryProject::new("unrelated-launch-directory");
    let mut state = state();
    let projected = state.snapshot.as_mut().unwrap();
    projected.workspaces[0].new_workspace_cwd = project.cwd();
    projected.panes[0].foreground_cwd = None;
    projected.panes[0].cwd = None;
    state.open_workspace_context_menu("ws_1".into(), 2, 3);
    activate_history(&mut state);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::StatusInfo(_))
    ));
    assert!(!state.poll_conversation_history());
}

#[test]
fn conversation_history_recent_project_menu_opens_without_starting_a_terminal() {
    let project = HistoryProject::new("recent-project-record");
    let original = std::fs::read(project.0.join(".agents-go/history.jsonl")).unwrap();
    let mut state = state();
    state.endpoints[0].methods = Some(
        ["project.list", "project.open", "project.forget"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    state.project_catalogs.insert(
        ClientEndpointId::Local,
        vec![crate::api::schema::ProjectInfo {
            cwd: project.cwd(),
            label: "saved project".into(),
        }],
    );
    state.compose(100, 30).unwrap();
    let recent = state.hits.recent_projects[0].0;
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: recent.x,
        row: recent.y,
        modifiers: KeyModifiers::empty(),
    })]);
    activate_history(&mut state);
    assert!(loaded_text(&mut state).contains("recent-project-record"));
    assert_eq!(
        std::fs::read(project.0.join(".agents-go/history.jsonl")).unwrap(),
        original
    );
}

#[test]
fn conversation_history_remote_endpoints_and_remote_clients_cannot_read_local_paths() {
    let project = HistoryProject::new("local-private-record");
    for source in [
        ClientShellKeybindingSource::RemoteLocal,
        ClientShellKeybindingSource::Endpoint,
    ] {
        let mut state = state();
        state.config.keybinding_source = source;
        state.snapshot.as_mut().unwrap().panes[0].foreground_cwd = Some(project.cwd());
        state.open_workspace_context_menu("ws_1".into(), 2, 3);
        activate_history(&mut state);
        assert!(matches!(
            state.overlay,
            Some(ClientShellOverlay::StatusInfo(_))
        ));
        assert!(!state.poll_conversation_history());
    }
    let mut state = state();
    state.active_endpoint_id = ClientEndpointId::Ssh(
        crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
    );
    state.snapshot.as_mut().unwrap().panes[0].foreground_cwd = Some(project.cwd());
    state.open_workspace_context_menu("ws_1".into(), 2, 3);
    activate_history(&mut state);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::StatusInfo(_))
    ));
    assert!(!state.poll_conversation_history());
}

#[test]
fn conversation_history_blocks_terminal_and_popup_input_until_closed() {
    let project = HistoryProject::new("read-only-history");
    let mut state = state();
    state.snapshot.as_mut().unwrap().panes[0].foreground_cwd = Some(project.cwd());
    state.open_workspace_context_menu("ws_1".into(), 2, 3);
    activate_history(&mut state);
    loaded_text(&mut state);
    // A server popup must not receive typing or paste while this client window is visible.
    state.popup_terminal_id = Some("popup-terminal".into());
    let outcome = state.handle_raw_events(vec![
        key(KeyCode::Char('x')),
        RawInputEvent::Text(crate::input::TextCommit::new("中文输入")),
        RawInputEvent::Paste("do not send to shell".into()),
    ]);
    assert!(outcome.requests.is_empty());
    assert!(state.clipboard_image_target().is_none());
    let close = state.handle_raw_events(vec![key(KeyCode::Esc)]);
    assert!(close.requests.is_empty());
    assert!(state.overlay.is_none());
    assert!(!state.poll_conversation_history());
    state.popup_terminal_id = None;
    let typing = state.handle_raw_events(vec![key(KeyCode::Char('x'))]);
    assert!(typing.requests.iter().any(|request| matches!(request,
        ClientMessage::ClientShellPaneInput { pane_id, .. } if pane_id == "pane_1"
    )));
}

#[test]
fn conversation_history_blocks_repeats_of_keys_held_before_opening() {
    let project = HistoryProject::new("held-key-history");
    let mut state = state();
    state.snapshot.as_mut().unwrap().panes[0].foreground_cwd = Some(project.cwd());
    let held = crate::input::TerminalKey::new(KeyCode::Char('x'), KeyModifiers::empty());
    let typing = state.handle_raw_events(vec![RawInputEvent::Key(held.clone())]);
    assert!(typing.requests.iter().any(|request| matches!(request,
        ClientMessage::ClientShellPaneInput { pane_id, .. } if pane_id == "pane_1"
    )));
    state.open_workspace_context_menu("ws_1".into(), 2, 3);
    activate_history(&mut state);
    let repeat = state.handle_raw_events(vec![RawInputEvent::Key(
        held.clone()
            .with_kind(crossterm::event::KeyEventKind::Repeat),
    )]);
    assert!(repeat.requests.is_empty());
    // Release the previously forwarded press so terminal key state cannot stick.
    let release = state.handle_raw_events(vec![RawInputEvent::Key(
        held.with_kind(crossterm::event::KeyEventKind::Release),
    )]);
    assert!(release.requests.iter().any(|request| matches!(request,
        ClientMessage::ClientShellPaneInput { events, .. }
            if matches!(events.as_slice(), [crate::protocol::ClientPaneInputEvent::Key {
                kind: crate::protocol::ClientKeyKind::Release, ..
            }])
    )));
}

#[test]
fn conversation_history_endpoint_reset_discards_the_pending_window() {
    let project = HistoryProject::new("old-endpoint-record");
    let mut state = state();
    state.snapshot.as_mut().unwrap().panes[0].foreground_cwd = Some(project.cwd());
    state.open_workspace_context_menu("ws_1".into(), 2, 3);
    activate_history(&mut state);
    state.reset_endpoint_projection();
    assert!(state.overlay.is_none());
    assert!(!state.poll_conversation_history());
}
