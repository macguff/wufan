# Windows TSF probe

## 当前状态（2026-09-30）

当前 DLL 是开发集成版本：TSF → Win32 overlapped IPC worker → Broker/librime → apartment view → 异步 edit session → host ACK 已连通。实际 WPF 和 WinForms/Win32 EDIT 输入已通过连续中文提交、数字候选、Escape 和文本框切换；这不等同于 G1–G10 或全部多宿主验收通过。

| 验收 | 实际结果 |
| --- | --- |
| `nihao woaini ` | `你好我爱你`，两个独立 commit 和 Applied ACK |
| `ni2` | librime 第二候选 `拟` |
| `ni`、Escape、`hao ` | `好` |
| 第一框输入 `ni` 后切到第二框输入 `hao ` | 旧 preedit 清除，第二框提交 `好` |
| 管道诊断 | `nihao ` → `你好`、`zhongwen ` → `中文` |

回调计数定位到一个 Test/Actual 配对问题：宿主可重复 Test 或仅 Test 而不调用 Actual；原先拒绝未消费 Test 之后的 Test 会丢键。开发 lane 现保留最新观察，Actual 检查 VK、LPARAM 特征、modifier 和 100 ms TTL，仅消费一次。新增三个回归用例。RuntimeCore 的模型 ledger 保持原有语义。

工作区 `cargo xtask ci` 通过：96 项测试（RuntimeCore 49、harness 31、TSF 12、Broker 4）、Clippy、格式和依赖边界检查；Windows x64 release DLL/Broker 编译通过。修复后累计三次完整 WPF 场景通过；最后一次 trace 记录 29 个 Actual 全部匹配并排队、29 个 view 交付和五次 Applied ACK，临时 TIP 注销成功。重复验收也发生两次前台焦点保护中止，这些不算通过；脚本现于 profile 激活后重新聚焦测试窗口，每次注入仍检查目标窗口。

## 提交 owner 增量接入（2026-09-30）

按 [ADR-007](ADR-007_HOST_COMMIT.md) 将真实提交接到 RuntimeCore 内的 `host_commit` 专用纯 reducer。worker 在 view 之前交付真实 Open/Request identity；Intent 必须匹配当前 scope 和在途请求。Apply effect 创建执行 guard，guard 析构时将完整 effect/scope 和宿主终态回流，由 reducer 形成 ACK。未执行的队列项被清空时为 Rejected；可能已写入的失败为 Indeterminate；完整成功有本地 HostRevision。旧 scope 失效后仍保留已授权结果义务，重复 terminal 不重复写入，32 项缓存淘汰后高水位继续拦截旧 commit。

本轮 `cargo xtask ci` 通过 103 项测试（RuntimeCore 55、harness 31、TSF 13、Broker 4），包括六个纯 commit reducer 用例及一个覆盖五种执行/焦点组合的 RAII guard 用例；Clippy、格式、依赖边界和 Windows x64 release 构建通过。新路径实际 WPF 完整场景通过，`target/tsf-core-commit.result` / `.trace` 保存证据，trace 收到五次 Applied ACK。当前仍未统一完整 RuntimeState 的按键/preedit/request 及 canonical trace；不能标作总状态机集成全部完成。

## 统一输入生命周期（2026-09-30，ADR-008）

实际 TSF 现由单个 RuntimeCore `input_lifecycle::State` 统一管理 Test/Actual、资格预测、32 项输入 FIFO、ticket、wire request_seq、preedit、commit 和 request completion；ADR-007 的 commit ledger 成为该状态内部组件。独立 TestDecision、composition 预测和 CommitOwner 已移除，旧 Slice 0 RuntimeState 不再用于实际 callback。

