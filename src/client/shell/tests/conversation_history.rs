// Modified for Agents Go by Agents Go contributors, 2026-10-28. See NOTICE.
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

fn continue_history_state(project: &HistoryProject) -> ClientShellState {
    let mut state = state();
    state.snapshot.as_mut().unwrap().panes[0].foreground_cwd = Some(project.cwd());
    state.open_workspace_context_menu("ws_1".into(), 2, 3);
    activate_history(&mut state);
    loaded_text(&mut state);
    state
}

fn history_endpoint_request(input: &ClientShellInput) -> crate::api::schema::Request {
    assert!(
        input.requests.is_empty(),
        "must not type into the current terminal"
    );
    let requests: Vec<_> = input
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(request.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1, "expected one endpoint request");
    requests[0].clone()
}

fn history_created_result(workspace: bool) -> crate::api::schema::ResponseResult {
    let mut result = serde_json::json!({
        "type": if workspace { "workspace_created" } else { "tab_created" },
        "tab": {"tab_id": "tab_new", "workspace_id": "ws_new", "number": 2,
            "label": "continued", "focused": true, "pane_count": 1, "agent_status": "unknown"},
        "root_pane": {"pane_id": "pane_new", "terminal_id": "term_new", "workspace_id": "ws_new",
            "tab_id": "tab_new", "focused": true, "agent_status": "unknown", "revision": 0}
    });
    if workspace {
        result["workspace"] = serde_json::json!({
            "workspace_id": "ws_new", "number": 2, "label": "continued", "focused": true,
            "pane_count": 1, "tab_count": 1, "active_tab_id": "tab_new", "agent_status": "unknown"
        });
    }
    serde_json::from_value(result).unwrap()
}

fn change_history_field(project: &HistoryProject, field: &str, value: &str) {
    let path = project.0.join(".agents-go/history.jsonl");
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record[field] = value.into();
    std::fs::write(
        path,
        format!("{}\n", serde_json::to_string(&record).unwrap()),
    )
    .unwrap();
}

#[test]
fn conversation_history_continue_uses_clicked_workspace_record_cwd_and_created_pane() {
    use crate::api::schema::Method;
    let project = HistoryProject::new("this is history, not shell input");
    let nested = project.0.join("nested");
    std::fs::create_dir(&nested).unwrap();
    change_history_field(&project, "cwd", nested.to_str().unwrap());
    let before = std::fs::read(project.0.join(".agents-go/history.jsonl")).unwrap();
    let mut state = continue_history_state(&project);
    let snapshot = state.snapshot.as_mut().unwrap();
    let mut second = snapshot.workspaces[0].clone();
    second.workspace_id = "ws_2".into();
    snapshot.workspaces.push(second);
    snapshot.focused_workspace_id = Some("ws_2".into());
    let create = history_endpoint_request(&state.handle_raw_events(vec![key(KeyCode::Char('c'))]));
    assert!(matches!(&create.method, Method::TabCreate(p)
        if p.workspace_id.as_deref() == Some("ws_1")
            && p.cwd.as_deref() == nested.to_str() && p.focus));
    assert!(state.overlay.is_none());
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &create.id, Ok(history_created_result(false)));
    let [ClientShellAction::Endpoint { request, .. }] = &actions[..] else {
        panic!("expected agent start on the new terminal");
    };
    let Method::AgentStart(start) = &request.method else {
        panic!("expected agent.start");
    };
    assert_eq!(start.kind, "codex");
    assert_eq!(start.pane_id, "pane_new");
    assert_eq!(start.args, ["resume", "history-window-test"]);
    assert!(start.name.starts_with("history-"));
    assert!(start.name.len() <= 32);
    assert!(start
        .name
        .bytes()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, b'-' | b'_')));
    assert_eq!(
        std::fs::read(project.0.join(".agents-go/history.jsonl")).unwrap(),
        before
    );
}

#[test]
fn conversation_history_continue_uses_selected_agent_and_original_session() {
    use crate::api::schema::Method;
    let project = HistoryProject::new("older codex message");
    crate::project_context::journal::capture(
        &project.0,
        "claude",
        &serde_json::json!({
            "hook_event_name": "Stop", "session_id": "claude-original", "cwd": project.cwd(),
            "last_assistant_message": "newer Claude reply; never execute this"
        }),
    )
    .unwrap();
    for (select_older, expected_agent, expected_args) in [
        (false, "claude", vec!["--resume", "claude-original"]),
        (true, "codex", vec!["resume", "history-window-test"]),
    ] {
        let mut state = continue_history_state(&project);
        // Use the same advertised methods as the real endpoint handshake.
        state.set_endpoint_methods(Some(
            crate::server::client_commands::supported_client_shell_method_names()
                .iter()
                .map(|method| (*method).to_owned())
                .collect(),
        ));
        if select_older {
            state.handle_raw_events(vec![key(KeyCode::End)]);
        }
        let create =
            history_endpoint_request(&state.handle_raw_events(vec![key(KeyCode::Char('C'))]));
        let (_, actions) =
            state.handle_endpoint_result("boot-1", &create.id, Ok(history_created_result(false)));
        assert!(
            matches!(&actions[..], [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, Method::AgentStart(p)
                if p.kind == expected_agent && p.args == expected_args && p.pane_id == "pane_new"))
        );
    }
}

