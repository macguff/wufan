#![forbid(unsafe_code)]
//! Serial Broker state: no host edits and no raw librime dependency.
use ime_protocol::{
    wire::*, BrokerGeneration, ClientInstanceId, CommitId, MessageIdentity, SessionId,
};
use ime_rime_adapter::{EngineSession, Key, RimeEngine};
use std::collections::BTreeMap;

const MAX_CLIENTS: usize = 8;
const MAX_SESSIONS: usize = 32;
const MAX_SESSION_RECORDS: usize = 128;

struct Session {
    identity: MessageIdentity,
    native: Option<EngineSession>,
    pending: Option<CommitIntent>,
    last: (Request, Reply),
}
struct Client {
    highest: u64,
    sessions: BTreeMap<SessionId, Session>,
}

pub struct Broker<E: RimeEngine> {
    engine: E,
    generation: BrokerGeneration,
    clients: BTreeMap<ClientInstanceId, Client>,
    next_commit: u64,
}
impl<E: RimeEngine> Broker<E> {
    pub fn new(engine: E, generation: BrokerGeneration) -> Self {
        Self {
            engine,
            generation,
            clients: BTreeMap::new(),
            next_commit: 1,
        }
    }
    /// `client` is allocated by the server after OS peer authorization.
    pub fn connect(&mut self, client: ClientInstanceId) -> Result<Hello, String> {
        if self.clients.len() >= MAX_CLIENTS || self.clients.contains_key(&client) {
            return Err("client capacity or duplicate incarnation".into());
        }
        self.clients.insert(
            client,
            Client {
                highest: 0,
                sessions: BTreeMap::new(),
            },
        );
        Ok(Hello {
            version: VERSION,
            client_instance_id: client,
            broker_generation: self.generation,
            learning_enabled: false,
        })
    }
    pub fn disconnect(&mut self, client: ClientInstanceId) {
        if let Some(client) = self.clients.remove(&client) {
            for session in client.sessions.into_values() {
                if let Some(native) = session.native {
                    self.engine.close(native);
                }
            }
        }
    }
    pub fn handle(&mut self, authorized: ClientInstanceId, request: Request) -> Reply {
        let id = request.identity;
        let error = |code| Reply {
            version: VERSION,
            identity: id,
            result: ReplyResult::Error { code },
        };
        if request.version != VERSION
            || id.client_instance_id != authorized
            || id.broker_generation != self.generation
        {
            return error(ErrorCode::StaleIdentity);
        }
        let Some(client) = self.clients.get_mut(&authorized) else {
            return error(ErrorCode::StaleIdentity);
        };
        if let Some(session) = client.sessions.get(&id.session_id) {
            if session.last.0 == request {
                return session.last.1.clone();
            }
        }
        if request.action == Action::Open {
            if id.session_id.0 <= client.highest
                || id.request_seq.0 != 1
                || id.focus_epoch.0 == 0
                || id.composition_epoch.0 == 0
            {
                return error(ErrorCode::StaleIdentity);
            }
            let active = client
                .sessions
                .values()
                .filter(|s| s.native.is_some())
                .count();
            if active >= MAX_SESSIONS || client.sessions.len() >= MAX_SESSION_RECORDS {
                return error(ErrorCode::Capacity);
            }
            client.highest = id.session_id.0;
            let native = match self.engine.open() {
                Ok(s) => s,
                Err(_) => return error(ErrorCode::EngineFailure),
            };
            let reply = Reply {
                version: VERSION,
                identity: id,
                result: ReplyResult::Opened,
            };
            client.sessions.insert(
                id.session_id,
                Session {
                    identity: id,
                    native: Some(native),
                    pending: None,
                    last: (request, reply.clone()),
                },
            );
            return reply;
        }
        let Some(session) = client.sessions.get_mut(&id.session_id) else {
            return error(ErrorCode::StaleIdentity);
        };
        if id.focus_epoch != session.identity.focus_epoch
            || id.composition_epoch != session.identity.composition_epoch
        {
            return error(ErrorCode::StaleIdentity);
        }
        if session.identity.request_seq.0.checked_add(1) != Some(id.request_seq.0) {
            return error(ErrorCode::InvalidSequence);
        }
        let Some(native) = session.native else {
            return error(ErrorCode::InvalidState);
        };
        let result = match &request.action {
            Action::Key { code, modifiers } => {
                if session.pending.is_some() {
                    return error(ErrorCode::InvalidState);
                }
                // X11 keysyms and Rime masks; no arbitrary signed integers.
                if !(0..=0x1fffffff).contains(code) || modifiers & !0x400000ff != 0 {
                    return error(ErrorCode::InvalidKey);
                }
                match self.engine.key(
                    native,
                    Key {
                        code: *code,
                        modifiers: *modifiers,
                    },
                ) {
                    Ok(view) => {
                        if let Some(text) = view.commit {
                            let Some(next) = self.next_commit.checked_add(1) else {
                                self.engine.close(native);
                                session.native = None;
                                return error(ErrorCode::Capacity);
                            };
                            session.pending = Some(CommitIntent {
                                id: CommitId(self.next_commit),
                                text,
                            });
                            self.next_commit = next;
                        }
                        ReplyResult::View {
                            consumed: view.consumed,
                            preedit: view.preedit,
                            cursor_utf16: view.cursor_utf16,
                            candidates: view
                                .candidates
                                .into_iter()
                                .map(|c| WireCandidate {
                                    text: c.text,
                                    comment: c.comment,
                                })
                                .collect(),
                            highlighted: view.highlighted,
                            page: view.page,
                            last_page: view.last_page,
                            commit: session.pending.clone(),
                        }
                    }
                    Err(_) => {
                        self.engine.close(native);
                        session.native = None;
                        ReplyResult::Error {
                            code: ErrorCode::EngineFailure,
                        }
                    }
                }
            }
            Action::Clear => {
                if session.pending.is_some() {
                    return error(ErrorCode::InvalidState);
                }
                match self.engine.clear(native) {
                    Ok(()) => ReplyResult::Cleared,
                    Err(_) => {
                        self.engine.close(native);
                        session.native = None;
                        ReplyResult::Error {
                            code: ErrorCode::EngineFailure,
                        }
                    }
                }
            }
            Action::Close => {
                self.engine.close(native);
                session.native = None;
                session.pending = None;
                ReplyResult::Closed
            }
            Action::CommitAck { commit_id, outcome } => {
                let Some(pending) = session.pending.as_ref() else {
                    return error(ErrorCode::InvalidState);
                };
                if pending.id != *commit_id {
                    return error(ErrorCode::StaleIdentity);
                }
                let applied = *outcome == CommitOutcome::Applied;
                let failed = applied && self.engine.commit_applied(native, &pending.text).is_err();
                session.pending = None;
                if !applied || failed {
                    self.engine.close(native);
                    session.native = None;
                }
                if failed {
                    ReplyResult::Error {
                        code: ErrorCode::EngineFailure,
                    }
                } else {
                    ReplyResult::Acknowledged
                }
            }
            Action::Open => unreachable!(),
        };
        session.identity = id;
        let reply = Reply {
            version: VERSION,
            identity: id,
            result,
        };
        session.last = (request, reply.clone());
        reply
    }
}
impl<E: RimeEngine> Drop for Broker<E> {
    fn drop(&mut self) {
        let clients: Vec<_> = self.clients.keys().copied().collect();
        for client in clients {
            self.disconnect(client);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ime_protocol::{Epoch, RequestSeq};
    use ime_rime_adapter::EngineView;
    use std::{cell::Cell, rc::Rc};
    #[derive(Default)]
    struct Calls {
        opens: Cell<usize>,
        keys: Cell<usize>,
        applied: Cell<usize>,
        closes: Cell<usize>,
    }
    struct Fake(Rc<Calls>);
    impl RimeEngine for Fake {
        fn open(&mut self) -> Result<EngineSession, String> {
            self.0.opens.set(self.0.opens.get() + 1);
            Ok(EngineSession(self.0.opens.get()))
        }
        fn close(&mut self, _: EngineSession) {
            self.0.closes.set(self.0.closes.get() + 1);
        }
        fn key(&mut self, _: EngineSession, _: Key) -> Result<EngineView, String> {
            self.0.keys.set(self.0.keys.get() + 1);
            Ok(EngineView {
                consumed: true,
                commit: Some("你好".into()),
                ..Default::default()
            })
        }
        fn clear(&mut self, _: EngineSession) -> Result<(), String> {
            Ok(())
        }
        fn commit_applied(&mut self, _: EngineSession, _: &str) -> Result<(), String> {
            self.0.applied.set(self.0.applied.get() + 1);
            Ok(())
        }
    }
    fn request(seq: u64, action: Action) -> Request {
        Request {
            version: VERSION,
            identity: MessageIdentity {
                client_instance_id: ClientInstanceId(1),
                broker_generation: BrokerGeneration(7),
                session_id: SessionId(1),
                focus_epoch: Epoch(1),
                composition_epoch: Epoch(1),
                request_seq: RequestSeq(seq),
            },
            action,
        }
    }
    fn fixture() -> (Broker<Fake>, Rc<Calls>) {
        let calls = Rc::new(Calls::default());
        let mut broker = Broker::new(Fake(calls.clone()), BrokerGeneration(7));
        broker.connect(ClientInstanceId(1)).unwrap();
        assert_eq!(
            broker
                .handle(ClientInstanceId(1), request(1, Action::Open))
                .result,
            ReplyResult::Opened
        );
        (broker, calls)
    }
    fn key() -> Action {
        Action::Key {
            code: 32,
            modifiers: 0,
        }
    }
    #[test]
    fn duplicate_key_and_ack_do_not_repeat_engine_effects() {
        let (mut broker, calls) = fixture();
        let first = broker.handle(ClientInstanceId(1), request(2, key()));
        assert_eq!(first, broker.handle(ClientInstanceId(1), request(2, key())));
        assert_eq!(calls.keys.get(), 1);
        assert_eq!(calls.applied.get(), 0);
        let ack = request(
            3,
            Action::CommitAck {
                commit_id: CommitId(1),
                outcome: CommitOutcome::Applied,
            },
        );
        let applied = broker.handle(ClientInstanceId(1), ack.clone());
        assert_eq!(applied.result, ReplyResult::Acknowledged);
        assert_eq!(broker.handle(ClientInstanceId(1), ack), applied);
        assert_eq!(calls.applied.get(), 1);
    }
    #[test]
    fn pending_commit_and_wrong_identity_cannot_mutate() {
        let (mut broker, calls) = fixture();
        broker.handle(ClientInstanceId(1), request(2, key()));
        assert_eq!(
            broker.handle(ClientInstanceId(1), request(3, key())).result,
            ReplyResult::Error {
                code: ErrorCode::InvalidState
            }
        );
        let mut stale = request(3, Action::Clear);
        stale.identity.focus_epoch = Epoch(2);
        assert_eq!(
            broker.handle(ClientInstanceId(1), stale).result,
            ReplyResult::Error {
                code: ErrorCode::StaleIdentity
            }
        );
        assert_eq!(
            broker
                .handle(ClientInstanceId(1), request(4, Action::Close))
                .result,
            ReplyResult::Error {
                code: ErrorCode::InvalidSequence
            }
        );
        assert_eq!(calls.keys.get(), 1);
        assert_eq!(calls.applied.get(), 0);
    }
    #[test]
    fn non_applied_outcomes_close_session_without_learning_or_replay() {
        for outcome in [CommitOutcome::Rejected, CommitOutcome::Indeterminate] {
            let (mut broker, calls) = fixture();
            broker.handle(ClientInstanceId(1), request(2, key()));
            let ack = request(
                3,
                Action::CommitAck {
                    commit_id: CommitId(1),
                    outcome,
                },
            );
            let result = broker.handle(ClientInstanceId(1), ack.clone());
            assert_eq!(broker.handle(ClientInstanceId(1), ack), result);
            assert_eq!(calls.closes.get(), 1);
            assert_eq!(calls.applied.get(), 0);
            assert_eq!(
                broker.handle(ClientInstanceId(1), request(4, key())).result,
                ReplyResult::Error {
                    code: ErrorCode::InvalidState
                }
            );
            assert_eq!(
                broker
                    .handle(ClientInstanceId(1), request(1, Action::Open))
                    .result,
                ReplyResult::Error {
                    code: ErrorCode::StaleIdentity
                }
            );
        }
    }
    #[test]
    fn capacity_and_disconnect_are_bounded() {
        let (mut broker, calls) = fixture();
        for session in 2..=32 {
            let mut open = request(1, Action::Open);
            open.identity.session_id = SessionId(session);
            assert_eq!(
                broker.handle(ClientInstanceId(1), open).result,
                ReplyResult::Opened
            );
        }
        let mut open = request(1, Action::Open);
        open.identity.session_id = SessionId(33);
        assert_eq!(
            broker.handle(ClientInstanceId(1), open).result,
            ReplyResult::Error {
                code: ErrorCode::Capacity
            }
        );
        broker.disconnect(ClientInstanceId(1));
        assert_eq!(calls.opens.get(), 32);
        assert_eq!(calls.closes.get(), 32);
    }
}
