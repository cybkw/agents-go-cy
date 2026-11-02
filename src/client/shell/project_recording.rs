// Modified for Agents Go by Agents Go contributors, 2026-10-27. See NOTICE.
//! Default project-local capture, kept out of snapshot/render and input I/O.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use super::*;

struct PendingSetup {
    cwd: String,
    receiver: Receiver<Result<PathBuf, String>>,
}

#[derive(Default)]
pub(super) struct ProjectRecording {
    dirty: bool,
    next_scan: Option<Instant>,
    seen: HashSet<String>,
    queue: VecDeque<String>,
    pending: Option<PendingSetup>,
    results: HashMap<String, Result<PathBuf, String>>,
}

impl ProjectRecording {
    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn finish_pending(&mut self) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("自动接入任务意外结束，请重新打开工具重试。".into())
            }
        };
        let pending = self.pending.take().expect("pending setup");
        self.results.insert(pending.cwd, result);
    }

    fn start_next(&mut self, executable: &Path) {
        if self.pending.is_some() {
            return;
        }
        let Some(cwd) = self.queue.pop_front() else {
            return;
        };
        let executable = executable.to_path_buf();
        let directory = cwd.clone();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("project-recording".into())
            .spawn(move || {
                let result =
                    crate::project_context::auto_enable(Path::new(&directory), &executable);
                let _ = sender.send(result);
            }) {
            Ok(_) => self.pending = Some(PendingSetup { cwd, receiver }),
            Err(error) => {
                self.results
                    .insert(cwd, Err(format!("无法启动自动接入任务：{error}")));
            }
        }
    }
}

impl ClientShellConfig {
    pub(crate) fn with_context_recorder(mut self, executable: Option<PathBuf>) -> Self {
        self.context_recorder = executable;
        self
    }
}