请求按 `Engine → Editing → AwaitingTransport → Finished` 执行。所有 Show/Hide/commit 都携带 HostEditGuard，编辑拒绝或失效会回传结果；worker 等待每个本地 host result，commit 还要收到正确的 wire Acknowledged，之后才发 Finished。只有匹配的 Finished 才释放下一键。旧 commit 的结果义务仍被保留，但在它完成前，新 scope 只排队，不派发。

工作区 108 项测试通过（RuntimeCore 63、harness 31、TSF 10、Broker 4）；旧 Adapter 的三个按键测试迁至纯 lifecycle 回归，新增八个覆盖 FIFO/容量、Test 配对/TTL、preedit 完成顺序、ACK 序号、旧 scope、deadline 和重放的用例。guard 回归现覆盖 preedit/commit 各五种执行与失效组合。Clippy、格式、依赖边界与 Windows x64 release 构建通过。

标准 WPF 场景通过，但旧焦点场景在切换前出现 Test 没有 Actual，可能未形成 preedit，因此未将其作为旧 preedit 清理证据。补充 Escape/焦点切换前的真实 Rime preedit 断言，并让程序化 Clear 先稳定 300 ms 后，严格版分别在 80 ms、30 ms 和零间隔输入下全部通过：连续提交 `你好我爱你`、数字候选 `ni2` → `拟`、Escape 后输入 `好`、切换文本框后旧 preedit 清除且新框输入 `好`。每轮 29 个 Actual、accepted、view、host result 和 Finished 一致，均收到五次 Applied ACK。

证据分别保存到 `target/tsf-unified-strict.result` / `.trace`、`target/tsf-unified-30.result` / `.trace`、`target/tsf-unified-0.result` / `.trace`。临时注册辅助进程报告 `Cleanup exit code: 0`，HKLM TIP 注册项已不存在。这些结果覆盖 WPF 合成按键场景；物理键、auto-repeat 和其他宿主仍待验收。

新增 `-KeyDelayMs 0` 可在每组场景内连续注入按键、检验 FIFO；默认仍为 80 ms。每个注入保留前台窗口检查。设计边界见 [ADR-008](ADR-008_INPUT_LIFECYCLE.md)；canonical trace 格式迁移、原总模型对照和多宿主实际验收仍待完成。

## 注册与本地验收

### WPF 与 Win32 EDIT 共用验收（2026-09-30）

`tools/tsf-input-smoke/Scenarios.cs` 保存同一套严格场景，`Host.Wpf.cs` 和 `Host.WinForms.cs` 分别适配两种文本控件。Win32 EDIT 的 `Text` 只反映已提交文本，真实 preedit 通过 TSF/IMM bridge 的 `ImmGetCompositionStringW(GCS_COMPSTR)` 查询；Escape 和焦点切换前仍必须验证 `ni` 与 inline 候选标记，切换后同时检查旧 composition 和已提交文本为空。

新增只读 apartment 消息窗口探针：`WM_APP + 0x574` 返回 ready/idle 两位状态，`WM_APP + 0x575` 返回已接受输入计数。查询只读本地状态，不调用 COM、等待 IPC 或驱动 reducer。idle 包括无在途请求、无排队键、无旧 commit 义务。脚本只查询当前线程的 `Wufan.BrokerPump.*` 窗口，等待本批接收计数达到预期且管道空闲后才检查结果。每个等待有 5 秒截止时间，每次注入仍校验前台窗口。

本轮先发现原生 EDIT 的 preedit 不能用 `Text` 读取，以及固定等待时间会在异步请求未完成或 WPF Clear 重建 context 后过早开始下一场景。增加原生 composition 查询及状态屏障后，以下四组全部通过；早前的前台保护中止和断言失败均不计为通过。

| 宿主 | 输入间隔 | Actual / accepted / view / host result / Finished | Applied ACK |
| --- | --- | --- | --- |
| WPF | 80 ms | 44 / 44 / 44 / 44 / 44 | 7 |
| WPF | 0 ms | 44 / 44 / 44 / 44 / 44 | 7 |
| WinForms / Win32 EDIT | 80 ms | 44 / 44 / 44 / 44 / 44 | 7 |
| WinForms / Win32 EDIT | 0 ms | 44 / 44 / 44 / 44 / 44 | 7 |

