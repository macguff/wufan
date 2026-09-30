# Wufan Windows x64 技术内测包

当前为内部候选包：安装脚本已实现，UAC 安装/升级/回滚与干净机器验收尚未完成。第三方原生依赖和词典许可清单尚未收齐，`manifest.json` 的 `distributionReady` 为 `false` 时不得分发给内测用户。Chromium 尚未验收；已验收宿主是开发验收工具中的 WPF TextBox 与 Win32 EDIT。

## 环境与范围

- Windows x64，使用 64 位 Windows PowerShell 5.1。安装者须属于本机管理员组，但下面命令从**普通权限**终端执行。UAC 必须使用同一 Windows 账号；首版不支持标准用户输入另一管理员账号的安装方式。
- 干净机器需安装 Microsoft [Visual C++ x64 Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe)。包内不捆绑该安装程序。
- 固定 librime 1.17.0、`wufan_pinyin`，inline 候选，学习关闭。应用按实际宿主 EXE 的绝对路径授权；路径变动后需重新配置。此开发授权策略不等同于生产签名校验。
- 包内 DLL 与 Broker 相邻，不依赖仓库、Rust、Visual Studio 或 7-Zip。Broker 以普通用户权限运行；TSF 注册 helper 单独请求 UAC。首版每台机器仅允许一个 Windows 账号拥有此 TIP，并限定该账号的单一登录会话使用。
- 下载后先通过发布者提供的可信渠道核对 ZIP SHA256，再解压。manifest 可以检测内容损坏，不能证明发布者身份；候选包尚未签名。

## 安装和配置

在解压目录打开普通权限的 64 位 PowerShell。`$hostExe` 替换为实际要输入中文的桌面应用 EXE；空白白名单不会接收普通应用的按键。

```powershell
$hostExe = 'C:\Path\To\YourApplication.exe'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\wufan.ps1 -Action Install -ClientExe $hostExe
```

允许同一账号的 UAC。安装会复制到 `%LOCALAPPDATA%\Wufan\beta\versions\<版本>-<manifest摘要>`、注册 TIP、在独立用户数据目录部署词典，然后启动 Broker。首次部署最多等待 90 秒。注册或启动失败会尝试恢复此前状态；失败详情保留在事务及 helper 结果中。

用 `Win+Space` 选择 **Wufan 技术内测**；语言栏菜单切换中文/英文模式，也可使用 `Ctrl+Space`。在所授权的实际宿主中输入 `nihao` 后空格提交。仅已完成宿主验收的应用可列为支持范围。

更改完整白名单，用 PowerShell 数组传给控制器：

```powershell
& .\tools\wufan.ps1 -Action Configure -ClientExe @('C:\Path\App1.exe', 'C:\Path\App2.exe')
& .\tools\wufan.ps1 -Action Configure -ClientExe @()  # 清空白名单
& .\tools\wufan.ps1 -Action Configure -EnableAutostart
& .\tools\wufan.ps1 -Action Configure -DisableAutostart
```

登录启动默认关闭；启用后只写当前账号的 `HKCU\...\Run\WufanTechnicalBeta`。配置变更会重启已运行 Broker，使新授权立即生效。开发环境已有的临时 TSF 注册或同名启动项必须先通过原工具移除；安装器会拒绝覆盖未由它管理的注册。

## 启动、退出和状态

```powershell
& .\tools\wufan.ps1 -Action Start
& .\tools\wufan.ps1 -Action Status
& .\tools\wufan.ps1 -Action Stop
```

退出 Broker 后，输入管道断开并清理 preedit，未恢复前应用按键按现有故障降级路径放行。`Stop` 保留已安装 TIP 与启动设置；若要禁止下次登录启动，请另用 `Configure -DisableAutostart`。控制器按 PID、进程开始时间、完整路径、账号 SID 和 Windows 会话识别自己启动的 Broker，拒绝按进程名批量退出。

默认根目录布局：

```text
%LOCALAPPDATA%\Wufan\beta\
  versions\<id>\           不原位覆盖的 DLL、Broker、runtime、工具与 manifest
  data\rime\<id>\          各版本独立部署数据与 librime 错误日志
  config.json              实际宿主白名单、可选登录启动
  state.json               活动版本、上一版本及其配置
  transaction.json         持久化事务快照，Pending/Completed/Recovered
  broker-process.json      受控 Broker 身份
  broker-launch.json       进程创建前持久化的唯一启动 token
  run\                     启动日志、UAC 请求与结果
```

