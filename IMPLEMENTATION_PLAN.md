# Windows IME 实现计划

依据：[Slice 0 v0.2](1.md)、[Runtime Architecture v1.0](2.md)、[ADR-006 v0.3](3.md)。本计划以文档的批准边界为准：当前可以初始化仓库并实现纯 RuntimeCore；真实 TSF 集成须先完成 Slice 0 出口验收和 TSF spike 的 G1–G10。

## 交付顺序与门槛

| 阶段 | 主要交付物 | 进入下一阶段的门槛 |
| --- | --- | --- |
| 0A 仓库基线 | Cargo workspace、`protocol`/`runtime-core`/`test-harness`/`xtask`、固定工具链、依赖边界检查 | 纯核心可在非 Windows 环境构建；CI 能拒绝违规依赖和 unsafe |
| 0B 状态机骨架 | 封闭类型、纯 `reduce`、同步回复、Effect/结果闭环、确定性 ID | 同一输入可得到逐字节相同的状态、回复、Effect ID 和顺序；S1–S3 有断言 |
| 0C 核心业务转换 | key、focus、session、composition、commit、UI intent、故障降级 | I2/I3/I6 与非法消息门禁通过模型和确定性调度验证 |
| 0D 部署与独立 Memory 模型 | drain/cancel、D6 持久提交、启动恢复；EventDot 去重模型 | I4/I5 的故障矩阵通过；部署不能产生部分提交状态 |
| 0E Slice 0 验收 | M01–M26、随机探索、prefix safety、trace replay、故障注入报告 | [阶段 0 出口条件](1.md#22-阶段-0-出口条件)全部满足，才进入真实 TSF Adapter |
| 1 TSF discovery | Rust + windows-rs 最小 Stub、fake Broker、真实宿主观测 | 两周 timebox 内全部通过 G1–G10；否则切换唯一 C++ raw COM fallback |
| 2 安全异步 IPC | named pipe、认证、版本化 envelope、有界队列、deadline | 重复、乱序、断连、恶意 frame 和身份伪造测试通过 |
| 3 librime Broker | `rime-sys`、`rime-adapter`、`RimeEngine`、session strand | fake engine 与真实 librime 的协议/生命周期测试通过 |
| 4 宿主与候选窗 | 多宿主故障矩阵、native HWND 候选窗 | 焦点、DPI、geometry、UI stall 和宿主关闭测试通过 |
| 5 部署与配置联调 | Deployment Manager、D6 恢复、Control Center | 事务恢复矩阵通过后扩展 Control Center 功能 |

阶段门槛是依赖关系，不是日历承诺。Slice 0 的模型工作可以并行开发，但集成顺序按表执行。Windows 原生验证须在 Windows/MSVC 环境完成。

## 当前进度

| 工作包 | 状态 | 已落地内容 / 剩余边界 |
| --- | --- | --- |
| 0A-1 | 基线已建 | Cargo workspace、锁定工具链、xtask、PowerShell 转发器、依赖/unsafe 检查、Linux CI；Windows/MSVC 原生 release build 已在远端 runner 通过。 |
| 0B-1 | 骨架已建 | 基础 ID、消息身份、RuntimeState、同步 key reply、纯 reducer；完整封闭业务类型仍随后续工作包扩展。 |
| 0B-2 | 基础闭环已建 | S1–S3 reducer 断言、结构 hash v5、model/legal-state prefix oracle、带解码的 canonical trace/replay、按 EffectId/result class/scope 消费的义务门禁和驱动 Effects 的确定性 fake scheduler（key/key-up/UI request、composition、commit）已有；固定 trace 有 100 次逐字节重放断言，`proptest` 已覆盖可收缩的随机 runtime prefix 与 replay；stale/invalid effect、request、UI、candidate 和 commit 输入只留下诊断且不改业务状态；现已增加 reducer 转移返回位置覆盖门禁，多轨迹验收报告仍待整理。 |
| 0C-1 | 部分完成 | 有界 Test/Actual keydown 与 keyup ledger、显式 DecisionTtlExpired、`ContextPushed/ContextPopped`、focus/session/generation 门禁、单在途 FIFO mutating queue、M04 队列溢出 passthrough recovery、composition start/update/terminate、stale completion cancel、mutating deadline（含 commit indeterminate）、HostClosing/deactivation 和独立 modifier down/up bookkeeping 已有；TSF 的实际 keyup 回调配对仍待 Windows 验证，所有 race 也未穷尽。 |
| 0C-2 | 部分完成 | at-most-once commit reducer、terminal cache、重复 intent/result、reject/indeterminate/timeout 区分；CandidateView revision/count 门禁、UIActionIntent 与键盘共享 FIFO/request_seq、stale geometry 拒绝模型、OS 身份/signature/dev-trust 策略模型已有；composition update/termination 与 full-ledger/cache/queue 分支有针对性测试。 |
| 0D | 模型雏形 | D6 前后恢复/rollback、active sessions drain gate 与 EventDot/version-vector 独立模型已有；deployment 的 32 × 1,000 故障交错现在逐步检查 phase-local D6 invariant。真实持久化 adapter 与各存储故障点注入仍缺。 |
| 0E | 模型场景部分覆盖 | M01–M26 用例映射见 [SLICE0_SCENARIO_MATRIX.md](SLICE0_SCENARIO_MATRIX.md)；二进制轨迹格式及校验解码见 [TRACE_FORMAT.md](crates/test-harness/TRACE_FORMAT.md)。Runtime 固定 16 × 1,000 步 + nightly 新 seed，另有 32 个至少 1,000 步的可收缩 `proptest` trace；随机 Runtime prefix 经 fake scheduler 核验合法状态、S1/S3、Applied/journal 一致性与无重复 ApplyHostCommit，专门属性验证六个身份字段和 commit 终态/I6。deployment/Memory 各 32 × 1,000 步探索，并逐步检查 D6 invariant。RuntimeCore 确定性门禁要求所有 reducer `TransitionResult` 构造位置均被场景触达；另补 commit deadline、空队列 pump、迟到与重复 EngineUpdate、候选视图过期及焦点切换中 Applying commit 场景，清除容量预检及身份门禁后的不可达重复分支；修复重复焦点通知丢失 composition，以及 Applying commit 期间 composition 更新和队列提前派发。fake scheduler 覆盖四类终态 Effect 的成功、拒绝、不确定结果，以及 commit 写入前后两种不确定窗口；S2/S3 oracle 有反例测试。Rust 1.91.1 本机 `xtask ci`、nextest（80 项）及带 branch 覆盖率编译的 80 项测试均通过。本机 LLVM coverage 合计 region 93.53%、line 96.47%；RuntimeCore 文件 branch 为 271/364（74.45%）；reducer 源分支审计为 182/200，另 18 个方向由结构性 invariant 或先行门禁判定不可达，本机审计通过。CI 已配置发布 branch 报告、审计清单及原始覆盖率数据，Slice 0 提交 de7b912 的远端 core、coverage、Windows/MSVC CI 已通过，覆盖率与探针 artifact 已发布。 |
| 1 | 探针已起步 | `ime-windows-tsf` 实现了 COM class factory、TSF activation/key sink、HKCU 注册、语言栏中/英按钮与菜单、Ctrl+Space 保留键、open/close 与 conversion compartment 同步，以及基于宿主 exe 路径的异步持久模式记忆。G1 的 100 次注册/激活/释放/注销门禁已在 Windows CI 通过；当前普通按键仍透传，模式与真实 Windows 指示器、Word/Chromium/Win32 宿主的同步尚未验证，G2–G10 集成门槛仍未全部通过。参见 [WINDOWS_PROBE.md](WINDOWS_PROBE.md)。 |
| 2–5 | 未开始 | 安全 IPC、librime、候选窗和部署尚未实现；当前产物不能作为可用中文输入法发布。 |

当前验证使用锁定的 Rust 1.91.1；完整 `cargo run --locked -p xtask -- ci` 通过（49 个 RuntimeCore、31 个 harness 测试），Windows target `cargo check --locked --workspace --target x86_64-pc-windows-msvc` 与 TSF crate 的 Windows target Clippy 通过。Windows DLL 的原生链接、注册/注销与 COM 激活已在远端 runner 通过一次；100 次 G1 生命周期门禁与真实宿主运行仍待验证。Commit effect scope 与 host journal 已按 `(client_instance_id, session_id, commit_id)` 绑定，并有 I2/I4/I5/I6 合并 oracle 用例。真实宿主验证仍未执行；现在只能把 DLL 作为待验证的透传探针，不能当作可用中文输入法。

**验收边界说明：**三份文档同时要求 Slice 0 完成 M01–M26，又将真实 TSF/I1 和 OS 身份认证放在后续集成。执行时将 M01–M26 在 Slice 0 全部做成可重放的模型/假 Adapter 场景，其中 M24/M25 验证认证策略和失败转换；到 Slice 1/2 再用真实签名、pipe security context 和 callback 等待图复验 M24/M25 与 I1。模型通过不得标作真实平台验证通过。

## 0A：仓库与构建基线

1. 建立 ADR-006 冻结的目录与 Cargo workspace：`crates/{protocol,runtime-core,broker,broker-transport,rime-sys,rime-adapter,windows-tsf,candidate-ui,test-harness,xtask}`、`apps/control-center`、`native/tsf-stub-cpp`。尚未实现的组件仅保留目录/清晰的占位配置，不提前制造可运行假实现。
2. 入库 `rust-toolchain.toml` 及 Windows 侧版本清单，固定 Rust、MSVC、Windows SDK、Windows App SDK、.NET SDK、LLVM/libclang、CMake/Ninja 和关键构建工具版本。正式目标先用 `x86_64-pc-windows-msvc`；release 设置 `panic = "abort"`，dev/test 使用 unwind。
3. 让 `xtask` 成为唯一顶层编排入口；`build.ps1` 只转发参数。CI 检查 Cargo 依赖图：`runtime-core`/`protocol` 不依赖 Windows、Tokio、librime，Broker 不直接依赖 `rime-sys`，Stub 不嵌 Tokio。
4. `protocol`、`runtime-core` 启用 `#![forbid(unsafe_code)]`。先提供 `cargo test`、nextest、fmt/clippy 和纯核心 Miri 的入口；Windows/FFI 检查单列为平台任务。

**提交物：**可重复的空骨架构建、依赖图检查、工具链清单、最小 CI。工具链的具体版本在初始化时以可安装且能通过 CI 的版本锁定，不由 runner 镜像隐式决定。

## 0B：纯状态机最小闭环

1. 在 `protocol` 定义 ID newtype、epoch、版本、消息身份和稳定 reason code；区分 wire 类型与 RuntimeCore 内部状态。所有跨进程 mutating 消息携带 `client_instance_id / broker_generation / session_id / focus_epoch / composition_epoch / request_seq`。
2. 在 `runtime-core` 建立 `RuntimeState`、封闭 `Event`、`ImmediateReply`、`Effect`、`TransitionResult`。先用约 300–500 行最小骨架表达 `State + Event → State' + ImmediateReply? + Effect[]`，再逐步扩展，避免一次写完整业务表。
3. 由 state 内计数器分配 Session/Request/Timer/Effect ID；时间和外部观察值只能作为 Event 输入。Effect 执行结果必须带 `EffectId` 回流，经 `OutstandingEffects` 的 result class 与 scope gate 消费。
4. 建立 reducer 统一出口检查：Effect batch 规范与 S2、callback 回复 S3、瞬态 obligation S1、Effect 权限、非法状态恢复。相关 Effect 有先后依赖时拆成多个 transition，不靠同一列表的顺序完成事务。
5. 同步实现 canonical trace、结构状态 hash、逐事件 replay 与最小 fake Effect Adapter。trace 不记录真实输入文本；输入相关诊断用合成 token ID。

**验收：**相同初态和事件序列运行 100 次，状态 hash、每步回复、Effect ID/顺序与 trace 逐字节一致；每个 trace prefix 满足 S1–S3；未知 Effect ID/result class/scope 不产生 host mutation。

## 0C：核心业务转换

按以下顺序扩展 reducer；每组转换和相应 race 测试一同落地：

1. **按键同步契约：**固定容量/TTL 的 `KeyDecisionLedger`；test 创建决策，actual 匹配唯一最早未消费项。零匹配、多匹配、过期、特征冲突、容量满一律同步 Pass。auto-repeat 的真实关联策略留给 Slice 1 discovery。
2. **身份与生命周期：**Broker 认证和 generation 分开；先按 incarnation/session/effect gate 判旧，再按 focus/composition/request gate 判旧。focus ambiguity、context 变化、关闭与断连只能 cancel/close/passthrough/fresh start。
3. **统一排序：**Stub 为按键与 `UIActionIntent` 分配同一 `request_seq`，一个 session 同时只允许一个 mutating request 在途；旧 view、旧 geometry、乱序消息丢弃。
4. **原子 host commit：**只有 commit reducer 生成 `ApplyHostCommit`。ActiveCommitLedger 管在途真相，bounded terminal cache 吸收 late duplicate；`Applying` 不重复执行，`Applied` 可重发 ACK，`Rejected` 不隐式重试，`Indeterminate` 关闭旧 session。RuntimeCore 只观察 succeeded/rejected/indeterminate 三类终态，不观察 COM 中间步骤。
5. **有界故障处理：**mutating/control/physical bookkeeping 三类队列分别建模；deadline、容量超限、Broker hang/crash 均收敛到文档列出的合法状态，保持 keyup/modifier 清理路径。

**验收：**I2、I3、I6 的机械 oracle 通过；M01–M18、M26 及 commit/focus/timeout 的 success、reject、indeterminate 交错可由 deterministic scheduler 复现。I1 在此阶段只检查模型不存在同步等待 Effect；真实 callback 非阻塞性留给 Slice 1 平台集成测试。

## 0D：部署与 Memory 模型

1. 实现 `Preparing → Draining → Activating → PersistingCommit → Stable` 与 rollback。Draining 取消现有 composition，新 session 暂时 passthrough；旧 runtime 引用归零后才激活。`RuntimeActivated` 仅进入 `PersistingCommit`，D6 journal 持久化成功才更新 `last_committed`。
2. fake 持久化 Adapter 控制 source/generated/active pointer/journal 各故障点；启动时只选择完整已提交的 deployment。无法确认上一版本时禁止接收新 session。
3. 建立独立 Memory pure model，验证 `EventDot(replica_id, seq)`、version vector 和 A→B→C→A 回环去重。此阶段不接入实时排序、持久化或同步 UI。

**验收：**I4、I5 与 M19–M23 通过；deadline 参数 10/100/250 ms/2 s 下重跑 M02、M05、M19，失败路径可以不同，但最终 invariant 不变。

## 0E：Slice 0 出口验收

- M01–M26 全通过；每个 Effect 的 success/reject/indeterminate 都由确定性调度器覆盖。
- proptest 对合法和恶意事件生成 trace；每个 seed 至少 1,000 步，固定回归 seeds 进入 CI，失败 trace 能 shrink 并稳定重放；覆盖全部 reducer 分支。
- 每个 prefix 验证 I2–I6 中适用的模型性质、S1–S3 和合法状态集合；把 I1 报告明确标为集成验证待完成项。
- 对无效 epoch/generation、未知 effect、重复 terminal、错序 request 证明只产生诊断或安全降级，不发生 host mutation。
- 输出场景矩阵、覆盖率、回归 seeds 与 trace 格式说明。通过后才让真实 TSF Adapter 替换 fake executor；仍使用 fake Broker。

## Slice 1–5：集成路线

### Slice 1：TSF discovery 与最小 Adapter

用 Rust + windows-rs 做两周 spike，测真实 TestKey/Key/auto-repeat、edit session 内 commit/termination、caret geometry、DPI、context stack、callback 等待图、COM apartment 和生命周期。G1–G10 必须全部通过；任一失败，Stub 改用 thin C++ raw COM + WRL/项目自有最小 RAII，RuntimeCore 与协议保持原样。真实 callback 以 duration、等待图和 Broker kill injection 验证 I1；测试宿主至少覆盖 Word、Chromium 和传统 Win32。

**G9/G10 交付内容（语言栏与模式状态）：**

- 中英状态由 `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` 统一表示，激活/切换/停用时与 Windows 输入指示器同步；全/半角与中文标点等走 `GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`。
- Ctrl+Space、Shift 等切换键经 `ITfKeystrokeMgr::PreserveKey` 注册、由 `OnPreservedKey` 处理，只产生一次同步决策，不阻塞 host callback。
- 语言栏模式按钮用 `ITfLangBarItemButton` + `ITfLangBarItemMgr::AddItem` 注册，图标/文本随模式更新；右键 `GetMenu` 提供自定义 `ITfMenu`。菜单项不直接 mutation 引擎，只产生带 session/revision 的 UI intent。
- **分应用状态记忆（跨重启持久化）**：中/英状态按前台应用（前台进程/session）分别记忆，切回该应用时恢复，并跨重启持久化；应用身份从 OS 获取，不信任 wire payload；该记忆必须可机械测试，并在宿主关闭/崩溃后收敛到合法默认值。记忆的 key、存储位置、owner 与恢复语义在实现阶段确定。
- 悬浮状态条/候选窗不在 Slice 1，留给 Slice 4 的 `candidate-ui`；Slice 1 只做 TSF 语言栏项。

### Slice 2：安全异步 IPC

Broker 侧用 Tokio named pipe；Stub 侧用 Win32 overlapped I/O，不创建 Tokio runtime。完成 OS security context 身份获取、受信签名策略、协议版本/envelope、frame 上限、单在途 mutation、有界队列、deadline、stale response gate 和 UI intent 回路。用 fake Broker 测断连、乱序、重复、恶意 frame 与同 SID 伪客户端。

### Slice 3：librime Broker

将 checked-in `bindings.rs` 放进 `rime-sys`，只允许显式 `cargo xtask bindgen-rime` 更新；`rime-adapter` 封装生命周期、编码、线程和错误，Broker 只依赖安全 `RimeEngine` trait。先用 fake engine 保持协议测试稳定，再接 librime；并行 capability test 通过前保持 serial lane。学习仅在 `CommitApplied` ACK 后执行。

### Slice 4：候选窗与宿主故障矩阵

候选窗使用不抢焦点的 native HWND、独立 UI 线程、DirectWrite 文本布局和 Direct2D 绘制；只消费带 session/epoch/revision 的不可变 view model。测试 Win32、Electron、现代文本框、RDP、elevated host、锁屏恢复、多用户切换、DPI 变化和 Candidate UI stall。

### Slice 5：Deployment 与 Control Center

将 Slice 0 部署状态机接入真实 prepare/activate/journal/rollback，验证 D6 前后中断恢复。D6 通过后实现 C# + WinUI 3 Control Center；GUI 只经 Broker/Deployment Manager IPC 写 managed overlay，不直接写 YAML。安装器方案另立 ADR，且须交付 Windows App SDK runtime 与原生产物。

## 首批工作包

按依赖顺序开工：

1. `0A-1`：初始化 workspace、工具链 pin、`xtask` 与依赖边界 CI。
2. `0B-1`：协议 ID/newtype、状态/事件/回复/Effect 类型及最小 reducer。
3. `0B-2`：OutstandingEffects、确定性 ID、S1–S3 检查、trace/replay。
4. `0C-1`：KeyDecisionLedger 与 focus/session 门禁，先完成 M10、M13–M15。
5. `0C-2`：统一 request 顺序与 commit ledger，完成 M05–M08、M16–M17。

每个工作包以可运行测试和可复现 trace 收尾；发现需要改变 owner、commit point、identity gate、failure state 或 source of truth 时，先更新 ADR，再修改实现。