表中为新增 repeat/Broker 故障场景后的最新结果，原基础场景为每组 29 个请求和五次 Applied ACK。证据为 `target/tsf-{Wpf,WinForms}-{80,0}.result` / `.trace`。辅助进程报告 `Cleanup exit code: 0`，HKLM TIP 项已不存在。本机 108 项测试和检查及 Windows x64 release 构建通过。Windows CI 已增加两类 C# 宿主的编译检查；远端 CI 和桌面矩阵不是同一验收，新增 CI 配置尚未推送运行。Word、Chromium、其他 Win32 控件、物理键盘长按、Broker hang/请求在途/提交 ACK 窗口的故障矩阵仍待验收。

### Repeat 与 Broker 退出/重启验收（2026-09-30）

共用场景在真实 `nihao` preedit 下注入三次 Backspace down、最后一次 keyup，再提交 `你`。WPF 的原始消息观察确认两次 WM_KEYDOWN bit 30，但其 TSF 转发 LPARAM 归一化，TSF repeat 计数为 0。Win32 EDIT 的 TSF 入口确认两次 repeat，而消息在 WinForms filter 前被 TSF 消费，filter 可见计数为 0。两个入口中至少一个必须确认重复标志，三个编辑请求必须全部完成；不能将普通三次 tap 当作 repeat 通过。键始终在 finally 释放。本轮是注入式重复键验收，物理键盘长按与系统 repeat 速度设置仍待验证。

Broker 故障场景先确认 `ni` preedit，再终止脚本创建且校验过可执行路径的 Broker 进程。在未注入新按键的情况下，等待旧 preedit 清空及 session 不再 ready；随后 `abc` 三键透传，accepted 计数不变。启动同一登录上下文的新 Broker，等待就绪后输入 `hao `，最终必须为 `abc好`，不能重放旧 `ni`。

修复了验收发现的三处问题：

- worker 每次空闲队列轮询超时后执行 overlapped pipe 的 `PeekNamedPipe`，主动发现 Broker 退出；空闲时出现未请求数据同样按协议错误关闭。该检查只在 worker 运行，不进入 TSF callback，也不验证活进程 hang。
- transport reset 失效输入/编辑授权并清理旧 composition，同时保留仍聚焦的 context。focus/context/关闭边界仍清除 binding，防止重连后首键因重新 bind 被透传。旧清理不会覆盖重入产生的新 context。
- host_commit terminal 查找键增加 Broker generation，避免新 Broker 重用 client/session/commit 编号时撞到旧缓存；扩展原有 scope 回归验证编号重用可获新授权、旧结果不能完成新 effect。

trace 计数新增第 12 项 `repeated`；消息探针 `WM_APP + 0x576` 返回 TSF 已接受 repeat 数。本轮每组有 47 次 Test、44 次 Actual/accepted/view/host result/Finished，差值为 Broker 停止时透传的三个英文键。七次 Applied ACK 包括原五次、repeat 后的 `你`、重启后的 `好`。

在 VS Developer PowerShell 中运行：

```powershell
.\scripts\tsf_input_matrix.ps1
# 已构建时可加 -SkipBuild；单宿主可加 -HostKind WinForms。
```

脚本先编译宿主，再弹出一次 UAC 临时注册；所有输入宿主运行在调用者权限下，结束时通知辅助进程注销。默认覆盖两类宿主的 80 ms 和零间隔输入。

### 技术内测故障恢复门槛（2026-09-30）

依据 [TECH_BETA_PLAN.md](TECH_BETA_PLAN.md) 的 B2，在独立编译的 `fault-injection` Broker 上增加以下验收；普通 release 已实测拒绝 `--dev-fault`，不包含可执行注入逻辑。

