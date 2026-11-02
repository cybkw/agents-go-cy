// Modified for Agents Go by Agents Go contributors, 2026-11-02. See NOTICE.
//! Import only a selected native rollout; account configuration stays in its current home.
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const MAX_ROLLOUT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SCAN_ENTRIES: usize = 100_000;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub(crate) struct ImportLocations {
    pub target_home: PathBuf,
    pub user_home: PathBuf,
}

pub(crate) fn is_native_session_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

impl ImportLocations {
    pub(crate) fn from_environment() -> Result<Self, String> {
        let user_home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or("无法确定当前用户目录。")?;
        if !user_home.is_absolute() {
            return Err("当前用户目录必须是绝对路径。".into());
        }
        let target_home = std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| user_home.join(".codex"));
        let target_home = if target_home.is_absolute() {
            target_home
        } else {
            std::env::current_dir()
                .map_err(|error| format!("无法确定当前 Codex 目录：{error}"))?
                .join(target_home)
        };
        Ok(Self {
            target_home,
            user_home,
        })
    }

    /// All directory inspection and copying belongs to the caller's background worker.
    pub(crate) fn prepare(
        &self,
        session_id: &str,
        cwd: &Path,
        transcript_path: Option<&Path>,
    ) -> Result<(), String> {
        if !is_native_session_id(session_id)
            || !cwd.is_absolute()
            || !self.target_home.is_absolute()
        {
            return Err("无法导入：会话 ID 或项目目录无效。".into());
        }
        reject_links(&self.target_home)?;
        let mut budget = MAX_SCAN_ENTRIES;
        let existing = find_rollouts(&self.target_home.join("sessions"), session_id, &mut budget)?;
        if !existing.is_empty() {
            // Never replace the destination account's version, even if a source has grown.
            for path in existing {
                read_rollout(&path, session_id)?;
            }
            return Ok(());
        }

        let mut sources = Vec::new();
        if let Some(path) = transcript_path {
            if !path.is_absolute() || !rollout_name_matches(path, session_id) {
                return Err("无法导入：记录中的原生会话路径无效。".into());
            }
            match fs::symlink_metadata(path) {
                Ok(_) => sources.push(path.to_owned()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("无法读取原生会话：{error}")),
            }
        }
        // A captured path also supports custom homes. Legacy records fall back to the
        // user's conventional account homes; never search arbitrary project trees.
        if sources.is_empty() {
            let mut homes = vec![self.target_home.clone()];
            reject_links(&self.user_home)?;
            for entry in fs::read_dir(&self.user_home)
                .map_err(|error| format!("无法查找 Codex 账号目录：{error}"))?
            {
                consume_budget(&mut budget)?;
                let entry = entry.map_err(|error| format!("无法读取账号目录：{error}"))?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if (name == ".codex" || name.starts_with(".codex-"))
                    && entry
                        .file_type()
                        .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                    && !homes.contains(&entry.path())
                {
                    homes.push(entry.path());
                }
            }
            for home in homes {
                for store in ["sessions", "archived_sessions"] {
                    for path in find_rollouts(&home.join(store), session_id, &mut budget)? {
                        if !sources.contains(&path) {
                            sources.push(path);
                        }
                    }
                }
            }
        }
        let mut selected: Option<(PathBuf, Vec<u8>)> = None;
        for source in sources {
            let bytes = read_rollout(&source, session_id)?;
            if let Some((_, previous)) = &selected {
                if previous != &bytes {
                    return Err(
                        "多个账号中存在内容不同的同 ID 会话，请明确指定原生会话来源。".into(),
                    );
                }
            } else {
                selected = Some((source, bytes));
            }
        }
        let (source, bytes) = selected.ok_or("未找到原生会话文件，请确认记录来源仍保留在本机。")?;
        let relative = rollout_relative_path(&source)?;
        let destination = self.target_home.join("sessions").join(relative);
        let destination_dir = destination.parent().ok_or("原生会话文件名无效。")?;
        reject_links(destination_dir)?;
        fs::create_dir_all(destination_dir)
            .map_err(|error| format!("无法创建当前账号会话目录：{error}"))?;
        reject_links(destination_dir)?;
        let _lock = lock_destination(&self.target_home)?;
        // Two clients can finish source discovery together. Recheck while holding
        // the per-account import lock, including differently named copies of one ID.
        let mut budget = MAX_SCAN_ENTRIES;
        let existing = find_rollouts(&self.target_home.join("sessions"), session_id, &mut budget)?;
        if !existing.is_empty() {
            for path in existing {
                read_rollout(&path, session_id)?;
            }
            return Ok(());
        }
        let temporary = destination_dir.join(format!(
            ".agents-go-import-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("无法准备会话副本：{error}"))?;
        let result = (|| {
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("无法写入会话副本：{error}"))?;
            // Link only our completed independent copy. Publication cannot clobber a
            // concurrent import, and Codex never observes a partly copied JSONL file.
            match fs::hard_link(&temporary, &destination) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    read_rollout(&destination, session_id).map(|_| ())
                }
                Err(error) => Err(format!("无法安装会话副本：{error}")),
            }
        })();
        drop(file);
        let _ = fs::remove_file(&temporary);
        result
    }
}

