<!-- Modified for Agents Go by Agents Go contributors, 2026-10-01. See NOTICE. -->
# Agents Go

A terminal workspace for AI coding agents, with a visual project and conversation tree.

Agents Go is a modified version of [Herdr](https://github.com/herdrdev/herdr), adding a project and terminal conversation tree, visible agent activity, recent-project recovery, and Git branch labels. The CLI and executable keep the name `herdr`; this repository is not an official Herdr release.

Automatic update checks, self-updates, and automatic remote-server downloads are disabled. This repository currently provides source code; inherited Herdr installers and release documentation describe the upstream product.

[中文说明](README.md)

## Build and develop

The repository pins its Rust toolchain in rust-toolchain.toml. Install Rust, Zig 0.16, Python, Bun, and just as required by the platform build.

On Windows:

    just build

Start the packaged Windows build in PowerShell:

    & .\target\release\herdr.exe --session agents-go

For a debug build, run `cargo build --locked` and start `& .\target\debug\herdr.exe --session agents-go`.

Windows is the manually verified platform for this fork. Other platform build paths are inherited from Herdr and have not been manually verified for these modifications.

Run tests and checks:

    just test
    just check

The Windows build recipe packages the required ConPTY runtime. A successful build does not replace manual verification of terminal rendering and interaction.

## Short command

The executable is named `herdr`. To launch Agents Go with a short command, add an alias to your shell profile.

PowerShell (`$PROFILE`):

    function ag { herdr --session agents-go @args }

Bash or zsh (`~/.bashrc` or `~/.zshrc`):

    alias ag='herdr --session agents-go'

Note: `ag` is also the command for The Silver Searcher. If you already use it, pick another name such as `ago`.

## Project conversations and manual context restore

Run `herdr context enable --project .` once at the root of a project using the newly built executable. If it is not on PATH, use its absolute path, such as `.\target\debug\herdr.exe` when configuring this checkout. Both Codex and Claude Code are configured by default; repeat `--agent codex` or `--agent claude` to select agents. Setup merges project-local hooks and installs manual restore skills, preserving existing settings and leaving global Agent configuration untouched. Reopen the Agent session; review and trust Codex hooks through `/hooks`, and confirm the Claude project configuration.

Windows is the initial validation platform. Claude Code 2.1.139 or newer is required for direct execution arguments; Codex must support `UserPromptSubmit`, `Stop`, and `commandWindows`. Native hooks append user submissions and final assistant replies to `.agents-go/history.jsonl`, with durable project-wide sequence numbers, timestamps, Agent/session identity, and reply references. Projects remain isolated; nested Git projects and worktrees require their own setup. Internal reasoning and full tool traces are outside this log. There is no automatic history injection or handoff.

Explicitly invoke `$project-resume` in Codex or `/project-resume` in Claude Code to read recent history and summarize progress and pending work. Historical requests are background information, not new execution authorization. Include a current instruction to continue when that is what you want. Display limits never shorten the stored original. Read complete entries with `herdr context history --from 1 --through 20 --limit 20`, or use `--json` for structured output. `herdr context resume` prints bounded context. Read commands work without a workspace server and never send prompts or change history.

Recording starts when the hooks are active; existing conversations are not imported automatically. Stable turn or message IDs support retry deduplication. Claude events without these IDs are retained separately, including genuine repeated prompts. Missing final replies are not invented. Write failures and malformed journals report errors while preserving existing data. Reads and writes currently scan the full journal; large long-lived histories have not been scale-tested.

## Known check limitations

The project owner manually verified directory changes, archiving after closing all conversations, reopening recent projects, and retaining recent-project entries after restart. The full automated suite has not completed on the development machine: its filesystem does not support the EFS test fixture, and two project-reopen tests that create real terminals did not finish. These limitations are recorded rather than claimed as passing checks.

## Upstream relationship

The source is derived from herdrdev/herdr and retains its Apache-2.0 license. The project aims to preserve useful upstream terminal and agent-runtime improvements while developing its own task-tree experience. Upstream changes are reviewed and integrated selectively; they are not synchronized automatically.

## License

Apache-2.0. See [LICENSE](LICENSE), [NOTICE](NOTICE), and the retained third-party license and notice files. Modified source files carry change notices; JSON files use adjacent `.license` notices.