1. **真实整进程挂起：**仅暂停脚本自建 Broker，发送一个在途键；2 秒 I/O deadline 后 epoch 失效、宿主可继续输入 `abc` 且不接收引擎键。finally 必须恢复进程；恢复后 `hao ` 提交 `abc好`，旧键不重放。
2. **引擎处理后 reply 停顿：**Broker 真实引擎处理输入后写入阶段 marker，延迟回复 5 秒；客户端期限到达后取消并排空旧 I/O。新 session 输入 `hao ` 只提交 `好`。
3. **在途 Broker 退出：**同样等待引擎处理后的 marker，但在回复前终止脚本自建进程。新 Broker 恢复后只提交新输入的 `好`。
4. **宿主已提交后的 ACK 停顿：**实际 `你好` 已写入宿主，Broker 已处理 Applied ACK，再延迟 wire Acknowledged。客户端期限失效后 `你好` 必须保留一次；新输入 `hao ` 得到 `你好好`，不能重复旧 commit。

marker 不含文本。reply/ACK 停顿由显式 feature 和 CLI 启用，进程级一次性触发，单独输出到 `target/fault-acceptance`；不是普通 release 的运行开关。宿主只读探针 `WM_APP + 0x577` 查询 transport epoch。50 ms UI 采样设置 500 ms 最大间隔门槛，用于发现等待期间明显阻塞；它不替代 I1 的逐 callback duration 门禁。

| 宿主 | 间隔 | Test / accepted / view / host result / Finished | Host Applied ACK | 最大 UI 采样间隔 |
| --- | --- | --- | --- | --- |
| WPF | 80 ms | 75 / 69 / 66 / 66 / 65 | 12 | 73 ms |
| WPF | 0 ms | 75 / 69 / 66 / 66 / 65 | 12 | 68 ms |
| Win32 EDIT | 80 ms | 75 / 69 / 66 / 66 / 65 | 12 | 77 ms |
| Win32 EDIT | 0 ms | 75 / 69 / 66 / 66 / 65 | 12 | 79 ms |

差值有明确原因：六个英文键在 Broker 退出和暂停期间透传；三个已接受的在途输入没有 host view 而失效；一个 commit 的宿主结果为 Applied，但 wire ACK 超时，没有成功 Finished。该 Applied commit 不能按失败输入重放。12 条是宿主 Applied ACK 记录，不表示 12 次 wire Acknowledged 全部成功。

四组结果为 `target/tsf-{Wpf,WinForms}-{80,0}-faults.result` / `.trace`，marker 在对应 `-faults-markers/`。矩阵脚本现同时保存 `.console.log` 供后续运行记录 UI 采样数据。本轮有一次在故障场景开始前被前台保护中止，不计通过。两次成功注册批次均注销；HKLM TIP 不存在，验收 Broker 无残留。108 项测试、格式/Clippy/边界检查、fault feature Clippy、普通及验收 Broker release 构建通过。新增 CI 检查尚未推送运行。

```powershell
# VS Developer PowerShell，保持验收窗口在前台；一次 UAC 临时注册。
.\scripts\tsf_input_matrix.ps1 -FaultScenarios
```

仍待：Chromium 实际 TSF 输入、安装/卸载/回滚、干净机器包验收、当前 worker 生命周期门禁、逐 callback duration、物理长按、提交中间写入与宿主关闭等更完整交错。B2 本轮通过不等同于内测包已可发布。

### 单宿主注册与运行

