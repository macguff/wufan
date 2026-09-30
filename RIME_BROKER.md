# Broker / librime 开发后端

2026-09-30：新增独立 Broker 后端，固定 librime **1.17.0**。参考 [Weasel](https://github.com/rime/weasel) 的进程外引擎和 Rime session 分工，使用本项目的六字段身份与提交 ACK 契约。

## 本地运行

需要 Windows x64、Visual Studio C++ Build Tools、仓库固定的 Rust 工具链、7-Zip（`7z.exe` 在 PATH）。在 VS Developer PowerShell 中进入仓库根目录：

```powershell
.\scripts\fetch_rime.ps1
cargo build --locked -p ime-broker
.\target\debug\ime-broker.exe --deploy
```

首次 `--deploy` 会在接收连接前编译词典，可能需要一些时间。保持该终端运行，另开终端运行同一个可执行文件：

```powershell
.\target\debug\ime-broker.exe --pipe-smoke 'nihao '
.\target\debug\ime-broker.exe --pipe-smoke 'zhongwen '
```

诊断客户端显示 preedit、候选和 commit；尾部空格用于选词。它把 stdout 作为诊断宿主，输出并 flush 提交文本后发送 Applied ACK。它不会向编辑器输入文字。以后正常启动可省略 `--deploy`；更换 schema 后需重新部署。

默认目录为 `target/rime-runtime`；用 `--runtime <绝对目录>` 指定另一份完整布局。布局为 `native/dist/lib/rime.dll`、`shared/` 和独立 `user/`，不使用现有 `%APPDATA%\Rime`。下载及部署不是 TSF callback 的工作，也尚未接入 D6 Deployment Manager。

## 组件边界

| 组件 | 实现 |
| --- | --- |
| `ime-rime-sys` | checked-in 64 位 ABI、显式路径 DLL 加载、函数表长度检查、DLL 生命周期 |
| `ime-rime-adapter` | 加载前核验固定 DLL 的 SHA256、安全 `RimeEngine` trait、owned UTF-8 文本/候选、UTF-8 cursor → UTF-16、native allocation RAII、单进程单 runtime、session 管理 |
| `ime-broker` | 单独 serial worker 创建/调用/销毁 librime；Tokio 管道和有界 job queue；不直接依赖 raw ABI |
| `ime-broker-transport` | frame 读取和 Windows OS 身份/ACL；无 Tokio，TSF 和诊断客户端共用 Win32 overlapped I/O、取消排空和 2 秒 deadline |
| `ime-protocol::wire` | version 1、六字段 identity、请求/回复/提交 ACK、LE u32 frame length、64 KiB 上限、拒绝未知字段 |

重新生成绑定必须显式执行：

```powershell
$env:LIBCLANG_PATH = '<LLVM 19.1.7 的 bin 目录>'
cargo xtask bindgen-rime
```

独立 bindgen 工具锁在 `tools/rime-bindgen/Cargo.lock`，header 已 vendored。普通构建/CI 不运行 bindgen，不需要 LLVM 或 librime 安装；运行 Broker 才需要上述 native runtime。

## 请求与故障语义

- 管道拒绝远程客户端，DACL 限制到当前 OS logon SID；首个实例要求 first-instance ownership。双方从 Windows 获取对端 PID、进程映像、session、logon SID 和 AuthenticationId，身份不由 wire 自报。
- **这是开发策略**：默认客户端必须来自 Broker 自身可执行路径；可重复的 `--dev-client <exe>` 显式批准 TSF 宿主。双方必须处于同一登录上下文；尚无生产签名 allowlist。TSF 通过 `WUFAN_BROKER_EXE` 指定预期 Broker，未设置时使用 DLL 同目录下的 `ime-broker.exe`。
- Hello 由 Broker 分配 connection incarnation 与 generation。Open 的 session ID 严格递增，request_seq 从 1 开始；后续请求严格加 1。focus/composition epoch 在同一 session 内固定，切换时 Close + 新 Open。
- 缓存每个 session 最近一份完整请求/回复。相同请求重发返回缓存；同 seq 不同内容和更早请求被拒绝，不再次调用引擎。每个 connection 最多 32 个 active session、128 个 session records；超过 record 上限应创建新 connection，不能复用旧 session ID。
- 引擎 commit 只生成 CommitIntent；pending commit 期间拒绝 Key/Clear，等待正确 commit ID 的 ACK。Applied 触发一次 adapter hook；Rejected/Indeterminate 关闭旧引擎 session，不隐式重试。Close/断连可释放 pending session。
- 最多 8 个连接、32 个排队 job，每个连接逐个处理请求。frame/engine reply/write deadline 为 5 秒，空闲读取为 60 秒。超限/错误关闭 connection；关闭后 serial lane 清理 session。阻塞在 native 调用中的引擎线程尚无强制中断机制，需终止并重新启动 Broker。
- TSF 编辑和原子提交仍由宿主 Adapter 执行；Broker 不接触 COM 或宿主文本。

## 学习与剩余工作

默认 librime 会在其内部 commit 时学习，早于宿主 Applied ACK。当前 `wufan_pinyin` **明确设置 `translator.enable_user_dict: false`**；启动检查部署后的 translator 列表及该值，不满足则拒绝启动。`commit_applied` 目前是明确的 no-op，Hello 报告 `learning_enabled: false`。这保证启动阶段没有提前学习，但尚未实现 ACK 后的学习。

TSF 已通过独立 worker 连接此后端；COM 对象留在 apartment，键回调只做本地 Test/Actual 配对和有界 `try_send`。消息窗口以 10 ms timer 交付 view，旧 epoch 回复被丢弃，异步 edit session 执行宿主写入。2026-09-30 按 [ADR-007](ADR-007_HOST_COMMIT.md) 增加 RuntimeCore `host_commit` 纯 reducer：真实 Open/Request 事件授权 commit，只有 Apply effect 可创建宿主执行 guard。guard 默认报告 Rejected，开始写入后失败报告 Indeterminate，完整成功报告 Succeeded/HostRevision；reducer 匹配 effect/scope 并形成 terminal ACK。尚未获授权的旧 transport receipt 只会被拒绝。Broker 收到 ACK 前不处理下一次 mutation。IPC worker 不在 callback 内 join，DLL 为 worker 固定到宿主进程结束。`ime-pinyin-engine` 当前只提供 key/reply DTO，不再执行本地词表查询。

2026-09-30 已运行真实 librime 管道诊断：`nihao ` → `你好`、`zhongwen ` → `中文`。WPF 实际输入已通过连续提交 `你好我爱你`、数字候选 `ni2` → `拟`、Escape 后重新组词和文本框焦点切换，并观察到宿主 Applied ACK。注册需实际 UAC 管理员权限；测试辅助脚本临时注册并自动注销，详见 [WINDOWS_PROBE.md](WINDOWS_PROBE.md)。

发现的 Test/Actual 丢键原因是将未消费的 Test 当成冲突并拒绝新 Test。Test 是宿主探测，可能没有 Actual，也可能重复；实际输入路径现保留最新 Test，Actual 仍检查键特征、modifier、100 ms TTL 并仅消费一次。提交由 RuntimeCore 子状态机授权和归类，具有唯一在途 commit、32 项 terminal cache、request/commit ID 高水位、重复与旧 scope gate；焦点失效保留旧结果义务，旧结果不能影响新 scope。

2026-09-30 按 ADR-008 将实际按键/preedit/request 与内部 commit ledger 合并到单个 input_lifecycle owner；worker 等待每个 host result，commit 再确认 wire ACK，只有 Finished 释放下一键。108 项工作区测试、Clippy、格式、依赖边界检查及 Windows x64 release 构建通过。带真实 preedit 前置断言的 WPF 验收在 80 ms、30 ms 和零间隔输入下均通过，每轮 29 个 Actual/accepted/view/host result/Finished 一致，并收到五次 Applied ACK；临时注册已注销。本轮新增 WinForms/Win32 EDIT 宿主，与 WPF 共用严格场景和只读 ready/idle/accepted 探针；80 ms 与零间隔矩阵四组通过，每组 29 个 Actual/accepted/view/host result/Finished 一致及五次 Applied ACK。原生 preedit 经 IMM composition 查询确认。原 RuntimeState 模型对照与 canonical trace 格式迁移仍待完成。Word/Chromium/其他 Win32 控件、物理键及 auto-repeat、生产信任策略、ACK 后学习、独立候选窗和 D6 部署仍待实现或验收。

## 固定来源

`fetch_rime.ps1` 在下载后、解压前验证 archive 的 SHA256，并验证词典文件：

| 内容 | 固定来源 / SHA256 |
| --- | --- |
| Windows x64 librime | release `1.17.0` / `33e7814`；`7478c7caa4ff6b37de86daba1f7ce4a994a4f5ba24872a820fb2b3a9b01fed15` |
| 解压后的 `rime.dll` | `86b4c7357d4c6d293ce5589b234d8859ca2ac30923a03bedfa3926eeaf97fb0b`；adapter 在加载前校验 |
| deps / OpenCC data | 同 release；`9ef5608d8a54ff52bbad7a9b4128de42b232f8e3dd1f5fd3bff42a0b1bacd7e8` |
| vendored `rime_api.h` | tag `1.17.0`；`6eb5629162c44761e7b878f1b122f464de5c8685bd3cd5ac1e8519badc6d6424` |
| luna_pinyin dictionary | `rime/rime-luna-pinyin@56b934b099dfbeab842320f13aa8b461a6ab3e42`；`75bcf6eb3ff62b129882ed89cc22b2d80a5347aa72bcfa2ccc839bac298e7314` |
| essay vocabulary | `rime/rime-essay@054920de4f54c9e5994276a96a4fc2a35cb51aa3`；`3e8512ebaa6657e35961b1f5762060717c647e3bf471d5e3b86e783b4e934ffb` |

保留词典头部来源声明；librime 的 BSD license 保存于 `crates/rime-sys/vendor/LICENSE`，脚本复制到 runtime 根目录。二进制、OpenCC 和字典仅下载到被忽略的 target 目录。当前已生成内部候选 ZIP，尚无可分发版本；重新分发前还需补齐各依赖、插件和词典的上游许可/数据来源，见 `config/beta/native-licenses.json`。

2026-09-30 扩展矩阵进一步通过注入式重复 Backspace 和 Broker 空闲 preedit 时退出/英文透传/重启：每组 44 个 Actual/accepted/view/host result/Finished、七次 Applied ACK，三个英文键透传。worker 增加空闲 pipe 断连检查；transport reset 保留当前 context binding；提交缓存键补 Broker generation，隔离重启后重用的编号。物理长按、Broker hang、请求在途及 ACK 窗口故障仍待验收。详见 [WINDOWS_PROBE.md](WINDOWS_PROBE.md)。

技术内测 B2：WPF/Win32 EDIT 在 80 ms 和零间隔下进一步通过 Broker 整进程挂起/恢复、引擎 reply deadline、在途退出及宿主 Applied 后 wire ACK deadline。三个在途输入失效不重放，已提交的你好在 ACK 超时后保留一次，后续输入继续工作。注入逻辑由 fault-injection feature 隔离到 target/fault-acceptance，普通 release 拒绝注入 CLI。打包和安装/卸载/回滚控制器已实现，实际事务验收、可分发包、许可汇总和 Chromium 仍待完成，见 [TECH_BETA_PLAN.md](TECH_BETA_PLAN.md)。

## 技术内测启动配置（2026-09-30）

仓库开发启动仍保留原默认路径。内测控制器显式传入 `--runtime <version>/runtime --user-data <install>/data/rime/<version> --deploy --ready-file <unique-path>`，不依赖编译时仓库位置，也不携带开发 user 目录。就绪文件仅在 librime 部署、schema 校验和独占 named pipe 建立后写入 PID。应用授权继续使用重复 `--dev-client <actual-exe>`；候选包学习关闭，生产签名策略待完成。

打包和安装入口见 [BETA_PACKAGE.md](BETA_PACKAGE.md)。发布包只允许无 `fault-injection` 的独立 release 构建；第三方原生/数据/插件许可尚在整理，内部候选不代表可分发包。
