# 小范围技术用户内测包

2026-09-30：用户指定此交付目标。目标是可安装、可卸载、可恢复，并能在已验收应用中输入中文的 Windows x64 内测包。达到下面出口条件后再发布；当前开发 DLL 不作为已完成内测包。

## 首版范围

- Windows x64；具体 Windows 10/11 版本在机器验收后列入支持表。
- 固定 librime 1.17.0、`wufan_pinyin`，中文提交、数字选词、Escape、焦点切换与退格。
- 首版保留 inline 候选，禁用学习；按明确应用路径授权开发客户端。
- 仅邀请愿意按说明安装、反馈应用版本与故障信息的技术用户。
- 发布包默认不记录输入文本，不设置验收专用键盘注入或 trace 环境变量。

## 发布出口

| 门槛 | 当前状态 | 验收要求 |
| --- | --- | --- |
| B1 输入闭环 | WPF、Win32 EDIT 已通过 | 80 ms/零间隔、真实 preedit、连续提交、候选、Escape、焦点、注入式 repeat |
| B2 故障恢复 | 本轮两类宿主四组通过 | 整进程暂停/恢复、key reply deadline、在途进程退出、已提交后的 wire ACK deadline；旧键/旧 commit 不重放，宿主继续响应。证据见 WINDOWS_PROBE；完整故障矩阵与 Chromium 仍单独验收 |
| B3 常用宿主 | Chromium 未验收 | 在实际 Chromium 文本控件及 textarea 验收；记录浏览器版本、Windows 版本、输入结果；不使用 DOM 写入代替 TSF 输入 |
| B4 可交付包 | 打包实现，许可与干净机器出口待完成 | 独立无故障 feature release 构建、固定 payload、逐文件 SHA256/源码摘要、固定 ZIP 顺序/时间戳、锁定依赖许可清单；不拷贝开发 user 数据。原生插件等许可尚未收齐，候选包 distributionReady=false；跨环境 PE 重编译字节一致性不在本轮承诺范围 |
| B5 安装/卸载/回滚 | 控制器实现，真实安装事务待验收 | 同账号 UAC 注册、普通用户 Broker、白名单与可选登录启动、版本独立 Rime 部署目录、持久化事务及失败补偿/Recover、上一版本与配置恢复；卸载保留文件/数据，移除所拥有 TIP/COM/启动项。UAC、升级、失败恢复与卸载尚未实际运行 |
| B6 生命周期与使用说明 | 使用说明已补，门禁待完成 | BETA_PACKAGE.md 记录打包、安装、启动、配置、退出、回滚、中断恢复、卸载和反馈；当前 worker 版生命周期门禁与干净机器验收待完成 |

发布包不得包含 `fault-injection` Broker、验收 marker、测试结果或构建缓存。故障 Broker 单独编译到 `target/fault-acceptance`；普通 release 必须拒绝 `--dev-fault`。故障通过不替代安装事务与生产签名策略；该版仍是明示范围的技术内测。

此前 B2 实际结果：WPF 与 Win32 EDIT，各在 80 ms 和零间隔下通过全部四种故障。普通 release 已实测拒绝注入参数；本机 108 项测试、Clippy/格式/依赖边界、故障 feature 的 Clippy 与两种 Broker release 构建通过。这些结果不覆盖本轮新安装脚本；当前尚无可发布安装包，B3–B6 的实际出口仍需完成。

本轮实现入口：`scripts/package_beta.ps1` 与 `tools/beta/wufan.ps1`；使用说明见 [BETA_PACKAGE.md](BETA_PACKAGE.md)。Broker 新增 `--user-data` 与 `--ready-file`，控制器在独立数据目录完成部署并确认就绪后再完成安装。注册 helper 在同一进程内调用 DLL 导出并持有机器注册互斥锁，避免控制器中断后 regsvr32 子进程继续修改注册；现有不受管理的开发 TIP 与其他账号安装均拒绝覆盖。PID 持久化前的窄中断窗口通过预写唯一启动 token 恢复识别。此设计尚需真实事务验收，不将静态语法检查或构建完成计为安装成功。

本轮已完成独立 Windows x64 release 构建并生成内部候选 ZIP；四个 PowerShell 入口的静态解析无语法错误。新脚本未运行 UAC 安装、升级、失败注入、回滚或卸载，也未进行重复 ZIP 字节对照和干净机器验收；此前 108 项测试不作为本轮新增脚本的验收结果。已获得固定提交的 glog、leveldb、yaml-cpp、marisa、OpenCC 和两份词典许可原文，并从固定二进制 version-info 发现 Lua/octagram/predict 插件依赖；剩余许可继续列入清单。`-ReleaseReady` 同时要求许可完成及 `config/beta/release-gates.json` 实际出口全部 accepted，默认候选包禁止据此宣称可发布。

## 实现顺序

### 2026-09-30 提交检查点

- 本次提交包含此前积累的 Broker/librime 接入、TSF 异步客户端与宿主 ACK、统一输入生命周期、两类宿主输入/故障验收工具，以及本轮打包、启动配置和安装事务实现。
- 被中断的最后一次打包已生成 ZIP 与 SHA256 文件：`target/beta-packages/wufan-0.1.0-beta.1-windows-x64-9ce09746347b.zip`，大小 5,953,309 字节；SHA256 为 `243e9c69b9d43f439de557ffa82566bb1f0790ab133f1e2470891ae03e61efd2`。归档位于忽略的 target 下，Git 仅保存实现、来源清单与本进度记录。
- 该内部候选的 manifest 记录 `distributionReady=false`、`licenseReviewComplete=false`；Rust 依赖许可文本缺项为空，原生插件/数据/项目许可审查仍未完成。归档生成时基于 `fc1a587` 加未提交工作区内容，manifest 保存了源码内容摘要；本检查点文档编辑后的提交需在下一次打包时生成新的快照标识。
- 本轮独立 release 构建和候选包生成完成；尚未执行真实 UAC 安装、升级、卸载、失败恢复或回滚，也未验收 Chromium、干净机器、worker 生命周期和重复归档字节一致性。当前不是可分发内测版本。
- 下一步先完成同账号普通用户控制器的安装事务验收，覆盖首次安装、升级失败补偿、进程/控制器中断后 Recover、手动回滚和保留数据的卸载；随后完成 Chromium、许可清单、重复打包与干净机器出口。

### 后续顺序

1. 验收已实现的安装、升级、失败补偿、中断恢复、回滚与卸载闭环，修复事务阻塞。
2. 完成 B3 Chromium 实际输入、许可清单及重复归档一致性验收。
3. 在干净机器运行包，完成当前 worker 生命周期门禁，并补齐支持环境和使用说明。
4. 所有出口完成后形成具体发布候选包和验收报告，再执行用户授权的发布。

详细开发证据见 [WINDOWS_PROBE.md](WINDOWS_PROBE.md)；总体架构边界见 [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md)。独立候选窗、学习、Word、ARM64 和普通用户更新体验列入后续版本，不将其标记为本版已实现。
