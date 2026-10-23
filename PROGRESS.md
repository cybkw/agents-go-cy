<!-- Modified for Agents Go by Agents Go contributors, 2026-10-01. See NOTICE. -->
# Agents Go 进度

最后更新：2026-10-23

## 当前状态

2026-10-23 双击入口更新：`start.cmd` 改为启动 `target/debug/herdr.exe` 新版界面，通过显式会话名、socket 与配置路径连接原 `agents-go` 会话；不传会覆盖 socket 选择的 `--session`。原后台未运行时，先隐藏启动已打包的 release 服务器并等待 API 就绪，以继续使用原会话数据。debug 缺失时提示 `cargo build --locked`，所需 release 服务器缺失时提示 `just build`，启动失败保留窗口提示。已从 `C:\Windows` 工作目录验证连接现有服务器；另用独立临时 APPDATA 验证无服务器时的启动路径，新版客户端成功连上新建的隔离服务器。测试会话和窗口均已停止，原 13 个终端与焦点保持不变；7 项文档契约及 `git diff --check` 通过。中英文 README 已同步。

2026-10-23 新版客户端重启与真实界面验证：按用户要求，以默认 `target/debug/herdr.exe` 连接现有 release 服务器的 `agents-go` 会话，确认连接成功后通过 `Ctrl+B`、`Q` 正常退出旧 release 客户端。服务器未重启；前后 13 个 pane 的 ID、terminal ID 与聚焦 pane 一致，现有 Agent 进程继续运行。debug 默认使用独立的 `agents-go-dev` 目录，因此本次显式指定现有 API socket、会话名和本地配置副本；仅副本启用 `experimental.allow_nested` 以允许从当前 Agent 环境启动界面，未修改全局配置或 `start.cmd`。当前工作台界面为新版 debug 客户端，后台服务器仍为原 release 程序。

本轮再次验证：`client::shell::` 387 项通过、3 项性能辅助测试按原规则忽略，`project_context::` 30 项及 `just lint` 通过。通过定向 Win32 控制台输入与真实屏幕回读，确认项目右键「对话记录」入口、19 条记录最新在前、最新用户消息完整正文、Home/End 键盘选择、鼠标选择助手正文、刷新保留选中项及 Esc 关闭全部正常；验证前后账本 SHA-256 一致。证据保存在被 Git 忽略的 `.tmp/history-live-validation.json` 及同目录屏幕回读／运行时快照中。长正文滚动、分页及窄屏继续以已有回归测试为依据，本轮真实项目仅 19 条记录，未将这些场景额外标记为真实界面验收通过。完整 `just test`／`just check` 仍未运行，既有 EFS 与真实终端测试限制保留。

2026-10-22 对话记录窗口：已实现「本地项目／最近项目右键 → 对话记录」只读入口，首次后台读取最近 100 条并按编号倒序排列，展示角色、Agent、UTC 时间、摘要与完整正文；支持键鼠选择、正文滚动、刷新、加载更早及窄屏列表／正文切换。读取复用现有账本锁与校验，窗口关闭或端点切换后不会接收旧结果；未启用、空账本与读取错误均有提示。远程端点不会把远端路径当作本地路径读取。窗口不向 Agent 发送内容，普通输入、粘贴及打开前已按住按键的重复事件均被拦截，必要的按键释放仍交回原终端。项目路径取被点击工作区实际 pane 的目录，不受新终端目录策略影响；未新增依赖或修改公共协议。

