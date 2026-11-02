//! CLI coverage without a workspace server, a PTY, or an installed AI agent.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

fn project() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".tmp/project-context-tests")
        .join(format!(
            "中文 project-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_herdr"))
        .arg("context")
        .args(args)
        .arg("--project")
        .arg(root)
        .output()
        .unwrap()
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn hook(mut command: Command, payload: &Value) -> Child {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(payload).unwrap())
        .unwrap();
    child
}

fn capture(root: &Path, agent: &str, payload: &Value) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr"));
    command
        .args(["context", "capture", "--agent", agent, "--project"])
        .arg(root);
    hook(command, payload)
}

fn prompt(root: &Path, session: &str, turn: usize, text: &str) -> Value {
    json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": session,
        "turn_id": format!("turn-{turn}"),
        "cwd": root,
        "prompt": text,
    })
}

fn records(root: &Path) -> Vec<Value> {
    let output = cli(root, &["history", "--limit", "100", "--json"]);
    success(&output);
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["records"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn project_context_cli_records_and_manually_restores_without_mutation() {
    let root = project();
    success(&cli(&root, &["enable"]));
    let source = "保存原文\n\n```rust\nlet value = 1;\n```\n";
    let output = capture(&root, "codex", &prompt(&root, "codex-one", 1, source))
        .wait_with_output()
        .unwrap();
    success(&output);
    assert!(output.stdout.is_empty(), "capture must not inject context");
    let output = capture(
        &root,
        "codex",
        &json!({
            "hook_event_name": "Stop", "cwd": root,
            "session_id": "codex-one", "turn_id": "turn-1",
            "last_assistant_message": "已实现第一阶段，等待手动恢复。",
        }),
    )
    .wait_with_output()
    .unwrap();
    success(&output);
    for _ in 0..2 {
        success(
            &capture(
                &root,
                "claude",
                &json!({
                    "hook_event_name": "UserPromptSubmit", "cwd": root,
                    "session_id": "claude-one", "prompt": "继续",
                }),
            )
            .wait_with_output()
            .unwrap(),
        );
    }
    let saved = records(&root);
    assert_eq!(saved.len(), 4);
    assert_eq!(saved[0]["text"], source);
    assert_eq!(saved[1]["reply_to"], 1);
    assert_eq!(saved[3]["seq"], 4);
    let before = std::fs::read(root.join(".agents-go/history.jsonl")).unwrap();
    let resume = cli(&root, &["resume", "--limit", "2"]);
    success(&resume);
    let text = String::from_utf8(resume.stdout).unwrap();
    assert!(text.contains("#0003–#0004"));
    assert!(!text.contains(source));
    let history = cli(&root, &["history", "--from", "1", "--through", "1"]);
    success(&history);
    assert!(String::from_utf8(history.stdout).unwrap().contains(source));
    assert_eq!(
        before,
        std::fs::read(root.join(".agents-go/history.jsonl")).unwrap()
    );
    success(&cli(&root, &["enable"]));
    assert_eq!(
        before,
        std::fs::read(root.join(".agents-go/history.jsonl")).unwrap()
    );
}

#[test]
fn project_context_cli_serializes_separate_processes_and_isolates_projects() {
    let root = project();
    let other = project();
    success(&cli(&root, &["enable", "--agent", "codex"]));
    success(&cli(&other, &["enable", "--agent", "codex"]));
    let children: Vec<_> = (0..8)
        .map(|turn| capture(&root, "codex", &prompt(&root, "parallel", turn, "独立提交")))
        .collect();
    for child in children {
        success(&child.wait_with_output().unwrap());
    }
    let saved = records(&root);
    assert_eq!(saved.len(), 8);
    for (index, record) in saved.iter().enumerate() {
        assert_eq!(record["seq"], (index + 1) as u64);
    }
    success(
        &capture(&root, "codex", &prompt(&root, "parallel", 0, "独立提交"))
            .wait_with_output()
            .unwrap(),
    );
    assert_eq!(records(&root), saved);
    assert!(records(&other).is_empty());
    let foreign = capture(
        &root,
        "codex",
        &prompt(&other, "wrong-project", 1, "私人原文"),
    )
    .wait_with_output()
    .unwrap();
    assert!(!foreign.status.success());
    assert!(!String::from_utf8_lossy(&foreign.stderr).contains("私人原文"));
    assert_eq!(records(&root), saved);
}

#[test]
fn project_context_shared_capture_routes_by_cwd_and_skips_unenabled_projects() {
    let first = project();
    let second = project();
    let unenabled = project();
    std::fs::create_dir(unenabled.join(".git")).unwrap();
    for root in [&first, &second] {
        std::fs::create_dir(root.join(".agents-go")).unwrap();
        std::fs::write(root.join(".agents-go/project.json"), r#"{"version":1}"#).unwrap();
    }
    for (root, session) in [(&first, "first"), (&second, "second"), (&unenabled, "skip")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr"));
        command.args(["context", "capture", "--agent", "codex", "--if-enabled"]);
        let output = hook(command, &prompt(root, session, 1, session))
            .wait_with_output()
            .unwrap();
        success(&output);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
    assert!(!unenabled.join(".agents-go").exists());
    let first_records = records(&first);
    let second_records = records(&second);
    assert_eq!(first_records.len(), 1);
    assert_eq!(second_records.len(), 1);
    assert_eq!(first_records[0]["seq"], 1);
    assert_eq!(second_records[0]["seq"], 1);
    assert_eq!(first_records[0]["text"], "first");
    assert_eq!(second_records[0]["text"], "second");
}

#[test]
fn project_context_shared_capture_reports_broken_configuration_and_rejects_project_constraint() {
    let root = project();
    std::fs::create_dir(root.join(".agents-go")).unwrap();
    std::fs::write(root.join(".agents-go/project.json"), "broken").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr"));
    command.args(["context", "capture", "--agent", "codex", "--if-enabled"]);
    let output = hook(command, &prompt(&root, "broken", 1, "不得泄漏的原文"))
        .wait_with_output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("不得泄漏的原文"));
    assert!(!root.join(".agents-go/history.jsonl").exists());
    let output = cli(&root, &["capture", "--agent", "codex", "--if-enabled"]);
    assert_eq!(output.status.code(), Some(2));
}

#[cfg(windows)]
#[test]
fn project_context_installed_windows_hooks_deliver_original_json_to_the_real_cli() {
    let root = project();
    success(&cli(&root, &["enable"]));
    let codex: Value =
        serde_json::from_slice(&std::fs::read(root.join(".codex/hooks.json")).unwrap()).unwrap();
    let native = &codex["hooks"]["UserPromptSubmit"][0]["hooks"][0];
    let encoded = native["commandWindows"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap();
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", encoded]);
    let original = "中文原文\n多行 ` $() ' \" 保持不变";
    let output = hook(command, &prompt(&root, "native-codex", 1, original))
        .wait_with_output()
        .unwrap();
    success(&output);
    assert!(output.stdout.is_empty());
    let claude: Value =
        serde_json::from_slice(&std::fs::read(root.join(".claude/settings.local.json")).unwrap())
            .unwrap();
    let native = &claude["hooks"]["Stop"][0]["hooks"][0];
    let mut command = Command::new(native["command"].as_str().unwrap());
    command.args(
        native["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap()),
    );
    let output = hook(
        command,
        &json!({
            "hook_event_name": "Stop", "cwd": root,
            "session_id": "native-claude", "last_assistant_message": "接续上下文\n完整回复",
        }),
    )
    .wait_with_output()
    .unwrap();
    success(&output);
    assert!(output.stdout.is_empty());
    let saved = records(&root);
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[0]["text"], original);
    assert_eq!(saved[1]["text"], "接续上下文\n完整回复");
}

#[cfg(windows)]
#[test]
fn project_context_shared_windows_hook_routes_real_git_worktrees_and_skips_unenabled() {
    let main = project();
    let linked = project();
    let skipped = project();
    let git = |args: &[&std::ffi::OsStr]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(args)
            .output()
            .unwrap();
        success(&output);
    };
    git(&["init".as_ref()]);
    git(&[
        "-c".as_ref(),
        "user.name=Fixture".as_ref(),
        "-c".as_ref(),
        "user.email=fixture@example.invalid".as_ref(),
        "-c".as_ref(),
        "commit.gpgsign=false".as_ref(),
        "commit".as_ref(),
        "--allow-empty".as_ref(),
        "-m".as_ref(),
        "fixture".as_ref(),
    ]);
    for directory in [&linked, &skipped] {
        git(&[
            "worktree".as_ref(),
            "add".as_ref(),
            "--detach".as_ref(),
            directory.as_os_str(),
        ]);
    }
    success(&cli(&linked, &["enable", "--agent", "codex"]));
    assert!(!main.join(".agents-go/project.json").exists());
    success(&cli(&main, &["enable", "--agent", "codex"]));
    let settings: Value =
        serde_json::from_slice(&std::fs::read(main.join(".codex/hooks.json")).unwrap()).unwrap();
    for (root, session) in [(&main, "main"), (&linked, "linked"), (&skipped, "skipped")] {
        let cwd = root.join("src");
        std::fs::create_dir(&cwd).unwrap();
        for (event, text_key, text) in [
            ("UserPromptSubmit", "prompt", "中文原文\n` $() ' \""),
            ("Stop", "last_assistant_message", "完整回复"),
        ] {
            let encoded = settings["hooks"][event][0]["hooks"][0]["commandWindows"]
                .as_str()
                .unwrap()
                .split_whitespace()
                .last()
                .unwrap();
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", encoded]);
            let mut payload = prompt(&cwd, session, 1, "");
            payload["hook_event_name"] = json!(event);
            payload[text_key] = json!(text);
            let output = hook(command, &payload).wait_with_output().unwrap();
            success(&output);
            assert!(output.stdout.is_empty());
            assert!(
                output.stderr.is_empty(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    assert!(!skipped.join(".agents-go").exists());
    for (root, session) in [(&main, "main"), (&linked, "linked")] {
        let saved = records(root);
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0]["seq"], 1);
        assert_eq!(saved[0]["text"], "中文原文\n` $() ' \"");
        assert_eq!(saved[1]["seq"], 2);
        assert_eq!(saved[1]["reply_to"], 1);
        assert!(saved.iter().all(|record| record["session_id"] == session));
    }
}
