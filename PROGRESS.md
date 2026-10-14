<!-- Modified for Agents Go by Agents Go contributors, 2026-10-01. See NOTICE. -->
# Agents Go 进度

最后更新：2026-10-14

## 当前状态

2026-10-14：新增根目录 `start.cmd`，支持双击启动 release 程序的 `agents-go` 会话；自动切换到脚本所在目录，程序缺失或启动失败时保留提示。已从 `C:\Windows` 工作目录运行脚本，确认启动的 `herdr.exe` 使用 `--session agents-go`；`git diff --check` 通过。窗口显示与交互仍需手动确认。

Agents Go 已公开于 https://github.com/zhangxq0606-ctrl/agents-go；默认分支与当前本地分支均为 main，origin 指向本仓库，upstream 指向 herdrdev/herdr。功能改造提交为 9d487e2，公开准备提交为 9eb7bc3；已核验 GitHub 提交、README、LICENSE 和 NOTICE 与本地一致。Cargo 包名和可执行文件名继续保留 herdr（用户约定），不做全面品牌迁移。

## 已完成

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

- 2026-10-01：集成上游 3 个提交并保留原作者：`d4e335e`（文档删外部链接）、`07e3840`（修复 codex 空闲检测，更新 codex/pi 清单）、`d6b40d4`（codex 空闲文档）。`cargo test --bin herdr detect` 137 项通过，`cargo clippy --all-targets --locked -D warnings` 通过。`347f9c9`（Windows 原生通知，需新增依赖）暂缓。
- 2026-10-01：在 AGENTS.md 补入上游改动汇报规范（编号一眼卡片、难度分级、交互原则、提交方式由 agent 决策后报批）与 Windows 本地验证约定（默认 target、运行中会话的验证路径）。
- 2026-10-01：README 默认语言改为中文，英文版转为 `README.en.md`；同步 docs/next 与发布流水线（versions.mjs、release.py、justfile、release.yml）中的文件名引用，删除 `README.zh-CN.md`。`bun test scripts/docs/` 7 项通过。
- 2026-10-01：侧栏四项细节优化完成并通过针对性验证（`cargo fmt --check`、`clippy -D warnings`、`task_tree` 27 项、`context/menu` 52 项回归），用户重启手动查看确认：分支标记 `▏`、最近项目行高 2、右键菜单中文化、运行状态 2×3 点阵提速。改动文件：`src/client/shell/{task_tree,context_menu,mouse}.rs` 及对应测试。
- 2026-10-01：main 已推送，GitHub 仓库已改为公开，默认分支为 main；README、LICENSE、NOTICE 的 Git blob 与本地一致。保留完整上游提交历史，本次未追加功能测试或构建。
- 2026-10-01：用户手动验收核心归档/恢复链路通过；公开准备中补齐修改声明和第三方归属指针，扫描 2,904 个文本文件未发现命中的凭据模式或个人路径。LICENSE 附录占位符为 Apache 标准示例，保留原文。
- 2026-09-30：隔离 Agents Go 本机配置目录和自动更新入口，清空旧 Herdr 发布资产清单，修正社区入口与维护者元数据并同步 API schema；`cargo fmt --check`/`just lint`、150 项维护测试（含 5 项跳过）、文档/热路径/集成资产检查，以及任务树与配置隔离针对性测试通过。完整 nextest 在 2,355 项通过后因本机文件系统不支持 EFS（OS error 50）失败；排除该环境限制后继续运行时，两项涉及默认终端创建的项目重开测试长时间不返回，已停止该轮，需单独定位。Release 编译产物已生成；Windows 打包结果仍待确认，手动 UI 验收状态见当前记录。
- 2026-09-30：创建 Agents Go 仓库并重写项目介绍、协作规范；后续公开与推送状态见当前记录。
