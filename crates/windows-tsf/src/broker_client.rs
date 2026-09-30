//! Owned messages only across the worker boundary; no COM objects leave the apartment.
use ime_broker_transport::{
    client::Pipe,
    security::{authorize_server, PeerContext},
};
use ime_pinyin_engine::{EngineReply, Preedit};
use ime_protocol::{wire::*, Epoch, MessageIdentity, RequestSeq, SessionId};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    time::{Duration, Instant},
};

struct Shared {
    metrics: [AtomicU64; 12],
    epoch: AtomicU64,
    ready: AtomicU64,
    queued: AtomicUsize,
    stopped: AtomicBool,
}
struct KeyJob {
    epoch: u64,
    identity: MessageIdentity,
    ticket: u64,
    code: i32,
}
struct Ack {
    identity: MessageIdentity,
    id: Option<ime_protocol::CommitId>,
    outcome: CommitOutcome,
}
pub(super) enum WorkerEvent {
    Opened {
        epoch: u64,
        identity: MessageIdentity,
    },
    Finished {
        epoch: u64,
        identity: MessageIdentity,
        ticket: u64,
    },
    View {
        epoch: u64,
        identity: MessageIdentity,
        ticket: u64,
        reply: EngineReply,
        receipt: ViewReceipt,
    },
    Failed(u64),
}
pub(super) struct ViewReceipt {
    ack: Option<Ack>,
    sender: SyncSender<Ack>,
    shared: Arc<Shared>,
}
impl ViewReceipt {
    pub(super) fn identity(&self) -> MessageIdentity {
        self.ack.as_ref().expect("owned transport receipt").identity
    }
    pub(super) fn commit_id(&self) -> Option<ime_protocol::CommitId> {
        self.ack.as_ref().expect("owned transport receipt").id
    }
    pub(super) fn complete(&mut self, status: ime_runtime_core::CommitStatus) {
        if let Some(ack) = &mut self.ack {
            ack.outcome = match status {
                ime_runtime_core::CommitStatus::Applied { .. } => CommitOutcome::Applied,
                ime_runtime_core::CommitStatus::Rejected => CommitOutcome::Rejected,
                _ => CommitOutcome::Indeterminate,
            };
        }
    }
    pub(super) fn disarm(&mut self) {
        self.ack = None;
    }
}
impl Drop for ViewReceipt {
    fn drop(&mut self) {
        if let Some(ack) = self.ack.take() {
            self.shared.metrics[9].fetch_add(1, Ordering::Relaxed);
            if self.sender.try_send(ack).is_err() {
                self.shared.ready.store(0, Ordering::Release);
                self.shared.epoch.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
}
pub(super) struct BrokerClient {
    shared: Arc<Shared>,
    keys: SyncSender<KeyJob>,
    events: Receiver<WorkerEvent>,
}
impl BrokerClient {
    pub(super) fn start(executable: String) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            metrics: std::array::from_fn(|_| AtomicU64::new(0)),
            epoch: AtomicU64::new(1),
            ready: AtomicU64::new(0),
            queued: AtomicUsize::new(0),
            stopped: AtomicBool::new(false),
        });
        let (keys, key_rx) = mpsc::sync_channel(32);
        let (acks, ack_rx) = mpsc::sync_channel(4);
        let (event_tx, events) = mpsc::sync_channel(32);
        let state = shared.clone();
        std::thread::Builder::new()
            .name("wufan-tsf-ipc".into())
            .spawn(move || {
                let mut expected = match PeerContext::current() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                expected.executable = executable;
                while !state.stopped.load(Ordering::Acquire) {
                    let epoch = state.epoch.load(Ordering::Acquire);
                    let result = lane(&expected, epoch, &state, &key_rx, &ack_rx, &acks, &event_tx);
                    state.ready.store(0, Ordering::Release);
                    if let Err(error) = &result {
                        worker_trace(&format!("Broker worker: {error}"));
                    }
                    if result.is_err() {
                        let _ = event_tx.try_send(WorkerEvent::Failed(epoch));
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            keys,
            events,
        })
    }
    pub(super) fn epoch(&self) -> u64 {
        self.shared.epoch.load(Ordering::Acquire)
    }
    pub(super) fn ready(&self) -> bool {
        self.shared.ready.load(Ordering::Acquire) == self.epoch()
            && self.shared.queued.load(Ordering::Acquire) < 32
    }
    pub(super) fn reset(&self) {
        self.shared.ready.store(0, Ordering::Release);
        self.shared.epoch.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn submit(&self, identity: MessageIdentity, ticket: u64, code: i32) -> bool {
        if !self.ready() || identity.focus_epoch.0 != self.epoch() {
            return false;
        }
        self.shared.queued.fetch_add(1, Ordering::AcqRel);
        if self
            .keys
            .try_send(KeyJob {
                epoch: self.epoch(),
                identity,
                ticket,
                code,
            })
            .is_err()
        {
            self.shared.queued.fetch_sub(1, Ordering::AcqRel);
            self.reset();
            return false;
        }
        true
    }
    pub(super) fn poll(&self) -> Option<WorkerEvent> {
        let event = self.events.try_recv().ok();
        if matches!(event, Some(WorkerEvent::View { .. })) {
            self.count(8);
        }
        event
    }
    pub(super) fn count(&self, index: usize) {
        self.shared.metrics[index].fetch_add(1, Ordering::Relaxed);
    }
    pub(super) fn accepted_count(&self) -> u64 {
        self.shared.metrics[4].load(Ordering::Relaxed)
    }
    pub(super) fn repeated_count(&self) -> u64 {
        self.shared.metrics[11].load(Ordering::Relaxed)
    }
    pub(super) fn feature(&self, index: usize, value: u64) {
        self.shared.metrics[index].store(value, Ordering::Relaxed);
    }
}
impl Drop for BrokerClient {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.ready.store(0, Ordering::Release);
    }
}

fn exchange(
    pipe: &Pipe,
    identity: MessageIdentity,
    action: Action,
    shared: &Shared,
) -> Result<Reply, String> {
    pipe.send(
        &Request {
            version: VERSION,
            identity,
            action,
        },
        &shared.stopped,
    )?;
    let reply: Reply = pipe.receive(&shared.stopped)?;
    if reply.version != VERSION
        || reply.identity != identity
        || matches!(reply.result, ReplyResult::Error { .. })
    {
        return Err("invalid/stale Broker reply".into());
    }
    Ok(reply)
}
fn lane(
    expected: &PeerContext,
    epoch: u64,
    shared: &Arc<Shared>,
    keys: &Receiver<KeyJob>,
    ack_rx: &Receiver<Ack>,
    ack_tx: &SyncSender<Ack>,
    events: &SyncSender<WorkerEvent>,
) -> Result<(), String> {
    let pipe = Pipe::open(&expected.pipe_name())?;
    authorize_server(&pipe, expected)?;
    let hello: Hello = pipe.receive(&shared.stopped)?;
    if hello.version != VERSION || hello.learning_enabled {
        return Err("incompatible Broker".into());
    }
    let mut identity = MessageIdentity {
        client_instance_id: hello.client_instance_id,
        broker_generation: hello.broker_generation,
        session_id: SessionId(1),
        focus_epoch: Epoch(epoch),
        composition_epoch: Epoch(epoch),
        request_seq: RequestSeq(1),
    };
    if !matches!(
        exchange(&pipe, identity, Action::Open, shared)?.result,
        ReplyResult::Opened
    ) {
        return Err("expected Opened reply".into());
    }
    events
        .try_send(WorkerEvent::Opened { epoch, identity })
        .map_err(|_| "apartment session queue full")?;
    shared.ready.store(epoch, Ordering::Release);
    let mut expected_seq = 2u64;
    let mut previous_metrics = [0u64; 12];
    while !shared.stopped.load(Ordering::Acquire) && shared.epoch.load(Ordering::Acquire) == epoch {
        let metrics: [u64; 12] = std::array::from_fn(|i| shared.metrics[i].load(Ordering::Relaxed));
        if metrics != previous_metrics {
            worker_trace(&format!(
                "TSF counts test/test_decision/actual/eaten/accepted/reset/features/views/host_results/finished/repeated: {metrics:?}"
            ));
            previous_metrics = metrics;
        }
        let job = match keys.recv_timeout(Duration::from_millis(10)) {
            Ok(job) => job,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                pipe.check_idle()?;
                continue;
            }
            Err(_) => break,
        };
        shared.queued.fetch_sub(1, Ordering::AcqRel);
        if job.epoch != epoch {
            continue;
        }
        let proposed = job.identity;
        if proposed.client_instance_id != identity.client_instance_id
            || proposed.broker_generation != identity.broker_generation
            || proposed.session_id != identity.session_id
            || proposed.focus_epoch != identity.focus_epoch
            || proposed.composition_epoch != identity.composition_epoch
            || proposed.request_seq.0 != expected_seq
        {
            return Err("invalid lifecycle SendKey identity/sequence".into());
        }
        identity = proposed;
        let view = exchange(
            &pipe,
            identity,
            Action::Key {
                code: job.code,
                modifiers: 0,
            },
            shared,
        )?;
        let ReplyResult::View {
            consumed,
            preedit,
            candidates,
            commit,
            ..
        } = view.result
        else {
            return Err("expected engine view".into());
        };
        if !consumed {
            return Err("predicted key was not consumed by Rime".into());
        }
        let pending_id = commit.as_ref().map(|c| c.id);
        let receipt = ViewReceipt {
            ack: Some(Ack {
                identity,
                id: pending_id,
                outcome: CommitOutcome::Rejected,
            }),
            sender: ack_tx.clone(),
            shared: shared.clone(),
        };
        let display = if preedit.is_empty() {
            Preedit::Hide
        } else {
            let labels = candidates
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{}.{}", i + 1, c.text))
                .collect::<Vec<_>>()
                .join("  ");
            Preedit::Show(format!("{preedit}  [{labels}]"))
        };
        events
            .try_send(WorkerEvent::View {
                epoch,
                identity,
                ticket: job.ticket,
                reply: EngineReply {
                    consumed,
                    preedit: display,
                    commit: commit.map(|c| c.text),
                },
                receipt,
            })
            .map_err(|_| "apartment view queue full")?;
        // Every view waits for its actual host edit result. Commit additionally
        // requires the wire ACK before Finished may release the next key.
        {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if shared.stopped.load(Ordering::Acquire)
                    || shared.epoch.load(Ordering::Acquire) != epoch
                {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err("host edit result deadline".into());
                }
                match ack_rx.recv_timeout(Duration::from_millis(10)) {
                    Ok(ack) if ack.identity == identity && ack.id == pending_id => {
                        let completed_identity = identity;
                        expected_seq = identity
                            .request_seq
                            .0
                            .checked_add(1)
                            .ok_or("request sequence exhausted")?;
                        if let Some(id) = pending_id {
                            worker_trace(&format!("Host commit ACK {:?}", ack.outcome));
                            identity.request_seq = RequestSeq(expected_seq);
                            let acknowledged = exchange(
                                &pipe,
                                identity,
                                Action::CommitAck {
                                    commit_id: id,
                                    outcome: ack.outcome,
                                },
                                shared,
                            )?;
                            if !matches!(acknowledged.result, ReplyResult::Acknowledged) {
                                return Err("expected commit Acknowledged reply".into());
                            }
                            expected_seq = expected_seq
                                .checked_add(1)
                                .ok_or("ACK sequence exhausted")?;
                        }
                        if ack.outcome != CommitOutcome::Applied {
                            return Err("host edit did not apply".into());
                        }
                        events
                            .try_send(WorkerEvent::Finished {
                                epoch,
                                identity: completed_identity,
                                ticket: job.ticket,
                            })
                            .map_err(|_| "apartment completion queue full")?;
                        shared.metrics[10].fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                    Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return Err("ACK channel closed".into()),
                }
            }
        }
    }
    Ok(()) // Disconnect invalidates this incarnation; pending commit is never replayed.
}

fn worker_trace(message: &str) {
    use std::io::Write;
    if let Some(path) = std::env::var_os("WUFAN_TSF_PROBE_TRACE") {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{message}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ime_protocol::{BrokerGeneration, ClientInstanceId, CommitId};
    #[test]
    fn receipt_reports_rejected_indeterminate_and_applied_once() {
        for (status, expected) in [
            (
                ime_runtime_core::CommitStatus::Rejected,
                CommitOutcome::Rejected,
            ),
            (
                ime_runtime_core::CommitStatus::Indeterminate,
                CommitOutcome::Indeterminate,
            ),
            (
                ime_runtime_core::CommitStatus::Applied {
                    host_revision: ime_protocol::HostRevision(1),
                },
                CommitOutcome::Applied,
            ),
        ] {
            let shared = Arc::new(Shared {
                metrics: std::array::from_fn(|_| AtomicU64::new(0)),
                epoch: AtomicU64::new(1),
                ready: AtomicU64::new(1),
                queued: AtomicUsize::new(0),
                stopped: AtomicBool::new(false),
            });
            let (sender, receiver) = mpsc::sync_channel(1);
            let identity = MessageIdentity {
                client_instance_id: ClientInstanceId(1),
                broker_generation: BrokerGeneration(2),
                session_id: SessionId(3),
                focus_epoch: Epoch(4),
                composition_epoch: Epoch(5),
                request_seq: RequestSeq(6),
            };
            let mut receipt = ViewReceipt {
                ack: Some(Ack {
                    identity,
                    id: Some(CommitId(7)),
                    outcome: CommitOutcome::Rejected,
                }),
                sender,
                shared,
            };
            receipt.complete(status);
            drop(receipt);
            let ack = receiver.try_recv().unwrap();
            assert_eq!(ack.identity, identity);
            assert_eq!(ack.id, Some(CommitId(7)));
            assert_eq!(ack.outcome, expected);
            assert!(receiver.try_recv().is_err());
        }
    }

    #[test]
    fn core_guard_reports_preedit_and_commit_host_results_once() {
        use crate::runtime_adapter::RuntimeOwner;
        use ime_runtime_core::{input_lifecycle::Event, KeyObservation};
        for commit in [false, true] {
            for (attempt, apply, invalidate, mut expected) in [
                (false, false, false, CommitOutcome::Rejected),
                (true, false, false, CommitOutcome::Indeterminate),
                (true, true, false, CommitOutcome::Applied),
                (false, false, true, CommitOutcome::Rejected),
                (true, true, true, CommitOutcome::Applied),
            ] {
                let shared = Arc::new(Shared {
                    metrics: std::array::from_fn(|_| AtomicU64::new(0)),
                    epoch: AtomicU64::new(1),
                    ready: AtomicU64::new(1),
                    queued: AtomicUsize::new(0),
                    stopped: AtomicBool::new(false),
                });
                let (sender, receiver) = mpsc::sync_channel(1);
                let identity = MessageIdentity {
                    client_instance_id: ClientInstanceId(1),
                    broker_generation: BrokerGeneration(2),
                    session_id: SessionId(3),
                    focus_epoch: Epoch(4),
                    composition_epoch: Epoch(5),
                    request_seq: RequestSeq(2),
                };
                let owner = RuntimeOwner::default();
                owner.event(Event::Opened(MessageIdentity {
                    request_seq: RequestSeq(1),
                    ..identity
                }));
                let key = KeyObservation {
                    token: 0,
                    virtual_key: 0x4e,
                    scan_code: 1,
                    modifiers: 0,
                    repeat: false,
                    now_ms: 0,
                };
                owner.event(Event::Test {
                    key,
                    available: true,
                });
                owner.event(Event::Actual {
                    key,
                    available: true,
                });
                let receipt = ViewReceipt {
                    ack: Some(Ack {
                        identity,
                        id: commit.then_some(CommitId(1)),
                        outcome: CommitOutcome::Rejected,
                    }),
                    sender,
                    shared,
                };
                let reply = EngineReply {
                    consumed: true,
                    preedit: if commit {
                        Preedit::Hide
                    } else {
                        Preedit::Show("synthetic".into())
                    },
                    commit: commit.then_some("synthetic".into()),
                };
                let mut guard = owner.admit(receipt, 1, &reply).expect("core authorization");
                if attempt {
                    guard.attempting();
                }
                if invalidate {
                    owner.invalidate();
                }
                if apply {
                    guard.applied();
                }
                drop(guard);
                let ack = receiver.try_recv().unwrap();
                assert_eq!(ack.identity, identity);
                assert_eq!(ack.id, commit.then_some(CommitId(1)));
                if !commit && invalidate && apply {
                    expected = CommitOutcome::Indeterminate;
                }
                assert_eq!(ack.outcome, expected);
                assert!(receiver.try_recv().is_err());
            }
        }
    }
}
