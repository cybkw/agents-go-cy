// Modified for Agents Go by Agents Go contributors, 2026-10-27. See NOTICE.
use std::path::{Path, PathBuf};

/// Install project-local capture without ever treating a home or volume as a project.
pub(crate) fn auto_enable(cwd: &Path, executable: &Path) -> Result<PathBuf, String> {
    let cwd = cwd
        .canonicalize()
        .map_err(|error| format!("无法解析项目目录：{error}"))?;
    validate_directory(&cwd)?;
    // Share marker validation and the nested Git/worktree boundary with capture
    // and history reads, so an invalid parent marker cannot create a second log.
    let root = super::resolve_root(&cwd, true)?;
    validate_directory(&root)?;
    super::enable(&root, executable, &["codex".into(), "claude".into()])?;
    Ok(root)
}

pub(super) fn validate_directory(directory: &Path) -> Result<(), String> {
    if directory.parent().is_none() {
        return Err("磁盘根目录不自动启用对话记录，请打开具体项目。".into());
    }
    // Windows Agent tools can use HOME or USERPROFILE. Protect both, including
    // aliases whose canonical path names the same directory.
    let homes = std::iter::once(crate::worktree::expand_tilde_path("~")).chain(
        ["HOME", "USERPROFILE"].into_iter().filter_map(|key| {
            std::env::var_os(key)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        }),
    );
    if homes
        .filter(|home| home.is_absolute())
        .filter_map(|home| home.canonicalize().ok())
        .any(|home| home == directory)
    {
        return Err("用户目录不自动启用对话记录，避免修改全局 Agent 配置。请打开具体项目。".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn directory() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "agents-go-context-automatic-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    }

    fn install(cwd: &Path) -> Result<PathBuf, String> {
        auto_enable(cwd, &std::env::current_exe().unwrap())
    }

    #[test]
    fn automatic_capture_uses_git_root_and_keeps_nested_worktree_independent() {
        let root = directory();
        let cwd = root.join("src/nested");
        fs::create_dir(root.join(".git")).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        assert_eq!(install(&cwd).unwrap(), root);
        assert!(!cwd.join(".agents-go").exists());
        let worktree = root.join("worktree");
        fs::create_dir_all(worktree.join("src")).unwrap();
        fs::write(worktree.join(".git"), "gitdir: elsewhere").unwrap();
        assert_eq!(install(&worktree.join("src")).unwrap(), worktree);
        assert!(worktree.join(".agents-go/project.json").is_file());
    }

    #[test]
    fn automatic_capture_enables_non_git_project_and_preserves_existing_data() {
        let root = directory();
        fs::create_dir(root.join(".codex")).unwrap();
        fs::write(root.join(".codex/hooks.json"),
            br#"{"custom":true,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo keep"}]}]}}"#).unwrap();
        assert_eq!(install(&root).unwrap(), root);
        let hooks = fs::read(root.join(".codex/hooks.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&hooks).unwrap();
        assert_eq!(parsed["custom"], true);
        assert_eq!(
            parsed["hooks"]["Stop"][0]["hooks"][0]["command"],
            "echo keep"
        );
        fs::write(root.join(".agents-go/history.jsonl"), "existing history\n").unwrap();
        assert_eq!(install(&root).unwrap(), root);
        assert_eq!(fs::read(root.join(".codex/hooks.json")).unwrap(), hooks);
        assert_eq!(
            fs::read_to_string(root.join(".agents-go/history.jsonl")).unwrap(),
            "existing history\n"
        );
        assert!(root.join(".claude/settings.local.json").is_file());
        fs::create_dir(root.join("assets")).unwrap();
        assert_eq!(install(&root.join("assets")).unwrap(), root);
        assert!(!root.join("assets/.agents-go").exists());
    }

    #[test]
    fn automatic_capture_fails_on_invalid_marker_without_installing_in_child() {
        let root = directory();
        fs::create_dir_all(root.join(".agents-go")).unwrap();
        fs::write(root.join(".agents-go/project.json"), "{broken").unwrap();
        let cwd = root.join("src");
        fs::create_dir(&cwd).unwrap();
        assert!(install(&cwd).unwrap_err().contains("invalid"));
        assert!(!cwd.join(".agents-go").exists());
        assert!(!root.join(".codex").exists());
    }

    #[test]
    fn automatic_capture_refuses_volume_and_home_before_writing() {
        let _environment = crate::integration::integration_env_lock();
        let cwd = directory();
        let volume = cwd.ancestors().last().unwrap();
        assert!(install(volume).unwrap_err().contains("根目录"));
        let home = crate::worktree::expand_tilde_path("~");
        if home.is_absolute() && home.is_dir() {
            assert!(install(&home).unwrap_err().contains("用户目录"));
        }
    }

    #[test]
    fn automatic_capture_completes_single_agent_configuration() {
        let root = directory();
        super::super::enable(&root, &std::env::current_exe().unwrap(), &["codex".into()]).unwrap();
        let hooks = fs::read(root.join(".codex/hooks.json")).unwrap();
        assert_eq!(install(&root).unwrap(), root);
        assert_eq!(fs::read(root.join(".codex/hooks.json")).unwrap(), hooks);
        assert!(root.join(".claude/settings.local.json").is_file());
    }
}