impl ClientShellState {
    pub(crate) fn poll_project_recording(&mut self, now: Instant) -> bool {
        let Some(executable) = self.config.context_recorder.as_deref() else {
            return false;
        };
        self.project_recording.finish_pending();
        // A remote machine's directories must never be installed on this host.
        // Observer clients have no recorder; remote-local/endpoint clients cannot
        // opt into writing just by receiving a local-shaped snapshot.
        if self.config.keybinding_source == ClientShellKeybindingSource::Local {
            if let Some(snapshot) = self.endpoints.iter().find_map(|endpoint| {
                (endpoint.endpoint_id.is_local() && endpoint.status == ClientEndpointStatus::Online)
                    .then_some(endpoint.snapshot.as_deref())
                    .flatten()
            }) {
                if self.project_recording.dirty
                    && self
                        .project_recording
                        .next_scan
                        .is_none_or(|deadline| now >= deadline)
                {
                    self.project_recording.dirty = false;
                    self.project_recording.next_scan = Some(now + Duration::from_secs(1));
                    // Once per changed-directory batch, in memory only. Known
                    // directories allocate nothing and never touch the filesystem.
                    for pane in &snapshot.panes {
                        let Some(cwd) = pane.foreground_cwd.as_deref().or(pane.cwd.as_deref())
                        else {
                            continue;
                        };
                        if !cwd.is_empty() && !self.project_recording.seen.contains(cwd) {
                            self.project_recording.seen.insert(cwd.to_owned());
                            self.project_recording.queue.push_back(cwd.to_owned());
                        }
                    }
                }
                self.project_recording.start_next(executable);
            }
        }
        if !self.active_endpoint_id.is_local() {
            return false;
        }
        let Some(ClientShellOverlay::ConversationHistory(history)) = self.overlay.as_mut() else {
            return false;
        };
        if !self.project_recording.seen.contains(history.cwd()) {
            return false;
        }
        if history.take_recording_setup_retry()
            && self
                .project_recording
                .results
                .get(history.cwd())
                .is_some_and(Result::is_err)
        {
            self.project_recording.results.remove(history.cwd());
            self.project_recording
                .queue
                .push_back(history.cwd().to_owned());
        }
        let status = self
            .project_recording
            .results
            .get(history.cwd())
            .map(|result| result.as_ref().map(|_| ()).map_err(Clone::clone));
        history.set_recording_setup(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Project(PathBuf);

    impl Project {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "agents-go-auto-recording-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ));
            fs::create_dir_all(root.join(".git")).unwrap();
            Self(root)
        }

        fn cwd(&self) -> String {
            self.0.to_str().unwrap().into()
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn state(project: &Project, recorder: bool) -> ClientShellState {
        let mut config = ClientShellConfig::from_config(&Config::default());
        if recorder {
            config.context_recorder = Some(std::env::current_exe().unwrap());
        }
        let mut state = ClientShellState::new(config);
        let mut snapshot = super::super::tests::snapshot();
        snapshot.panes[0].cwd = Some(project.cwd());
        snapshot.panes[0].foreground_cwd = Some(project.cwd());
        state.set_snapshot(Box::new(snapshot));
        state.set_pane_surface(super::super::tests::surface());
        state
    }

    fn settle(state: &mut ClientShellState, cwd: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            state.poll_project_recording(Instant::now());
            if state.project_recording.results.contains_key(cwd) {
                break;
            }
            assert!(Instant::now() < deadline, "recording setup timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn restored_and_new_project_directories_install_once_without_creating_history() {
        let first = Project::new();
        let second = Project::new();
        let mut state = state(&first, true);
        assert!(
            !first.0.join(".agents-go").exists(),
            "snapshot delivery must not do I/O"
        );
        settle(&mut state, &first.cwd());
        assert!(state.project_recording.results[&first.cwd()].is_ok());
        assert!(first.0.join(".codex/hooks.json").is_file());
        assert!(first.0.join(".claude/settings.local.json").is_file());
        assert!(!first.0.join(".agents-go/history.jsonl").exists());
        let hooks = fs::read(first.0.join(".codex/hooks.json")).unwrap();

        let mut next = state.snapshot.as_deref().unwrap().clone();
        next.revision += 1;
        let mut new_pane = next.panes[0].clone();
        new_pane.pane_id = "second".into();
        new_pane.foreground_cwd = Some(second.cwd());
        next.panes.push(new_pane);
        // Duplicate panes must not schedule duplicate installs.
        next.panes.push(next.panes[0].clone());
        state.project_recording.next_scan = Some(Instant::now() + Duration::from_secs(60));
        state.set_snapshot(Box::new(next));
        state.poll_project_recording(Instant::now());
        assert_eq!(
            state.project_recording.seen.len(),
            1,
            "directory scans are throttled"
        );
        state.project_recording.next_scan = None;
        settle(&mut state, &second.cwd());
        assert_eq!(state.project_recording.seen.len(), 2);
        assert_eq!(state.project_recording.results.len(), 2);
        assert!(state.project_recording.queue.is_empty());
        assert!(state.project_recording.pending.is_none());
        assert_eq!(fs::read(first.0.join(".codex/hooks.json")).unwrap(), hooks);
    }

    #[test]
    fn observers_remote_clients_and_offline_endpoints_never_install() {
        let project = Project::new();
        let mut observer = state(&project, false);
        observer.poll_project_recording(Instant::now());
        assert!(observer.project_recording.seen.is_empty());
        for source in [
            ClientShellKeybindingSource::RemoteLocal,
            ClientShellKeybindingSource::Endpoint,
        ] {
            let mut remote = state(&project, true);
            remote.config.keybinding_source = source;
            remote.poll_project_recording(Instant::now());
            assert!(remote.project_recording.seen.is_empty());
        }
        let mut offline = state(&project, true);
        offline.endpoints[0].status = ClientEndpointStatus::Reconnecting;
        offline.poll_project_recording(Instant::now());
        assert!(offline.project_recording.seen.is_empty());
        let mut remote = state(&project, true);
        let ssh = ClientEndpointId::Ssh(
            crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        );
        remote.active_endpoint_id = ssh.clone();
        remote.endpoints[0].endpoint_id = ssh;
        remote.poll_project_recording(Instant::now());
        assert!(remote.project_recording.seen.is_empty());
        assert!(!project.0.join(".agents-go").exists());
    }

    #[test]
    fn history_open_during_auto_setup_refreshes_into_the_trust_reminder() {
        let project = Project::new();
        let mut state = state(&project, true);
        state.overlay = Some(ClientShellOverlay::ConversationHistory(
            ConversationHistoryOverlay::new(project.cwd()),
        ));
        settle(&mut state, &project.cwd());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            state.poll_conversation_history();
            let frame = state.compose(140, 30).unwrap();
            let mut text = String::new();
            for row in frame.frame.cells.chunks(usize::from(frame.frame.width)) {
                let mut column = 0;
                while column < row.len() {
                    let symbol = row[column].symbol.as_str();
                    text.push_str(symbol);
                    // The wire frame includes a continuation cell for wide glyphs.
                    column += UnicodeWidthStr::width(symbol).max(1);
                }
                text.push('\n');
            }
            if text.contains("重开会话确认 Hook") {
                assert!(!text.contains("尚未启用"));
                assert!(!text.contains("采集中"));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "history did not refresh after installation"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn failed_setup_is_retained_without_hot_loop_retries() {
        let project = Project::new();
        fs::create_dir_all(project.0.join(".codex")).unwrap();
        fs::write(project.0.join(".codex/hooks.json"), "broken JSON").unwrap();
        let mut state = state(&project, true);
        settle(&mut state, &project.cwd());
        assert!(state.project_recording.results[&project.cwd()].is_err());
        let mut next = state.snapshot.as_deref().unwrap().clone();
        next.revision += 1;
        state.set_snapshot(Box::new(next));
        state.poll_project_recording(Instant::now() + Duration::from_secs(2));
        assert!(state.project_recording.pending.is_none());
        assert!(state.project_recording.queue.is_empty());
        assert!(!project.0.join(".agents-go/project.json").exists());
        assert_eq!(
            fs::read_to_string(project.0.join(".codex/hooks.json")).unwrap(),
            "broken JSON"
        );
    }

    #[test]
    fn explicit_history_refresh_retries_setup_after_configuration_is_repaired() {
        let project = Project::new();
        fs::create_dir_all(project.0.join(".codex")).unwrap();
        fs::write(project.0.join(".codex/hooks.json"), "broken JSON").unwrap();
        let mut state = state(&project, true);
        settle(&mut state, &project.cwd());
        assert!(state.project_recording.results[&project.cwd()].is_err());
        state.overlay = Some(ClientShellOverlay::ConversationHistory(
            ConversationHistoryOverlay::new(project.cwd()),
        ));
        state.poll_project_recording(Instant::now());
        // Simulate fixing the reported configuration problem, then use the
        // window's existing refresh action instead of waiting for a restart.
        fs::write(project.0.join(".codex/hooks.json"), "{}").unwrap();
        let Some(ClientShellOverlay::ConversationHistory(history)) = state.overlay.as_mut() else {
            panic!("expected history window");
        };
        history.handle_key(
            &crate::input::TerminalKey::new(KeyCode::Char('r'), KeyModifiers::empty()),
            &ConversationHistoryHits::default(),
        );
        state.poll_project_recording(Instant::now());
        settle(&mut state, &project.cwd());
        assert!(state.project_recording.results[&project.cwd()].is_ok());
        assert!(project.0.join(".agents-go/project.json").is_file());
    }
}
