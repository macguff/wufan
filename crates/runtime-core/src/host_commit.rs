//! Incremental development adapter's pure host-commit owner (ADR-007).
//! Text and COM handles never enter this state. Only Apply authorizes a write.

use crate::{CommitStatus, Effect, EffectOutcome, EffectScope};
use ime_protocol::{CommitId, EffectId, HostRevision, MessageIdentity};

const TERMINAL_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    SessionOpened(MessageIdentity),
    RequestIssued(MessageIdentity),
    RequestCompleted(MessageIdentity),
    Intent {
        identity: MessageIdentity,
        commit_id: CommitId,
        token_id: u64,
    },
    HostResult {
        effect_id: EffectId,
        scope: EffectScope,
        outcome: EffectOutcome,
        host_revision: Option<HostRevision>,
    },
    Invalidated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Apply(Effect),
    Pending,
    Ack {
        identity: MessageIdentity,
        commit_id: CommitId,
        status: CommitStatus,
    },
    Ignored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    identity: MessageIdentity,
    commit_id: CommitId,
    token_id: u64,
    status: CommitStatus,
}

impl Entry {
    fn scope(self) -> EffectScope {
        EffectScope::Commit {
            client_instance_id: self.identity.client_instance_id,
            session: self.identity.session_id,
            epoch: self.identity.composition_epoch,
            commit_id: self.commit_id,
        }
    }

