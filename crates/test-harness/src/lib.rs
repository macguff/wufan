#![forbid(unsafe_code)]

//! Small deterministic runner shared by model tests and trace replay.

use ime_protocol::{
    BrokerGeneration, ClientInstanceId, CommitId, EffectId, Epoch, HostRevision, MessageIdentity,
    RequestSeq, SessionId,
};
use ime_runtime_core::{reduce, Event, RuntimeState, TransitionResult};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub mod auth_model;
pub mod deployment_model;
pub mod geometry_model;
pub mod memory_model;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeCommitBehavior {
    Success,
    Rejected,
    IndeterminateBeforeWrite,
    IndeterminateAfterWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeCompositionBehavior {
    Success,
    Rejected,
    Indeterminate,
}

/// Deterministic fake adapter. Its journal is the model oracle for host writes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeEffectAdapter {
    pub commit_behavior: FakeCommitBehavior,
    pub composition_behavior: FakeCompositionBehavior,
    pub host_mutations: BTreeMap<(ClientInstanceId, SessionId, CommitId), (u32, HostRevision)>,
    seen_effect_ids: BTreeSet<(ClientInstanceId, EffectId)>,
    next_host_revision: u64,
}

impl Default for FakeEffectAdapter {
    fn default() -> Self {
        Self {
            commit_behavior: FakeCommitBehavior::Success,
            composition_behavior: FakeCompositionBehavior::Success,
            host_mutations: BTreeMap::new(),
            seen_effect_ids: BTreeSet::new(),
            next_host_revision: 1,
        }
    }
}

impl FakeEffectAdapter {
    pub fn execute(&mut self, effect: ime_runtime_core::Effect) -> Option<Event> {
        use ime_runtime_core::{Effect, EffectOutcome, EffectResultClass, EffectScope};
        match effect {
            Effect::SendKey { identity, .. } | Effect::SelectCandidate { identity, .. } => {
                Some(Event::RequestCompleted { identity })
            }
            Effect::BeginHostComposition {
                effect_id,
                session,
                epoch,
                ..
            }
            | Effect::UpdateHostComposition {
                effect_id,
                session,
                epoch,
                ..
            } => Some(Event::EffectResult {
                effect_id,
                result_class: if matches!(effect, Effect::BeginHostComposition { .. }) {
                    EffectResultClass::CompositionStart
                } else {
                    EffectResultClass::CompositionUpdate
                },
                scope: EffectScope::Composition { session, epoch },
                outcome: match self.composition_behavior {
                    FakeCompositionBehavior::Success => EffectOutcome::Succeeded,
                    FakeCompositionBehavior::Rejected => EffectOutcome::Rejected,
                    FakeCompositionBehavior::Indeterminate => EffectOutcome::Indeterminate,
                },
                host_revision: None,
            }),
            Effect::TerminateHostComposition {
                effect_id,
                session,
                epoch,
                ..
            }
            | Effect::CancelComposition {
                effect_id,
                session,
                epoch,
                ..
            } => Some(Event::EffectResult {
                effect_id,
                result_class: EffectResultClass::CompositionTermination,
                scope: EffectScope::Composition { session, epoch },
                outcome: match self.composition_behavior {
                    FakeCompositionBehavior::Success => EffectOutcome::Succeeded,
                    FakeCompositionBehavior::Rejected => EffectOutcome::Rejected,
                    FakeCompositionBehavior::Indeterminate => EffectOutcome::Indeterminate,
                },
                host_revision: None,
            }),
            Effect::ApplyHostCommit {
                effect_id,
                client_instance_id,
                session,
                epoch,
                commit_id,
                ..
            } => {
                let scope = EffectScope::Commit {
                    client_instance_id,
                    session,
                    epoch,
                    commit_id,
                };
                if !self.seen_effect_ids.insert((client_instance_id, effect_id)) {
                    return None;
                }
                if let Some((_, revision)) = self
                    .host_mutations
                    .get(&(client_instance_id, session, commit_id))
                    .copied()
                {
                    return Some(commit_result(
                        effect_id,
                        scope,
                        EffectOutcome::Succeeded,
                        Some(revision),
                    ));
                }
                let (outcome, host_revision) = match self.commit_behavior {
                    FakeCommitBehavior::Success => (
                        EffectOutcome::Succeeded,
                        Some(self.write_once(client_instance_id, session, commit_id)),
                    ),
                    FakeCommitBehavior::Rejected => (EffectOutcome::Rejected, None),
                    FakeCommitBehavior::IndeterminateBeforeWrite => {
                        (EffectOutcome::Indeterminate, None)
                    }
                    FakeCommitBehavior::IndeterminateAfterWrite => (
                        EffectOutcome::Indeterminate,
                        Some(self.write_once(client_instance_id, session, commit_id)),
                    ),
                };
                Some(commit_result(effect_id, scope, outcome, host_revision))
            }
            _ => None,
        }
    }

    fn write_once(
        &mut self,
        client_instance_id: ClientInstanceId,
        session: SessionId,
        commit_id: CommitId,
    ) -> HostRevision {
        let revision = HostRevision(self.next_host_revision);
        self.next_host_revision = self.next_host_revision.saturating_add(1);
        self.host_mutations
            .insert((client_instance_id, session, commit_id), (1, revision));
        revision
    }

    pub fn mutation_count(
        &self,
        client_instance_id: ClientInstanceId,
        session: SessionId,
        commit_id: CommitId,
    ) -> u32 {
        self.host_mutations
            .get(&(client_instance_id, session, commit_id))
            .map_or(0, |(count, _)| *count)
    }
}

fn commit_result(
    effect_id: EffectId,
    scope: ime_runtime_core::EffectScope,
    outcome: ime_runtime_core::EffectOutcome,
    host_revision: Option<HostRevision>,
) -> Event {
    Event::EffectResult {
        effect_id,
        result_class: ime_runtime_core::EffectResultClass::HostCommit,
        scope,
        outcome,
        host_revision,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Replay {
    pub state: RuntimeState,
    pub transitions: Vec<TransitionResult>,
    pub canonical_trace: Vec<u8>,
}

/// Delta-debug a failing event sequence while preserving a caller-supplied failure predicate.
pub fn shrink_event_trace(mut trace: Vec<Event>, fails: impl Fn(&[Event]) -> bool) -> Vec<Event> {
    if !fails(&trace) || trace.len() < 2 {
        return trace;
    }
    let mut granularity = 2usize;
    while trace.len() >= 2 {
        let chunk = trace.len().div_ceil(granularity);
        let mut reduced = false;
        for start in (0..trace.len()).step_by(chunk) {
            let end = (start + chunk).min(trace.len());
            let mut candidate = trace.clone();
            candidate.drain(start..end);
            if !candidate.is_empty() && fails(&candidate) {
                trace = candidate;
                granularity = granularity.saturating_sub(1).max(2);
                reduced = true;
                break;
            }
        }
        if reduced {
            continue;
        }
        if granularity >= trace.len() {
            break;
        }
        granularity = (granularity * 2).min(trace.len());
    }
    trace
}

#[cfg(test)]
fn trace_panics(initial: RuntimeState, events: &[Event]) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        events
            .iter()
            .fold(initial, |state, event| reduce(state, *event).next_state);
    }))
    .is_err()
}

#[cfg(test)]
fn persist_failure_trace(
    directory: &std::path::Path,
    seed: u64,
    canonical_trace: &[u8],
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(format!("runtime-{seed}.trace"));
    std::fs::write(&path, canonical_trace)?;
    Ok(path)
}

pub fn replay(initial: RuntimeState, events: impl IntoIterator<Item = Event>) -> Replay {
    let mut state = initial;
    let mut transitions = Vec::new();
    let mut canonical_trace = b"wufan-trace-v5".to_vec();
    let initial_bytes = initial.canonical_bytes();
    canonical_trace.extend_from_slice(&(initial_bytes.len() as u64).to_le_bytes());
    canonical_trace.extend_from_slice(&initial_bytes);
    for event in events {
        encode_event(&mut canonical_trace, event);
        let transition = reduce(state, event);
        let encoded = transition.canonical_bytes();
        canonical_trace.extend_from_slice(&(encoded.len() as u64).to_le_bytes());
        canonical_trace.extend_from_slice(&encoded);
        state = transition.next_state;
        transitions.push(transition);
    }
    Replay {
        state,
        transitions,
        canonical_trace,
    }
}

/// Deterministic single-thread scheduler: reduce each host/broker input, execute its Effects
/// through the fake adapter, then reduce the resulting events before accepting the next input.
pub fn run_with_fake_adapter(
    initial: RuntimeState,
    events: impl IntoIterator<Item = Event>,
    adapter: &mut FakeEffectAdapter,
) -> Replay {
    let mut state = initial;
    let mut transitions = Vec::new();
    let mut canonical_trace = b"wufan-trace-v5".to_vec();
    let initial_bytes = initial.canonical_bytes();
    canonical_trace.extend_from_slice(&(initial_bytes.len() as u64).to_le_bytes());
    canonical_trace.extend_from_slice(&initial_bytes);
    let mut step_count = 0usize;
    for input in events {
        let mut ready = VecDeque::from([input]);
        while let Some(event) = ready.pop_front() {
            step_count += 1;
            assert!(
                step_count <= 1_000_000,
                "fake scheduler exceeded bounded step budget"
            );
            encode_event(&mut canonical_trace, event);
            let transition = reduce(state, event);
            for effect in transition.effects.into_iter().flatten() {
                if let Some(response) = adapter.execute(effect) {
                    ready.push_back(response);
                }
            }
            let encoded = transition.canonical_bytes();
            canonical_trace.extend_from_slice(&(encoded.len() as u64).to_le_bytes());
            canonical_trace.extend_from_slice(&encoded);
            state = transition.next_state;
            transitions.push(transition);
        }
    }
    Replay {
        state,
        transitions,
        canonical_trace,
    }
}

