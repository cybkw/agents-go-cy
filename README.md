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

首次使用先在项目根目录运行 `just build`，生成 release 程序及所需的 ConPTY 组件；再运行 `cargo build --locked`，生成双击入口使用的新版 debug 客户端。之后选择以下任一方式启动。

在项目根目录的 PowerShell 中执行：

    & .\target\release\herdr.exe --session agents-go

鼠标双击启动：

1. 在资源管理器中打开项目根目录。
2. 双击 `start.cmd`，脚本自动切换到项目目录并打开 `agents-go` 会话。
3. 如需从桌面启动，右键 `start.cmd`，选择「发送到」→「桌面快捷方式」（Windows 11 可先点「显示更多选项」），之后双击桌面快捷方式即可。

`start.cmd` 当前使用 `target\debug\herdr.exe` 显示新版界面，先运行 `cargo build --locked` 生成该程序；无需加入 PATH。脚本通过显式 socket 路径连接原 `agents-go` 会话；后台未运行时，先用 `target\release\herdr.exe` 恢复原会话，再连接新版界面。release 程序及 ConPTY 组件由 `just build` 生成。程序缺失或启动失败时会保留提示。

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

在想记录对话的项目根目录执行一次（以下示例使用新编译的调试程序）：

    & .\target\debug\herdr.exe context enable --project .

以上适用于当前源码目录；给其他项目启用时使用程序绝对路径，或已加入 PATH 的新版 `herdr context enable --project .`。`--agent codex` 或 `--agent claude` 可只启用其中一个，默认同时启用。接入配置和恢复 Skill 都安装在指定项目中，保留已有 hook，不修改全局 Agent 配置。Codex 需要重新打开项目会话，在 `/hooks` 中查看并信任新配置；Claude Code 需要重新打开项目会话并确认项目配置。

当前接入首先验证 Windows；Claude Code 需 2.1.139 及以上（支持直接执行参数），Codex 需支持 `UserPromptSubmit`、`Stop` 和 `commandWindows`。启用后，原生事件会把用户提交原文和每轮最终回复保存在 `.agents-go/history.jsonl`，按实际落盘顺序连续编号，记录时间、Agent、会话和回复关联。编号跨重启延续；不同项目独立保存，嵌套 Git 项目或 worktree 需要单独启用。不会采集模型内部思考或完整工具轨迹，也不会自动加载历史或交接任务。

需要接续时，在 Codex 对话框输入 `$project-resume`，在 Claude Code 中输入 `/project-resume`。恢复 Skill 只在明确调用时读取近期记录、整理进度和待办；若本次同时要求继续任务，则按当前项目规范执行。长记录只限制本次展示长度，原始记录保留，可按编号补读：

    herdr context resume
    herdr context history --from 1 --through 20 --limit 20
    herdr context history --limit 100 --json

这些读取命令无需启动工作台服务器，不改记录，也不向其他 Agent 自动发送内容。命令不在 PATH 时，使用启用时的程序绝对路径。

在新版工作台中，也可以右键侧栏的本地项目或最近项目，点击「对话记录」。窗口按编号从新到旧显示记录，首次载入最近 100 条；列表显示编号、角色、Agent、UTC 时间和摘要，右侧展示选中记录的完整正文。使用鼠标或 `↑`、`↓` 选择，`Tab`、`←`、`→` 切换列表与正文，滚轮或 `PageUp`、`PageDown` 滚动正文；`R` 刷新，`N` 或「加载更早」读取旧记录，`Esc` 关闭。窄窗口先展示列表，按 `Enter` 查看正文，`Esc` 返回列表。加载在后台执行；窗口只读，不会把记录发送给 Agent。远程项目暂通过项目所在机器的 `context history` 命令查看。

记录通过 hook 开始生效，既有对话不会自动回填。稳定轮次或消息 ID 用于避免同事件重发；Claude 事件未提供这些 ID 时保留每次发言，包括正文相同的重复提问。没有最终回复的回合不会伪造回复；写入失败或账本损坏会报错并保留现有数据。当前读写会扫描整个账本，大量长期历史的性能尚未做规模验证。

## 已知检查限制

项目维护者已手动验证切换目录、关闭全部对话后归档、点击最近项目重开，以及重启后的最近项目列表保留。开发机上的完整自动测试尚未全部完成：文件系统不支持 EFS 测试，两项创建真实终端的项目重开测试未能结束。这些限制保留记录，不视为测试通过。

## 上游关系

本项目源自 herdrdev/herdr，并保留 Apache-2.0 许可证。项目会继续关注 Herdr 的终端、智能体运行时及性能改进，并经审查后选择性集成；不会自动同步上游改动。

## 许可证

项目采用 Apache-2.0。请查阅 [LICENSE](LICENSE)、[NOTICE](NOTICE) 及保留的第三方许可证与归属文件。修改的源文件已注明改动；JSON 文件的声明放在相邻的 `.license` 文件中。
