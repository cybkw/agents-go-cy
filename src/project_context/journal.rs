use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const CONTEXT_DIR: &str = ".agents-go";
const HISTORY_FILE: &str = "history.jsonl";
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Record {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub agent: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub role: String,
    pub text: String,
    pub cwd: String,
    pub reply_to: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event_key: Option<String>,
}

/// Append complete hook messages only. Enabling a project belongs to the caller;
/// receiving a hook must never create a context directory by itself.
pub(crate) fn capture(root: &Path, agent: &str, payload: &Value) -> Result<Option<Record>, String> {
    capture_with_timeout(root, agent, payload, LOCK_TIMEOUT)
}

fn capture_with_timeout(
    root: &Path,
    agent: &str,
    payload: &Value,
    lock_timeout: Duration,
) -> Result<Option<Record>, String> {
    if !matches!(agent, "codex" | "claude") {
        return Err("Unsupported project context agent.".to_owned());
    }
    if payload
        .get("agent_id")
        .is_some_and(|id| !id.is_null() && id.as_str() != Some(""))
    {
        return Ok(None);
    }
    let (event, role, text_field) = match payload.get("hook_event_name").and_then(Value::as_str) {
        Some("UserPromptSubmit") => ("UserPromptSubmit", "user", "prompt"),
        Some("Stop") => ("Stop", "assistant", "last_assistant_message"),
        _ => return Ok(None),
    };
    let session_id = required_string(payload, "session_id")?;
    let cwd = required_string(payload, "cwd")?;
    let text = match payload.get(text_field) {
        None | Some(Value::Null) if role == "assistant" => return Ok(None),
        Some(Value::String(text)) if role == "assistant" && text.trim().is_empty() => {
            return Ok(None);
        }
        Some(Value::String(text)) => text,
        _ => return Err(format!("Hook field {text_field} must be a string.")),
    };
    // Claude's common hook input uses prompt_id for the current user prompt;
    // Codex provides turn_id. Store both agents' turn identity in one field.
    let turn_id = if agent == "claude" {
        optional_string(payload, "prompt_id")?
    } else {
        optional_string(payload, "turn_id")?
    };
    let message_id = optional_string(payload, "message_id")?;
    // Claude hooks without a stable identity can repeat real user messages.
    // Text alone is never sufficient to decide that two hooks are duplicates.
    let event_key = message_id
        .map(|id| ("message", id))
        .or_else(|| turn_id.map(|id| ("turn", id)))
        .map(|(kind, id)| {
            let identity = serde_json::to_vec(&(agent, session_id, kind, id, event, text))
                .expect("serializing hook identity strings cannot fail");
            format!("{:x}", Sha256::digest(identity))
        });

    let context_dir = root.join(CONTEXT_DIR);
    if !context_dir.is_dir() {
        return Err("Project context is not enabled.".to_owned());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(context_dir.join(HISTORY_FILE))
        .map_err(|err| format!("Could not open project history: {err}"))?;
    // Separate hook processes share this lock. Read the authoritative sequence
    // under the same lock that protects the complete append and durable flush.
    lock_history(&file, false, lock_timeout)?;
    let records = load_records(&mut file)?;
    if event_key.as_ref().is_some_and(|key| {
        records
            .iter()
            .any(|record| record.event_key.as_ref() == Some(key))
    }) {
        return Ok(None);
    }
    let seq = records
        .last()
        .map_or(Some(1), |record| record.seq.checked_add(1))
        .ok_or_else(|| "Project history sequence is exhausted.".to_owned())?;
    let reply_to = if role == "assistant" {
        previous_user(&records, agent, session_id, turn_id)
    } else {
        None
    };
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock precedes the Unix epoch.".to_owned())?
        .as_millis()
        .try_into()
        .map_err(|_| "Project history timestamp is out of range.".to_owned())?;
    let record = Record {
        seq,
        timestamp_ms,
        agent: agent.to_owned(),
        session_id: session_id.to_owned(),
        turn_id: turn_id.map(str::to_owned),
        role: role.to_owned(),
        text: text.to_owned(),
        cwd: cwd.to_owned(),
        reply_to,
        event_key,
    };
    let mut encoded = serde_json::to_vec(&record)
        .map_err(|_| "Could not encode project history record.".to_owned())?;
    encoded.push(b'\n');
    file.seek(SeekFrom::End(0))
        .and_then(|_| file.write_all(&encoded))
        .and_then(|_| file.sync_data())
        .map_err(|err| format!("Could not save project history: {err}"))?;
    Ok(Some(record))
}

/// Select an inclusive sequence range, then retain its most recent `limit`
/// entries. Opening a missing history for reading never creates any files.
pub(crate) fn read(
    root: &Path,
    from: Option<u64>,
    through: Option<u64>,
    limit: usize,
) -> Result<Vec<Record>, String> {
    read_with_timeout(root, from, through, limit, LOCK_TIMEOUT)
}

fn read_with_timeout(
    root: &Path,
    from: Option<u64>,
    through: Option<u64>,
    limit: usize,
    lock_timeout: Duration,
) -> Result<Vec<Record>, String> {
    if limit == 0 {
        return Err("Project history limit must be greater than zero.".to_owned());
    }
    if matches!((from, through), (Some(start), Some(end)) if start > end) {
        return Err("Project history range starts after its end.".to_owned());
    }
    let mut file = match File::open(root.join(CONTEXT_DIR).join(HISTORY_FILE)) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("Could not open project history: {err}")),
    };
    lock_history(&file, true, lock_timeout)?;
    let mut records = load_records(&mut file)?;
    records.retain(|record| {
        from.is_none_or(|start| record.seq >= start) && through.is_none_or(|end| record.seq <= end)
    });
    if records.len() > limit {
        records.drain(..records.len() - limit);
    }
    Ok(records)
}