fn lock_destination(home: &Path) -> Result<fs::File, String> {
    let path = home.join(".agents-go-native-import.lock");
    reject_links(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| format!("无法锁定会话导入：{error}"))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(fs::TryLockError::WouldBlock) => {
                return Err("另一个会话导入正在进行，请稍后重试。".into())
            }
            Err(fs::TryLockError::Error(error)) => {
                return Err(format!("无法锁定会话导入：{error}"))
            }
        }
    }
}

fn consume_budget(budget: &mut usize) -> Result<(), String> {
    *budget = budget
        .checked_sub(1)
        .ok_or("会话目录条目过多，请明确指定原生会话来源。")?;
    Ok(())
}

fn rollout_name_matches(path: &Path, session_id: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with("rollout-") && name.ends_with(&format!("-{session_id}.jsonl"))
        })
}

fn rollout_relative_path(path: &Path) -> Result<&Path, String> {
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("原生会话路径包含上级目录跳转。".into());
    }
    let store = rollout_store_root(path)?;
    let relative = path.strip_prefix(store).map_err(|_| "原生会话路径无效。")?;
    let components: Vec<_> = relative.components().collect();
    if components.len() > 4
        || components[..components.len().saturating_sub(1)]
            .iter()
            .any(|part| {
                let name = part.as_os_str().to_string_lossy();
                !matches!(name.len(), 2 | 4) || !name.bytes().all(|byte| byte.is_ascii_digit())
            })
    {
        return Err("原生会话日期目录无效。".into());
    }
    Ok(relative)
}

fn rollout_store_root(path: &Path) -> Result<&Path, String> {
    path.ancestors()
        .skip(1)
        .find(|ancestor| {
            ancestor
                .file_name()
                .is_some_and(|name| name == "sessions" || name == "archived_sessions")
        })
        .ok_or_else(|| "原生会话不在受支持的 sessions 目录中。".into())
}

fn find_rollouts(
    root: &Path,
    session_id: &str,
    budget: &mut usize,
) -> Result<Vec<PathBuf>, String> {
    let resolved_root = match fs::canonicalize(root) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("无法解析会话存储目录：{error}")),
    };
    let mut found = Vec::new();
    // Account stores may deliberately be junctions to a shared sessions root.
    // Resolve that root once, but never traverse links inside its dated tree.
    let mut pending = vec![(resolved_root, PathBuf::new(), 0)];
    while let Some((directory, relative, depth)) = pending.pop() {
        match fs::symlink_metadata(&directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("无法检查会话目录：{error}")),
            Ok(metadata) if metadata.file_type().is_symlink() => continue,
            Ok(metadata) if !metadata.is_dir() => return Err("会话存储路径不是目录。".into()),
            Ok(_) => {}
        }
        for entry in
            fs::read_dir(directory).map_err(|error| format!("无法读取会话目录：{error}"))?
        {
            consume_budget(budget)?;
            let entry = entry.map_err(|error| format!("无法读取会话文件：{error}"))?;
            let kind = entry
                .file_type()
                .map_err(|error| format!("无法检查会话文件：{error}"))?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_file() && rollout_name_matches(&entry.path(), session_id) {
                found.push(root.join(&relative).join(entry.file_name()));
            } else if kind.is_dir() && depth < 3 {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if matches!(name.len(), 2 | 4) && name.bytes().all(|byte| byte.is_ascii_digit()) {
                    pending.push((entry.path(), relative.join(entry.file_name()), depth + 1));
                }
            }
        }
    }
    Ok(found)
}

