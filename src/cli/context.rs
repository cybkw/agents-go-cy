use std::io::Read;
use std::path::{Path, PathBuf};

use clap::ArgMatches;

use crate::project_context::{self, journal};

pub(super) fn run_context_command(args: &[String]) -> std::io::Result<i32> {
    let matches = match super::spec::context_command()
        .bin_name("herdr context")
        .try_get_matches_from(std::iter::once("context").chain(args.iter().map(String::as_str)))
    {
        Ok(matches) => matches,
        Err(error) => {
            let code = if error.use_stderr() { 2 } else { 0 };
            crate::platform::begin_cli_output();
            error.print()?;
            return Ok(code);
        }
    };
    match run(&matches) {
        Ok(Some(output)) => {
            println!("{output}");
            Ok(0)
        }
        Ok(None) => Ok(0),
        Err(error) => {
            eprintln!("project context: {error}");
            // Exit 1 reports a hook failure without the blocking meaning of exit 2.
            Ok(1)
        }
    }
}

fn run(matches: &ArgMatches) -> Result<Option<String>, String> {
    let (command, options) = matches.subcommand().ok_or("missing context command")?;
    match command {
        "enable" => {
            let directory = requested_directory(options)?;
            let root = directory
                .canonicalize()
                .map_err(|err| format!("cannot resolve project directory: {err}"))?;
            if !root.is_dir() {
                return Err("project path must be a directory".into());
            }
            let agents: Vec<String> = options
                .get_many::<String>("agent")
                .map(|values| values.cloned().collect())
                .unwrap_or_else(|| vec!["codex".into(), "claude".into()]);
            let executable = std::env::current_exe()
                .map_err(|err| format!("cannot resolve recorder executable: {err}"))?;
            let paths = project_context::enable(&root, &executable, &agents)?;
            let mut output = format!(
                "项目对话记录已启用：{}\n自动保存用户发言和每轮最终回复；上下文只在手动恢复时读取。\n",
                root.display()
            );
            for path in paths {
                output.push_str(&format!("\n{}", path.display()));
            }
            if agents.iter().any(|agent| agent == "codex") {
                output.push_str("\n\nCodex：重新打开项目会话，并通过 /hooks 查看、信任新配置；用 $project-resume 手动恢复。");
            }
            if agents.iter().any(|agent| agent == "claude") {
                output.push_str(
                    "\nClaude Code：重新打开项目会话并确认项目配置；用 /project-resume 手动恢复。",
                );
            }
            Ok(Some(output))
        }
        "capture" => {
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .map_err(|err| format!("cannot read hook input: {err}"))?;
            let payload: serde_json::Value = serde_json::from_str(&input)
                .map_err(|_| "hook input must be valid JSON".to_string())?;
            let agent = options
                .get_one::<String>("agent")
                .ok_or("capture requires --agent")?;
            let project = options.get_one::<String>("project").map(Path::new);
            project_context::capture(project, agent, &payload)?;
            // Native hooks must not add text or context to the agent's conversation.
            Ok(None)
        }
        "history" | "resume" => {
            let root = project_context::project_root(&requested_directory(options)?)?;
            let from = positive_number(options, "from")?;
            let through = positive_number(options, "through")?;
            if matches!((from, through), (Some(first), Some(last)) if first > last) {
                return Err("--from must not exceed --through".into());
            }
            let limit = number_as_usize(options, "limit")?;
            let records = journal::read(&root, from, through, limit)?;
            if command == "resume" {
                let budget = number_as_usize(options, "max-chars")?;
                Ok(Some(project_context::resume_text(&root, &records, budget)))
            } else if options.get_flag("json") {
                serde_json::to_string_pretty(&serde_json::json!({
                    "project": root,
                    "records": records,
                }))
                .map(Some)
                .map_err(|err| format!("cannot encode project history: {err}"))
            } else if records.is_empty() {
                Ok(Some("该项目暂无匹配的对话记录。".into()))
            } else {
                Ok(Some(journal::render_markdown(&records)))
            }
        }
        _ => Err("unknown context command".into()),
    }
}

fn requested_directory(options: &ArgMatches) -> Result<PathBuf, String> {
    match options.get_one::<String>("project") {
        Some(path) => Ok(PathBuf::from(path)),
        None => {
            std::env::current_dir().map_err(|err| format!("cannot read current directory: {err}"))
        }
    }
}

fn positive_number(options: &ArgMatches, name: &str) -> Result<Option<u64>, String> {
    options
        .get_one::<String>(name)
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| format!("--{name} must be a positive integer"))
        })
        .transpose()
}

fn number_as_usize(options: &ArgMatches, name: &str) -> Result<usize, String> {
    let value = positive_number(options, name)?.ok_or_else(|| format!("missing --{name}"))?;
    usize::try_from(value).map_err(|_| format!("--{name} is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str]) -> ArgMatches {
        super::super::spec::context_command()
            .try_get_matches_from(std::iter::once("context").chain(args.iter().copied()))
            .unwrap()
    }

    #[test]
    fn project_context_rejects_invalid_ranges_before_reading_messages() {
        let matches = options(&["history", "--from", "0"]);
        let (_, flags) = matches.subcommand().unwrap();
        assert!(positive_number(flags, "from").is_err());
        let matches = options(&["resume", "--max-chars", "wrong"]);
        let (_, flags) = matches.subcommand().unwrap();
        assert!(number_as_usize(flags, "max-chars").is_err());
        let matches = options(&["history", "--limit", "2", "--from", "3", "--through", "5"]);
        let (_, flags) = matches.subcommand().unwrap();
        assert_eq!(number_as_usize(flags, "limit").unwrap(), 2);
        assert_eq!(positive_number(flags, "from").unwrap(), Some(3));
        assert_eq!(positive_number(flags, "through").unwrap(), Some(5));
    }
}