fn lock_history(file: &File, shared: bool, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let result = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        match result {
            Ok(()) => return Ok(()),
            Err(TryLockError::Error(err)) => {
                return Err(format!("Could not lock project history: {err}"));
            }
            Err(TryLockError::WouldBlock) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err("Project history is busy; try again shortly.".to_owned());
                }
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
        }
    }
}

pub(crate) fn render_markdown(records: &[Record]) -> String {
    let mut rendered = String::from("# Project conversation history\n\n");
    for record in records {
        rendered.push_str(&format!(
            "## #{:04} {} · {}\n\n- timestamp_ms: {}\n- session_id: {}\n- cwd: {}\n",
            record.seq,
            record.role,
            record.agent,
            record.timestamp_ms,
            quoted(&record.session_id),
            quoted(&record.cwd),
        ));
        if let Some(turn_id) = &record.turn_id {
            rendered.push_str(&format!("- turn_id: {}\n", quoted(turn_id)));
        }
        if let Some(reply_to) = record.reply_to {
            rendered.push_str(&format!("- reply_to: #{reply_to:04}\n"));
        }
        // Messages can contain their own Markdown fences. A longer outer fence
        // preserves the complete original body and keeps record headings apart.
        let mut longest = 0;
        let mut run = 0;
        for byte in record.text.bytes() {
            run = if byte == b'`' { run + 1 } else { 0 };
            longest = longest.max(run);
        }
        let fence = "`".repeat((longest + 1).max(3));
        rendered.push_str(&format!("\n{fence}text\n"));
        rendered.push_str(&record.text);
        if !record.text.ends_with('\n') {
            rendered.push('\n');
        }
        rendered.push_str(&format!("{fence}\n\n"));
    }
    rendered
}

fn required_string<'a>(payload: &'a Value, field: &str) -> Result<&'a str, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("Hook field {field} must be a nonempty string."))
}

fn optional_string<'a>(payload: &'a Value, field: &str) -> Result<Option<&'a str>, String> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok((!text.is_empty()).then_some(text.as_str())),
        _ => Err(format!("Hook field {field} must be a string.")),
    }
}

fn previous_user(
    records: &[Record],
    agent: &str,
    session_id: &str,
    turn_id: Option<&str>,
) -> Option<u64> {
    let same_session = || {
        records.iter().rev().filter(|record| {
            record.agent == agent && record.session_id == session_id && record.role == "user"
        })
    };
    if let Some(turn_id) = turn_id {
        if let Some(record) =
            same_session().find(|record| record.turn_id.as_deref() == Some(turn_id))
        {
            return Some(record.seq);
        }
        // Older hooks may omit a turn ID. Do not override an explicitly
        // different turn just because its prompt arrived more recently.
        return same_session()
            .next()
            .filter(|record| record.turn_id.is_none())
            .map(|record| record.seq);
    }
    same_session().next().map(|record| record.seq)
}