默认不设置 TSF 输入 trace 或验收键盘注入变量。Broker 启动时清除自身继承的 `WUFAN_*` 开发变量；如果宿主应用继承了开发终端设置的 `WUFAN_BROKER_EXE` 或 trace 变量，应先清除这些变量并重新启动宿主。不自动修改用户或系统环境变量。

## 升级、回滚与中断恢复

从新包运行 `Install`；安装器保留原版本文件和数据，注册切换到新 DLL，验证 Broker 启动成功后完成事务。因 TSF DLL 已在应用中加载并固定，升级或回滚后须退出并重新打开这些应用，必要时注销登录。首版没有热替换 DLL 功能。

```powershell
& .\tools\wufan.ps1 -Action Rollback  # 重新注册上一版本，恢复其配置及启动选项
& .\tools\wufan.ps1 -Action Recover   # 恢复 Pending 事务前的注册、配置和启动状态
```

安装步骤中断或注册/配置补偿失败时保留 `Pending` 事务，阻止进一步安装/配置/启动。`Status` 和 `Stop` 仍可使用。再次运行 `Recover` 时可能再次请求 UAC；不要手工编辑事务以跳过恢复。注册、配置和启动项恢复完成后标记 `Recovered`；若原 Broker 无法重启，会提示错误并保持恢复后的版本，允许修正配置再 `Start`。此机制提供重试恢复，不保证断电期间 Windows 注册表与文件系统跨资源原子提交。

## 卸载

```powershell
& .\tools\wufan.ps1 -Action Uninstall
```

卸载停止受控 Broker，移除本安装所有的机器 TIP、当前用户 COM 注册与登录启动项，清空活动版本。用户配置、每版本 Rime 数据、版本目录和 `%LOCALAPPDATA%\Wufan\mode-memory` 保留。保留 DLL 文件是为了让尚未关闭的宿主安全退出；卸载完成后重启这些应用或注销。

若要彻底删除保留的文件，确认卸载成功、关闭所有曾加载输入法的应用并注销后，再手工删除 `%LOCALAPPDATA%\Wufan\beta` 和所需的 mode-memory 数据。脚本不会递归删除仍可能加载的 DLL 或用户数据。

## 故障反馈

提供 Windows/应用版本、实际宿主 EXE 路径、`Status` 输出、事务 phase 与 helper `.result` 的错误信息。`run\broker.err.log`、librime 日志和用户数据可能含本机路径或诊断细节，发送前自行检查；不要默认上传整个数据目录。不要启用输入文本 trace 收集日常输入。

## 开发者打包

先运行 `scripts\fetch_rime.ps1` 准备固定依赖。在 x64 Visual Studio Developer shell（`VsDevCmd.bat -arch=x64 -host_arch=x64`）中执行：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\package_beta.ps1 -Version 0.1.0-beta.1
```

输出到 `target\beta-packages`：目录、ZIP、`.zip.sha256`。脚本使用独立 `target\beta-build` 构建无故障 feature 的 release；固定 ZIP 条目顺序、时间戳与属性，manifest 包含逐文件长度/哈希、源码提交与工作区内容摘要、锁定依赖和许可清单。不会复制开发 user 目录、验收 marker、PDB、LIB 或构建缓存。

“可重复”指同一源码快照、相同已构建二进制、相同打包 PowerShell/.NET 环境得到相同包字节；不声称不同 MSVC、源码路径或 Rust 环境重编译产生相同 PE 文件。相同内容复打包不会覆盖版本目录；可能保留供检查的 `.staging-*` 目录。

`-ReleaseReady` 要求许可清单完成，且 `config/beta/release-gates.json` 中 Chromium、安装事务、干净机器、worker 生命周期和重复归档一致性均有验收记录并标为 `accepted`，否则拒绝输出可分发包。这些字段当前都是 `pending`；只有实际验收后才可更新。manifest 分别记录许可和验收状态。完整发布出口见仓库 `TECH_BETA_PLAN.md`。
