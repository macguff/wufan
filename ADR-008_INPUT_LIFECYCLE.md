# ADR-008：实际输入路径的统一生命周期

2026-09-30，落实用户要求的按键、preedit 和请求生命周期统一。

实际 TSF 每个服务持有一个 RuntimeCore `input_lifecycle::State`。它包含 ADR-007 的 commit ledger，而不是另一份提交真相。原 Slice 0 RuntimeState 保留为模型参考；实际 Adapter 不再维护独立 TestDecision、composition 预测或 request sequence。

- Test/Actual、按键资格、100 ms 配对 TTL、32 项总输入容量、ticket、FIFO、单在途请求和 wire request_seq 由统一 reducer 决定。Test 是探测，最新观察可替换未消费观察。
- authenticated Open 交付后才接收输入。SendKey effect 包含完整身份与 code，worker 只验证并执行，不能独立分配序号。
- 每个请求依次经过 Engine、Editing、AwaitingTransport。所有 preedit Show/Hide 与 commit 都须 reducer 授权；RAII guard 回流真实宿主结果。Show/Hide 不产生 wire ACK，但 worker 等待本地完成；commit 则等待宿主结果并完成 wire ACK。
- Finished 只能匹配当前请求的完整身份、ticket 和 AwaitingTransport 阶段。收到 Finished 后才派发 FIFO 的下一键。编辑会话尚未成功不能被当作 request completion。
- 无 modifier 的文字键和当前预计 composition 的控制键才被接收。预测只为同步资格判断，真实 host composition 的存活仍由 TSF lease 负责；COM lease 不是另一份业务状态。
- focus/context/模式/断连/超时统一 invalidate，取消待派发键和旧 edit 授权。容量满则同步 Pass，不接收新键；已接收的 FIFO 继续完成。已经开始的 commit 保留原 effect 结果义务；旧结果只能完成旧 receipt，不能提前完成新请求。旧 commit 结果未返回前，新会话可排队但不能派发；结果返回后由消息泵派发。
- guard 回流不调用 COM、IPC 或新请求派发。后续 effect 由 apartment 消息泵执行；key callback 只做本地 reducer 和非阻塞 try_send。

worker 的预期 wire seq 是验证器，reducer 的 next_seq 是唯一 allocator。commit ACK 占用紧随 Key 的一个 seq；reducer 在接受该 commit view 时预留。身份/序号不回绕。

Broker 退出由 worker 空闲 pipe 检查或在途 I/O 失败交付 Failed；apartment 清理 composition 并失效输入授权。transport reset 保留仍聚焦的 COM context binding，focus/context/关闭失效则释放 binding，避免重连后首个 Test 因重新 bind 再次失效。保留 binding 不保留旧 session、preedit 或请求。提交 terminal 查找以 Broker generation 区分重启后重用的 client/session/commit 编号；effect_id 继续在 owner 生命周期内递增，旧结果不得完成新授权。

完整旧 RuntimeState 的 wire composition epoch 与现有 Broker session 固定 epoch 不兼容，因此没有用虚构 EngineUpdate/EffectResult 来强行套入。后续迁移原总模型或扩展 wire epoch 时必须保留这一统一 owner 的实际事件闭环。多宿主门槛、canonical trace 格式迁移、签名策略、候选窗和 D6 仍单独验收。