fn encode_event(out: &mut Vec<u8>, event: Event) {
    use ime_runtime_core::{EffectOutcome, EffectResultClass, EffectScope, KeyPhase};
    fn u64le(out: &mut Vec<u8>, value: u64) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    match event {
        Event::HostKey { phase, key } => {
            out.push(0);
            out.push(match phase {
                KeyPhase::Test => 0,
                KeyPhase::Actual => 1,
            });
            u64le(out, key.token);
            out.extend_from_slice(&key.virtual_key.to_le_bytes());
            out.extend_from_slice(&key.scan_code.to_le_bytes());
            out.extend_from_slice(&key.modifiers.to_le_bytes());
            out.push(u8::from(key.repeat));
            u64le(out, key.now_ms);
        }
        Event::HostKeyUp { phase, key } => {
            out.push(16);
            out.push(match phase {
                KeyPhase::Test => 0,
                KeyPhase::Actual => 1,
            });
            u64le(out, key.token);
            out.extend_from_slice(&key.virtual_key.to_le_bytes());
            out.extend_from_slice(&key.scan_code.to_le_bytes());
            out.extend_from_slice(&key.modifiers.to_le_bytes());
            out.push(u8::from(key.repeat));
            u64le(out, key.now_ms);
        }
        Event::ContextPushed { session } => {
            out.push(17);
            u64le(out, session.0);
        }
        Event::ContextPopped { session } => {
            out.push(18);
            u64le(out, session.0);
        }
        Event::DecisionTtlExpired { token, now_ms } => {
            out.push(19);
            u64le(out, token);
            u64le(out, now_ms);
        }
        Event::PhysicalKey { virtual_key, down } => {
            out.push(13);
            out.extend_from_slice(&virtual_key.to_le_bytes());
            out.push(u8::from(down));
        }
        Event::FocusGained { session } => {
            out.push(1);
            u64le(out, session.0);
        }
        Event::FocusLost { session } => {
            out.push(2);
            u64le(out, session.0);
        }
        Event::BrokerDisconnected => out.push(3),
        Event::HostClosing => out.push(14),
        Event::InputMethodDeactivated => out.push(15),
        Event::BrokerReady { generation } => {
            out.push(6);
            u64le(out, generation.0);
        }
        Event::RequestCompleted { identity } => {
            out.push(7);
            encode_identity(out, identity);
        }
        Event::DeadlineExpired { request_seq } => {
            out.push(10);
            u64le(out, request_seq.0);
        }
        Event::PumpMutatingQueue => out.push(8),
        Event::EngineUpdate {
            identity,
            preedit_token_id,
        } => {
            out.push(9);
            encode_identity(out, identity);
            match preedit_token_id {
                None => out.push(0),
                Some(token_id) => {
                    out.push(1);
                    u64le(out, token_id);
                }
            }
        }
        Event::CandidateViewUpdated {
            identity,
            revision,
            candidate_count,
        } => {
            out.push(11);
            encode_identity(out, identity);
            u64le(out, revision);
            out.extend_from_slice(&candidate_count.to_le_bytes());
        }
        Event::UIActionIntent {
            identity,
            revision,
            candidate_index,
        } => {
            out.push(12);
            encode_identity(out, identity);
            u64le(out, revision);
            out.extend_from_slice(&candidate_index.to_le_bytes());
        }
        Event::EffectResult {
            effect_id,
            result_class,
            scope,
            outcome,
            host_revision,
        } => {
            out.push(4);
            u64le(out, effect_id.0);
            out.push(match result_class {
                EffectResultClass::CompositionTermination => 0,
                EffectResultClass::CompositionStart => 1,
                EffectResultClass::CompositionUpdate => 2,
                EffectResultClass::HostCommit => 3,
            });
            match scope {
                EffectScope::Composition { session, epoch } => {
                    out.push(0);
                    u64le(out, session.0);
                    u64le(out, epoch.0);
                }
                EffectScope::Commit {
                    client_instance_id,
                    session,
                    epoch,
                    commit_id,
                } => {
                    out.push(1);
                    u64le(out, client_instance_id.0);
                    u64le(out, session.0);
                    u64le(out, epoch.0);
                    u64le(out, commit_id.0);
                }
            }
            out.push(match outcome {
                EffectOutcome::Succeeded => 0,
                EffectOutcome::Rejected => 1,
                EffectOutcome::Indeterminate => 2,
            });
            out.push(u8::from(host_revision.is_some()));
            if let Some(revision) = host_revision {
                u64le(out, revision.0);
            }
        }
        Event::CommitIntent {
            identity,
            commit_id,
            token_id,
        } => {
            out.push(5);
            encode_identity(out, identity);
            u64le(out, commit_id.0);
            u64le(out, token_id);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceDecodeError {
    InvalidHeader,
    Truncated,
    InitialStateMismatch,
    UnknownEventTag(u8),
    InvalidEnumValue,
}

struct TraceReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> TraceReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], TraceDecodeError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(TraceDecodeError::Truncated)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(TraceDecodeError::Truncated)?;
        self.offset = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, TraceDecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, TraceDecodeError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, TraceDecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn identity(&mut self) -> Result<MessageIdentity, TraceDecodeError> {
        Ok(MessageIdentity {
            client_instance_id: ClientInstanceId(self.u64()?),
            broker_generation: BrokerGeneration(self.u64()?),
            session_id: SessionId(self.u64()?),
            focus_epoch: Epoch(self.u64()?),
            composition_epoch: Epoch(self.u64()?),
            request_seq: RequestSeq(self.u64()?),
        })
    }
}

/// Recover input events from a canonical trace, checking its schema and initial state.
pub fn decode_trace_events(
    initial_state: RuntimeState,
    canonical_trace: &[u8],
) -> Result<Vec<Event>, TraceDecodeError> {
    const HEADER: &[u8] = b"wufan-trace-v5";
    if !canonical_trace.starts_with(HEADER) {
        return Err(TraceDecodeError::InvalidHeader);
    }
    let mut reader = TraceReader::new(canonical_trace);
    reader.take(HEADER.len())?;
    let state_len = usize::try_from(reader.u64()?).map_err(|_| TraceDecodeError::Truncated)?;
    if reader.take(state_len)? != initial_state.canonical_bytes() {
        return Err(TraceDecodeError::InitialStateMismatch);
    }
    let mut events = Vec::new();
    while reader.remaining() != 0 {
        let event = decode_event(&mut reader)?;
        let transition_len =
            usize::try_from(reader.u64()?).map_err(|_| TraceDecodeError::Truncated)?;
        reader.take(transition_len)?;
        events.push(event);
    }
    Ok(events)
}

fn decode_event(reader: &mut TraceReader<'_>) -> Result<Event, TraceDecodeError> {
    use ime_runtime_core::{
        EffectOutcome, EffectResultClass, EffectScope, KeyObservation, KeyPhase,
    };
    let tag = reader.u8()?;
    let invalid = || TraceDecodeError::InvalidEnumValue;
    match tag {
        0 => {
            let phase = match reader.u8()? {
                0 => KeyPhase::Test,
                1 => KeyPhase::Actual,
                _ => return Err(invalid()),
            };
            Ok(Event::HostKey {
                phase,
                key: KeyObservation {
                    token: reader.u64()?,
                    virtual_key: reader.u16()?,
                    scan_code: reader.u16()?,
                    modifiers: reader.u16()?,
                    repeat: match reader.u8()? {
                        0 => false,
                        1 => true,
                        _ => return Err(invalid()),
                    },
                    now_ms: reader.u64()?,
                },
            })
        }
        16 => {
            let phase = match reader.u8()? {
                0 => KeyPhase::Test,
                1 => KeyPhase::Actual,
                _ => return Err(invalid()),
            };
            Ok(Event::HostKeyUp {
                phase,
                key: KeyObservation {
                    token: reader.u64()?,
                    virtual_key: reader.u16()?,
                    scan_code: reader.u16()?,
                    modifiers: reader.u16()?,
                    repeat: match reader.u8()? {
                        0 => false,
                        1 => true,
                        _ => return Err(invalid()),
                    },
                    now_ms: reader.u64()?,
                },
            })
        }
        17 => Ok(Event::ContextPushed {
            session: SessionId(reader.u64()?),
        }),
        18 => Ok(Event::ContextPopped {
            session: SessionId(reader.u64()?),
        }),
        19 => Ok(Event::DecisionTtlExpired {
            token: reader.u64()?,
            now_ms: reader.u64()?,
        }),
        1 => Ok(Event::FocusGained {
            session: SessionId(reader.u64()?),
        }),
        2 => Ok(Event::FocusLost {
            session: SessionId(reader.u64()?),
        }),
        3 => Ok(Event::BrokerDisconnected),
        4 => {
            let effect_id = EffectId(reader.u64()?);
            let result_class = match reader.u8()? {
                0 => EffectResultClass::CompositionTermination,
                1 => EffectResultClass::CompositionStart,
                2 => EffectResultClass::CompositionUpdate,
                3 => EffectResultClass::HostCommit,
                _ => return Err(invalid()),
            };
            let scope = match reader.u8()? {
                0 => EffectScope::Composition {
                    session: SessionId(reader.u64()?),
                    epoch: Epoch(reader.u64()?),
                },
                1 => EffectScope::Commit {
                    client_instance_id: ClientInstanceId(reader.u64()?),
                    session: SessionId(reader.u64()?),
                    epoch: Epoch(reader.u64()?),
                    commit_id: CommitId(reader.u64()?),
                },
                _ => return Err(invalid()),
            };
            let outcome = match reader.u8()? {
                0 => EffectOutcome::Succeeded,
                1 => EffectOutcome::Rejected,
                2 => EffectOutcome::Indeterminate,
                _ => return Err(invalid()),
            };
            let host_revision = match reader.u8()? {
                0 => None,
                1 => Some(HostRevision(reader.u64()?)),
                _ => return Err(invalid()),
            };
            Ok(Event::EffectResult {
                effect_id,
                result_class,
                scope,
                outcome,
                host_revision,
            })
        }
        5 => Ok(Event::CommitIntent {
            identity: reader.identity()?,
            commit_id: CommitId(reader.u64()?),
            token_id: reader.u64()?,
        }),
        6 => Ok(Event::BrokerReady {
            generation: BrokerGeneration(reader.u64()?),
        }),
        7 => Ok(Event::RequestCompleted {
            identity: reader.identity()?,
        }),
        8 => Ok(Event::PumpMutatingQueue),
        9 => {
            let identity = reader.identity()?;
            let preedit_token_id = match reader.u8()? {
                0 => None,
                1 => Some(reader.u64()?),
                _ => return Err(invalid()),
            };
            Ok(Event::EngineUpdate {
                identity,
                preedit_token_id,
            })
        }
        10 => Ok(Event::DeadlineExpired {
            request_seq: RequestSeq(reader.u64()?),
        }),
        11 => Ok(Event::CandidateViewUpdated {
            identity: reader.identity()?,
            revision: reader.u64()?,
            candidate_count: reader.u16()?,
        }),
        12 => Ok(Event::UIActionIntent {
            identity: reader.identity()?,
            revision: reader.u64()?,
            candidate_index: reader.u16()?,
        }),
        13 => Ok(Event::PhysicalKey {
            virtual_key: reader.u16()?,
            down: match reader.u8()? {
                0 => false,
                1 => true,
                _ => return Err(invalid()),
            },
        }),
        14 => Ok(Event::HostClosing),
        15 => Ok(Event::InputMethodDeactivated),
        unknown => Err(TraceDecodeError::UnknownEventTag(unknown)),
    }
}

fn encode_identity(out: &mut Vec<u8>, identity: ime_protocol::MessageIdentity) {
    out.extend_from_slice(&identity.client_instance_id.0.to_le_bytes());
    out.extend_from_slice(&identity.broker_generation.0.to_le_bytes());
    out.extend_from_slice(&identity.session_id.0.to_le_bytes());
    out.extend_from_slice(&identity.focus_epoch.0.to_le_bytes());
    out.extend_from_slice(&identity.composition_epoch.0.to_le_bytes());
    out.extend_from_slice(&identity.request_seq.0.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use ime_protocol::{ClientInstanceId, SessionId};
    use ime_runtime_core::{CompositionState, Effect, KeyObservation, KeyPhase};
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 32,
            max_shrink_iters: 4_096,
            .. ProptestConfig::default()
        })]

        #[test]
        fn proptest_runtime_prefix_safety(seed in any::<u64>(), inputs in prop::collection::vec(any::<u64>(), 1_000..=1_256)) {
            use ime_protocol::{BrokerGeneration, CommitId, EffectId, Epoch, HostRevision, MessageIdentity, RequestSeq};
            use ime_runtime_core::{EffectOutcome, EffectResultClass, EffectScope};

            let client = ClientInstanceId(seed.max(1));
            let mut state = RuntimeState::new(client);
            let mut events = Vec::with_capacity(inputs.len());
            for value in inputs {
                let session = state.active_session.unwrap_or(SessionId(value % 19 + 1));
                let identity = MessageIdentity {
                    client_instance_id: client,
                    broker_generation: state.broker_generation.unwrap_or(BrokerGeneration(value % 7 + 1)),
                    session_id: session,
                    focus_epoch: state.focus_epoch,
                    composition_epoch: state.composition_epoch,
                    request_seq: RequestSeq(value % 32 + 1),
                };
                let key = KeyObservation {
                    token: value,
                    virtual_key: (value % 256) as u16,
                    scan_code: (value % 128) as u16,
                    modifiers: (value % 16) as u16,
                    repeat: value & 1 != 0,
                    now_ms: value % 1_000,
                };
                let event = match value % 17 {
                    0 => Event::HostKey { phase: KeyPhase::Test, key },
                    1 => Event::HostKey { phase: KeyPhase::Actual, key },
                    2 => Event::PhysicalKey { virtual_key: key.virtual_key, down: value & 2 != 0 },
                    3 => Event::FocusGained { session },
                    4 => Event::FocusLost { session },
                    5 => Event::BrokerDisconnected,
                    6 => Event::BrokerReady { generation: BrokerGeneration(value % 7 + 1) },
                    7 => Event::RequestCompleted { identity },
                    8 => Event::EngineUpdate { identity, preedit_token_id: (value & 2 == 0).then_some(value) },
                    9 => Event::EffectResult {
                        effect_id: EffectId(value % 32 + 1),
                        result_class: match value % 4 {
                            0 => EffectResultClass::CompositionTermination,
                            1 => EffectResultClass::CompositionStart,
                            2 => EffectResultClass::CompositionUpdate,
                            _ => EffectResultClass::HostCommit,
                        },
                        scope: if value & 4 == 0 {
                            EffectScope::Composition { session, epoch: Epoch(value % 8) }
                        } else {
                            EffectScope::Commit { client_instance_id: client, session, epoch: Epoch(value % 8), commit_id: CommitId(value % 16 + 1) }
                        },
                        outcome: match value % 3 {
                            0 => EffectOutcome::Succeeded,
                            1 => EffectOutcome::Rejected,
                            _ => EffectOutcome::Indeterminate,
                        },
                        host_revision: (value & 8 == 0).then_some(HostRevision(value % 32 + 1)),
                    },
                    10 => Event::CommitIntent { identity, commit_id: CommitId(value % 16 + 1), token_id: value },
                    12 => Event::HostKeyUp { phase: KeyPhase::Test, key },
                    13 => Event::HostKeyUp { phase: KeyPhase::Actual, key },
                    14 => Event::ContextPushed { session },
                    15 => Event::ContextPopped { session },
                    16 => Event::DecisionTtlExpired { token: value, now_ms: value % 1_000 },
                    _ => if value & 1 == 0 { Event::HostClosing } else { Event::InputMethodDeactivated },
                };
                let transition = reduce(state, event);
                prop_assert!(transition.next_state.model_invariants_hold());
                let reply_matches = match event {
                    Event::HostKey { phase: KeyPhase::Test, .. } => matches!(transition.immediate, Some(ime_runtime_core::ImmediateReply::TestKey(_))),
                    Event::HostKey { phase: KeyPhase::Actual, .. } => matches!(transition.immediate, Some(ime_runtime_core::ImmediateReply::Key(_))),
                    Event::HostKeyUp { phase: KeyPhase::Test, .. } => matches!(transition.immediate, Some(ime_runtime_core::ImmediateReply::TestKeyUp(_))),
                    Event::HostKeyUp { phase: KeyPhase::Actual, .. } => matches!(transition.immediate, Some(ime_runtime_core::ImmediateReply::KeyUp(_))),
                    _ => transition.immediate.is_none(),
                };
                prop_assert!(reply_matches, "S3 reply mismatch for {event:?}");
                state = transition.next_state;
                events.push(event);
            }
            let first = replay(RuntimeState::new(client), events.iter().copied());
            let decoded = decode_trace_events(RuntimeState::new(client), &first.canonical_trace).unwrap();
            let second = replay(RuntimeState::new(client), decoded);
            prop_assert_eq!(first, second);

            let initial = RuntimeState::new(client);
            let mut adapter = FakeEffectAdapter::default();
            let scheduled = run_with_fake_adapter(initial, events.iter().copied(), &mut adapter);
            let scheduled_events = decode_trace_events(initial, &scheduled.canonical_trace).unwrap();
            prop_assert_eq!(scheduled_events.len(), scheduled.transitions.len());
            let mut emitted_commits = std::collections::BTreeSet::new();
            for (event, transition) in scheduled_events.into_iter().zip(&scheduled.transitions) {
                prop_assert!(transition.next_state.model_invariants_hold());
                let reply_matches = match event {
                    Event::HostKey { phase: KeyPhase::Test, .. } => matches!(transition.immediate, Some(ime_runtime_core::ImmediateReply::TestKey(_))),
                    Event::HostKey { phase: KeyPhase::Actual, .. } => matches!(transition.immediate, Some(ime_runtime_core::ImmediateReply::Key(_))),
                    Event::HostKeyUp { phase: KeyPhase::Test, .. } => matches!(transition.immediate, Some(ime_runtime_core::ImmediateReply::TestKeyUp(_))),
                    Event::HostKeyUp { phase: KeyPhase::Actual, .. } => matches!(transition.immediate, Some(ime_runtime_core::ImmediateReply::KeyUp(_))),
                    _ => transition.immediate.is_none(),
                };
                prop_assert!(reply_matches, "scheduled S3 reply mismatch for {event:?}");
                for effect in transition.effects.iter().flatten() {
                    if let Effect::ApplyHostCommit { client_instance_id, session, commit_id, .. } = effect {
                        prop_assert!(
                            emitted_commits.insert((client_instance_id.0, session.0, commit_id.0)),
                            "duplicate ApplyHostCommit identity in one trace"
                        );
                    }
                }
                for entry in transition.next_state.commit_entries() {
                    if matches!(entry.status, ime_runtime_core::CommitStatus::Applied { .. }) {
                        prop_assert_eq!(
                            adapter.mutation_count(entry.identity.client_instance_id, entry.identity.session_id, entry.commit_id),
                            1,
                            "Applied status must agree with exactly one host journal write"
                        );
                    }
                }
            }
        }

        #[test]
        fn proptest_commit_oracle_preserves_i2_and_i6(session_value in 1u64..=u64::MAX, commit_value in 1u64..=u64::MAX, token_id in any::<u64>(), behavior_value in 0u8..4) {
            use ime_protocol::{ClientInstanceId, CommitId, Epoch, SessionId};
            use ime_runtime_core::{CommitStatus, Effect};

            let session = SessionId(session_value);
            let commit_id = CommitId(commit_value);
            let initial = active_request_state(session, Epoch(9));
            let intent = commit_event(session, commit_id, Epoch(9), token_id);
            let applying = reduce(initial, intent);
            let Some(effect @ Effect::ApplyHostCommit { .. }) = applying.effects[0] else {
                prop_assert!(false, "valid commit intent must create exactly one host effect");
                return Ok(());
            };
            prop_assert!(applying.next_state.model_invariants_hold());

            let behavior = match behavior_value {
                0 => FakeCommitBehavior::Success,
                1 => FakeCommitBehavior::Rejected,
                2 => FakeCommitBehavior::IndeterminateBeforeWrite,
                _ => FakeCommitBehavior::IndeterminateAfterWrite,
            };
            let should_write = matches!(behavior, FakeCommitBehavior::Success | FakeCommitBehavior::IndeterminateAfterWrite);
            let mut adapter = FakeEffectAdapter { commit_behavior: behavior, ..FakeEffectAdapter::default() };
            let terminal_event = adapter.execute(effect).expect("commit effect has one terminal result");
            let terminal = reduce(applying.next_state, terminal_event);
            prop_assert!(terminal.next_state.model_invariants_hold());
            let writes = adapter.mutation_count(ClientInstanceId(5), session, commit_id);
            prop_assert_eq!(writes, u32::from(should_write));

            let acknowledged_applied = terminal.effects.iter().flatten().any(|effect| {
                matches!(effect, Effect::SendCommitApplied { commit_id: id, .. } if *id == commit_id)
            });
            if acknowledged_applied {
                prop_assert_eq!(writes, 1, "Applied ACK requires exactly one host mutation");
            }
            let applied = terminal.next_state.commit_entries().any(|entry| {
                entry.commit_id == commit_id
                    && matches!(entry.status, CommitStatus::Applied { .. })
            });
            prop_assert_eq!(applied, acknowledged_applied);

            // Replaying the same intent may resend a terminal ACK, never host mutation.
            let duplicate = reduce(terminal.next_state, intent);
            prop_assert!(duplicate.next_state.model_invariants_hold());
            prop_assert!(!duplicate.effects.iter().flatten().any(|effect| {
                matches!(effect, Effect::ApplyHostCommit { .. })
            }), "duplicate intent must not issue another host commit");
            prop_assert_eq!(adapter.mutation_count(ClientInstanceId(5), session, commit_id), writes);
            let duplicate_result = reduce(terminal.next_state, terminal_event);
            prop_assert_eq!(duplicate_result.next_state.commit_entries().collect::<Vec<_>>(), terminal.next_state.commit_entries().collect::<Vec<_>>());
            prop_assert!(
                matches!(duplicate_result.effects[0], Some(ime_runtime_core::Effect::RecordDiagnostic { .. })),
                "duplicate terminal result should only record a diagnostic"
            );
            prop_assert!(!duplicate_result.effects.iter().flatten().any(|effect| {
                matches!(effect, Effect::ApplyHostCommit { .. })
            }), "duplicate terminal result must not issue another host commit");
        }

        #[test]
        fn proptest_stale_commit_identity_is_rejected_before_host_effect(
            session_value in 1u64..=u64::MAX,
            commit_value in 1u64..=u64::MAX,
            stale_dimension in 0u8..6,
        ) {
            use ime_protocol::{ClientInstanceId, CommitId, Epoch, SessionId};

            let session = SessionId(session_value);
            let initial = active_request_state(session, Epoch(9));
            let Event::CommitIntent { mut identity, commit_id: _, token_id: _ } =
                commit_event(session, CommitId(commit_value), Epoch(9), 17) else { unreachable!() };
            match stale_dimension {
                0 => identity.client_instance_id = ClientInstanceId(identity.client_instance_id.0 + 1),
                1 => identity.broker_generation = ime_protocol::BrokerGeneration(identity.broker_generation.0 + 1),
                2 => identity.session_id = SessionId(identity.session_id.0 ^ 1),
                3 => identity.focus_epoch = Epoch(identity.focus_epoch.0 + 1),
                4 => identity.composition_epoch = Epoch(identity.composition_epoch.0 + 1),
                _ => identity.request_seq = ime_protocol::RequestSeq(identity.request_seq.0 + 1),
            }
            let stale = Event::CommitIntent { identity, commit_id: CommitId(commit_value), token_id: 17 };
            let transition = reduce(initial, stale);
            prop_assert_eq!(transition.next_state.mode, initial.mode);
            prop_assert_eq!(transition.next_state.active_session, initial.active_session);
            prop_assert_eq!(transition.next_state.composition, initial.composition);
            prop_assert!(
                matches!(transition.effects[0], Some(ime_runtime_core::Effect::RecordDiagnostic { .. })),
                "stale commit identity should only record a diagnostic"
            );
            prop_assert!(!transition.effects.iter().flatten().any(|effect| {
                matches!(effect, ime_runtime_core::Effect::ApplyHostCommit { .. })
            }), "stale identity must not issue a host commit");
        }
    }

    #[test]
    fn seeded_mixed_traces_preserve_prefix_invariants_and_replay_identically() {
        use ime_protocol::{
            BrokerGeneration, ClientInstanceId, CommitId, EffectId, Epoch, HostRevision,
            MessageIdentity, RequestSeq,
        };
        use ime_runtime_core::{EffectOutcome, EffectResultClass, EffectScope};

        fn next(random: &mut u64) -> u64 {
            *random ^= *random << 13;
            *random ^= *random >> 7;
            *random ^= *random << 17;
            *random
        }
        fn event(random: &mut u64, state: RuntimeState) -> Event {
            let value = next(random);
            let session = state.active_session.unwrap_or(SessionId(value % 7 + 1));
            let identity = MessageIdentity {
                client_instance_id: state.client_instance_id,
                broker_generation: state
                    .broker_generation
                    .unwrap_or(BrokerGeneration(value % 5 + 1)),
                session_id: session,
                focus_epoch: if value & 1 == 0 {
                    state.focus_epoch
                } else {
                    Epoch(value % 10)
                },
                composition_epoch: if value & 2 == 0 {
                    state.composition_epoch
                } else {
                    Epoch(value % 10)
                },
                request_seq: RequestSeq(value % 20 + 1),
            };
            match value % 23 {
                0 => Event::HostKey {
                    phase: KeyPhase::Test,
                    key: KeyObservation {
                        token: value,
                        virtual_key: (value % 256) as u16,
                        scan_code: (value % 128) as u16,
                        modifiers: (value % 8) as u16,
                        repeat: value & 4 != 0,
                        now_ms: value % 500,
                    },
                },
                13 => Event::PhysicalKey {
                    virtual_key: [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C]
                        [(value as usize >> 8) % 8],
                    down: value & 1 == 0,
                },
                14 => Event::HostClosing,
                15 => Event::InputMethodDeactivated,
                16 => Event::PumpMutatingQueue,
                17 => Event::HostKeyUp {
                    phase: KeyPhase::Test,
                    key: KeyObservation {
                        token: value,
                        virtual_key: (value % 256) as u16,
                        scan_code: (value % 128) as u16,
                        modifiers: (value % 8) as u16,
                        repeat: value & 4 != 0,
                        now_ms: value % 500,
                    },
                },
                18 => Event::HostKeyUp {
                    phase: KeyPhase::Actual,
                    key: KeyObservation {
                        token: value,
                        virtual_key: (value % 256) as u16,
                        scan_code: (value % 128) as u16,
                        modifiers: (value % 8) as u16,
                        repeat: value & 4 != 0,
                        now_ms: value % 500,
                    },
                },
                19 => Event::ContextPushed { session },
                20 => Event::ContextPopped { session },
                22 => Event::DecisionTtlExpired {
                    token: value,
                    now_ms: value % 500,
                },
                1 => Event::HostKey {
                    phase: KeyPhase::Actual,
                    key: KeyObservation {
                        token: value,
                        virtual_key: (value % 256) as u16,
                        scan_code: (value % 128) as u16,
                        modifiers: (value % 8) as u16,
                        repeat: value & 4 != 0,
                        now_ms: value % 500,
                    },
                },
                2 => Event::FocusGained { session },
                3 => Event::FocusLost { session },
                4 => Event::BrokerDisconnected,
                5 => Event::BrokerReady {
                    generation: BrokerGeneration(value % 5 + 1),
                },
                6 => Event::RequestCompleted { identity },
                7 => Event::DeadlineExpired {
                    request_seq: RequestSeq(value % 20 + 1),
                },
                8 => Event::EngineUpdate {
                    identity,
                    preedit_token_id: (value & 1 == 0).then_some(value % 100),
                },
                9 => Event::CandidateViewUpdated {
                    identity,
                    revision: value % 5 + 1,
                    candidate_count: (value % 8) as u16,
                },
                10 => Event::UIActionIntent {
                    identity,
                    revision: value % 5 + 1,
                    candidate_index: (value % 8) as u16,
                },
                11 => Event::CommitIntent {
                    identity,
                    commit_id: CommitId(value % 12 + 1),
                    token_id: value % 100,
                },
                _ => {
                    let composition = value & 1 == 0;
                    Event::EffectResult {
                        effect_id: EffectId(value % 20 + 1),
                        result_class: match (value >> 8) % 4 {
                            0 => EffectResultClass::CompositionStart,
                            1 => EffectResultClass::CompositionUpdate,
                            2 => EffectResultClass::CompositionTermination,
                            _ => EffectResultClass::HostCommit,
                        },
                        scope: if composition {
                            EffectScope::Composition {
                                session,
                                epoch: Epoch(value % 10),
                            }
                        } else {
                            EffectScope::Commit {
                                client_instance_id: identity.client_instance_id,
                                session,
                                epoch: Epoch(value % 10),
                                commit_id: CommitId(value % 12 + 1),
                            }
                        },
                        outcome: match (value >> 8) % 3 {
                            0 => EffectOutcome::Succeeded,
                            1 => EffectOutcome::Rejected,
                            _ => EffectOutcome::Indeterminate,
                        },
                        host_revision: (value & 2 == 0).then_some(HostRevision(value % 100 + 1)),
                    }
                }
            }
        }

        let mut seeds: Vec<u64> = (1..=16).collect();
        if let Ok(extra_seed) = std::env::var("WUFAN_EXTRA_SEED") {
            if let Ok(extra_seed) = extra_seed.parse::<u64>() {
                let extra_seed = extra_seed.max(1);
                if !seeds.contains(&extra_seed) {
                    seeds.push(extra_seed);
                }
            }
        }
        let mut seen_event_kinds = [false; 22];
        let mut seen_result_classes = [false; 4];
        let mut seen_result_outcomes = [false; 3];
        for seed in seeds {
            let mut random = seed;
            let mut state = RuntimeState::new(ClientInstanceId(seed));
            let mut events = Vec::with_capacity(1_000);
            for _ in 0..1_000 {
                let next_event = event(&mut random, state);
                let event_kind = match next_event {
                    Event::HostKey {
                        phase: KeyPhase::Test,
                        ..
                    } => 0,
                    Event::HostKey {
                        phase: KeyPhase::Actual,
                        ..
                    } => 1,
                    Event::HostKeyUp {
                        phase: KeyPhase::Test,
                        ..
                    } => 17,
                    Event::HostKeyUp {
                        phase: KeyPhase::Actual,
                        ..
                    } => 18,
                    Event::ContextPushed { .. } => 19,
                    Event::ContextPopped { .. } => 20,
                    Event::DecisionTtlExpired { .. } => 21,
                    Event::PhysicalKey { .. } => 2,
                    Event::FocusGained { .. } => 3,
                    Event::FocusLost { .. } => 4,
                    Event::BrokerDisconnected => 5,
                    Event::BrokerReady { .. } => 6,
                    Event::RequestCompleted { .. } => 7,
                    Event::DeadlineExpired { .. } => 8,
                    Event::PumpMutatingQueue => 9,
                    Event::EngineUpdate { .. } => 10,
                    Event::CandidateViewUpdated { .. } => 11,
                    Event::UIActionIntent { .. } => 12,
                    Event::EffectResult { .. } => 13,
                    Event::CommitIntent { .. } => 14,
                    Event::HostClosing => 15,
                    Event::InputMethodDeactivated => 16,
                };
                seen_event_kinds[event_kind] = true;
                if let Event::EffectResult {
                    result_class,
                    outcome,
                    ..
                } = next_event
                {
                    seen_result_classes[result_class as usize] = true;
                    seen_result_outcomes[outcome as usize] = true;
                }
                let transition = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || reduce(state, next_event),
                )) {
                    Ok(transition) => transition,
                    Err(_) => {
                        events.push(next_event);
                        let minimized = shrink_event_trace(events.clone(), |trace| {
                            trace_panics(RuntimeState::new(ClientInstanceId(seed)), trace)
                        });
                        let initial = RuntimeState::new(ClientInstanceId(seed));
                        let trace_bytes =
                            replay(initial, minimized.iter().copied()).canonical_trace;
                        let artifact_dir = std::env::var_os("WUFAN_TRACE_ARTIFACT_DIR")
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| {
                                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                                    .join("regression-traces")
                            });
                        let artifact_result =
                            persist_failure_trace(&artifact_dir, seed, &trace_bytes);
                        panic!(
                            "seed={seed}; minimized failing trace: {minimized:#?}; artifact={artifact_result:?}"
                        );
                    }
                };
                assert!(transition.next_state.model_invariants_hold(), "seed={seed}");
                assert_prefix_reply_contract(next_event, transition.immediate, seed);
                state = transition.next_state;
                events.push(next_event);
            }
            let first = replay(
                RuntimeState::new(ClientInstanceId(seed)),
                events.iter().copied(),
            );
            let second = replay(RuntimeState::new(ClientInstanceId(seed)), events);
            let recovered = decode_trace_events(
                RuntimeState::new(ClientInstanceId(seed)),
                &first.canonical_trace,
            )
            .expect("every generated trace decodes");
            let third = replay(RuntimeState::new(ClientInstanceId(seed)), recovered);
            assert_eq!(first.canonical_trace, second.canonical_trace, "seed={seed}");
            assert_eq!(first, third, "decoded replay diverged; seed={seed}");
            assert_eq!(
                first.state.structural_hash(),
                second.state.structural_hash(),
                "seed={seed}"
            );
        }
        assert!(
            seen_event_kinds.into_iter().all(|seen| seen),
            "every event kind must be generated"
        );
        assert!(
            seen_result_classes.into_iter().all(|seen| seen),
            "every effect result class must be generated"
        );
        assert!(
            seen_result_outcomes.into_iter().all(|seen| seen),
            "every effect outcome must be generated"
        );
    }

    fn assert_prefix_reply_contract(
        event: Event,
        reply: Option<ime_runtime_core::ImmediateReply>,
        seed: u64,
    ) {
        use ime_runtime_core::{ImmediateReply, KeyPhase};

        let valid = match event {
            Event::HostKey {
                phase: KeyPhase::Test,
                ..
            } => matches!(reply, Some(ImmediateReply::TestKey(_))),
            Event::HostKey {
                phase: KeyPhase::Actual,
                ..
            } => matches!(reply, Some(ImmediateReply::Key(_))),
            Event::HostKeyUp {
                phase: KeyPhase::Test,
                ..
            } => matches!(reply, Some(ImmediateReply::TestKeyUp(_))),
            Event::HostKeyUp {
                phase: KeyPhase::Actual,
                ..
            } => matches!(reply, Some(ImmediateReply::KeyUp(_))),
            _ => reply.is_none(),
        };
        if !valid {
            panic!("S3 reply contract violated at seed={seed}: {event:?} -> {reply:?}");
        }
    }

    fn active_request_state(
        session: SessionId,
        composition_epoch: ime_protocol::Epoch,
    ) -> RuntimeState {
        let connected = ime_runtime_core::reduce(
            RuntimeState::new(ClientInstanceId(5)),
            Event::BrokerReady {
                generation: ime_protocol::BrokerGeneration(2),
            },
        )
        .next_state;
        let focused =
            ime_runtime_core::reduce(connected, Event::FocusGained { session }).next_state;
        let tested = ime_runtime_core::reduce(
            focused,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: KeyObservation {
                    token: 1,
                    virtual_key: 65,
                    scan_code: 30,
                    modifiers: 0,
                    repeat: false,
                    now_ms: 1,
                },
            },
        );
        let mut state = ime_runtime_core::reduce(
            tested.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: KeyObservation {
                    token: 1,
                    virtual_key: 65,
                    scan_code: 30,
                    modifiers: 0,
                    repeat: false,
                    now_ms: 2,
                },
            },
        )
        .next_state;
        state.composition = CompositionState::Active {
            epoch: composition_epoch,
        };
        state.composition_epoch = composition_epoch;
        state
    }

    fn commit_event(
        session: SessionId,
        commit_id: CommitId,
        epoch: ime_protocol::Epoch,
        token_id: u64,
    ) -> Event {
        Event::CommitIntent {
            identity: ime_protocol::MessageIdentity {
                client_instance_id: ClientInstanceId(5),
                broker_generation: ime_protocol::BrokerGeneration(2),
                session_id: session,
                focus_epoch: ime_protocol::Epoch(1),
                composition_epoch: epoch,
                request_seq: ime_protocol::RequestSeq(1),
            },
            commit_id,
            token_id,
        }
    }

    #[test]
    fn replay_produces_byte_identical_trace_and_state_hash() {
        let initial = RuntimeState::new(ClientInstanceId(5));
        let events = [
            Event::FocusGained {
                session: SessionId(8),
            },
            Event::HostKey {
                phase: KeyPhase::Test,
                key: KeyObservation {
                    token: 1001,
                    virtual_key: 65,
                    scan_code: 30,
                    modifiers: 0,
                    repeat: false,
                    now_ms: 7,
                },
            },
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: KeyObservation {
                    token: 1001,
                    virtual_key: 65,
                    scan_code: 30,
                    modifiers: 0,
                    repeat: false,
                    now_ms: 8,
                },
            },
        ];
        let first = replay(initial, events);
        let decoded =
            decode_trace_events(initial, &first.canonical_trace).expect("canonical trace decodes");
        let round_trip = replay(initial, decoded);
        assert_eq!(round_trip, first);
        for _ in 0..100 {
            let next = replay(initial, events);
            assert_eq!(next.canonical_trace, first.canonical_trace);
            assert_eq!(next.state.structural_hash(), first.state.structural_hash());
        }
    }

    #[test]
    fn failure_trace_shrinker_keeps_a_replayable_minimal_counterexample() {
        let events = vec![
            Event::FocusGained {
                session: SessionId(8),
            },
            Event::BrokerDisconnected,
            Event::HostClosing,
            Event::InputMethodDeactivated,
        ];
        let minimized = shrink_event_trace(events, |trace| {
            trace
                .iter()
                .any(|event| matches!(event, Event::HostClosing))
        });
        assert_eq!(minimized, vec![Event::HostClosing]);
        let first = replay(
            RuntimeState::new(ClientInstanceId(5)),
            minimized.iter().copied(),
        );
        let second = replay(
            RuntimeState::new(ClientInstanceId(5)),
            minimized.iter().copied(),
        );
        assert_eq!(first.canonical_trace, second.canonical_trace);
    }

    #[test]
    fn minimized_failure_trace_artifact_is_written_as_replayable_bytes() {
        let initial = RuntimeState::new(ClientInstanceId(5));
        let events = [Event::HostClosing, Event::InputMethodDeactivated];
        let trace = replay(initial, events).canonical_trace;
        let directory =
            std::env::temp_dir().join(format!("wufan-trace-artifact-test-{}", std::process::id()));
        let path = persist_failure_trace(&directory, 5, &trace).unwrap();
        let persisted = std::fs::read(&path).unwrap();
        assert_eq!(persisted, trace);
        assert_eq!(decode_trace_events(initial, &persisted).unwrap(), events);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn trace_decoder_rejects_wrong_version_initial_state_and_truncation() {
        let initial = RuntimeState::new(ClientInstanceId(12));
        let trace = replay(initial, [Event::HostClosing]).canonical_trace;
        let mut wrong_version = trace.clone();
        wrong_version[0] = b'x';
        assert_eq!(
            decode_trace_events(initial, &wrong_version),
            Err(TraceDecodeError::InvalidHeader)
        );
        assert_eq!(
            decode_trace_events(RuntimeState::new(ClientInstanceId(13)), &trace),
            Err(TraceDecodeError::InitialStateMismatch)
        );
        assert_eq!(
            decode_trace_events(initial, &trace[..trace.len() - 1]),
            Err(TraceDecodeError::Truncated)
        );
    }

    #[test]
    fn fake_adapter_observes_at_most_one_host_mutation() {
        let session = SessionId(8);
        let commit_id = CommitId(77);
        let state = active_request_state(session, ime_protocol::Epoch(4));
        let intent = commit_event(session, commit_id, ime_protocol::Epoch(4), 9876);
        let transition = ime_runtime_core::reduce(state, intent);
        let apply = transition.effects[0].expect("new commit creates host effect");
        let mut adapter = FakeEffectAdapter::default();
        let result = adapter
            .execute(apply)
            .expect("host commit returns terminal result");
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(5), session, commit_id),
            1
        );

        let completed = ime_runtime_core::reduce(transition.next_state, result);
        assert!(matches!(
            completed.effects[0],
            Some(Effect::SendCommitApplied { .. })
        ));
        let duplicate_intent = ime_runtime_core::reduce(completed.next_state, intent);
        assert!(matches!(
            duplicate_intent.effects[0],
            Some(Effect::SendCommitApplied { .. })
        ));
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(5), session, commit_id),
            1
        );
        let duplicate_terminal = ime_runtime_core::reduce(completed.next_state, result);
        assert_eq!(
            duplicate_terminal
                .next_state
                .commit_entries()
                .collect::<Vec<_>>(),
            completed.next_state.commit_entries().collect::<Vec<_>>()
        );
        assert!(matches!(
            duplicate_terminal.effects[0],
            Some(Effect::RecordDiagnostic { .. })
        ));

        // Duplicate delivery of the same EffectId is suppressed by the adapter.
        assert_eq!(adapter.execute(apply), None);
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(5), session, commit_id),
            1
        );
    }

    #[test]
    fn commit_oracle_keys_host_writes_by_client_incarnation() {
        let mut adapter = FakeEffectAdapter::default();
        for client_instance_id in [ClientInstanceId(5), ClientInstanceId(99)] {
            let effect = Effect::ApplyHostCommit {
                effect_id: EffectId(1),
                client_instance_id,
                session: SessionId(8),
                epoch: ime_protocol::Epoch(3),
                commit_id: CommitId(77),
                token_id: 123,
            };
            assert!(adapter.execute(effect).is_some());
        }
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(5), SessionId(8), CommitId(77)),
            1
        );
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(99), SessionId(8), CommitId(77)),
            1
        );
        assert_eq!(adapter.host_mutations.len(), 2);
    }

    #[test]
    fn cross_model_i2_i4_i5_i6_oracle_checks_commit_deployment_and_memory_facts() {
        use ime_runtime_core::{EffectOutcome, RuntimeMode};

        let session = SessionId(8);
        for (commit_id, behavior, expected_writes) in [
            (CommitId(201), FakeCommitBehavior::Success, 1),
            (CommitId(202), FakeCommitBehavior::Rejected, 0),
            (
                CommitId(203),
                FakeCommitBehavior::IndeterminateBeforeWrite,
                0,
            ),
            (
                CommitId(204),
                FakeCommitBehavior::IndeterminateAfterWrite,
                1,
            ),
        ] {
            let intent = commit_event(session, commit_id, ime_protocol::Epoch(4), commit_id.0);
            let transition = reduce(
                active_request_state(session, ime_protocol::Epoch(4)),
                intent,
            );
            let mut adapter = FakeEffectAdapter {
                commit_behavior: behavior,
                ..FakeEffectAdapter::default()
            };
            let result = adapter.execute(transition.effects[0].unwrap()).unwrap();
            assert_eq!(
                adapter.mutation_count(ClientInstanceId(5), session, commit_id),
                expected_writes
            );
            let terminal = reduce(transition.next_state, result);
            assert!(terminal.next_state.model_invariants_hold());
            match behavior {
                FakeCommitBehavior::Success => assert!(matches!(
                    terminal.effects[0],
                    Some(Effect::SendCommitApplied { .. })
                )),
                FakeCommitBehavior::Rejected => assert!(matches!(
                    terminal.effects[0],
                    Some(Effect::SendCommitRejected { .. })
                )),
                FakeCommitBehavior::IndeterminateBeforeWrite
                | FakeCommitBehavior::IndeterminateAfterWrite => {
                    assert_eq!(terminal.next_state.mode, RuntimeMode::Passthrough);
                    assert_eq!(terminal.effects, [None, None]);
                }
            }
            if matches!(
                behavior,
                FakeCommitBehavior::IndeterminateBeforeWrite
                    | FakeCommitBehavior::IndeterminateAfterWrite
            ) {
                let stale = reduce(terminal.next_state, intent);
                assert!(!stale
                    .effects
                    .iter()
                    .flatten()
                    .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));
                assert_eq!(
                    adapter.mutation_count(ClientInstanceId(5), session, commit_id),
                    expected_writes
                );
            }
            if behavior == FakeCommitBehavior::Success {
                assert!(matches!(
                    result,
                    Event::EffectResult {
                        outcome: EffectOutcome::Succeeded,
                        ..
                    }
                ));
            }
        }

        let mut deployment = crate::deployment_model::DeploymentMachine::stable(1);
        deployment.prepare(2);
        deployment.sessions_drained();
        deployment.runtime_activated(2);
        deployment.commit_persisted(2);
        deployment.recover_after_crash(true);
        assert!(deployment.sessions_may_start());
        assert_eq!(
            deployment.snapshot.active_pointer,
            deployment.snapshot.journal_committed
        );

        let mut replica_a = crate::memory_model::MemoryReplica::new(1);
        let dot = replica_a.record(crate::memory_model::LearnDelta {
            item_id: 7,
            amount: 1,
        });
        let snapshot = replica_a.snapshot();
        let mut replica_b = crate::memory_model::MemoryReplica::new(2);
        assert_eq!(replica_b.merge(&snapshot), 1);
        assert_eq!(replica_b.merge(&snapshot), 0);
        assert_eq!(replica_b.application_count(dot), 1);
    }

    #[test]
    fn fake_scheduler_drives_key_completions_and_commit_terminal_effects() {
        let initial = reduce(
            RuntimeState::new(ClientInstanceId(5)),
            Event::BrokerReady {
                generation: ime_protocol::BrokerGeneration(2),
            },
        )
        .next_state;
        let initial = reduce(
            initial,
            Event::FocusGained {
                session: SessionId(8),
            },
        )
        .next_state;
        let mut key_events = Vec::new();
        for index in 0..30u64 {
            let key = KeyObservation {
                token: index + 1,
                virtual_key: (65 + index) as u16,
                scan_code: (30 + index) as u16,
                modifiers: 0,
                repeat: false,
                now_ms: index * 30,
            };
            key_events.push(Event::HostKey {
                phase: KeyPhase::Test,
                key,
            });
            key_events.push(Event::HostKey {
                phase: KeyPhase::Actual,
                key,
            });
        }
        let mut adapter = FakeEffectAdapter::default();
        let scheduled = run_with_fake_adapter(initial, key_events, &mut adapter);
        let sequences: Vec<_> = scheduled
            .transitions
            .iter()
            .flat_map(|transition| transition.effects.iter().flatten())
            .filter_map(|effect| {
                if let Effect::SendKey { identity, .. } = effect {
                    Some(identity.request_seq)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            sequences,
            (1..=30).map(ime_protocol::RequestSeq).collect::<Vec<_>>()
        );
        let decoded = decode_trace_events(initial, &scheduled.canonical_trace).unwrap();
        assert_eq!(replay(initial, decoded), scheduled);

        let session = SessionId(8);
        let commit_id = CommitId(305);
        // Replaying from the pre-commit state exercises ApplyHostCommit and its terminal result.
        let mut adapter = FakeEffectAdapter::default();
        let commit_scheduled = run_with_fake_adapter(
            active_request_state(session, ime_protocol::Epoch(9)),
            [commit_event(session, commit_id, ime_protocol::Epoch(9), 88)],
            &mut adapter,
        );
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(5), session, commit_id),
            1
        );
        assert!(commit_scheduled
            .transitions
            .iter()
            .any(|transition| transition
                .effects
                .iter()
                .flatten()
                .any(|effect| matches!(effect, Effect::SendCommitApplied { .. }))));
        let decoded = decode_trace_events(
            active_request_state(session, ime_protocol::Epoch(9)),
            &commit_scheduled.canonical_trace,
        )
        .unwrap();
        assert_eq!(
            replay(
                active_request_state(session, ime_protocol::Epoch(9)),
                decoded
            ),
            commit_scheduled
        );
    }

    #[test]
    fn composition_adapter_injects_success_reject_and_indeterminate_results() {
        use ime_runtime_core::{EffectOutcome, RuntimeMode};

        for (behavior, expected) in [
            (FakeCompositionBehavior::Success, EffectOutcome::Succeeded),
            (FakeCompositionBehavior::Rejected, EffectOutcome::Rejected),
            (
                FakeCompositionBehavior::Indeterminate,
                EffectOutcome::Indeterminate,
            ),
        ] {
            let connected = reduce(
                RuntimeState::new(ime_protocol::ClientInstanceId(5)),
                Event::BrokerReady {
                    generation: ime_protocol::BrokerGeneration(2),
                },
            )
            .next_state;
            let focused = reduce(
                connected,
                Event::FocusGained {
                    session: SessionId(8),
                },
            )
            .next_state;
            let observation = KeyObservation {
                token: 1,
                virtual_key: 65,
                scan_code: 30,
                modifiers: 0,
                repeat: false,
                now_ms: 1,
            };
            let tested = reduce(
                focused,
                Event::HostKey {
                    phase: KeyPhase::Test,
                    key: observation,
                },
            );
            let actual = reduce(
                tested.next_state,
                Event::HostKey {
                    phase: KeyPhase::Actual,
                    key: observation,
                },
            );
            let Some(Effect::SendKey { mut identity, .. }) = actual.effects[0] else {
                panic!("key dispatched");
            };
            identity.composition_epoch =
                ime_protocol::Epoch(actual.next_state.composition_epoch.0 + 1);
            let started = reduce(
                actual.next_state,
                Event::EngineUpdate {
                    identity,
                    preedit_token_id: Some(7),
                },
            );
            let Some(begin) = started.effects[0] else {
                panic!("composition effect emitted");
            };
            let mut adapter = FakeEffectAdapter {
                composition_behavior: behavior,
                ..FakeEffectAdapter::default()
            };
            let Some(Event::EffectResult { outcome, .. }) = adapter.execute(begin) else {
                panic!("adapter returns terminal result");
            };
            assert_eq!(outcome, expected);
            let response = adapter
                .execute(begin)
                .expect("same effect can be replayed deterministically");
            let finished = reduce(started.next_state, response);
            match expected {
                EffectOutcome::Succeeded => assert!(matches!(
                    finished.next_state.composition,
                    CompositionState::Active { .. }
                )),
                EffectOutcome::Rejected | EffectOutcome::Indeterminate => {
                    assert_eq!(finished.next_state.mode, RuntimeMode::Passthrough)
                }
            }
        }
    }

    #[test]
    fn fake_scheduler_covers_every_terminal_effect_result() {
        use ime_protocol::{Epoch, MessageIdentity, RequestSeq};
        use ime_runtime_core::{CommitStatus, EffectOutcome, EffectResultClass, RuntimeMode};

        let session = SessionId(8);
        for (result_class, preedit_token_id) in [
            (EffectResultClass::CompositionStart, Some(7)),
            (EffectResultClass::CompositionUpdate, Some(8)),
            (EffectResultClass::CompositionTermination, None),
        ] {
            for (behavior, outcome) in [
                (FakeCompositionBehavior::Success, EffectOutcome::Succeeded),
                (FakeCompositionBehavior::Rejected, EffectOutcome::Rejected),
                (
                    FakeCompositionBehavior::Indeterminate,
                    EffectOutcome::Indeterminate,
                ),
            ] {
                let mut initial = active_request_state(session, Epoch(9));
                let epoch = if result_class == EffectResultClass::CompositionStart {
                    initial.composition = CompositionState::Idle;
                    initial.composition_epoch = Epoch(0);
                    Epoch(1)
                } else {
                    Epoch(9)
                };
                let identity = MessageIdentity {
                    client_instance_id: ClientInstanceId(5),
                    broker_generation: ime_protocol::BrokerGeneration(2),
                    session_id: session,
                    focus_epoch: Epoch(1),
                    composition_epoch: epoch,
                    request_seq: RequestSeq(1),
                };
                let mut adapter = FakeEffectAdapter {
                    composition_behavior: behavior,
                    ..FakeEffectAdapter::default()
                };
                let run = run_with_fake_adapter(
                    initial,
                    [Event::EngineUpdate {
                        identity,
                        preedit_token_id,
                    }],
                    &mut adapter,
                );
                let events = decode_trace_events(initial, &run.canonical_trace).unwrap();
                assert!(
                    events.iter().any(|event| matches!(
                        event,
                        Event::EffectResult {
                            result_class: actual_class,
                            outcome: actual_outcome,
                            ..
                        } if *actual_class == result_class && *actual_outcome == outcome
                    )),
                    "missing {result_class:?}/{outcome:?} terminal result"
                );
                assert_eq!(replay(initial, events), run);
                if outcome == EffectOutcome::Succeeded {
                    assert_eq!(run.state.mode, RuntimeMode::Normal);
                    assert_eq!(
                        run.state.composition,
                        if result_class == EffectResultClass::CompositionTermination {
                            CompositionState::Idle
                        } else {
                            CompositionState::Active { epoch }
                        }
                    );
                } else {
                    assert_eq!(run.state.mode, RuntimeMode::Passthrough);
                    assert_eq!(run.state.composition, CompositionState::Idle);
                }
                assert!(adapter.host_mutations.is_empty());
            }
        }

        for (behavior, outcome, expected_writes) in [
            (FakeCommitBehavior::Success, EffectOutcome::Succeeded, 1),
            (FakeCommitBehavior::Rejected, EffectOutcome::Rejected, 0),
            (
                FakeCommitBehavior::IndeterminateBeforeWrite,
                EffectOutcome::Indeterminate,
                0,
            ),
            (
                FakeCommitBehavior::IndeterminateAfterWrite,
                EffectOutcome::Indeterminate,
                1,
            ),
        ] {
            let initial = active_request_state(session, Epoch(9));
            let commit_id = CommitId(305);
            let mut adapter = FakeEffectAdapter {
                commit_behavior: behavior,
                ..FakeEffectAdapter::default()
            };
            let run = run_with_fake_adapter(
                initial,
                [commit_event(session, commit_id, Epoch(9), 88)],
                &mut adapter,
            );
            let events = decode_trace_events(initial, &run.canonical_trace).unwrap();
            assert!(
                events.iter().any(|event| matches!(
                    event,
                    Event::EffectResult {
                        result_class: EffectResultClass::HostCommit,
                        outcome: actual_outcome,
                        ..
                    } if *actual_outcome == outcome
                )),
                "missing commit {outcome:?} terminal result"
            );
            assert_eq!(replay(initial, events), run);
            assert_eq!(
                adapter.mutation_count(ClientInstanceId(5), session, commit_id),
                expected_writes
            );
            let status = run
                .state
                .commit_entries()
                .find(|entry| entry.commit_id == commit_id)
                .expect("commit must have a terminal cache entry")
                .status;
            assert!(match outcome {
                EffectOutcome::Succeeded => matches!(status, CommitStatus::Applied { .. }),
                EffectOutcome::Rejected => status == CommitStatus::Rejected,
                EffectOutcome::Indeterminate => status == CommitStatus::Indeterminate,
            });
            assert_eq!(
                run.state.mode,
                if outcome == EffectOutcome::Indeterminate {
                    RuntimeMode::Passthrough
                } else {
                    RuntimeMode::Normal
                }
            );
        }
    }

    #[test]
    fn m01_thirty_keys_per_second_remain_ordered_and_unique() {
        let connected = reduce(
            RuntimeState::new(ime_protocol::ClientInstanceId(5)),
            Event::BrokerReady {
                generation: ime_protocol::BrokerGeneration(2),
            },
        )
        .next_state;
        let mut state = reduce(
            connected,
            Event::FocusGained {
                session: SessionId(8),
            },
        )
        .next_state;
        let mut adapter = FakeEffectAdapter::default();
        let mut observed_sequences = Vec::new();
        // Thirty unique physical keys over a one-second synthetic interval.
        for index in 0..30u64 {
            let observation = KeyObservation {
                token: index + 1,
                virtual_key: (65 + index) as u16,
                scan_code: (30 + index) as u16,
                modifiers: 0,
                repeat: false,
                now_ms: index * 1_000 / 30,
            };
            let tested = reduce(
                state,
                Event::HostKey {
                    phase: KeyPhase::Test,
                    key: observation,
                },
            );
            assert_eq!(
                tested.immediate,
                Some(ime_runtime_core::ImmediateReply::TestKey(
                    ime_runtime_core::EatDecision::Eat
                ))
            );
            let actual = reduce(
                tested.next_state,
                Event::HostKey {
                    phase: KeyPhase::Actual,
                    key: observation,
                },
            );
            let Some(Effect::SendKey { identity, .. }) = actual.effects[0] else {
                panic!("each key dispatched once");
            };
            observed_sequences.push(identity.request_seq);
            let completion = adapter.execute(actual.effects[0].unwrap()).unwrap();
            state = reduce(actual.next_state, completion).next_state;
        }
        assert_eq!(
            observed_sequences,
            (1..=30).map(ime_protocol::RequestSeq).collect::<Vec<_>>()
        );
        assert_eq!(observed_sequences.iter().collect::<BTreeSet<_>>().len(), 30);
    }

    #[test]
    fn m08_out_of_order_completion_cannot_advance_the_mutation_queue() {
        let connected = reduce(
            RuntimeState::new(ime_protocol::ClientInstanceId(5)),
            Event::BrokerReady {
                generation: ime_protocol::BrokerGeneration(2),
            },
        )
        .next_state;
        let focused = reduce(
            connected,
            Event::FocusGained {
                session: SessionId(8),
            },
        )
        .next_state;
        let make_key = |token, vk| KeyObservation {
            token,
            virtual_key: vk,
            scan_code: vk,
            modifiers: 0,
            repeat: false,
            now_ms: token,
        };
        let first_test = reduce(
            focused,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: make_key(1, 65),
            },
        );
        let first = reduce(
            first_test.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: make_key(1, 65),
            },
        );
        let Some(Effect::SendKey {
            identity: first_identity,
            ..
        }) = first.effects[0]
        else {
            panic!("first key dispatched");
        };
        let second_test = reduce(
            first.next_state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: make_key(2, 66),
            },
        );
        let second = reduce(
            second_test.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: make_key(2, 66),
            },
        );
        assert!(second.effects.iter().all(Option::is_none));
        let forged_second = ime_protocol::MessageIdentity {
            request_seq: ime_protocol::RequestSeq(2),
            ..first_identity
        };
        let out_of_order = reduce(
            second.next_state,
            Event::RequestCompleted {
                identity: forged_second,
            },
        );
        assert_eq!(
            out_of_order.next_state.last_request_seq,
            second.next_state.last_request_seq
        );
        assert!(matches!(
            out_of_order.effects[0],
            Some(Effect::RecordDiagnostic { .. })
        ));
        let correct = reduce(
            second.next_state,
            Event::RequestCompleted {
                identity: first_identity,
            },
        );
        assert!(
            matches!(correct.effects[0], Some(Effect::SendKey { identity, .. }) if identity.request_seq == ime_protocol::RequestSeq(2))
        );
    }

    #[test]
    fn m03_old_broker_generation_response_is_dropped_after_restart() {
        let connected = reduce(
            RuntimeState::new(ime_protocol::ClientInstanceId(5)),
            Event::BrokerReady {
                generation: ime_protocol::BrokerGeneration(2),
            },
        )
        .next_state;
        let focused = reduce(
            connected,
            Event::FocusGained {
                session: SessionId(8),
            },
        )
        .next_state;
        let observation = KeyObservation {
            token: 1,
            virtual_key: 65,
            scan_code: 30,
            modifiers: 0,
            repeat: false,
            now_ms: 1,
        };
        let tested = reduce(
            focused,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: observation,
            },
        );
        let actual = reduce(
            tested.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: observation,
            },
        );
        let Some(Effect::SendKey {
            identity: old_identity,
            ..
        }) = actual.effects[0]
        else {
            panic!("request dispatched");
        };
        let disconnected = reduce(actual.next_state, Event::BrokerDisconnected);
        let restarted = reduce(
            disconnected.next_state,
            Event::BrokerReady {
                generation: ime_protocol::BrokerGeneration(3),
            },
        );
        let refocused = reduce(
            restarted.next_state,
            Event::FocusGained {
                session: SessionId(9),
            },
        );
        let stale = reduce(
            refocused.next_state,
            Event::EngineUpdate {
                identity: old_identity,
                preedit_token_id: Some(99),
            },
        );
        assert_eq!(stale.next_state, refocused.next_state);
        assert!(!stale
            .effects
            .iter()
            .flatten()
            .any(|effect| matches!(effect, Effect::BeginHostComposition { .. })));
    }

    #[test]
    fn m05_m06_crash_after_host_write_does_not_reapply_old_incarnation_commit() {
        let session = SessionId(8);
        let commit_id = CommitId(55);
        let initial = active_request_state(session, ime_protocol::Epoch(9));
        let identity = match commit_event(session, commit_id, ime_protocol::Epoch(9), 72) {
            Event::CommitIntent { identity, .. } => identity,
            _ => unreachable!(),
        };
        let applying = reduce(
            initial,
            commit_event(session, commit_id, ime_protocol::Epoch(9), 72),
        );
        let Some(effect @ Effect::ApplyHostCommit { .. }) = applying.effects[0] else {
            panic!("single host commit effect");
        };
        let duplicate_while_applying = reduce(
            applying.next_state,
            commit_event(session, commit_id, ime_protocol::Epoch(9), 72),
        );
        assert!(
            matches!(duplicate_while_applying.effects[0], Some(Effect::SendCommitPending { commit_id: id, .. }) if id == commit_id)
        );
        let mut adapter = FakeEffectAdapter::default();
        let terminal_not_delivered = adapter.execute(effect).expect("host write happened");
        assert!(matches!(
            terminal_not_delivered,
            Event::EffectResult {
                outcome: ime_runtime_core::EffectOutcome::Succeeded,
                ..
            }
        ));
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(5), session, commit_id),
            1
        );

        let crashed = reduce(applying.next_state, Event::BrokerDisconnected);
        assert!(!crashed
            .effects
            .iter()
            .flatten()
            .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));
        let restarted = reduce(
            reduce(
                RuntimeState::new(ime_protocol::ClientInstanceId(99)),
                Event::BrokerReady {
                    generation: ime_protocol::BrokerGeneration(3),
                },
            )
            .next_state,
            Event::FocusGained { session },
        );
        let delayed = reduce(
            restarted.next_state,
            Event::CommitIntent {
                identity,
                commit_id,
                token_id: 72,
            },
        );
        assert_eq!(delayed.next_state.mode, restarted.next_state.mode);
        assert_eq!(delayed.next_state.focus, restarted.next_state.focus);
        assert_eq!(
            delayed.next_state.active_session,
            restarted.next_state.active_session
        );
        assert!(matches!(
            delayed.effects[0],
            Some(Effect::RecordDiagnostic { .. })
        ));
        assert!(!delayed
            .effects
            .iter()
            .flatten()
            .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(5), session, commit_id),
            1
        );
    }

    #[test]
    fn required_deadline_values_preserve_m02_m05_m19_failure_invariants() {
        use crate::deployment_model::{DeploymentAction, DeploymentMachine, DeploymentPhase};
        let deadlines_ms = [10u64, 100, 250, 2_000];
        let session = SessionId(8);
        let commit_id = CommitId(66);
        let base = active_request_state(session, ime_protocol::Epoch(9));
        let commit = commit_event(session, commit_id, ime_protocol::Epoch(9), 73);
        let applying = reduce(base, commit);
        let Some(Effect::ApplyHostCommit { effect_id, .. }) = applying.effects[0] else {
            panic!("commit is applying");
        };
        let mut expected_passthrough = None;
        for _deadline_ms in deadlines_ms {
            let expired = reduce(
                applying.next_state,
                Event::DeadlineExpired {
                    request_seq: ime_protocol::RequestSeq(1),
                },
            );
            assert_eq!(
                expired.next_state.mode,
                ime_runtime_core::RuntimeMode::Passthrough
            );
            assert!(expired.next_state.model_invariants_hold());
            assert!(!expired
                .effects
                .iter()
                .flatten()
                .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));
            let late_terminal = reduce(
                expired.next_state,
                Event::EffectResult {
                    effect_id,
                    result_class: ime_runtime_core::EffectResultClass::HostCommit,
                    scope: ime_runtime_core::EffectScope::Commit {
                        client_instance_id: ClientInstanceId(5),
                        session,
                        epoch: ime_protocol::Epoch(9),
                        commit_id,
                    },
                    outcome: ime_runtime_core::EffectOutcome::Succeeded,
                    host_revision: Some(ime_protocol::HostRevision(500)),
                },
            );
            assert_eq!(
                late_terminal
                    .next_state
                    .commit_entries()
                    .collect::<Vec<_>>(),
                expired.next_state.commit_entries().collect::<Vec<_>>()
            );
            assert!(matches!(
                late_terminal.effects[0],
                Some(Effect::RecordDiagnostic { .. })
            ));
            if let Some(expected) = expected_passthrough {
                assert_eq!(expired.next_state, expected);
            } else {
                expected_passthrough = Some(expired.next_state);
            }

            let repeated = reduce(expired.next_state, commit);
            assert!(!repeated
                .effects
                .iter()
                .flatten()
                .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));

            let mut deployment = DeploymentMachine::stable(100);
            assert_eq!(deployment.prepare(101), DeploymentAction::CancelSessions);
            deployment.sessions_drained();
            deployment.runtime_activated(101);
            assert_eq!(deployment.phase, DeploymentPhase::PersistingCommit);
            deployment.recover_after_crash(true);
            assert_eq!(deployment.phase, DeploymentPhase::Stable);
            assert_eq!(deployment.snapshot.journal_committed, 100);
            assert_eq!(deployment.snapshot.active_pointer, 100);
        }
    }

    #[test]
    fn indeterminate_commit_after_write_is_never_retried() {
        let session = SessionId(8);
        let commit_id = CommitId(78);
        let state = active_request_state(session, ime_protocol::Epoch(4));
        let intent = commit_event(session, commit_id, ime_protocol::Epoch(4), 9877);
        let transition = ime_runtime_core::reduce(state, intent);
        let mut adapter = FakeEffectAdapter {
            commit_behavior: FakeCommitBehavior::IndeterminateAfterWrite,
            ..Default::default()
        };
        let result = adapter.execute(transition.effects[0].unwrap()).unwrap();
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(5), session, commit_id),
            1
        );

        let uncertain = ime_runtime_core::reduce(transition.next_state, result);
        assert_eq!(
            uncertain.next_state.mode,
            ime_runtime_core::RuntimeMode::Passthrough
        );
        let retried = ime_runtime_core::reduce(uncertain.next_state, intent);
        assert!(!retried
            .effects
            .iter()
            .flatten()
            .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));
        assert_eq!(
            adapter.mutation_count(ClientInstanceId(5), session, commit_id),
            1
        );
    }
}