#[test]
fn conversation_history_continue_recent_project_mouse_creates_workspace_then_starts_agent() {
    use crate::api::schema::Method;
    let project = HistoryProject::new("recent resume record");
    let mut state = state();
    state.set_endpoint_methods(Some(
        crate::server::client_commands::supported_client_shell_method_names()
            .iter()
            .map(|method| (*method).to_owned())
            .collect(),
    ));
    state.project_catalogs.insert(
        ClientEndpointId::Local,
        vec![crate::api::schema::ProjectInfo {
            cwd: project.cwd(),
            label: "closed project".into(),
        }],
    );
    state.compose(140, 35).unwrap();
    let recent = state.hits.recent_projects[0].0;
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: recent.x,
        row: recent.y,
        modifiers: KeyModifiers::empty(),
    })]);
    activate_history(&mut state);
    loaded_text(&mut state);
    let button = state.hits.conversation_history.continue_session;
    assert!(button.width > 0);
    let create = history_endpoint_request(&state.handle_raw_events(vec![RawInputEvent::Mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: button.x,
            row: button.y,
            modifiers: KeyModifiers::empty(),
        },
    )]));
    assert!(matches!(&create.method, Method::WorkspaceCreate(p)
        if p.cwd.as_deref() == Some(project.cwd().as_str()) && p.source_workspace_id.is_none() && p.focus));
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &create.id, Ok(history_created_result(true)));
    assert!(
        matches!(&actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, Method::AgentStart(p)
            if p.pane_id == "pane_new" && p.args == ["resume", "history-window-test"]))
    );
}

#[test]
fn conversation_history_continue_create_failures_never_start_an_agent() {
    let project = HistoryProject::new("resume failure");
    for code in ["endpoint_timeout", "endpoint_cancelled", "invalid_cwd"] {
        let mut state = continue_history_state(&project);
        let create =
            history_endpoint_request(&state.handle_raw_events(vec![key(KeyCode::Char('c'))]));
        let (_, actions) = state.handle_endpoint_result(
            "boot-1",
            &create.id,
            Err(ClientShellEndpointError {
                code: Some(code.into()),
                message: "creation failed".into(),
            }),
        );
        assert!(actions.is_empty());
        assert!(state.visible_endpoint_notice.is_some());
        // A late success for a completed/failed request must not start anything either.
        assert!(state
            .handle_endpoint_result("boot-1", &create.id, Ok(history_created_result(false)))
            .1
            .is_empty());
    }
}

#[test]
fn conversation_history_continue_ignores_old_boot_and_reset_responses() {
    let project = HistoryProject::new("resume stale response");
    for reset in [false, true] {
        let mut state = continue_history_state(&project);
        let create =
            history_endpoint_request(&state.handle_raw_events(vec![key(KeyCode::Char('c'))]));
        if reset {
            state.reset_endpoint_projection();
        }
        let boot = if reset { "boot-1" } else { "old-boot" };
        let (_, actions) =
            state.handle_endpoint_result(boot, &create.id, Ok(history_created_result(false)));
        assert!(actions.is_empty());
    }
}

#[test]
fn conversation_history_continue_rejects_invalid_record_identifiers_and_directories() {
    for (field, value) in [
        ("session_id", "--last"),
        ("session_id", "a;touch-bad"),
        ("session_id", "a\nexit"),
        ("session_id", "../session"),
        ("session_id", ""),
        ("agent", "unknown-agent"),
        ("cwd", "relative-directory"),
        ("cwd", "\ninvalid"),
    ] {
        let project = HistoryProject::new("untrusted ledger fields");
        change_history_field(&project, field, value);
        let mut state = continue_history_state(&project);
        let outcome = state.handle_raw_events(vec![key(KeyCode::Char('c'))]);
        assert!(
            outcome.actions.is_empty(),
            "unexpected request for {field}={value:?}"
        );
        assert!(outcome.requests.is_empty());
        assert!(matches!(
            state.overlay,
            Some(ClientShellOverlay::ConversationHistory(_))
        ));
    }
}

