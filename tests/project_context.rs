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