fn load_records(file: &mut File) -> Result<Vec<Record>, String> {
    let mut reader = BufReader::new(file);
    let mut records: Vec<Record> = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        let bytes = reader
            .read_until(b'\n', &mut line)
            .map_err(|err| format!("Could not read project history: {err}"))?;
        if bytes == 0 {
            break;
        }
        let line_number = records.len() + 1;
        let invalid = || format!("Invalid project history record at line {line_number}.");
        // Every successful append ends with a newline. An incomplete tail is
        // evidence of an interrupted write and must never be silently replaced.
        if line.last() != Some(&b'\n') {
            return Err(invalid());
        }
        let record: Record = serde_json::from_slice(&line).map_err(|_| invalid())?;
        let expected = records
            .last()
            .map_or(Some(1), |record| record.seq.checked_add(1));
        if Some(record.seq) != expected
            || !matches!(record.agent.as_str(), "codex" | "claude")
            || !matches!(record.role.as_str(), "user" | "assistant" | "event")
            || record.session_id.trim().is_empty()
            || record.cwd.trim().is_empty()
            || record
                .reply_to
                .is_some_and(|seq| seq == 0 || seq >= record.seq)
        {
            return Err(invalid());
        }
        records.push(record);
    }
    Ok(records)
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("serializing a string cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn project() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agents-go-context-{}-{unique}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(path.join(CONTEXT_DIR)).unwrap();
        path
    }

    fn prompt(session: &str, text: &str) -> Value {
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": session,
            "cwd": "F:/project/中文",
            "prompt": text,
        })
    }

    fn stop(session: &str, text: &str) -> Value {
        serde_json::json!({
            "hook_event_name": "Stop",
            "session_id": session,
            "cwd": "F:/project/中文",
            "last_assistant_message": text,
        })
    }

    fn saved(root: &Path, agent: &str, payload: &Value) -> Record {
        capture(root, agent, payload).unwrap().unwrap()
    }

    #[test]
    fn records_roles_agents_and_full_multiline_text_in_save_order() {
        let root = project();
        let original = "  主人原文\n```rust\nlet x = 1;\n```\n\n末尾\n";
        let first = saved(&root, "codex", &prompt("codex-session", original));
        let second = saved(&root, "claude", &prompt("claude-session", "继续"));
        let third = saved(&root, "codex", &stop("codex-session", "完成\n\n检查通过"));
        let fourth = saved(&root, "claude", &stop("claude-session", "接着工作"));
        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
        assert_eq!(third.reply_to, Some(1));
        assert_eq!(fourth.reply_to, Some(2));
        assert_eq!(first.role, "user");
        assert_eq!(third.role, "assistant");
        assert!(first.timestamp_ms > 0);
        let records = read(&root, None, None, 20).unwrap();
        assert_eq!(records, vec![first, second, third, fourth]);
        assert_eq!(records[0].text, original);
        let markdown = render_markdown(&records);
        assert!(markdown.contains("## #0001 user · codex"));
        assert!(markdown.contains("## #0004 assistant · claude"));
        assert!(markdown.contains(&format!("````text\n{original}````\n")));
        assert!(markdown.contains("- reply_to: #0001"));
    }

    #[test]
    fn repeated_real_messages_without_stable_identity_are_preserved() {
        let root = project();
        for agent in ["claude", "codex"] {
            let repeated = prompt("session", "继续");
            saved(&root, agent, &repeated);
            saved(&root, agent, &repeated);
            let repeated = stop("session", "已完成");
            saved(&root, agent, &repeated);
            saved(&root, agent, &repeated);
        }
        assert_eq!(read(&root, None, None, 20).unwrap().len(), 8);
    }

    #[test]
    fn codex_stable_turn_is_idempotent_but_preserves_continuations() {
        let root = project();
        let mut user = prompt("session", "开始");
        user["turn_id"] = Value::String("turn-1".to_owned());
        saved(&root, "codex", &user);
        assert!(capture(&root, "codex", &user).unwrap().is_none());
        let mut reply = stop("session", "第一段");
        reply["turn_id"] = Value::String("turn-1".to_owned());
        assert_eq!(saved(&root, "codex", &reply).reply_to, Some(1));
        assert!(capture(&root, "codex", &reply).unwrap().is_none());
        reply["last_assistant_message"] = Value::String("第二段".to_owned());
        assert_eq!(saved(&root, "codex", &reply).seq, 3);
        reply["turn_id"] = Value::String("turn-2".to_owned());
        assert_eq!(saved(&root, "codex", &reply).seq, 4);
    }

    #[test]
    fn stable_message_ids_deduplicate_claude_without_dropping_distinct_messages() {
        let root = project();
        let mut user = prompt("session", "继续");
        user["message_id"] = Value::String("message-1".to_owned());
        saved(&root, "claude", &user);
        assert!(capture(&root, "claude", &user).unwrap().is_none());
        user["message_id"] = Value::String("message-2".to_owned());
        assert_eq!(saved(&root, "claude", &user).seq, 2);
        user["prompt"] = Value::String("继续补充".to_owned());
        assert_eq!(saved(&root, "claude", &user).seq, 3);
    }

    #[test]
    fn claude_prompt_ids_deduplicate_and_link_late_replies_to_their_prompt() {
        let root = project();
        let first_id = "550e8400-e29b-41d4-a716-446655440000";
        let second_id = "550e8400-e29b-41d4-a716-446655440001";
        let mut first = prompt("session", "继续");
        first["prompt_id"] = Value::String(first_id.to_owned());
        let first_record = saved(&root, "claude", &first);
        assert_eq!(first_record.turn_id.as_deref(), Some(first_id));

        let mut second = first.clone();
        second["prompt_id"] = Value::String(second_id.to_owned());
        let second_record = saved(&root, "claude", &second);
        assert_eq!(second_record.seq, 2);
        assert_eq!(second_record.text, first_record.text);
        assert_eq!(second_record.turn_id.as_deref(), Some(second_id));
        assert!(capture(&root, "claude", &first).unwrap().is_none());

        let mut reply = stop("session", "第一问的迟到回复");
        reply["prompt_id"] = Value::String(first_id.to_owned());
        let reply_record = saved(&root, "claude", &reply);
        assert_eq!(reply_record.seq, 3);
        assert_eq!(reply_record.reply_to, Some(1));
        assert!(capture(&root, "claude", &reply).unwrap().is_none());
        reply["last_assistant_message"] = Value::String("第一问的补充回复".to_owned());
        assert_eq!(saved(&root, "claude", &reply).reply_to, Some(1));
        assert_eq!(read(&root, None, None, 10).unwrap().len(), 4);
    }

    #[test]
    fn claude_without_prompt_or_message_id_keeps_repeats_and_latest_reply() {
        let root = project();
        let user = prompt("session", "继续");
        saved(&root, "claude", &user);
        assert_eq!(saved(&root, "claude", &user).seq, 2);
        let reply = stop("session", "已完成");
        assert_eq!(saved(&root, "claude", &reply).reply_to, Some(2));
        assert_eq!(saved(&root, "claude", &reply).reply_to, Some(2));
        assert_eq!(read(&root, None, None, 10).unwrap().len(), 4);
    }

    #[test]
    fn codex_uses_turn_id_and_ignores_claude_prompt_identity() {
        let root = project();
        let mut user = prompt("session", "继续");
        user["prompt_id"] = Value::String("claude-prompt-1".to_owned());
        assert_eq!(saved(&root, "codex", &user).turn_id, None);
        assert_eq!(saved(&root, "codex", &user).seq, 2);
        user["turn_id"] = Value::String("codex-turn".to_owned());
        assert_eq!(saved(&root, "codex", &user).seq, 3);
        user["prompt_id"] = Value::String("claude-prompt-2".to_owned());
        assert!(capture(&root, "codex", &user).unwrap().is_none());
    }

    #[test]
    fn late_replies_use_matching_turn_and_never_cross_agent_or_session() {
        let root = project();
        let mut first = prompt("session", "第一问");
        first["turn_id"] = Value::String("turn-1".to_owned());
        saved(&root, "codex", &first);
        let mut second = prompt("session", "第二问");
        second["turn_id"] = Value::String("turn-2".to_owned());
        saved(&root, "codex", &second);
        saved(&root, "codex", &prompt("other-session", "另一会话"));
        saved(&root, "claude", &prompt("session", "另一 agent"));
        let mut reply = stop("session", "迟到第一答");
        reply["turn_id"] = Value::String("turn-1".to_owned());
        assert_eq!(saved(&root, "codex", &reply).reply_to, Some(1));
        reply["turn_id"] = Value::String("unknown-turn".to_owned());
        assert_eq!(saved(&root, "codex", &reply).reply_to, None);
        assert_eq!(
            saved(&root, "claude", &stop("missing-session", "无关联")).reply_to,
            None
        );
        assert_eq!(
            saved(&root, "claude", &stop("session", "有明确关联")).reply_to,
            Some(4)
        );
    }

    #[test]
    fn turn_reply_can_link_to_a_prompt_without_turn_identity() {
        let root = project();
        saved(&root, "codex", &prompt("session", "开始"));
        let mut reply = stop("session", "继续");
        reply["turn_id"] = Value::String("turn-1".to_owned());
        assert_eq!(saved(&root, "codex", &reply).reply_to, Some(1));
    }

    #[test]
    fn reopening_ledger_retains_numbering_and_dedup_identity() {
        let root = project();
        let mut user = prompt("session", "一");
        user["turn_id"] = Value::String("turn-1".to_owned());
        saved(&root, "codex", &user);
        let root = root.canonicalize().unwrap();
        assert!(capture(&root, "codex", &user).unwrap().is_none());
        assert_eq!(saved(&root, "claude", &prompt("session", "二")).seq, 2);
    }

    #[test]
    fn concurrent_separate_opens_serialize_global_sequence() {
        let root = Arc::new(project());
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|worker| {
                let root = Arc::clone(&root);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for index in 0..5 {
                        saved(
                            &root,
                            "claude",
                            &prompt(&format!("worker-{worker}"), &format!("message-{index}")),
                        );
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let records = read(&root, None, None, 100).unwrap();
        assert_eq!(records.len(), 40);
        assert_eq!(
            records.iter().map(|record| record.seq).collect::<Vec<_>>(),
            (1..=40).collect::<Vec<_>>()
        );
    }

    #[test]
    fn concurrent_duplicate_codex_events_write_once() {
        let root = Arc::new(project());
        let barrier = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let root = Arc::clone(&root);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let mut user = prompt("session", "一次");
                    user["turn_id"] = Value::String("turn-1".to_owned());
                    barrier.wait();
                    capture(&root, "codex", &user).unwrap()
                })
            })
            .collect();
        let appended = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(Option::is_some)
            .count();
        assert_eq!(appended, 1);
        assert_eq!(read(&root, None, None, 10).unwrap().len(), 1);
    }

    #[test]
    fn busy_ledger_bounds_read_and_write_waits_without_changing_records() {
        let root = project();
        saved(&root, "claude", &prompt("session", "已保存原文"));
        let path = root.join(CONTEXT_DIR).join(HISTORY_FILE);
        let original = std::fs::read(&path).unwrap();
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        owner.lock().unwrap();

        let timeout = Duration::from_millis(5);
        let started = Instant::now();
        let error = read_with_timeout(&root, None, None, 10, timeout).unwrap_err();
        assert!(error.contains("busy"));
        assert!(started.elapsed() >= timeout);
        let started = Instant::now();
        let error = capture_with_timeout(
            &root,
            "claude",
            &prompt("session", "private conversation"),
            timeout,
        )
        .unwrap_err();
        assert!(error.contains("busy"));
        assert!(!error.contains("private conversation"));
        assert!(started.elapsed() >= timeout);

        owner.unlock().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(read(&root, None, None, 10).unwrap().len(), 1);
        assert_eq!(
            saved(&root, "claude", &prompt("session", "解锁后继续")).seq,
            2
        );
    }

    #[test]
    fn projects_have_independent_sequences_and_records() {
        let first = project();
        let second = project();
        saved(&first, "codex", &prompt("session", "项目一"));
        saved(&first, "codex", &prompt("session", "项目一续"));
        assert_eq!(saved(&second, "codex", &prompt("session", "项目二")).seq, 1);
        assert_eq!(read(&first, None, None, 10).unwrap().len(), 2);
        assert_eq!(read(&second, None, None, 10).unwrap()[0].text, "项目二");
    }

    #[test]
    fn range_is_inclusive_and_limit_returns_latest_matching_records_in_order() {
        let root = project();
        for number in 1..=6 {
            saved(&root, "claude", &prompt("session", &number.to_string()));
        }
        let records = read(&root, Some(2), Some(5), 2).unwrap();
        assert_eq!(
            records.iter().map(|record| record.seq).collect::<Vec<_>>(),
            vec![4, 5]
        );
        assert_eq!(read(&root, None, Some(3), 1).unwrap()[0].seq, 3);
        assert!(read(&root, Some(20), None, 2).unwrap().is_empty());
        assert!(read(&root, None, None, 0).is_err());
        assert!(read(&root, Some(5), Some(2), 2).is_err());
    }

    #[test]
    fn read_does_not_create_missing_ledger_or_project_directory() {
        let root = project();
        assert!(read(&root, None, None, 10).unwrap().is_empty());
        assert!(!root.join(CONTEXT_DIR).join(HISTORY_FILE).exists());
        let missing = root.join("missing-project");
        assert!(read(&missing, None, None, 10).unwrap().is_empty());
        assert!(!missing.exists());
        assert!(capture(&missing, "codex", &prompt("session", "不要启用")).is_err());
        assert!(!missing.exists());
    }

    #[test]
    fn invalid_tail_is_reported_without_overwriting_or_leaking_message_text() {
        for tail in ["{\"text\":\"private conversation", "not-json\n", "\n"] {
            let root = project();
            saved(&root, "claude", &prompt("session", "已保存原文"));
            let path = root.join(CONTEXT_DIR).join(HISTORY_FILE);
            OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(tail.as_bytes())
                .unwrap();
            let original = std::fs::read(&path).unwrap();
            let error = read(&root, None, None, 10).unwrap_err();
            assert!(error.contains("line 2"));
            assert!(!error.contains("private conversation"));
            assert!(capture(&root, "claude", &prompt("session", "不覆盖")).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
    }

    #[test]
    fn noncontiguous_sequence_and_missing_final_newline_are_rejected() {
        let root = project();
        let mut record = saved(&root, "codex", &prompt("session", "原文"));
        record.seq = 3;
        let path = root.join(CONTEXT_DIR).join(HISTORY_FILE);
        let encoded = serde_json::to_string(&record).unwrap();
        std::fs::write(&path, format!("{encoded}\n")).unwrap();
        assert!(read(&root, None, None, 10).is_err());
        record.seq = 1;
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(capture(&root, "codex", &prompt("session", "不拼接")).is_err());
    }

    #[test]
    fn missing_final_output_and_subagents_never_create_fake_records() {
        let root = project();
        let missing =
            serde_json::json!({"hook_event_name":"Stop","session_id":"session","cwd":"project"});
        assert!(capture(&root, "codex", &missing).unwrap().is_none());
        for text in ["", "  \r\n  "] {
            assert!(capture(&root, "claude", &stop("session", text))
                .unwrap()
                .is_none());
        }
        let mut child = prompt("session", "子任务");
        child["agent_id"] = Value::String("subagent-1".to_owned());
        assert!(capture(&root, "claude", &child).unwrap().is_none());
        let unknown = serde_json::json!({"hook_event_name":"PreToolUse"});
        assert!(capture(&root, "codex", &unknown).unwrap().is_none());
        assert!(!root.join(CONTEXT_DIR).join(HISTORY_FILE).exists());
    }

    #[test]
    fn invalid_hook_fields_fail_before_creating_ledger() {
        let root = project();
        let mut payload = prompt("session", "私密正文");
        payload["session_id"] = Value::Null;
        assert!(!capture(&root, "codex", &payload)
            .unwrap_err()
            .contains("私密正文"));
        let mut payload = prompt("session", "原文");
        payload["cwd"] = Value::String(" ".to_owned());
        assert!(capture(&root, "codex", &payload).is_err());
        let mut payload = prompt("session", "原文");
        payload["prompt"] = serde_json::json!(["不能强制转换"]);
        assert!(capture(&root, "codex", &payload).is_err());
        let mut payload = stop("session", "原文");
        payload["last_assistant_message"] = Value::Bool(true);
        assert!(capture(&root, "claude", &payload).is_err());
        assert!(capture(&root, "other", &prompt("session", "原文")).is_err());
        assert!(!root.join(CONTEXT_DIR).join(HISTORY_FILE).exists());
    }
}