#[test]
fn conversation_history_continue_checks_both_capabilities_before_creating_terminal() {
    let project = HistoryProject::new("old server");
    for methods in [vec!["tab.create"], vec!["agent.start"]] {
        let mut state = continue_history_state(&project);
        state.set_endpoint_methods(Some(methods.into_iter().map(str::to_owned).collect()));
        let outcome = state.handle_raw_events(vec![key(KeyCode::Char('c'))]);
        assert!(outcome.actions.is_empty());
        assert!(outcome.requests.is_empty());
        assert!(matches!(
            state.overlay,
            Some(ClientShellOverlay::ConversationHistory(_))
        ));
    }
}

#[test]
fn conversation_history_continue_empty_history_cannot_open_terminal() {
    let project = HistoryProject::new("removed fixture message");
    std::fs::write(project.0.join(".agents-go/history.jsonl"), "").unwrap();
    let mut state = continue_history_state(&project);
    assert_eq!(state.hits.conversation_history.continue_session.width, 0);
    let outcome = state.handle_raw_events(vec![key(KeyCode::Char('c'))]);
    assert!(outcome.actions.is_empty());
    assert!(outcome.requests.is_empty());
}

#[test]
fn conversation_history_continue_repeat_key_does_not_create_terminal() {
    let project = HistoryProject::new("ignore held continue key");
    let mut state = continue_history_state(&project);
    let repeat = crate::input::TerminalKey::new(KeyCode::Char('c'), KeyModifiers::empty())
        .with_kind(crossterm::event::KeyEventKind::Repeat);
    let outcome = state.handle_raw_events(vec![RawInputEvent::Key(repeat)]);
    assert!(outcome.actions.is_empty());
    assert!(outcome.requests.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConversationHistory(_))
    ));
}

#[test]
fn conversation_history_continue_held_key_after_launch_does_not_reach_old_terminal() {
    let project = HistoryProject::new("continue key lease");
    let mut state = continue_history_state(&project);
    let continue_key = crate::input::TerminalKey::new(KeyCode::Char('c'), KeyModifiers::empty());
    let outcome = state.handle_raw_events(vec![
        RawInputEvent::Key(continue_key.clone()),
        RawInputEvent::Key(
            continue_key
                .clone()
                .with_kind(crossterm::event::KeyEventKind::Repeat),
        ),
        RawInputEvent::Key(continue_key.with_kind(crossterm::event::KeyEventKind::Release)),
    ]);
    let request = history_endpoint_request(&outcome);
    assert!(matches!(
        request.method,
        crate::api::schema::Method::TabCreate(_)
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn conversation_history_continue_closed_workspace_requires_reopening_history() {
    let project = HistoryProject::new("project closed during browsing");
    let mut state = continue_history_state(&project);
    state.snapshot.as_mut().unwrap().workspaces.clear();
    let outcome = state.handle_raw_events(vec![key(KeyCode::Char('c'))]);
    assert!(outcome.actions.is_empty());
    assert!(outcome.requests.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConversationHistory(_))
    ));
}

#[test]
fn conversation_history_continue_start_failure_reports_error_without_replaying_messages() {
    let project = HistoryProject::new("must not send this message as a prompt");
    let before = std::fs::read(project.0.join(".agents-go/history.jsonl")).unwrap();
    let mut state = continue_history_state(&project);
    let create = history_endpoint_request(&state.handle_raw_events(vec![key(KeyCode::Char('c'))]));
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &create.id, Ok(history_created_result(false)));
    let [ClientShellAction::Endpoint { request, .. }] = &actions[..] else {
        panic!("expected agent start request");
    };
    let (_, followup) = state.handle_endpoint_result(
        "boot-1",
        &request.id,
        Err(ClientShellEndpointError {
            code: Some("agent_target_busy".into()),
            message: "new terminal is busy".into(),
        }),
    );
    assert!(followup.is_empty());
    assert!(state.visible_endpoint_notice.is_some());
    assert_eq!(
        std::fs::read(project.0.join(".agents-go/history.jsonl")).unwrap(),
        before
    );
}

const IMPORT_SESSION_ID: &str = "01a18646-aa12-7950-be94-751538d844ef";

fn native_import_history_state(project: &HistoryProject, with_source: bool) -> ClientShellState {
    let user_home = project.0.join("import-user");
    let target_home = user_home.join(".codex");
    std::fs::create_dir_all(&target_home).unwrap();
    change_history_field(project, "session_id", IMPORT_SESSION_ID);
    if with_source {
        let source_dir = user_home.join(".codex-account-b/sessions/2026/10/29");
        std::fs::create_dir_all(&source_dir).unwrap();
        let path = source_dir.join(format!(
            "rollout-2026-10-29T14-09-00-{IMPORT_SESSION_ID}.jsonl"
        ));
        let metadata = serde_json::json!({
            "type": "session_meta",
            "payload": { "id": IMPORT_SESSION_ID, "cwd": project.cwd() }
        });
        std::fs::write(&path, format!("{metadata}\n")).unwrap();
        change_history_field(project, "transcript_path", path.to_str().unwrap());
    }
    let mut state = continue_history_state(project);
    state.config.codex_import_locations =
        Ok(crate::project_context::native_import::ImportLocations {
            target_home,
            user_home,
        });
    state.set_endpoint_methods(Some(
        crate::server::client_commands::supported_client_shell_method_names()
            .iter()
            .map(|method| (*method).to_owned())
            .collect(),
    ));
    state
}