注册参考 [Weasel Register.cpp](https://github.com/rime/weasel/blob/master/WeaselTSF/Register.cpp)，通过 `ITfInputProcessorProfileMgr::RegisterProfile` 安装。HKLM CTF TIP 注册需要实际 UAC 管理员权限；普通 medium token 即使账号属于 Administrators 也会返回 `E_ACCESSDENIED`。测试激活需要 `TF_IPPMF_ENABLEPROFILE`。conversion compartment 使用 `VT_I4`。

先在 VS Developer PowerShell 准备后端：

```powershell
.\scripts\fetch_rime.ps1
cargo build --locked -p ime-windows-tsf -p ime-broker --release --target x86_64-pc-windows-msvc
```

推荐让 UAC 辅助进程只负责注册，WPF 宿主在普通用户权限下运行。以下命令在仓库根目录执行；辅助进程完成后自动注销，异常时最多等待三分钟：

```powershell
$wrapper = Join-Path $PWD 'scripts/run_tsf_smoke_elevated.ps1'
$ready = Join-Path $PWD 'target/tsf-registration.ready'
$done = Join-Path $PWD 'target/tsf-registration.done'
Remove-Item -LiteralPath $ready, $done -Force -ErrorAction SilentlyContinue
Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -ArgumentList @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$wrapper`"", '-RegisterOnly'
)
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path -LiteralPath $ready)) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Registration helper did not become ready' }
        Start-Sleep -Milliseconds 200
    }
    powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File .\scripts\tsf_input_smoke.ps1 -SkipBuild -RegistrationReady
} finally {
    [IO.File]::WriteAllText($done, 'DONE')
}
```

测试打开两个 WPF 文本框；保持该窗口在前台。脚本启动同一登录上下文下的 Broker，显式批准 PowerShell 宿主，设置测试中文模式，并在结束时停止 Broker。`target/tsf-desktop-smoke.result` 保存结果；`target/tsf-registration-helper.result` 保存注销结果；Broker 日志在 `target/tsf-broker.out` / `.err`。加 `-CompileOnly` 可只编译 C# 宿主。

沙箱命令可能获得不同 AuthenticationId；Broker 与客户端需在同一父进程登录上下文下启动。测试强制中文模式绕过持久化偏好，未验收模式记忆或 Windows 指示器。

## 生命周期与待完成边界

- COM 对象留在 apartment；IPC worker 只传 owned 消息。键回调执行本地判断和有界 `try_send`。消息窗口用 10 ms timer 交付 view，旧 epoch 回复丢弃。
- 每个 context 一次在途 write session，最多 32 项 FIFO；独立 edit ticket 在 context/focus 切换时失效。只清除成功写入的 preedit 范围，延迟 cleanup 不碰已终止 composition。sink 使用 weak 引用，edit session/sink 计入 COM 生命周期。
- 实际按键、preedit、请求和内部 commit ledger 已由单个 input_lifecycle owner 管理。只有统一 reducer 的 Apply 授权允许创建 HostEditGuard；所有宿主结果回流后才确认并完成请求，旧 commit 不隐式重试。原 RuntimeState 模型对照和 canonical trace 格式迁移仍待完成。
- worker 不在 callback 内 join；DLL 固定到宿主进程结束。历史提交 `ba9866e` 的 100 次 G1 生命周期 CI 已通过，当前 worker 版本的生命周期门禁需要重新验收。
- 开发 pipe 仅批准明确的宿主路径和同一 OS 登录上下文；生产签名信任策略未实现。librime 学习明确禁用，ACK 后学习未实现。
- 候选暂时显示在 inline preedit；独立候选窗与真实 D6 部署门槛仍待完成。内测打包/安装控制器已有实现和内部候选 ZIP，真实安装事务尚未验收，详见 TECH_BETA_PLAN。本文已记录 WPF/Win32 EDIT 的四种故障矩阵通过；Word、Chromium、其他 Win32 控件、物理按键/auto-repeat、callback 等待图及更完整故障交错仍待验收。
- 语言栏中/英按钮、右键菜单、Ctrl+Space 和按宿主 exe 的模式记忆已有代码；实际多宿主同步仍待验证。

后端版本、固定来源与运行说明见 [RIME_BROKER.md](RIME_BROKER.md)。完整阶段门槛见 [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md)。