fn reject_links(path: &Path) -> Result<(), String> {
    for component in path.ancestors() {
        match fs::symlink_metadata(component) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("会话路径包含符号链接，无法安全导入。".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("无法检查会话路径：{error}")),
        }
    }
    Ok(())
}

fn read_rollout(path: &Path, session_id: &str) -> Result<Vec<u8>, String> {
    let relative = rollout_relative_path(path)?;
    let mut resolved = rollout_store_root(path)?
        .canonicalize()
        .map_err(|error| format!("无法解析会话存储目录：{error}"))?;
    for component in relative.components() {
        resolved.push(component);
        let metadata = fs::symlink_metadata(&resolved)
            .map_err(|error| format!("无法检查原生会话：{error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("会话存储根目录以下包含符号链接，无法安全导入。".into());
        }
    }
    let path = resolved.as_path();
    let metadata = fs::metadata(path).map_err(|error| format!("无法读取原生会话：{error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_ROLLOUT_BYTES {
        return Err("原生会话文件无效或超过 128 MiB，请手动处理。".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_ROLLOUT_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("无法读取原生会话：{error}"))?;
    let after = fs::metadata(path).map_err(|error| format!("无法复核原生会话：{error}"))?;
    if metadata.len() != after.len()
        || bytes.len() as u64 != metadata.len()
        || metadata.modified().ok() != after.modified().ok()
    {
        return Err("原生会话仍在写入，请等待原会话写入完成后重试。".into());
    }
    if bytes.len() as u64 > MAX_ROLLOUT_BYTES || bytes.last() != Some(&b'\n') {
        return Err("原生会话文件不完整，请等待原会话写入完成后重试。".into());
    }
    let mut lines = bytes.split(|byte| *byte == b'\n');
    let first: Value = serde_json::from_slice(lines.next().unwrap_or_default())
        .map_err(|_| "原生会话元数据损坏。")?;
    let saved_cwd = first
        .pointer("/payload/cwd")
        .and_then(Value::as_str)
        .ok_or("原生会话缺少项目目录。")?;
    if first.get("type").and_then(Value::as_str) != Some("session_meta")
        || first.pointer("/payload/id").and_then(Value::as_str) != Some(session_id)
        || !Path::new(saved_cwd).is_absolute()
    {
        return Err("原生会话的 ID 不匹配或项目目录无效。".into());
    }
    for line in lines.filter(|line| !line.is_empty()) {
        let value: Value =
            serde_json::from_slice(line).map_err(|_| "原生会话正文损坏，无法导入。")?;
        if !value.is_object() || value.get("type").and_then(Value::as_str).is_none() {
            return Err("原生会话正文格式无效。".into());
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01a18646-aa12-7950-be94-751538d844ef";

    struct Fixture {
        root: PathBuf,
        locations: ImportLocations,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "agents-go-native-import-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("project")).unwrap();
            let locations = ImportLocations {
                target_home: root.join(".codex"),
                user_home: root.clone(),
            };
            fs::create_dir_all(&locations.target_home).unwrap();
            fs::write(locations.target_home.join("auth.json"), b"unchanged-auth").unwrap();
            fs::write(
                locations.target_home.join("config.toml"),
                b"unchanged-config",
            )
            .unwrap();
            Self { root, locations }
        }

        fn cwd(&self) -> PathBuf {
            self.root.join("project")
        }

        fn content(&self, body: &str) -> Vec<u8> {
            format!(
                "{}\n{}\n",
                serde_json::json!({"type":"session_meta", "payload":{"id":ID,"cwd":self.cwd()}}),
                serde_json::json!({"type":"response_item","payload":{"text":body}})
            )
            .into_bytes()
        }

        fn write(&self, home: &str, store: &str, body: &str) -> PathBuf {
            let path = self
                .root
                .join(home)
                .join(store)
                .join("2026/10/29")
                .join(format!("rollout-2026-10-29T14-09-00-{ID}.jsonl"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, self.content(body)).unwrap();
            path
        }

        fn prepare(&self) -> Result<(), String> {
            self.locations.prepare(ID, &self.cwd(), None)
        }

        fn destinations(&self) -> Vec<PathBuf> {
            let mut budget = MAX_SCAN_ENTRIES;
            find_rollouts(
                &self.locations.target_home.join("sessions"),
                ID,
                &mut budget,
            )
            .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn native_import_copies_history_without_source_or_account_changes() {
        let f = Fixture::new();
        let source = f.write(".codex-account-b", "sessions", "original text");
        let bytes = fs::read(&source).unwrap();
        f.prepare().unwrap();
        let target = f.destinations().pop().unwrap();
        assert_eq!(fs::read(&target).unwrap(), bytes);
        fs::write(&target, f.content("new current-account progress")).unwrap();
        assert_eq!(
            fs::read(&source).unwrap(),
            bytes,
            "must be an independent copy"
        );
        assert_eq!(
            fs::read(f.locations.target_home.join("auth.json")).unwrap(),
            b"unchanged-auth"
        );
        assert_eq!(
            fs::read(f.locations.target_home.join("config.toml")).unwrap(),
            b"unchanged-config"
        );
    }

    #[test]
    fn native_import_existing_target_wins_and_repeated_import_is_idempotent() {
        let f = Fixture::new();
        f.write(".codex-account-b", "sessions", "original");
        f.prepare().unwrap();
        let target = f.destinations().pop().unwrap();
        fs::write(&target, f.content("continued in current account")).unwrap();
        f.prepare().unwrap();
        assert_eq!(f.destinations().len(), 1);
        assert_eq!(
            fs::read(&target).unwrap(),
            f.content("continued in current account")
        );
    }

    #[test]
    fn native_import_parallel_clients_publish_one_complete_independent_copy() {
        let f = Fixture::new();
        let source = f.write(".codex-account-b", "sessions", "body");
        std::thread::scope(|scope| {
            let a = scope.spawn(|| f.prepare());
            let b = scope.spawn(|| f.prepare());
            a.join().unwrap().unwrap();
            b.join().unwrap().unwrap();
        });
        assert_eq!(f.destinations().len(), 1);
        assert_eq!(
            fs::read(f.destinations().pop().unwrap()).unwrap(),
            fs::read(source).unwrap()
        );
    }

    #[test]
    fn native_import_rejects_divergent_same_id_sources_but_accepts_identical_copies() {
        let f = Fixture::new();
        f.write(".codex-account-b", "sessions", "same");
        let other = f.write(".codex-account-c", "sessions", "different");
        assert!(f.prepare().unwrap_err().contains("内容不同"));
        assert!(f.destinations().is_empty());
        fs::write(other, f.content("same")).unwrap();
        f.prepare().unwrap();
        assert_eq!(f.destinations().len(), 1);
    }

    #[test]
    fn native_import_reads_archive_and_explicit_custom_home() {
        let f = Fixture::new();
        let source = f.write("custom-home", "archived_sessions", "custom");
        assert!(f.prepare().is_err());
        f.locations.prepare(ID, &f.cwd(), Some(&source)).unwrap();
        assert_eq!(
            fs::read(f.destinations().pop().unwrap()).unwrap(),
            f.content("custom")
        );
        let archived = Fixture::new();
        archived.write(".codex-account-b", "archived_sessions", "archived");
        archived.prepare().unwrap();
    }

    #[test]
    fn native_import_explicit_source_disambiguates_different_account_copies() {
        let f = Fixture::new();
        let source = f.write(".codex-account-b", "sessions", "selected source");
        f.write(".codex-account-c", "sessions", "different copy");
        f.locations.prepare(ID, &f.cwd(), Some(&source)).unwrap();
        assert_eq!(
            fs::read(f.destinations().pop().unwrap()).unwrap(),
            f.content("selected source")
        );
    }

    #[test]
    fn native_import_rejects_invalid_json_torn_tail_id_and_relative_directory() {
        for mode in ["json", "tail", "id", "cwd"] {
            let f = Fixture::new();
            let source = f.write(".codex-account-b", "sessions", "body");
            let mut bytes = fs::read(&source).unwrap();
            match mode {
                "json" => bytes.extend_from_slice(b"not json\n"),
                "tail" => {
                    bytes.pop();
                }
                "id" => {
                    bytes = String::from_utf8(bytes)
                        .unwrap()
                        .replace(ID, "01a18646-aa12-7950-be94-751538d844e0")
                        .into_bytes();
                }
                "cwd" => {
                    bytes = format!(
                        "{}\n",
                        serde_json::json!({"type":"session_meta","payload":{"id":ID,"cwd":"relative"}})
                    )
                    .into_bytes();
                }
                _ => unreachable!(),
            }
            fs::write(&source, &bytes).unwrap();
            assert!(f.prepare().is_err(), "{mode}");
            assert!(f.destinations().is_empty());
            assert_eq!(fs::read(source).unwrap(), bytes);
        }
    }

    #[test]
    fn native_import_preserves_original_metadata_after_session_changed_directory() {
        let f = Fixture::new();
        let source = f.write(".codex-account-b", "sessions", "original");
        let before = fs::read(&source).unwrap();
        let changed = f.root.join("changed-project");
        fs::create_dir_all(&changed).unwrap();
        f.locations.prepare(ID, &changed, None).unwrap();
        assert_eq!(fs::read(f.destinations().pop().unwrap()).unwrap(), before);
    }

    #[test]
    fn native_import_rejects_unsafe_identifiers_and_non_rollout_hint() {
        let f = Fixture::new();
        for id in [
            "../session",
            "--help",
            "not-native-id",
            "01a18646-aa12-7950-be94-751538d844eg",
        ] {
            assert!(f.locations.prepare(id, &f.cwd(), None).is_err());
        }
        assert!(f
            .locations
            .prepare(
                ID,
                &f.cwd(),
                Some(&f.locations.target_home.join("auth.json"))
            )
            .is_err());
        assert!(f
            .locations
            .prepare(ID, &f.cwd(), Some(Path::new("relative.jsonl")))
            .is_err());
    }

    #[test]
    fn native_import_missing_source_and_scan_budget_do_not_create_history() {
        let f = Fixture::new();
        assert!(f.prepare().is_err());
        assert!(f.destinations().is_empty());
        f.write(".codex-account-b", "sessions", "text");
        assert!(find_rollouts(&f.root.join(".codex-account-b/sessions"), ID, &mut 0).is_err());
    }

    #[cfg(any(unix, windows))]
    fn link_store(link: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            // cmd's mklink treats forward slashes as switches, unlike Rust paths.
            let link: PathBuf = link.components().collect();
            let target: PathBuf = target.components().collect();
            let result = std::process::Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn native_import_reads_linked_store_root_but_refuses_nested_store_links() {
        let f = Fixture::new();
        let source = f.write("shared", "sessions", "body");
        let source_store = f.root.join("shared/sessions");
        let account = f.root.join(".codex-account-b");
        fs::create_dir_all(&account).unwrap();
        link_store(&account.join("sessions"), &source_store);
        f.prepare().unwrap();
        let alias = account
            .join("sessions")
            .join(source.strip_prefix(&source_store).unwrap());
        assert_eq!(
            read_rollout(&alias, ID).unwrap(),
            fs::read(&source).unwrap()
        );
        assert_eq!(
            fs::read(f.destinations().pop().unwrap()).unwrap(),
            fs::read(&source).unwrap()
        );

        let nested = Fixture::new();
        let source = nested.write("shared", "sessions", "body");
        let store = nested.root.join(".codex-account-b/sessions");
        fs::create_dir_all(store.join("2026")).unwrap();
        link_store(
            &store.join("2026/10"),
            &nested.root.join("shared/sessions/2026/10"),
        );
        assert!(nested.prepare().is_err());
        let alias = store.join("2026/10/29").join(source.file_name().unwrap());
        assert!(read_rollout(&alias, ID).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn native_import_publishes_transcript_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        f.write(".codex-account-b", "sessions", "private text");
        f.prepare().unwrap();
        let target = f.destinations().pop().unwrap();
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_import_refuses_symbolic_link_source() {
        let f = Fixture::new();
        let source = f.write("unsearched", "sessions", "body");
        let link = f.root.join(".codex-account-b");
        std::os::unix::fs::symlink(f.root.join("unsearched"), link).unwrap();
        assert!(f.prepare().is_err());
        assert!(f.locations.prepare(ID, &f.cwd(), Some(&source)).is_ok());
    }
}