fn settle_history_import(state: &mut ClientShellState) -> ClientShellInput {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let mut outcome = ClientShellInput::default();
        state.poll_conversation_history_continue(&mut outcome);
        if outcome.repaint {
            return outcome;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "native import timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn conversation_history_native_import_prepares_before_creation_and_keeps_current_account() {
    use crate::api::schema::Method;
    let project = HistoryProject::new("native import history");
    let mut state = native_import_history_state(&project, true);
    let initial = state.handle_raw_events(vec![key(KeyCode::Char('c'))]);
    assert!(initial.actions.is_empty());
    assert!(initial.requests.is_empty());
    let repeated = state.handle_raw_events(vec![key(KeyCode::Char('c'))]);
    assert!(repeated.actions.is_empty());
    let create = history_endpoint_request(&settle_history_import(&mut state));
    let Method::TabCreate(params) = &create.method else {
        panic!("expected tab creation")
    };
    let target = project.0.join("import-user").join(".codex");
    assert_eq!(
        params.env.get("CODEX_HOME").map(String::as_str),
        target.to_str()
    );
    assert_eq!(params.env.len(), 1);
    assert!(target
        .join(format!(
            "sessions/2026/10/29/rollout-2026-10-29T14-09-00-{IMPORT_SESSION_ID}.jsonl"
        ))
        .is_file());
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &create.id, Ok(history_created_result(false)));
    assert!(
        matches!(&actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, Method::AgentStart(params)
            if params.args == ["resume", IMPORT_SESSION_ID] && params.pane_id == "pane_new"))
    );
}

#[test]
fn conversation_history_native_import_failure_creates_no_terminal() {
    let project = HistoryProject::new("missing native history");
    let mut state = native_import_history_state(&project, false);
    state.handle_raw_events(vec![key(KeyCode::Char('c'))]);
    let outcome = settle_history_import(&mut state);
    assert!(outcome.actions.is_empty());
    assert!(outcome.requests.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConversationHistory(_))
    ));
    let frame = state.compose(140, 35).unwrap();
    let text: String = frame
        .frame
        .cells
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    let normalized: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    assert!(normalized.contains("无法继续"), "{text}");
}

#[test]
fn conversation_history_native_import_late_result_does_not_launch_after_close_restart_or_reset() {
    for action in 0..3 {
        let project = HistoryProject::new("late native import");
        let mut state = native_import_history_state(&project, true);
        state.handle_raw_events(vec![key(KeyCode::Char('c'))]);
        if action != 0 {
            if action == 1 {
                state.handle_raw_events(vec![key(KeyCode::Esc)]);
            } else {
                state.reset_endpoint_projection();
            }
            let mut outcome = ClientShellInput::default();
            state.poll_conversation_history_continue(&mut outcome);
            assert!(outcome.actions.is_empty());
            assert!(state.overlay.is_none());
        } else {
            state.snapshot.as_mut().unwrap().boot_id = "restarted-boot".into();
            let outcome = settle_history_import(&mut state);
            assert!(outcome.actions.is_empty());
            assert!(outcome.requests.is_empty());
        }
    }
}

#[test]
fn conversation_history_native_import_keeps_the_record_selected_when_c_was_pressed() {
    use crate::api::schema::Method;
    let project = HistoryProject::new("original native record");
    let mut state = native_import_history_state(&project, true);
    crate::project_context::journal::capture(
        &project.0,
        "claude",
        &serde_json::json!({
            "hook_event_name": "Stop", "session_id": "other-session", "cwd": project.cwd(),
            "last_assistant_message": "a different conversation"
        }),
    )
    .unwrap();
    state.handle_raw_events(vec![key(KeyCode::Char('R'))]);
    loaded_text(&mut state);
    state.handle_raw_events(vec![key(KeyCode::Char('c'))]);
    state.handle_raw_events(vec![key(KeyCode::Home)]);
    let create = history_endpoint_request(&settle_history_import(&mut state));
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &create.id, Ok(history_created_result(false)));
    assert!(
        matches!(&actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, Method::AgentStart(params)
            if params.kind == "codex" && params.args == ["resume", IMPORT_SESSION_ID]))
    );
}
