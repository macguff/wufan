//! Unified actual input lifecycle: keys -> request -> host edit -> transport.
//! Owned text, clocks, COM and IPC stay in adapters (ADR-008).
use crate::{
    host_commit, CommitStatus, EatDecision, Effect, EffectId, EffectOutcome, EffectScope,
    KeyObservation,
};
use ime_protocol::{CommitId, HostRevision, MessageIdentity, RequestSeq};

const CAPACITY: usize = 32;
const TEST_TTL_MS: u64 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    Opened(MessageIdentity),
    Test {
        key: KeyObservation,
        available: bool,
    },
    Actual {
        key: KeyObservation,
        available: bool,
    },
    View {
        identity: MessageIdentity,
        ticket: u64,
        visible: bool,
        commit_id: Option<CommitId>,
    },
    PreeditResult {
        identity: MessageIdentity,
        ticket: u64,
        outcome: EffectOutcome,
    },
    CommitResult {
        effect_id: EffectId,
        scope: EffectScope,
        outcome: EffectOutcome,
        host_revision: Option<HostRevision>,
    },
    Finished {
        identity: MessageIdentity,
        ticket: u64,
    },
    Deadline {
        identity: MessageIdentity,
    },
    Invalidated,
    Pump,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    None,
    Reset,
    Pending,
    SendKey {
        identity: MessageIdentity,
        ticket: u64,
        code: i32,
    },
    ApplyPreedit {
        identity: MessageIdentity,
        ticket: u64,
    },
    ApplyCommit(Effect),
    CompleteView {
        identity: MessageIdentity,
        ticket: u64,
        status: CommitStatus,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transition {
    pub state: State,
    pub decision: EatDecision,
    pub action: Action,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Engine,
    Preedit { visible: bool },
    Commit,
    AwaitingTransport,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Request {
    identity: MessageIdentity,
    ticket: u64,
    phase: Phase,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Key {
    ticket: u64,
    code: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct State {
    session: Option<MessageIdentity>,
    last_open: Option<MessageIdentity>,
    next_seq: u64,
    next_ticket: u64,
    tested: Option<KeyObservation>,
    anticipated: bool,
    visible: bool,
    request: Option<Request>,
    queue: [Option<Key>; CAPACITY],
    head: usize,
    len: usize,
    commits: host_commit::State,
}
impl Default for State {
    fn default() -> Self {
        Self {
            session: None,
            last_open: None,
            next_seq: 0,
            next_ticket: 1,
            tested: None,
            anticipated: false,
            visible: false,
            request: None,
            queue: [None; CAPACITY],
            head: 0,
            len: 0,
            commits: host_commit::State::default(),
        }
    }
}
impl State {
    fn commit_event(&mut self, event: host_commit::Event) -> host_commit::Action {
        let (state, action) = host_commit::reduce(self.commits, event);
        self.commits = state;
        action
    }
    fn invalidate(&mut self) {
        self.session = None;
        self.tested = None;
        self.anticipated = false;
        self.visible = false;
        self.request = None;
        self.queue = [None; CAPACITY];
        self.len = 0;
        self.head = 0;
        self.commit_event(host_commit::Event::Invalidated);
    }
    fn dispatch(&mut self) -> Action {
        let Some(session) = self.session else {
            return Action::None;
        };
        if self.request.is_some() || self.commits.has_active() || self.len == 0 {
            return Action::None;
        }
        let Some(next) = self.next_seq.checked_add(1) else {
            self.invalidate();
            return Action::Reset;
        };
        let key = self.queue[self.head].take().expect("occupied FIFO head");
        self.head = (self.head + 1) % CAPACITY;
        self.len -= 1;
        let identity = MessageIdentity {
            request_seq: RequestSeq(self.next_seq),
            ..session
        };
        self.next_seq = next;
        self.request = Some(Request {
            identity,
            ticket: key.ticket,
            phase: Phase::Engine,
        });
        self.commit_event(host_commit::Event::RequestIssued(identity));
        Action::SendKey {
            identity,
            ticket: key.ticket,
            code: key.code,
        }
    }
    fn eligible(&self, key: KeyObservation, available: bool) -> bool {
        available
            && key.modifiers == 0
            && self.session.is_some()
            && self.len + usize::from(self.request.is_some() || self.commits.has_active())
                < CAPACITY
            && code(key.virtual_key).is_some()
            && ((0x41..=0x5a).contains(&key.virtual_key) || self.anticipated)
    }
    pub fn queued_len(&self) -> usize {
        self.len
    }
    pub fn has_request(&self) -> bool {
        self.request.is_some()
    }
    pub fn is_idle(&self) -> bool {
        self.request.is_none() && self.len == 0 && !self.commits.has_active()
    }
    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }
    pub fn has_preedit(&self) -> bool {
        self.visible
    }
    pub fn invariants_hold(&self) -> bool {
        self.head < CAPACITY
            && self.len <= CAPACITY
            && self.queue.iter().flatten().count() == self.len
            && (0..self.len).all(|offset| self.queue[(self.head + offset) % CAPACITY].is_some())
            && self.len + usize::from(self.request.is_some() || self.commits.has_active())
                <= CAPACITY
            && self.request.map_or(true, |request| {
                self.session.is_some_and(|session| {
                    session.client_instance_id == request.identity.client_instance_id
                        && session.broker_generation == request.identity.broker_generation
                        && session.session_id == request.identity.session_id
                        && session.focus_epoch == request.identity.focus_epoch
                        && session.composition_epoch == request.identity.composition_epoch
                        && request.ticket < self.next_ticket
                })
            })
            && (!self.visible || self.session.is_some())
    }
}

fn code(key: u16) -> Option<i32> {
    Some(match key {
        0x41..=0x5a => i32::from(key + 0x20),
        0x08 => 0xff08,
        0x0d => 0xff0d,
        0x1b => 0xff1b,
        0x20 | 0x31..=0x39 => i32::from(key),
        _ => return None,
    })
}
fn matches_test(tested: KeyObservation, actual: KeyObservation) -> bool {
    tested.virtual_key == actual.virtual_key
        && tested.scan_code == actual.scan_code
        && tested.modifiers == actual.modifiers
        && tested.repeat == actual.repeat
        && actual.now_ms >= tested.now_ms
        && actual.now_ms - tested.now_ms <= TEST_TTL_MS
}

pub fn reduce(mut state: State, event: Event) -> Transition {
    let mut decision = EatDecision::Pass;
    let action = match event {
        Event::Opened(identity) => {
            if state.last_open == Some(identity) {
                Action::None
            } else if let Some(next) = identity.request_seq.0.checked_add(1) {
                state.invalidate();
                state.session = Some(identity);
                state.last_open = Some(identity);
                state.next_seq = next;
                state.commit_event(host_commit::Event::SessionOpened(identity));
                Action::None
            } else {
                state.invalidate();
                Action::Reset
            }
        }
        Event::Test { key, available } => {
            if state.eligible(key, available) {
                state.tested = Some(key);
                decision = EatDecision::Eat;
            } else {
                state.tested = None;
            }
            Action::None
        }
        Event::Actual { key, available } => {
            if state
                .tested
                .take()
                .is_some_and(|tested| matches_test(tested, key))
                && state.eligible(key, available)
            {
                if let Some(next) = state.next_ticket.checked_add(1) {
                    let ticket = state.next_ticket;
                    state.next_ticket = next;
                    let index = (state.head + state.len) % CAPACITY;
                    state.queue[index] = Some(Key {
                        ticket,
                        code: code(key.virtual_key).expect("eligible key"),
                    });
                    state.len += 1;
                    state.anticipated = (0x41..=0x5a).contains(&key.virtual_key)
                        || (state.anticipated && key.virtual_key == 0x08);
                    decision = EatDecision::Eat;
                    state.dispatch()
                } else {
                    state.invalidate();
                    Action::Reset
                }
            } else {
                Action::None
            }
        }
        Event::View {
            identity,
            ticket,
            visible,
            commit_id,
        } => {
            if let Some(request) = state
                .request
                .filter(|r| r.identity == identity && r.ticket == ticket)
            {
                if request.phase != Phase::Engine {
                    Action::Pending
                } else if let Some(commit_id) = commit_id {
                    if let Some(next) = state.next_seq.checked_add(1) {
                        state.next_seq = next; // Reserve the wire CommitAck sequence.
                        match state.commit_event(host_commit::Event::Intent {
                            identity,
                            commit_id,
                            token_id: ticket,
                        }) {
                            host_commit::Action::Apply(effect) => {
                                state.request.as_mut().expect("current request").phase =
                                    Phase::Commit;
                                if state.len == 0 {
                                    state.anticipated = false;
                                }
                                Action::ApplyCommit(effect)
                            }
                            _ => {
                                state.invalidate();
                                Action::Reset
                            }
                        }
                    } else {
                        state.invalidate();
                        Action::Reset
                    }
                } else {
                    state.request.as_mut().expect("current request").phase =
                        Phase::Preedit { visible };
                    if state.len == 0 {
                        state.anticipated = visible;
                    }
                    Action::ApplyPreedit { identity, ticket }
                }
            } else {
                Action::None
            }
        }
        Event::PreeditResult {
            identity,
            ticket,
            outcome,
        } => {
            if state.request.is_some_and(|r| {
                r.identity == identity
                    && r.ticket == ticket
                    && matches!(r.phase, Phase::Preedit { .. })
            }) {
                state.commit_event(host_commit::Event::RequestCompleted(identity));
                let status = host_commit::terminal_status(
                    outcome,
                    (outcome == EffectOutcome::Succeeded).then_some(HostRevision(0)),
                );
                if outcome == EffectOutcome::Succeeded {
                    if let Phase::Preedit { visible } =
                        state.request.expect("current request").phase
                    {
                        state.visible = visible;
                    }
                    state.request.as_mut().expect("current request").phase =
                        Phase::AwaitingTransport;
                } else {
                    state.invalidate();
                }
                Action::CompleteView {
                    identity,
                    ticket,
                    status,
                }
            } else {
                Action::None
            }
        }
        Event::CommitResult {
            effect_id,
            scope,
            outcome,
            host_revision,
        } => {
            match state.commit_event(host_commit::Event::HostResult {
                effect_id,
                scope,
                outcome,
                host_revision,
            }) {
                host_commit::Action::Ack {
                    identity, status, ..
                } => {
                    let ticket = state
                        .request
                        .filter(|r| r.identity == identity && r.phase == Phase::Commit)
                        .map(|r| r.ticket);
                    if let Some(ticket) = ticket {
                        if matches!(status, CommitStatus::Applied { .. }) {
                            state.visible = false;
                            state.request.as_mut().expect("current request").phase =
                                Phase::AwaitingTransport;
                        } else {
                            state.invalidate();
                        }
                        Action::CompleteView {
                            identity,
                            ticket,
                            status,
                        }
                    } else {
                        // Old authorized writes still resolve their original receipt.
                        Action::CompleteView {
                            identity,
                            ticket: 0,
                            status,
                        }
                    }
                }
                _ => Action::None,
            }
        }
        Event::Finished { identity, ticket } => {
            if state.request.is_some_and(|r| {
                r.identity == identity && r.ticket == ticket && r.phase == Phase::AwaitingTransport
            }) {
                state.request = None;
                state.dispatch()
            } else {
                Action::None
            }
        }
        Event::Deadline { identity } => {
            if state.request.is_some_and(|r| r.identity == identity) {
                state.invalidate();
                Action::Reset
            } else {
                Action::None
            }
        }
        Event::Invalidated => {
            state.invalidate();
            Action::None
        }
        Event::Pump => state.dispatch(),
    };
    debug_assert!(
        state.invariants_hold(),
        "input lifecycle obligation/FIFO invariant"
    );
    Transition {
        state,
        decision,
        action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ime_protocol::{BrokerGeneration, ClientInstanceId, Epoch, SessionId};
    fn opened() -> State {
        reduce(
            State::default(),
            Event::Opened(MessageIdentity {
                client_instance_id: ClientInstanceId(1),
                broker_generation: BrokerGeneration(2),
                session_id: SessionId(1),
                focus_epoch: Epoch(1),
                composition_epoch: Epoch(1),
                request_seq: RequestSeq(1),
            }),
        )
        .state
    }
    fn key(vk: u16, now_ms: u64) -> KeyObservation {
        KeyObservation {
            token: 0,
            virtual_key: vk,
            scan_code: vk,
            modifiers: 0,
            repeat: false,
            now_ms,
        }
    }
    fn press(state: State, vk: u16) -> Transition {
        let state = reduce(
            state,
            Event::Test {
                key: key(vk, 0),
                available: true,
            },
        )
        .state;
        reduce(
            state,
            Event::Actual {
                key: key(vk, 1),
                available: true,
            },
        )
    }
    fn sent(transition: Transition) -> (State, MessageIdentity, u64) {
        let Action::SendKey {
            identity, ticket, ..
        } = transition.action
        else {
            panic!("expected SendKey")
        };
        (transition.state, identity, ticket)
    }
    fn finish_preedit(
        state: State,
        identity: MessageIdentity,
        ticket: u64,
        visible: bool,
    ) -> Transition {
        let view = reduce(
            state,
            Event::View {
                identity,
                ticket,
                visible,
                commit_id: None,
            },
        );
        assert_eq!(view.action, Action::ApplyPreedit { identity, ticket });
        let result = reduce(
            view.state,
            Event::PreeditResult {
                identity,
                ticket,
                outcome: EffectOutcome::Succeeded,
            },
        );
        assert!(matches!(result.action, Action::CompleteView { .. }));
        reduce(result.state, Event::Finished { identity, ticket })
    }
    #[test]
    fn preedit_and_transport_results_both_precede_next_dispatch() {
        let (state, identity, ticket) = sent(press(opened(), 0x4e));
        let queued = press(state, 0x49);
        assert_eq!(queued.decision, EatDecision::Eat);
        assert_eq!(queued.action, Action::None);
        let early = reduce(queued.state, Event::Finished { identity, ticket });
        assert_eq!(early.state, queued.state);
        assert_eq!(early.action, Action::None);
        let view = reduce(
            early.state,
            Event::View {
                identity,
                ticket,
                visible: true,
                commit_id: None,
            },
        );
        assert_eq!(reduce(view.state, Event::Pump).action, Action::None);
        let result = reduce(
            view.state,
            Event::PreeditResult {
                identity,
                ticket,
                outcome: EffectOutcome::Succeeded,
            },
        );
        assert!(result.state.has_preedit());
        assert_eq!(reduce(result.state, Event::Pump).action, Action::None);
        let next = reduce(result.state, Event::Finished { identity, ticket });
        assert!(
            matches!(next.action, Action::SendKey { identity: next_id, ticket: 2, code: 0x69 } if next_id.request_seq == RequestSeq(3))
        );
        assert_eq!(
            reduce(next.state, Event::Finished { identity, ticket }).state,
            next.state
        );
    }
    #[test]
    fn commit_ack_reserves_sequence_and_requires_finished() {
        let (state, identity, ticket) = sent(press(opened(), 0x4e));
        let state = finish_preedit(state, identity, ticket, true).state;
        let (state, identity, ticket) = sent(press(state, 0x20));
        let queued = press(state, 0x48).state;
        let view = reduce(
            queued,
            Event::View {
                identity,
                ticket,
                visible: false,
                commit_id: Some(CommitId(1)),
            },
        );
        let Action::ApplyCommit(Effect::ApplyHostCommit {
            effect_id,
            client_instance_id,
            session,
            epoch,
            commit_id,
            ..
        }) = view.action
        else {
            panic!("commit authorization")
        };
        let result = reduce(
            view.state,
            Event::CommitResult {
                effect_id,
                scope: EffectScope::Commit {
                    client_instance_id,
                    session,
                    epoch,
                    commit_id,
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: Some(HostRevision(1)),
            },
        );
        assert!(!result.state.has_preedit());
        assert_eq!(reduce(result.state, Event::Pump).action, Action::None);
        let next = reduce(result.state, Event::Finished { identity, ticket });
        assert!(
            matches!(next.action, Action::SendKey { identity: id, ticket: 3, .. } if id.request_seq == RequestSeq(5))
        );
    }
    #[test]
    fn repeated_test_unmatched_actual_and_ttl_are_pure_decisions() {
        let tested = reduce(
            opened(),
            Event::Test {
                key: key(0x4e, 0),
                available: true,
            },
        );
        assert_eq!(tested.decision, EatDecision::Eat);
        let again = reduce(
            tested.state,
            Event::Test {
                key: key(0x4e, 0),
                available: true,
            },
        );
        assert_eq!(again.decision, EatDecision::Eat);
        let actual = reduce(
            again.state,
            Event::Actual {
                key: key(0x4e, 1),
                available: true,
            },
        );
        assert_eq!(actual.decision, EatDecision::Eat);
        assert_eq!(
            reduce(
                actual.state,
                Event::Actual {
                    key: key(0x4e, 1),
                    available: true
                }
            )
            .decision,
            EatDecision::Pass
        );
        assert_eq!(
            reduce(
                tested.state,
                Event::Actual {
                    key: key(0x4e, 101),
                    available: true
                }
            )
            .decision,
            EatDecision::Pass
        );
        assert_eq!(
            reduce(
                tested.state,
                Event::Actual {
                    key: key(0x49, 1),
                    available: true
                }
            )
            .decision,
            EatDecision::Pass
        );
    }
    #[test]
    fn bounded_fifo_preserves_accepted_keys_and_passes_overflow() {
        let (mut state, mut identity, mut ticket) = sent(press(opened(), 0x4e));
        for _ in 1..CAPACITY {
            let next = press(state, 0x49);
            assert_eq!(next.decision, EatDecision::Eat);
            assert_eq!(next.action, Action::None);
            state = next.state;
        }
        assert_eq!(state.queued_len(), CAPACITY - 1);
        assert_eq!(press(state, 0x48).decision, EatDecision::Pass);
        for expected in 2..=CAPACITY as u64 {
            let next = finish_preedit(state, identity, ticket, true);
            assert!(matches!(next.action, Action::SendKey { ticket, .. } if ticket == expected));
            (state, identity, ticket) = sent(next);
        }
        let last = finish_preedit(state, identity, ticket, true);
        assert_eq!(last.action, Action::None);
        assert!(!last.state.has_request());
    }
    #[test]
    fn rejected_preedit_and_matching_deadline_cancel_the_scope() {
        let (state, identity, ticket) = sent(press(opened(), 0x4e));
        let state = press(state, 0x49).state;
        let state = reduce(
            state,
            Event::View {
                identity,
                ticket,
                visible: true,
                commit_id: None,
            },
        )
        .state;
        for outcome in [EffectOutcome::Rejected, EffectOutcome::Indeterminate] {
            let result = reduce(
                state,
                Event::PreeditResult {
                    identity,
                    ticket,
                    outcome,
                },
            );
            assert!(!result.state.is_active());
            assert_eq!(result.state.queued_len(), 0);
            assert_eq!(
                reduce(result.state, Event::Finished { identity, ticket }).action,
                Action::None
            );
        }
        let wrong = MessageIdentity {
            request_seq: RequestSeq(99),
            ..identity
        };
        assert_eq!(
            reduce(state, Event::Deadline { identity: wrong }).state,
            state
        );
        assert_eq!(
            reduce(state, Event::Deadline { identity }).action,
            Action::Reset
        );
    }
    #[test]
    fn old_views_results_and_finished_cannot_mutate_new_scope() {
        let (state, old, old_ticket) = sent(press(opened(), 0x4e));
        let state = reduce(state, Event::Invalidated).state;
        let open = MessageIdentity {
            client_instance_id: ClientInstanceId(9),
            request_seq: RequestSeq(1),
            ..old
        };
        let state = reduce(state, Event::Opened(open)).state;
        let (state, _, _) = sent(press(state, 0x48));
        for event in [
            Event::View {
                identity: old,
                ticket: old_ticket,
                visible: true,
                commit_id: None,
            },
            Event::PreeditResult {
                identity: old,
                ticket: old_ticket,
                outcome: EffectOutcome::Succeeded,
            },
            Event::Finished {
                identity: old,
                ticket: old_ticket,
            },
        ] {
            assert_eq!(reduce(state, event).state, state);
        }
    }
    #[test]
    fn old_commit_obligation_blocks_new_send_until_its_result() {
        let (state, old, ticket) = sent(press(opened(), 0x4e));
        let view = reduce(
            state,
            Event::View {
                identity: old,
                ticket,
                visible: false,
                commit_id: Some(CommitId(1)),
            },
        );
        let Action::ApplyCommit(Effect::ApplyHostCommit {
            effect_id,
            client_instance_id,
            session,
            epoch,
            commit_id,
            ..
        }) = view.action
        else {
            panic!()
        };
        let state = reduce(view.state, Event::Invalidated).state;
        let new_open = MessageIdentity {
            client_instance_id: ClientInstanceId(9),
            request_seq: RequestSeq(1),
            ..old
        };
        let state = reduce(state, Event::Opened(new_open)).state;
        let accepted = press(state, 0x48);
        assert_eq!(accepted.decision, EatDecision::Eat);
        assert_eq!(accepted.action, Action::None);
        let result = reduce(
            accepted.state,
            Event::CommitResult {
                effect_id,
                scope: EffectScope::Commit {
                    client_instance_id,
                    session,
                    epoch,
                    commit_id,
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: Some(HostRevision(1)),
            },
        );
        assert!(
            matches!(result.action, Action::CompleteView { identity, ticket: 0, .. } if identity == old)
        );
        assert!(
            matches!(reduce(result.state, Event::Pump).action, Action::SendKey { identity, .. } if identity.client_instance_id == new_open.client_instance_id)
        );
    }
    #[test]
    fn deterministic_replay_and_disabled_scope_do_not_reopen() {
        let state = opened();
        let identity = state.session.unwrap();
        let invalidated = reduce(state, Event::Invalidated).state;
        assert!(!reduce(invalidated, Event::Opened(identity))
            .state
            .is_active());
        let expected = press(state, 0x4e);
        for _ in 0..100 {
            assert_eq!(press(state, 0x4e), expected);
        }
        assert_eq!(press(State::default(), 0x4e).decision, EatDecision::Pass);
        assert_eq!(press(opened(), 0x20).decision, EatDecision::Pass);
        let mut shifted = key(0x4e, 0);
        shifted.modifiers = 1;
        assert_eq!(
            reduce(
                opened(),
                Event::Test {
                    key: shifted,
                    available: true
                }
            )
            .decision,
            EatDecision::Pass
        );
    }
}
