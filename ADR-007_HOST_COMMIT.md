# ADR-007：开发 TSF 提交路径的 RuntimeCore owner

日期：2026-09-30。范围：用户授权继续实现的 Broker/TSF 中文输入开发闭环。

## 原因

开发路径已能输入中文，但原 CommitReceipt 直接决定 ACK。完整 RuntimeState 还要求按键、composition 和 request 生命周期，不能以虚构事件或手动写状态字段来伪装完整接入。

## 决定

在 RuntimeCore 内增加独立的纯 `host_commit` reducer，作为开发路径的提交 owner。它不读取文本、不调用 COM、不访问时钟或 IPC。apartment Adapter 输入真实 SessionOpened、RequestIssued、RequestCompleted、CommitIntent、HostResult 和 Invalidated 事件；worker 的 authenticated identity 和本地 epoch gate 仍先执行。

- 仅 reducer 的 Apply 授权允许一次宿主提交。授权携带唯一 effect ID 和完整 commit scope；TSF 排队的只是该授权及 owned 文本。
- Intent 必须匹配当前 session 的五字段 scope 和唯一真实在途请求。重复 Applying 返回 Pending，重复 terminal 只回 ACK，不生成第二个 Apply。
- 宿主执行结果按完整 effect/scope 匹配回流。TSF 的 RAII guard 在未执行时报告 Rejected，可能开始写入后报告 Indeterminate，完整成功时报告 Succeeded 和本地 host revision；只有 reducer 将终态转换成 ACK。
- 焦点/模式/断连使当前 scope 失效，但保留已授权 commit 的结果义务；旧执行结果只能完成自己的 ledger。Rejected/Indeterminate 关闭原 scope；旧提交不重试。
- 一个 active commit、32 个 terminal cache 项；缓存淘汰后通过 request 高水位拒绝旧 intent。effect ID、host revision 和 request 序号不允许回绕。
- 文本与 COM ownership 留在 TSF；ACK 在原 connection/request identity 下发送。guard 结果回流只借用独立 commit state，不访问 composition data 或调用 COM，避免队列清空时重入 composition 的 RefCell。

这一步不会把现有 RuntimeState 的 key/preedit reducer 标作已接入。后续统一 request/按键/preedit owner 时，须合并或替换开发子状态机，不能同时维护两套提交真相。RuntimeState 的现有模型行为保持不变，终态分类规则共用。

## 验收边界

复用既有中文输入验收范围验证三类结果、旧 scope、重复 intent/result、缓存淘汰和焦点切换。工作区模型检查与真实 WPF 输入分别记录；G1–G10、多宿主物理输入、生产身份策略和 ACK 后学习仍为独立门槛。