本轮验证：对话窗口 23 项回归包含于客户端 `client::shell::` 测试中，共 387 项通过、3 项性能辅助测试按原规则忽略；`project_context::` 30 项通过。`just lint`（格式与全 targets Clippy）、6 项热路径架构检查、7 项文档契约及 55 项集成资源测试通过。维护检查在 `PYTHONUTF8=1` 下通过：Python 150 项中 5 项按原规则跳过，发布工作流 4 项通过、1 项跳过；首次运行遇到默认 GBK 读取既有 UTF-8 vendor 文件错误，未修改测试或 vendor 文件。独立审查发现的长按重复输入穿透、新终端目录策略误导账本路径两项问题已修复并复核通过。正文换行按选中记录和宽度缓存，未扩大按 pane 数量执行的循环；长期超大账本性能仍未验证。默认 `target/debug/herdr.exe` 已通过 `cargo build --locked` 构建，可用该程序打开工作台验收；当前运行中的 `target/release/herdr.exe` 保持原版本。真实终端界面尚待手动验收，完整 `just test`／`just check` 未运行，既有 EFS 与真实终端测试限制保留。

2026-10-22 原生采集验收通过：用户在 Codex 界面完成信任后，只读 `hooks/list` 确认项目 `UserPromptSubmit`、`Stop` 均 enabled 且 trusted；真实用户消息和对应最终回复分别由原生事件自动保存为 #0007、#0008，没有手工补录。一次性本地观察脚本已核对会话、轮次和 reply_to；被 Git 忽略的 `.agents-go/native-validation.json` 返回 `status=passed`，确认用户与最终回复双向自动采集。Claude 原生采集、跨 Agent 恢复的界面验收及超长账本规模性能仍待验证。

2026-10-22：按用户确认完成“自动编号记录 → 手动恢复”首版，支持 Windows 下的 Codex 与 Claude Code，不自动交接。新增独立 `herdr context enable/capture/history/resume` 命令；项目 `.agents-go/history.jsonl` 保存用户原文与每轮最终回复，按落盘顺序编号、跨进程加锁、按稳定轮次/消息 ID 去重并关联回复。手动恢复 Skill 只读历史并汇报，不把旧任务自动执行。当前项目已安装项目级 hooks 与显式 Skill（Codex `$project-resume`、Claude `/project-resume`），补录本轮 5 条用户发言；运行文件与聊天记录已加入本项目 `.gitignore`，未更改全局 Agent 配置或公共 Socket API，未引入新依赖。

本轮验证：31 项项目记录单元测试、3 项独立 CLI 集成测试、18 项 CLI 命令契约测试、7 项文档契约测试通过；`cargo clippy --all-targets --locked -- -D warnings`、格式与补丁检查、恢复 Skill 校验通过。覆盖真实 Windows PowerShell hook 的 UTF-8/参数传输、跨进程编号、重复事件、Claude 迟到回复、损坏尾行保护、锁超时及原配置 ACL 保留。独立审查发现的权限保持和 Claude `prompt_id` 关联问题已修复。当前运行中的工作台使用 release 程序；本轮新命令构建于默认 `target/debug/`，接入绑定该程序，不覆盖正在运行的 release 文件。没有运行完整 `just test` / `just check`，既有 EFS 和真实终端挂起限制仍保留。

