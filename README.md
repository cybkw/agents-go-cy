<!-- Modified for Agents Go by Agents Go contributors, 2026-10-01. See NOTICE. -->
# Agents Go

一个面向 AI 编程智能体的终端工作台，用可视化任务树组织项目与终端对话。

Agents Go 是 [Herdr](https://github.com/herdrdev/herdr) 的改造版，增加了项目与终端对话任务树、智能体运行状态、最近项目恢复和 Git 分支显示。命令行与可执行文件仍叫 `herdr`，本仓库不是 Herdr 官方版本。

后台更新检查、自更新和远程服务器自动下载目前关闭。本仓库提供源码；继承的 Herdr 安装脚本与发布文档介绍的是上游产品。

[English](README.en.md)

## 构建与开发

Rust 工具链版本由 rust-toolchain.toml 固定。按平台准备 Rust、Zig 0.16、Python、Bun 和 just 等构建工具。

Windows 打包构建：

    just build

### 启动终端（Windows）

首次使用先在项目根目录运行 `just build`，生成 release 程序及所需的 ConPTY 组件。命令行与双击入口统一使用这份程序。之后选择以下任一方式启动。

在项目根目录的 PowerShell 中执行：

    & .\target\release\herdr.exe --session agents-go

鼠标双击启动：

1. 在资源管理器中打开项目根目录。
2. 双击 `start.cmd`，脚本自动切换到项目目录并打开 `agents-go` 会话。
3. 如需从桌面启动，右键 `start.cmd`，选择「发送到」→「桌面快捷方式」（Windows 11 可先点「显示更多选项」），之后双击桌面快捷方式即可。

`start.cmd` 使用 `target\release\herdr.exe` 打开 `agents-go` 会话；后台未运行时，程序自动启动同一份 release 后台并恢复会话。脚本无需加入 PATH，程序缺失或启动失败时会保留提示；可运行 `.\start.cmd --version` 核对版本。

如需直接输入 `herdr`，将本仓库 `target\release` 的绝对路径加入 PATH，并用 `Get-Command herdr` 确认解析到该目录。`herdr --session agents-go` 与双击入口使用同一会话；单独运行 `herdr` 时，若未设置会话环境变量，则使用 `default` 会话。更新源码后须重新运行 `just build`，修改源码不会自动更新程序；Windows 下重建前须停止占用 release 程序的后台，这会结束其中的终端进程。

如需调试版：

    cargo build --locked
    & .\target\debug\herdr.exe --session agents-go

本改造版目前在 Windows 上进行过手动验证。其他平台的构建路径沿用 Herdr，尚未手动验证这些改动。

运行测试与检查：

    just test
    just check

Windows 构建配方会打包所需的 ConPTY 运行组件。编译成功后仍需手动检查终端中的显示和交互。

## 短命令

可执行文件名为 `herdr`。如需用短命令启动 Agents Go，在 shell 配置里加一个别名。

PowerShell（`$PROFILE`）：

    function ag { herdr --session agents-go @args }

Bash 或 zsh（`~/.bashrc` 或 `~/.zshrc`）：

    alias ag='herdr --session agents-go'

注意：`ag` 同时是 The Silver Searcher 的命令名。若已使用该工具，请换一个名字，如 `ago`。

## 项目对话记录与手动恢复

本地交互工作台默认在打开项目时后台启用 Codex 和 Claude Code 对话记录；重启恢复的项目也会接入。已有 Hook 和记录保留，全局 Agent 配置不变。Git 项目使用所在仓库根目录，嵌套仓库／worktree 独立接入；普通目录使用实际项目目录，磁盘根目录和用户主目录跳过自动接入。远程项目及只读终端观察不会在本机安装配置。

配置安装后，请重新打开已运行的 Agent 项目会话：Codex 通过 `/hooks` 查看并信任新配置，Claude Code 确认项目配置。自动安装不代表 Agent 已加载或信任；Hook 生效后才开始记录，之前的对话不自动补录。可以在「对话记录」窗口查看自动接入、等待加载／信任或配置异常状态。

也可以在项目根目录手动启用：

    & .\target\release\herdr.exe context enable --project .

以上适用于当前源码目录；给其他项目启用时使用程序绝对路径，或已加入 PATH 的新版 `herdr context enable --project .`。手动命令支持 `--agent codex` 或 `--agent claude`，不指定时同时启用。恢复 Skill 和 Claude 配置安装在指定项目中；Git 工作树的 Codex Hook 安装在主仓库的 `.codex/hooks.json`，根据事件中的实际目录写入各工作树自己的账本，未启用的工作树跳过记录。此操作不会自动启用主仓库的账本，也不会修改 Hook 信任状态；工作树内已有的旧 Hook 文件保留。若自动接入因权限或配置损坏失败，先处理窗口中报告的原因，再在「对话记录」窗口按 `R` 重试；也可使用此命令手动重试。

当前接入首先验证 Windows；Claude Code 需 2.1.139 及以上（支持直接执行参数），Codex 需支持 `UserPromptSubmit`、`Stop` 和 `commandWindows`。启用后，原生事件会把用户提交原文和每轮最终回复保存在 `.agents-go/history.jsonl`，按实际落盘顺序连续编号，记录时间、Agent、会话和回复关联。编号跨重启延续；不同项目独立保存。不会采集模型内部思考或完整工具轨迹，也不会自动加载历史或交接任务。

需要接续时，在 Codex 对话框输入 `$project-resume`，在 Claude Code 中输入 `/project-resume`。恢复 Skill 只在明确调用时读取近期记录、整理进度和待办；若本次同时要求继续任务，则按当前项目规范执行。长记录只限制本次展示长度，原始记录保留，可按编号补读：

    herdr context resume
    herdr context history --from 1 --through 20 --limit 20
    herdr context history --limit 100 --json

这些读取命令无需启动工作台服务器，不改记录，也不向其他 Agent 自动发送内容。命令不在 PATH 时，使用启用时的程序绝对路径。

在新版工作台中，也可以右键侧栏的本地项目或最近项目，点击「对话记录」。窗口按编号从新到旧显示记录，首次载入最近 100 条；列表显示编号、角色、Agent、UTC 时间和摘要，右侧展示选中记录的完整正文。使用鼠标或 `↑`、`↓` 选择，`Tab`、`←`、`→` 切换列表与正文，滚轮或 `PageUp`、`PageDown` 滚动正文；`R` 刷新，`N` 或「加载更早」读取旧记录，`Esc` 关闭。窄窗口先展示列表，按 `Enter` 查看正文，`Esc` 返回列表。加载在后台执行；浏览和刷新不会把记录发送给 Agent。远程项目暂通过项目所在机器的 `context history` 命令查看。

选中记录后，点击「C 继续对话」或按 `C`，将在该项目中新增并聚焦终端标签页，按记录中的 Agent 和会话 ID 恢复原生 Codex／Claude Code 会话；最近关闭的项目会先创建工作区。新终端使用记录中的实际目录，不受新终端目录策略影响。继续的是该原生会话的最新进度，不会回滚到选中的消息，也不会自动重新发送记录正文。

Codex 原生会话会先在后台准备：当前账号的会话库已有该 ID 时直接沿用；否则将选中的原生会话复制到当前账号会话库，再恢复。当前账号按工作台启动时的 `CODEX_HOME` 确定，未设置时使用 `~/.codex`；不会切换到来源账号，也不复制登录凭据、账号配置或数据库，原文件保持原样。新记录保存原生会话路径以支持自定义来源目录；旧记录只在用户目录的 `.codex`、`.codex-*` 会话库中查找。来源缺失、损坏或无法唯一确定时，在记录窗口提示错误并保留窗口，不先创建终端。已有目标会话不会被来源副本覆盖。准备期间关闭窗口会取消后续启动，已复制的会话可保留。终端启动脚本若自行改写 `CODEX_HOME`，会影响最终使用的账号。Claude Code 沿用原有恢复方式；工具未安装等启动问题仍按终端提示处理。

记录通过 hook 开始生效，既有对话不会自动回填。稳定轮次或消息 ID 用于避免同事件重发；Claude 事件未提供这些 ID 时保留每次发言，包括正文相同的重复提问。没有最终回复的回合不会伪造回复；写入失败或账本损坏会报错并保留现有数据。当前读写会扫描整个账本，大量长期历史的性能尚未做规模验证。

## 已知检查限制

项目维护者已手动验证切换目录、关闭全部对话后归档、点击最近项目重开，以及重启后的最近项目列表保留。开发机上的完整自动测试尚未全部完成：文件系统不支持 EFS 测试，两项创建真实终端的项目重开测试未能结束。这些限制保留记录，不视为测试通过。

## 上游关系

本项目源自 herdrdev/herdr，并保留 Apache-2.0 许可证。项目会继续关注 Herdr 的终端、智能体运行时及性能改进，并经审查后选择性集成；不会自动同步上游改动。

## 许可证

项目采用 Apache-2.0。请查阅 [LICENSE](LICENSE)、[NOTICE](NOTICE) 及保留的第三方许可证与归属文件。修改的源文件已注明改动；JSON 文件的声明放在相邻的 `.license` 文件中。
