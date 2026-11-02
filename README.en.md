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

On Windows, double-click `start.cmd` to open the `agents-go` session using the same `target\release\herdr.exe` produced by `just build`. If the server is stopped, the executable starts its own release server and restores the session. No debug build is needed. Missing executables and startup failures leave a visible message. Run `.\start.cmd --version` to check the version.

To run `herdr` directly, add this checkout's absolute `target\release` directory to PATH and confirm it with `Get-Command herdr`. `herdr --session agents-go` selects the same session as the double-click entry; plain `herdr` uses `default` when no session environment variable is set. Rebuild with `just build` after source changes. On Windows, stop servers using the release executable before rebuilding; stopping them ends their pane processes.

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

The local interactive workspace enables Codex and Claude Code recording in the background when a project opens, including projects restored after restart. Existing hooks and records are preserved; global Agent configuration is untouched. Git projects use their repository root, with nested repositories and worktrees configured independently. Other projects use their actual directory. Volume roots and the user's home directory are excluded; remote projects and read-only terminal observers never install local configuration.

After installation, reopen any already-running Agent session; review and trust Codex hooks through `/hooks`, and confirm the Claude project configuration. Installed configuration does not prove that an Agent has loaded or trusted it. Recording begins when the hooks are active, with no automatic backfill. The conversation-history window shows setup progress, the loading/trust reminder, or a configuration error.

Manual setup remains available: run `herdr context enable --project .` at the project root using the newly built executable. If it is not on PATH, use its absolute path, such as `.\target\release\herdr.exe` when configuring this checkout. The manual command accepts `--agent codex` or `--agent claude`; omitting these installs both. Restore skills and Claude configuration stay in the requested project. For linked Git worktrees, Codex hooks are merged into the main checkout's `.codex/hooks.json` and route events by their actual directory to each enabled worktree's own ledger. Worktrees without recording enabled are skipped. This does not enable the main checkout's ledger or change hook trust; existing legacy hook files in linked worktrees are preserved. If automatic setup fails, resolve the reported permissions/configuration issue and press R in the conversation-history window to retry, or retry with this command.

Windows is the initial validation platform. Claude Code 2.1.139 or newer is required for direct execution arguments; Codex must support `UserPromptSubmit`, `Stop`, and `commandWindows`. Native hooks append user submissions and final assistant replies to `.agents-go/history.jsonl`, with durable project-wide sequence numbers, timestamps, Agent/session identity, and reply references. Projects remain isolated. Internal reasoning and full tool traces are outside this log. There is no automatic history injection or handoff.

Explicitly invoke `$project-resume` in Codex or `/project-resume` in Claude Code to read recent history and summarize progress and pending work. Historical requests are background information, not new execution authorization. Include a current instruction to continue when that is what you want. Display limits never shorten the stored original. Read complete entries with `herdr context history --from 1 --through 20 --limit 20`, or use `--json` for structured output. `herdr context resume` prints bounded context. Read commands work without a workspace server and never send prompts or change history.

Recording starts when the hooks are active; existing conversations are not imported automatically. Stable turn or message IDs support retry deduplication. Claude events without these IDs are retained separately, including genuine repeated prompts. Missing final replies are not invented. Write failures and malformed journals report errors while preserving existing data. Reads and writes currently scan the full journal; large long-lived histories have not been scale-tested.

In the updated workspace, right-click a local project or recent project in the sidebar and choose **对话记录** (conversation history). The window loads the newest 100 records first, showing sequence, role, Agent, UTC time, and a summary beside the selected record's full text. Click or use the arrow keys to select, use Tab or Left/Right to switch between the list and body, and scroll the body with the wheel or PageUp/PageDown. R refreshes, N or the older-records button loads earlier entries, and Esc closes. Narrow windows show the list first; Enter opens the body and Esc returns to the list. Loading runs in the background. Browsing and refreshing send nothing to Agents.

Select a record and click **C 继续对话** (continue conversation) or press C to create and focus a new terminal tab in that project and resume its native Codex or Claude Code session by ID. A recently closed project gets a new workspace first. The terminal starts in the record's actual directory, regardless of the new-terminal directory policy. This resumes the session's latest state; it does not rewind to the selected message or automatically resend the record text.

Native Codex sessions are prepared in the background first. An existing session with that ID in the current account's store takes precedence; otherwise, the selected native session is copied into that store before resuming. The current account is determined by `CODEX_HOME` when the workspace starts, or `~/.codex` if unset. Importing never switches to the source account or copies credentials, account configuration, or databases, and leaves the source file unchanged. New records retain the native transcript path to support custom source directories; older records search only `.codex` and `.codex-*` stores in the user's home directory. Missing, damaged, or ambiguous sources report an error in the history window before creating a terminal. Existing destination sessions are never overwritten. Closing the window during preparation cancels the subsequent launch; an already imported copy may remain. Shell startup scripts that override `CODEX_HOME` can change the account ultimately used. Claude Code keeps its existing resume behavior; missing tools and other launch failures are reported in the terminal. For remote projects, run `context history` on the project's machine.

## Known check limitations

The project owner manually verified directory changes, archiving after closing all conversations, reopening recent projects, and retaining recent-project entries after restart. The full automated suite has not completed on the development machine: its filesystem does not support the EFS test fixture, and two project-reopen tests that create real terminals did not finish. These limitations are recorded rather than claimed as passing checks.

## Upstream relationship

The source is derived from herdrdev/herdr and retains its Apache-2.0 license. The project aims to preserve useful upstream terminal and agent-runtime improvements while developing its own task-tree experience. Upstream changes are reviewed and integrated selectively; they are not synchronized automatically.

## License

Apache-2.0. See [LICENSE](LICENSE), [NOTICE](NOTICE), and the retained third-party license and notice files. Modified source files carry change notices; JSON files use adjacent `.license` notices.