    fn ack(self) -> Action {
        Action::Ack {
            identity: self.identity,
            commit_id: self.commit_id,
            status: self.status,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct State {
    session: Option<MessageIdentity>,
    enabled: bool,
    request: Option<MessageIdentity>,
    last_request: u64,
    last_commit_id: u64,
    next_effect: u64,
    active: Option<Entry>,
    terminals: [Option<Entry>; TERMINAL_CAPACITY],
    next_terminal: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            session: None,
            enabled: false,
            request: None,
            last_request: 0,
            last_commit_id: 0,
            next_effect: 1,
            active: None,
            terminals: [None; TERMINAL_CAPACITY],
            next_terminal: 0,
        }
    }
}
impl State {
    pub(crate) fn has_active(&self) -> bool {
        self.active.is_some()
    }
}

fn same_session(left: MessageIdentity, right: MessageIdentity) -> bool {
    left.client_instance_id == right.client_instance_id
        && left.broker_generation == right.broker_generation
        && left.session_id == right.session_id
        && left.focus_epoch == right.focus_epoch
        && left.composition_epoch == right.composition_epoch
}

/// The total RuntimeState and the incremental commit owner share classification.
pub(crate) fn terminal_status(
    outcome: EffectOutcome,
    revision: Option<HostRevision>,
) -> CommitStatus {
    match (outcome, revision) {
        (EffectOutcome::Succeeded, Some(host_revision)) => CommitStatus::Applied { host_revision },
        (EffectOutcome::Rejected, _) => CommitStatus::Rejected,
        _ => CommitStatus::Indeterminate,
    }
}

pub fn reduce(mut state: State, event: Event) -> (State, Action) {
    let action = match event {
        Event::SessionOpened(identity) => {
            // Open is supplied only by the authenticated transport. Duplicate
            // Ready cannot erase the request watermark within an incarnation.
            if state
                .session
                .map_or(true, |session| !same_session(session, identity))
            {
                state.session = Some(identity);
                state.enabled = true;
                state.request = None;
                state.last_request = identity.request_seq.0;
                state.last_commit_id = 0;
            }
            Action::Ignored
        }
        Event::RequestIssued(identity) => {
            if state.enabled
                && state
                    .session
                    .is_some_and(|session| same_session(session, identity))
                && state.active.is_none()
                && state.request.is_none()
                && identity.request_seq.0 > state.last_request
            {
                state.last_request = identity.request_seq.0;
                state.request = Some(identity);
            }
            Action::Ignored
        }
        Event::RequestCompleted(identity) => {
            if state.request == Some(identity) && state.active.is_none() {
                state.request = None;
            }
            Action::Ignored
        }
        Event::Invalidated => {
            state.enabled = false;
            state.request = None;
            // A detached write still owes exactly one result, even after focus loss.
            Action::Ignored
        }
        Event::Intent {
            identity,
            commit_id,
            token_id,
        } => {
            let existing = state
                .active
                .iter()
                .chain(state.terminals.iter().flatten())
                .find(|entry| {
                    entry.identity.client_instance_id == identity.client_instance_id
                        && entry.identity.broker_generation == identity.broker_generation
                        && entry.identity.session_id == identity.session_id
                        && entry.commit_id == commit_id
                });
            if let Some(entry) = existing {
                if entry.identity != identity || entry.token_id != token_id {
                    Action::Ignored
                } else if matches!(entry.status, CommitStatus::Applying { .. }) {
                    Action::Pending
                } else {
                    entry.ack()
                }
            } else if state.enabled
                && state.request == Some(identity)
                && commit_id.0 > state.last_commit_id
                && state
                    .session
                    .is_some_and(|session| same_session(session, identity))
                && state.active.is_none()
            {
                if let Some(next) = state.next_effect.checked_add(1) {
                    let effect_id = EffectId(state.next_effect);
                    state.next_effect = next;
                    state.last_commit_id = commit_id.0;
                    state.active = Some(Entry {
                        identity,
                        commit_id,
                        token_id,
                        status: CommitStatus::Applying { effect_id },
                    });
                    Action::Apply(Effect::ApplyHostCommit {
                        effect_id,
                        client_instance_id: identity.client_instance_id,
                        session: identity.session_id,
                        epoch: identity.composition_epoch,
                        commit_id,
                        token_id,
                    })
                } else {
                    state.enabled = false;
                    state.request = None;
                    Action::Ignored
                }
            } else {
                Action::Ignored
            }
        }
        Event::HostResult {
            effect_id,
            scope,
            outcome,
            host_revision,
        } => {
            if let Some(entry) = state.active.filter(|entry| {
                entry.status == CommitStatus::Applying { effect_id } && entry.scope() == scope
            }) {
                let terminal = Entry {
                    status: terminal_status(outcome, host_revision),
                    ..entry
                };
                state.active = None;
                if state.request == Some(entry.identity) {
                    state.request = None;
                }
                if !matches!(terminal.status, CommitStatus::Applied { .. })
                    && state
                        .session
                        .is_some_and(|session| same_session(session, entry.identity))
                {
                    state.enabled = false;
                    state.request = None;
                }
                state.terminals[state.next_terminal] = Some(terminal);
                state.next_terminal = (state.next_terminal + 1) % TERMINAL_CAPACITY;
                terminal.ack()
            } else {
                Action::Ignored
            }
        }
    };
    (state, action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ime_protocol::{BrokerGeneration, ClientInstanceId, Epoch, RequestSeq, SessionId};

    fn identity(seq: u64) -> MessageIdentity {
        MessageIdentity {
            client_instance_id: ClientInstanceId(1),
            broker_generation: BrokerGeneration(2),
            session_id: SessionId(3),
            focus_epoch: Epoch(4),
            composition_epoch: Epoch(5),
            request_seq: RequestSeq(seq),
        }
    }
    fn opened() -> State {
        reduce(State::default(), Event::SessionOpened(identity(1))).0
    }
    fn intent(seq: u64, id: u64) -> Event {
        Event::Intent {
            identity: identity(seq),
            commit_id: CommitId(id),
            token_id: seq,
        }
    }
    fn result(action: Action, outcome: EffectOutcome, revision: Option<HostRevision>) -> Event {
        let Action::Apply(Effect::ApplyHostCommit {
            effect_id,
            client_instance_id,
            session,
            epoch,
            commit_id,
            ..
        }) = action
        else {
            panic!("missing authorization")
        };
        Event::HostResult {
            effect_id,
            scope: EffectScope::Commit {
                client_instance_id,
                session,
                epoch,
                commit_id,
            },
            outcome,
            host_revision: revision,
        }
    }

    #[test]
    fn duplicate_intent_and_result_never_apply_twice() {
        let state = reduce(opened(), Event::RequestIssued(identity(2))).0;
        let (state, apply) = reduce(state, intent(2, 1));
        let (state, pending) = reduce(state, intent(2, 1));
        assert_eq!(pending, Action::Pending);
        let completion = result(apply, EffectOutcome::Succeeded, Some(HostRevision(1)));
        let (state, ack) = reduce(state, completion);
        assert!(matches!(
            ack,
            Action::Ack {
                status: CommitStatus::Applied { .. },
                ..
            }
        ));
        let (same, repeated) = reduce(state, intent(2, 1));
        assert_eq!(same, state);
        assert_eq!(repeated, ack);
        assert_eq!(reduce(state, completion), (state, Action::Ignored));
    }

    #[test]
    fn intent_requires_the_real_request_and_all_identity_fields() {
        assert_eq!(reduce(opened(), intent(2, 1)).1, Action::Ignored);
        let state = reduce(opened(), Event::RequestIssued(identity(2))).0;
        for field in 0..6 {
            let mut wrong = identity(2);
            match field {
                0 => wrong.client_instance_id.0 += 1,
                1 => wrong.broker_generation.0 += 1,
                2 => wrong.session_id.0 += 1,
                3 => wrong.focus_epoch.0 += 1,
                4 => wrong.composition_epoch.0 += 1,
                _ => wrong.request_seq.0 += 1,
            }
            assert_eq!(
                reduce(
                    state,
                    Event::Intent {
                        identity: wrong,
                        commit_id: CommitId(1),
                        token_id: 2
                    }
                ),
                (state, Action::Ignored)
            );
        }
        assert_eq!(reduce(state, Event::RequestIssued(identity(3))).0, state);
        let completed = reduce(state, Event::RequestCompleted(identity(2))).0;
        assert_eq!(reduce(completed, intent(2, 1)).1, Action::Ignored);
    }

    #[test]
    fn result_must_match_effect_and_scope() {
        let state = reduce(opened(), Event::RequestIssued(identity(2))).0;
        let (state, apply) = reduce(state, intent(2, 1));
        let Event::HostResult {
            effect_id,
            scope,
            outcome,
            host_revision,
        } = result(apply, EffectOutcome::Succeeded, Some(HostRevision(1)))
        else {
            unreachable!()
        };
        assert_eq!(
            reduce(
                state,
                Event::HostResult {
                    effect_id: EffectId(effect_id.0 + 1),
                    scope,
                    outcome,
                    host_revision
                }
            ),
            (state, Action::Ignored)
        );
        assert_eq!(
            reduce(
                state,
                Event::HostResult {
                    effect_id,
                    scope: EffectScope::Commit {
                        client_instance_id: ClientInstanceId(99),
                        session: SessionId(3),
                        epoch: Epoch(5),
                        commit_id: CommitId(1)
                    },
                    outcome,
                    host_revision
                }
            ),
            (state, Action::Ignored)
        );
    }

    #[test]
    fn focus_loss_keeps_old_result_obligation_without_reopening_old_scope() {
        let state = reduce(opened(), Event::RequestIssued(identity(2))).0;
        let (state, apply) = reduce(state, intent(2, 1));
        let state = reduce(state, Event::Invalidated).0;
        assert_eq!(reduce(state, intent(3, 2)).1, Action::Ignored);
        let (state, ack) = reduce(
            state,
            result(apply, EffectOutcome::Succeeded, Some(HostRevision(1))),
        );
        assert!(matches!(
            ack,
            Action::Ack {
                status: CommitStatus::Applied { .. },
                ..
            }
        ));
        let state = reduce(state, Event::SessionOpened(identity(1))).0;
        let state = reduce(state, Event::RequestIssued(identity(3))).0;
        assert_eq!(reduce(state, intent(3, 2)).1, Action::Ignored);
        for restarted in [false, true] {
            let mut fresh = identity(1);
            if restarted {
                // Broker restart can reuse client/session/commit IDs. Its new
                // generation must separate old terminal entries and results.
                fresh.broker_generation = BrokerGeneration(3);
            } else {
                fresh.client_instance_id = ClientInstanceId(10);
            }
            let fresh_state = reduce(state, Event::SessionOpened(fresh)).0;
            let fresh = MessageIdentity {
                request_seq: RequestSeq(2),
                ..fresh
            };
            let fresh_state = reduce(fresh_state, Event::RequestIssued(fresh)).0;
            let (fresh_state, fresh_apply) = reduce(
                fresh_state,
                Event::Intent {
                    identity: fresh,
                    commit_id: CommitId(1),
                    token_id: 3,
                },
            );
            assert!(matches!(fresh_apply, Action::Apply(_)));
            let stale_result = result(apply, EffectOutcome::Succeeded, Some(HostRevision(1)));
            assert_eq!(
                reduce(fresh_state, stale_result),
                (fresh_state, Action::Ignored)
            );
            assert!(matches!(
                reduce(fresh_state, result(fresh_apply, EffectOutcome::Succeeded, Some(HostRevision(2)))).1,
                Action::Ack { identity, status: CommitStatus::Applied { .. }, .. } if identity == fresh
            ));
        }
    }

    #[test]
    fn cache_eviction_and_changed_request_cannot_reapply_old_commit() {
        let mut state = opened();
        for id in 1..=40 {
            let seq = 2 * id;
            state = reduce(state, Event::RequestIssued(identity(seq))).0;
            let (next, apply) = reduce(state, intent(seq, id));
            state = reduce(
                next,
                result(apply, EffectOutcome::Succeeded, Some(HostRevision(id))),
            )
            .0;
        }
        assert_eq!(state.terminals.iter().flatten().count(), TERMINAL_CAPACITY);
        assert_eq!(reduce(state, intent(2, 1)).1, Action::Ignored);
        state = reduce(state, Event::RequestIssued(identity(82))).0;
        assert_eq!(reduce(state, intent(82, 1)).1, Action::Ignored);
        assert!(matches!(reduce(state, intent(82, 41)).1, Action::Apply(_)));
    }

    #[test]
    fn three_terminal_outcomes_and_missing_revision_are_classified() {
        for (outcome, revision, expected) in [
            (
                EffectOutcome::Succeeded,
                Some(HostRevision(1)),
                CommitStatus::Applied {
                    host_revision: HostRevision(1),
                },
            ),
            (EffectOutcome::Rejected, None, CommitStatus::Rejected),
            (
                EffectOutcome::Indeterminate,
                None,
                CommitStatus::Indeterminate,
            ),
            (EffectOutcome::Succeeded, None, CommitStatus::Indeterminate),
        ] {
            let state = reduce(opened(), Event::RequestIssued(identity(2))).0;
            let (state, apply) = reduce(state, intent(2, 1));
            let (state, ack) = reduce(state, result(apply, outcome, revision));
            assert!(matches!(ack, Action::Ack { status, .. } if status == expected));
            assert_eq!(
                state.enabled,
                matches!(expected, CommitStatus::Applied { .. })
            );
        }
    }
}