验收环境：codex-cli 0.163.0、Claude Code 2.1.251。首次重开未采集的原因已定位为新增 Hook 尚未获原生信任，用户在 `/hooks` 完成确认后通过上述真实验证。协议 schema、只读 `hooks/list` 与一次性观察脚本用于诊断和核验，未启动模型轮次或修改信任状态。当前每次读写扫描完整账本，长期大规模历史性能尚未验证。用法见 [README](README.md#项目对话记录与手动恢复)。

2026-10-22 远程同步准备：当前 `origin` 为 https://github.com/cybkw/agents-go-cy，分支为 `main`；已 fetch 确认同步前本地与远程无分歧。已有侧栏当前终端样式改动随本次代码同步，针对性回归 28 项通过、1 项性能辅助测试按原规则忽略；手动界面验收仍待确认。对话账本、项目 Hook、本机恢复 Skill 和临时诊断文件均保持 Git 忽略，不上传。

2026-10-14：新增根目录 `start.cmd`，支持双击启动 release 程序的 `agents-go` 会话；自动切换到脚本所在目录，程序缺失或启动失败时保留提示。已从 `C:\Windows` 工作目录运行脚本，确认启动的 `herdr.exe` 使用 `--session agents-go`；`git diff --check` 通过。窗口显示与交互仍需手动确认。

2026-10-09：cybkw/agents-go-cy Fork 已克隆到 Windows 本机。已安装并核验 Rust 1.96.1、Zig 0.16.0、Bun 1.4.2、just 1.58.0，以及 Visual Studio 2022 C++ Build Tools 和 Windows SDK。`just build` 成功，ConPTY 组件已打包；`herdr --version` 返回 0.9.3，`--help` 正常。安装过程通过系统代理下载 Rust，并将经 Zig 包哈希验证的依赖预先放入缓存，未修改源码或依赖锁定文件。本次交互界面手动验收与完整测试尚未执行。以下原维护者记录保留为历史状态。

Agents Go 已公开于 https://github.com/zhangxq0606-ctrl/agents-go；默认分支与当前本地分支均为 main，origin 指向本仓库，upstream 指向 herdrdev/herdr。功能改造提交为 9d487e2，公开准备提交为 9eb7bc3；已核验 GitHub 提交、README、LICENSE 和 NOTICE 与本地一致。Cargo 包名和可执行文件名继续保留 herdr（用户约定），不做全面品牌迁移。

## 已完成

- 项目级对话账本与显式恢复 Skill 的实现、命令集成及针对性自动验证已完成；Codex 原生用户消息与最终回复采集已通过真实会话验收，Claude 原生接入及跨 Agent 恢复的界面验收仍待完成。
- 任务树和最近项目目录记忆已实现，包含按终端实际工作目录归档和恢复项目。
- 用户已手动验收目录切换、关闭对话后归档、最近项目重开与重启后记录保留，确认正常；按用户指令不追加功能测试。
- 侧栏项目标题、Git 分支和 Agent 状态的视觉呈现已实现。
- 侧栏细节优化（2026-10-01）：Git 分支标记由缺字字形 `⎇` 改为 `▏`；最近项目行高由 1 增为 2 并修正滚动与预算；右键菜单条目全部中文化；运行状态点阵由 2×4 改为 2×3 Braille 旋转符并提高帧率。用户已重启手动查看确认。细节见 .local/prd/status-branch-redesign.md。
- 已移除上游赞助名单与图片，并同步清理 docs/next README 草稿中的引用。
- 最近一轮目录与任务树相关回归、格式及维护检查已通过；详细设计记录见 .local/prd/sidebar-tree.md。
- README 与项目协作规范已改写为 Agents Go 方向，并说明 Herdr 来源和 Apache-2.0 许可证。
- Agents Go 的 Cargo 仓库元数据、调试/正式配置目录、更新清单地址、Issue 模板和维护者名单已与本仓库对齐；版本检查默认关闭，Agents Go 发布渠道就绪前阻止手动更新和远程服务器自动下载。
- 稳定版与预览版分发清单已换为空资产占位；Herdr 的发布、网站部署和 issue 关闭任务保留了仅在官方仓库执行的条件。
- README 默认语言改为中文（2026-10-01）：根目录与 docs/next 的 `README.md` 换为中文，原英文内容转为 `README.en.md`，删除 `README.zh-CN.md`；同步更新 `scripts/docs/versions.mjs`、`versions.integration.test.ts`、`scripts/release.py`、`justfile`、`.github/workflows/release.yml` 中的文件名引用。`bun test scripts/docs/` 7 项通过。

## 进行中

- 自动测试的 EFS 环境限制和两项真实终端测试挂起仍有记录；手动验收已通过，本轮不继续排查，不以此阻塞公开源码。
- 上游仍有 1 个提交未集成（2026-10-01 fetch）：`347f9c9` Windows 原生可操作通知（feat，19 文件、新增 `windows` crate 依赖）；已确认可无冲突 cherry-pick，待船长决定是否授权新增依赖后处理。
- 本机 `ag` 会话运行 `target/debug/herdr.exe` 时，需要重链主程序的 `just check`/`just test`/`cargo build` 会因文件占用（os error 5）失败；会话期间改用 `cargo test --bin herdr <过滤>` 与 `cargo clippy`。约定见 AGENTS.md。
- 原工作区为浅克隆，补全历史时受既有 shallow.lock 阻挡；本次通过 .tmp/open-source/publish-filtered 中的完整提交历史接入本地提交并推送，未直接改动原工作区 .git 内部文件。后续补全原工作区历史时需处理该 Git 操作限制；不影响程序运行和公开仓库历史。

## 上游维护约定

官方仓库作为 upstream，Agents Go 仓库作为 origin。只 fetch 和比较，不自动合并；审阅官方性能改动后，按依赖关系选择 cherry-pick 或最小化移植。渲染或窗格规模相关优化记录可比的基准结果。集成的上游提交与验证结果记录在这里，并保留必要版权及许可证声明。

## 最近完成

- 2026-10-09：增强侧边栏当前终端样式，保留主题高亮底色、名称加粗提亮，左侧标记由 `▎` 改为 `▌`；非当前终端及状态颜色保持原有规则。`cargo test --locked --bin herdr task_tree` 28 项通过、1 项性能辅助测试按原规则忽略；`cargo fmt --check` 与 `git diff --check` 通过。尚未重建主程序或手动验收界面；为避免覆盖运行中会话的主程序，未执行完整 `just check` / `just build`。

- 2026-10-01：集成上游 3 个提交并保留原作者：`d4e335e`（文档删外部链接）、`07e3840`（修复 codex 空闲检测，更新 codex/pi 清单）、`d6b40d4`（codex 空闲文档）。`cargo test --bin herdr detect` 137 项通过，`cargo clippy --all-targets --locked -D warnings` 通过。`347f9c9`（Windows 原生通知，需新增依赖）暂缓。
- 2026-10-01：在 AGENTS.md 补入上游改动汇报规范（编号一眼卡片、难度分级、交互原则、提交方式由 agent 决策后报批）与 Windows 本地验证约定（默认 target、运行中会话的验证路径）。
- 2026-10-01：README 默认语言改为中文，英文版转为 `README.en.md`；同步 docs/next 与发布流水线（versions.mjs、release.py、justfile、release.yml）中的文件名引用，删除 `README.zh-CN.md`。`bun test scripts/docs/` 7 项通过。
- 2026-10-01：侧栏四项细节优化完成并通过针对性验证（`cargo fmt --check`、`clippy -D warnings`、`task_tree` 27 项、`context/menu` 52 项回归），用户重启手动查看确认：分支标记 `▏`、最近项目行高 2、右键菜单中文化、运行状态 2×3 点阵提速。改动文件：`src/client/shell/{task_tree,context_menu,mouse}.rs` 及对应测试。
- 2026-10-01：main 已推送，GitHub 仓库已改为公开，默认分支为 main；README、LICENSE、NOTICE 的 Git blob 与本地一致。保留完整上游提交历史，本次未追加功能测试或构建。
- 2026-10-01：用户手动验收核心归档/恢复链路通过；公开准备中补齐修改声明和第三方归属指针，扫描 2,904 个文本文件未发现命中的凭据模式或个人路径。LICENSE 附录占位符为 Apache 标准示例，保留原文。
- 2026-09-30：隔离 Agents Go 本机配置目录和自动更新入口，清空旧 Herdr 发布资产清单，修正社区入口与维护者元数据并同步 API schema；`cargo fmt --check`/`just lint`、150 项维护测试（含 5 项跳过）、文档/热路径/集成资产检查，以及任务树与配置隔离针对性测试通过。完整 nextest 在 2,355 项通过后因本机文件系统不支持 EFS（OS error 50）失败；排除该环境限制后继续运行时，两项涉及默认终端创建的项目重开测试长时间不返回，已停止该轮，需单独定位。Release 编译产物已生成；Windows 打包结果仍待确认，手动 UI 验收状态见当前记录。
- 2026-09-30：创建 Agents Go 仓库并重写项目介绍、协作规范；后续公开与推送状态见当前记录。
