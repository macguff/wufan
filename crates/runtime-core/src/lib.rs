#![forbid(unsafe_code)]

//! Deterministic, platform-independent runtime state machine.

use ime_protocol::{
    BrokerGeneration, ClientInstanceId, CommitId, EffectId, Epoch, HostRevision, MessageIdentity,
    RequestSeq, SessionId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EatDecision {
    Eat,
    Pass,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImmediateReply {
    TestKey(EatDecision),
    Key(EatDecision),
    TestKeyUp(EatDecision),
    KeyUp(EatDecision),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyPhase {
    Test,
    Actual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyObservation {
    pub token: u64,
    pub virtual_key: u16,
    pub scan_code: u16,
    pub modifiers: u16,
    pub repeat: bool,
    pub now_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    HostKey {
        phase: KeyPhase,
        key: KeyObservation,
    },
    HostKeyUp {
        phase: KeyPhase,
        key: KeyObservation,
    },
    PhysicalKey {
        virtual_key: u16,
        down: bool,
    },
    FocusGained {
        session: SessionId,
    },
    FocusLost {
        session: SessionId,
    },
    ContextPushed {
        session: SessionId,
    },
    ContextPopped {
        session: SessionId,
    },
    DecisionTtlExpired {
        token: u64,
        now_ms: u64,
    },
    BrokerDisconnected,
    HostClosing,
    InputMethodDeactivated,
    BrokerReady {
        generation: BrokerGeneration,
    },
    RequestCompleted {
        identity: MessageIdentity,
    },
    DeadlineExpired {
        request_seq: RequestSeq,
    },
    PumpMutatingQueue,
    EngineUpdate {
        identity: MessageIdentity,
        preedit_token_id: Option<u64>,
    },
    CandidateViewUpdated {
        identity: MessageIdentity,
        revision: u64,
        candidate_count: u16,
    },
    UIActionIntent {
        identity: MessageIdentity,
        revision: u64,
        candidate_index: u16,
    },
    EffectResult {
        effect_id: EffectId,
        result_class: EffectResultClass,
        scope: EffectScope,
        outcome: EffectOutcome,
        host_revision: Option<HostRevision>,
    },
    CommitIntent {
        identity: MessageIdentity,
        commit_id: CommitId,
        token_id: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectResultClass {
    CompositionTermination,
    CompositionStart,
    CompositionUpdate,
    HostCommit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectScope {
    Composition {
        session: SessionId,
        epoch: Epoch,
    },
    Commit {
        client_instance_id: ClientInstanceId,
        session: SessionId,
        epoch: Epoch,
        commit_id: CommitId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectOutcome {
    Succeeded,
    Rejected,
    Indeterminate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    RecordDiagnostic {
        effect_id: EffectId,
        code: DiagnosticCode,
    },
    SendKey {
        effect_id: EffectId,
        identity: MessageIdentity,
        token: u64,
        key_up: bool,
    },
    SelectCandidate {
        effect_id: EffectId,
        identity: MessageIdentity,
        candidate_index: u16,
    },
    CancelComposition {
        effect_id: EffectId,
        session: SessionId,
        epoch: Epoch,
        reason: Option<DiagnosticCode>,
    },
    BeginHostComposition {
        effect_id: EffectId,
        session: SessionId,
        epoch: Epoch,
        preedit_token_id: u64,
    },
    UpdateHostComposition {
        effect_id: EffectId,
        session: SessionId,
        epoch: Epoch,
        preedit_token_id: u64,
    },
    TerminateHostComposition {
        effect_id: EffectId,
        session: SessionId,
        epoch: Epoch,
    },
    SendRequestAck {
        effect_id: EffectId,
        identity: MessageIdentity,
    },
    ApplyHostCommit {
        effect_id: EffectId,
        client_instance_id: ClientInstanceId,
        session: SessionId,
        epoch: Epoch,
        commit_id: CommitId,
        token_id: u64,
    },
    SendCommitPending {
        effect_id: EffectId,
        commit_id: CommitId,
    },
    SendCommitApplied {
        effect_id: EffectId,
        commit_id: CommitId,
        host_revision: HostRevision,
    },
    SendCommitRejected {
        effect_id: EffectId,
        commit_id: CommitId,
    },
    EnterPassthrough {
        effect_id: EffectId,
        reason: Option<DiagnosticCode>,
    },
}

impl Effect {
    pub const fn id(self) -> EffectId {
        match self {
            Self::RecordDiagnostic { effect_id, .. }
            | Self::SendKey { effect_id, .. }
            | Self::SelectCandidate { effect_id, .. }
            | Self::CancelComposition { effect_id, .. }
            | Self::BeginHostComposition { effect_id, .. }
            | Self::UpdateHostComposition { effect_id, .. }
            | Self::TerminateHostComposition { effect_id, .. }
            | Self::SendRequestAck { effect_id, .. }
            | Self::ApplyHostCommit { effect_id, .. }
            | Self::SendCommitPending { effect_id, .. }
            | Self::SendCommitApplied { effect_id, .. }
            | Self::SendCommitRejected { effect_id, .. }
            | Self::EnterPassthrough { effect_id, .. } => effect_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticCode {
    KeyDecisionLedgerFull,
    MutatingQueueFull,
    KeyDecisionUnmatched,
    InvalidTransition,
    KeyDecisionExpired,
    StaleEffectResult,
    StaleUiAction,
    StaleCandidateView,
    StaleRequestCompletion,
    StaleCommitIntent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransitionResult {
    pub next_state: RuntimeState,
    pub immediate: Option<ImmediateReply>,
    pub effects: [Option<Effect>; 2],
}

#[cfg(test)]
thread_local! {
    static TRANSITION_RETURN_SITES: std::cell::RefCell<Option<std::collections::BTreeSet<u32>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
#[track_caller]
fn record_transition_return_site() {
    let line = std::panic::Location::caller().line();
    TRANSITION_RETURN_SITES.with(|sites| {
        if let Some(sites) = sites.borrow_mut().as_mut() {
            sites.insert(line);
        }
    });
}

impl TransitionResult {
    #[cfg_attr(test, track_caller)]
    fn unchanged(state: RuntimeState, immediate: Option<ImmediateReply>) -> Self {
        #[cfg(test)]
        record_transition_return_site();
        Self {
            next_state: state,
            immediate,
            effects: [None, None],
        }
    }

    #[cfg_attr(test, track_caller)]
    fn one(state: RuntimeState, immediate: Option<ImmediateReply>, effect: Effect) -> Self {
        #[cfg(test)]
        record_transition_return_site();
        Self {
            next_state: state,
            immediate,
            effects: [Some(effect), None],
        }
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = self.next_state.canonical_bytes();
        match self.immediate {
            None => out.push(0),
            Some(ImmediateReply::TestKey(decision)) => {
                out.push(1);
                out.push(decision as u8);
            }
            Some(ImmediateReply::Key(decision)) => {
                out.push(2);
                out.push(decision as u8);
            }
            Some(ImmediateReply::TestKeyUp(decision)) => {
                out.push(3);
                out.push(decision as u8);
            }
            Some(ImmediateReply::KeyUp(decision)) => {
                out.push(4);
                out.push(decision as u8);
            }
        }
        for effect in self.effects {
            match effect {
                None => out.push(0),
                Some(effect) => {
                    out.push(1);
                    encode_effect(&mut out, effect);
                }
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeMode {
    Normal,
    Passthrough,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusState {
    Unfocused,
    Focused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionState {
    Idle,
    Starting { epoch: Epoch, effect_id: EffectId },
    Active { epoch: Epoch },
    Updating { epoch: Epoch, effect_id: EffectId },
    Terminating { epoch: Epoch, effect_id: EffectId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitStatus {
    Applying { effect_id: EffectId },
    Applied { host_revision: HostRevision },
    Rejected,
    Indeterminate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitEntry {
    pub identity: MessageIdentity,
    pub commit_id: CommitId,
    pub status: CommitStatus,
}

const COMMIT_CACHE_CAPACITY: usize = 32;
const MUTATING_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QueuedKey {
    identity: MessageIdentity,
    action: MutatingAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MutatingAction {
    Key { token: u64, key_up: bool },
    SelectCandidate { index: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeyDecision {
    key: KeyObservation,
    key_up: bool,
    decision: EatDecision,
    expires_at_ms: u64,
}

const LEDGER_CAPACITY: usize = 64;
const OUTSTANDING_CAPACITY: usize = 64;
const DECISION_TTL_MS: u64 = 250;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EffectExpectation {
    effect_id: EffectId,
    result_class: EffectResultClass,
    scope: EffectScope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OutstandingEffects {
    entries: [Option<EffectExpectation>; OUTSTANDING_CAPACITY],
}

impl OutstandingEffects {
    const fn new() -> Self {
        Self {
            entries: [None; OUTSTANDING_CAPACITY],
        }
    }

    fn register(&mut self, expectation: EffectExpectation) -> bool {
        if let Some(slot) = self.entries.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(expectation);
            true
        } else {
            false
        }
    }

    fn matching_index(
        &self,
        effect_id: EffectId,
        result_class: EffectResultClass,
        scope: EffectScope,
    ) -> Option<usize> {
        self.entries.iter().position(|entry| {
            entry.is_some_and(|expected| {
                expected.effect_id == effect_id
                    && expected.result_class == result_class
                    && expected.scope == scope
            })
        })
    }

    fn remove_effect(&mut self, effect_id: EffectId) -> bool {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.is_some_and(|expected| expected.effect_id == effect_id))
        {
            self.entries[index] = None;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeyDecisionLedger {
    entries: [Option<KeyDecision>; LEDGER_CAPACITY],
}

impl KeyDecisionLedger {
    const fn new() -> Self {
        Self {
            entries: [None; LEDGER_CAPACITY],
        }
    }

    fn expire(&mut self, now_ms: u64) {
        for entry in &mut self.entries {
            if entry.is_some_and(|decision| decision.expires_at_ms <= now_ms) {
                *entry = None;
            }
        }
    }

    fn expire_token(&mut self, token: u64, now_ms: u64) -> bool {
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.is_some_and(|decision| {
                decision.key.token == token && decision.expires_at_ms <= now_ms
            })
        }) {
            self.entries[index] = None;
            true
        } else {
            false
        }
    }

    fn len(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_some()).count()
    }

    fn insert(&mut self, key: KeyObservation, key_up: bool, decision: EatDecision) {
        let slot = self
            .entries
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("capacity is checked before inserting a key decision");
        *slot = Some(KeyDecision {
            key,
            key_up,
            decision,
            expires_at_ms: key.now_ms.saturating_add(DECISION_TTL_MS),
        });
    }

    fn consume(&mut self, actual: KeyObservation, key_up: bool) -> Option<EatDecision> {
        let mut matches = self.entries.iter().enumerate().filter_map(|(index, item)| {
            item.filter(|entry| entry.key_up == key_up && same_key(entry.key, actual))
                .map(|entry| (index, entry))
        });
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        self.entries[first.0] = None;
        Some(first.1.decision)
    }
}

fn same_key(left: KeyObservation, right: KeyObservation) -> bool {
    left.virtual_key == right.virtual_key
        && left.scan_code == right.scan_code
        && left.modifiers == right.modifiers
        && left.repeat == right.repeat
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeState {
    pub client_instance_id: ClientInstanceId,
    pub mode: RuntimeMode,
    pub focus: FocusState,
    pub active_session: Option<SessionId>,
    pub composition: CompositionState,
    pub broker_generation: Option<BrokerGeneration>,
    pub focus_epoch: Epoch,
    pub last_request_seq: RequestSeq,
    pub composition_epoch: Epoch,
    next_effect_id: u64,
    pending_request: Option<MessageIdentity>,
    queued_keys: [Option<QueuedKey>; MUTATING_QUEUE_CAPACITY],
    queue_head: usize,
    queue_len: usize,
    key_decisions: KeyDecisionLedger,
    outstanding_effects: OutstandingEffects,
    commits: [Option<CommitEntry>; COMMIT_CACHE_CAPACITY],
    pending_commit: Option<CommitEntry>,
    pending_composition_ack: Option<MessageIdentity>,
    candidate_view_revision: u64,
    candidate_view_request_seq: RequestSeq,
    candidate_count: u16,
    physical_modifier_down: u16,
}

impl RuntimeState {
    pub const fn new(client_instance_id: ClientInstanceId) -> Self {
        Self {
            client_instance_id,
            mode: RuntimeMode::Normal,
            focus: FocusState::Unfocused,
            active_session: None,
            composition: CompositionState::Idle,
            broker_generation: None,
            focus_epoch: Epoch(0),
            last_request_seq: RequestSeq(0),
            composition_epoch: Epoch(0),
            next_effect_id: 1,
            pending_request: None,
            queued_keys: [None; MUTATING_QUEUE_CAPACITY],
            queue_head: 0,
            queue_len: 0,
            key_decisions: KeyDecisionLedger::new(),
            outstanding_effects: OutstandingEffects::new(),
            commits: [None; COMMIT_CACHE_CAPACITY],
            pending_commit: None,
            pending_composition_ack: None,
            candidate_view_revision: 0,
            candidate_view_request_seq: RequestSeq(0),
            candidate_count: 0,
            physical_modifier_down: 0,
        }
    }

    fn allocate_effect(&mut self) -> EffectId {
        let id = EffectId(self.next_effect_id);
        self.next_effect_id = self.next_effect_id.saturating_add(1);
        id
    }

    fn enqueue_key(&mut self, key: QueuedKey) -> bool {
        if self.queue_len == MUTATING_QUEUE_CAPACITY {
            return false;
        }
        let index = (self.queue_head + self.queue_len) % MUTATING_QUEUE_CAPACITY;
        self.queued_keys[index] = Some(key);
        self.queue_len += 1;
        true
    }

    fn dequeue_key(&mut self) -> Option<QueuedKey> {
        if self.queue_len == 0 {
            return None;
        }
        let item = self.queued_keys[self.queue_head].take();
        self.queue_head = (self.queue_head + 1) % MUTATING_QUEUE_CAPACITY;
        self.queue_len -= 1;
        item
    }

    fn dispatch_next_key(&mut self) -> Option<Effect> {
        if self.pending_request.is_some() || self.pending_commit.is_some() {
            return None;
        }
        while let Some(queued) = self.dequeue_key() {
            if self.mode == RuntimeMode::Normal
                && self.focus == FocusState::Focused
                && self.client_instance_id == queued.identity.client_instance_id
                && self.broker_generation == Some(queued.identity.broker_generation)
                && self.active_session == Some(queued.identity.session_id)
                && self.focus_epoch == queued.identity.focus_epoch
            {
                // Queued keys have not crossed the process boundary yet. Bind them
                // to the latest composition epoch when they reach the head.
                let identity = MessageIdentity {
                    composition_epoch: self.composition_epoch,
                    ..queued.identity
                };
                self.pending_request = Some(identity);
                let effect_id = self.allocate_effect();
                return Some(match queued.action {
                    MutatingAction::Key { token, key_up } => Effect::SendKey {
                        effect_id,
                        identity,
                        token,
                        key_up,
                    },
                    MutatingAction::SelectCandidate { index } => Effect::SelectCandidate {
                        effect_id,
                        identity,
                        candidate_index: index,
                    },
                });
            }
        }
        None
    }

    fn clear_queued_mutations(&mut self) {
        self.clear_queue_only();
        self.pending_request = None;
    }

    fn clear_queue_only(&mut self) {
        self.queued_keys = [None; MUTATING_QUEUE_CAPACITY];
        self.queue_head = 0;
        self.queue_len = 0;
    }

    /// Versioned structural encoding used by replay and state hashing.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = b"wufan-runtime-state-v5".to_vec();
        put_u64(&mut out, self.client_instance_id.0);
        out.push(self.mode as u8);
        out.push(self.focus as u8);
        encode_option_u64(&mut out, self.active_session.map(|id| id.0));
        encode_option_u64(
            &mut out,
            self.broker_generation.map(|generation| generation.0),
        );
        put_u64(&mut out, self.focus_epoch.0);
        put_u64(&mut out, self.last_request_seq.0);
        put_u64(&mut out, self.composition_epoch.0);
        put_u64(&mut out, self.candidate_view_revision);
        put_u64(&mut out, self.candidate_view_request_seq.0);
        put_u16(&mut out, self.candidate_count);
        put_u16(&mut out, self.physical_modifier_down);
        match self.composition {
            CompositionState::Idle => out.push(0),
            CompositionState::Starting { epoch, effect_id } => {
                out.push(1);
                put_u64(&mut out, epoch.0);
                put_u64(&mut out, effect_id.0);
            }
            CompositionState::Active { epoch } => {
                out.push(2);
                put_u64(&mut out, epoch.0);
            }
            CompositionState::Updating { epoch, effect_id } => {
                out.push(3);
                put_u64(&mut out, epoch.0);
                put_u64(&mut out, effect_id.0);
            }
            CompositionState::Terminating { epoch, effect_id } => {
                out.push(4);
                put_u64(&mut out, epoch.0);
                put_u64(&mut out, effect_id.0);
            }
        }
        put_u64(&mut out, self.next_effect_id);
        encode_identity_option(&mut out, self.pending_composition_ack);
        encode_identity_option(&mut out, self.pending_request);
        put_u64(&mut out, self.queue_head as u64);
        put_u64(&mut out, self.queue_len as u64);
        for entry in self.queued_keys {
            match entry {
                None => out.push(0),
                Some(entry) => {
                    out.push(1);
                    encode_identity(&mut out, entry.identity);
                    match entry.action {
                        MutatingAction::Key { token, key_up } => {
                            out.push(0);
                            put_u64(&mut out, token);
                            out.push(u8::from(key_up));
                        }
                        MutatingAction::SelectCandidate { index } => {
                            out.push(1);
                            put_u16(&mut out, index);
                        }
                    }
                }
            }
        }
        for entry in self.key_decisions.entries {
            match entry {
                None => out.push(0),
                Some(entry) => {
                    out.push(1);
                    encode_key(&mut out, entry.key);
                    out.push(entry.decision as u8);
                    out.push(u8::from(entry.key_up));
                    put_u64(&mut out, entry.expires_at_ms);
                }
            }
        }
        for entry in self.outstanding_effects.entries {
            match entry {
                None => out.push(0),
                Some(entry) => {
                    out.push(1);
                    put_u64(&mut out, entry.effect_id.0);
                    out.push(entry.result_class as u8);
                    match entry.scope {
                        EffectScope::Composition { session, epoch } => {
                            out.push(0);
                            put_u64(&mut out, session.0);
                            put_u64(&mut out, epoch.0);
                        }
                        EffectScope::Commit {
                            client_instance_id,
                            session,
                            epoch,
                            commit_id,
                        } => {
                            out.push(1);
                            put_u64(&mut out, client_instance_id.0);
                            put_u64(&mut out, session.0);
                            put_u64(&mut out, epoch.0);
                            put_u64(&mut out, commit_id.0);
                        }
                    }
                }
            }
        }
        for entry in self.commits {
            match entry {
                None => out.push(0),
                Some(entry) => {
                    out.push(1);
                    encode_identity(&mut out, entry.identity);
                    put_u64(&mut out, entry.commit_id.0);
                    encode_commit_status(&mut out, entry.status);
                }
            }
        }
        match self.pending_commit {
            None => out.push(0),
            Some(entry) => {
                out.push(1);
                encode_identity(&mut out, entry.identity);
                put_u64(&mut out, entry.commit_id.0);
                encode_commit_status(&mut out, entry.status);
            }
        }
        out
    }

    pub fn structural_hash(&self) -> u64 {
        fnv1a64(&self.canonical_bytes())
    }

    pub fn commit_entries(&self) -> impl Iterator<Item = CommitEntry> + '_ {
        self.commits.iter().flatten().copied()
    }

    pub fn structural_invariants_hold(&self) -> bool {
        let mut composition_count = 0;
        let mut commit_count = 0;
        let mut composition_matches = true;
        let mut commit_matches = true;
        for expectation in self.outstanding_effects.entries.iter().flatten() {
            match expectation.result_class {
                EffectResultClass::CompositionTermination
                | EffectResultClass::CompositionStart
                | EffectResultClass::CompositionUpdate => {
                    composition_count += 1;
                    composition_matches &= match (self.composition, expectation.result_class) {
                        (
                            CompositionState::Starting { epoch, effect_id },
                            EffectResultClass::CompositionStart,
                        )
                        | (
                            CompositionState::Updating { epoch, effect_id },
                            EffectResultClass::CompositionUpdate,
                        )
                        | (
                            CompositionState::Terminating { epoch, effect_id },
                            EffectResultClass::CompositionTermination,
                        ) => {
                            effect_id == expectation.effect_id
                                && matches!(expectation.scope, EffectScope::Composition { epoch: scope_epoch, .. } if scope_epoch == epoch)
                        }
                        _ => false,
                    };
                }
                EffectResultClass::HostCommit => {
                    commit_count += 1;
                    commit_matches &= self.pending_commit.is_some_and(|pending| {
                        matches!(pending.status, CommitStatus::Applying { effect_id } if effect_id == expectation.effect_id)
                            && matches!(expectation.scope, EffectScope::Commit { client_instance_id, session, commit_id, .. }
                                if client_instance_id == pending.identity.client_instance_id
                                    && session == pending.identity.session_id
                                    && commit_id == pending.commit_id)
                    });
                }
            }
        }
        let transient_composition = matches!(
            self.composition,
            CompositionState::Starting { .. }
                | CompositionState::Updating { .. }
                | CompositionState::Terminating { .. }
        );
        let composition_ok = match (transient_composition, self.pending_composition_ack) {
            (true, Some(_))
                if matches!(
                    self.composition,
                    CompositionState::Starting { .. } | CompositionState::Updating { .. }
                ) =>
            {
                composition_count == 1 && composition_matches
            }
            (true, _) if matches!(self.composition, CompositionState::Terminating { .. }) => {
                composition_count == 1 && composition_matches
            }
            (false, None) => composition_count == 0,
            _ => false,
        };
        let commit_ok = match self.pending_commit {
            Some(CommitEntry {
                status: CommitStatus::Applying { effect_id },
                ..
            }) => {
                commit_count == 1
                    && commit_matches
                    && self
                        .commits
                        .iter()
                        .flatten()
                        .any(|entry| entry.status == CommitStatus::Applying { effect_id })
            }
            _ => commit_count == 0,
        };
        composition_ok && commit_ok
    }

    /// Safety constraints for every reachable transition prefix, including failure paths.
    pub fn model_invariants_hold(&self) -> bool {
        if !self.structural_invariants_hold() {
            return false;
        }
        if (self.focus == FocusState::Focused) != self.active_session.is_some() {
            return false;
        }
        if let CompositionState::Active { epoch } = self.composition {
            if self.pending_commit.is_none()
                && (self.focus != FocusState::Focused
                    || self.mode == RuntimeMode::Passthrough
                    || epoch != self.composition_epoch)
            {
                return false;
            }
        }
        true
    }
}

fn encode_commit_status(out: &mut Vec<u8>, status: CommitStatus) {
    match status {
        CommitStatus::Applying { effect_id } => {
            out.push(0);
            put_u64(out, effect_id.0);
        }
        CommitStatus::Applied { host_revision } => {
            out.push(1);
            put_u64(out, host_revision.0);
        }
        CommitStatus::Rejected => out.push(2),
        CommitStatus::Indeterminate => out.push(3),
    }
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn encode_option_u64(out: &mut Vec<u8>, value: Option<u64>) {
    match value {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            put_u64(out, value);
        }
    }
}

fn encode_identity(out: &mut Vec<u8>, identity: MessageIdentity) {
    put_u64(out, identity.client_instance_id.0);
    put_u64(out, identity.broker_generation.0);
    put_u64(out, identity.session_id.0);
    put_u64(out, identity.focus_epoch.0);
    put_u64(out, identity.composition_epoch.0);
    put_u64(out, identity.request_seq.0);
}

fn encode_identity_option(out: &mut Vec<u8>, identity: Option<MessageIdentity>) {
    match identity {
        None => out.push(0),
        Some(identity) => {
            out.push(1);
            encode_identity(out, identity);
        }
    }
}

fn identity_is_current(state: &RuntimeState, identity: MessageIdentity) -> bool {
    identity_context_is_current(state, identity)
        && state.composition_epoch == identity.composition_epoch
}

fn begin_composition_cancel(state: &mut RuntimeState, session: SessionId, epoch: Epoch) -> Effect {
    begin_composition_cancel_with_reason(state, session, epoch, None)
}

fn begin_composition_cancel_with_reason(
    state: &mut RuntimeState,
    session: SessionId,
    epoch: Epoch,
    reason: Option<DiagnosticCode>,
) -> Effect {
    let effect_id = state.allocate_effect();
    let registered = state.outstanding_effects.register(EffectExpectation {
        effect_id,
        result_class: EffectResultClass::CompositionTermination,
        scope: EffectScope::Composition { session, epoch },
    });
    debug_assert!(registered, "outstanding effect capacity exhausted");
    state.composition = CompositionState::Terminating { epoch, effect_id };
    Effect::CancelComposition {
        effect_id,
        session,
        epoch,
        reason,
    }
}

fn recover_queue_overflow(mut state: RuntimeState, key_up: bool) -> TransitionResult {
    let session = state.active_session;
    let active_composition = state.composition;
    state.mode = RuntimeMode::Passthrough;
    state.focus = FocusState::Unfocused;
    state.focus_epoch = Epoch(state.focus_epoch.0.saturating_add(1));
    state.composition_epoch = Epoch(state.composition_epoch.0.saturating_add(1));
    state.active_session = None;
    state.key_decisions = KeyDecisionLedger::new();
    state.clear_queued_mutations();

    let effect = match (session, active_composition) {
        (Some(session), CompositionState::Active { epoch }) => {
            begin_composition_cancel_with_reason(
                &mut state,
                session,
                epoch,
                Some(DiagnosticCode::MutatingQueueFull),
            )
        }
        _ => {
            let effect_id = state.allocate_effect();
            Effect::EnterPassthrough {
                effect_id,
                reason: Some(DiagnosticCode::MutatingQueueFull),
            }
        }
    };
    TransitionResult::one(
        state,
        Some(callback_reply(KeyPhase::Test, key_up, EatDecision::Pass)),
        effect,
    )
}

fn callback_reply(phase: KeyPhase, key_up: bool, decision: EatDecision) -> ImmediateReply {
    match (phase, key_up) {
        (KeyPhase::Test, false) => ImmediateReply::TestKey(decision),
        (KeyPhase::Actual, false) => ImmediateReply::Key(decision),
        (KeyPhase::Test, true) => ImmediateReply::TestKeyUp(decision),
        (KeyPhase::Actual, true) => ImmediateReply::KeyUp(decision),
    }
}

fn diagnostic_transition(mut state: RuntimeState, code: DiagnosticCode) -> TransitionResult {
    let effect_id = state.allocate_effect();
    TransitionResult::one(state, None, Effect::RecordDiagnostic { effect_id, code })
}

fn reduce_key_callback(
    mut state: RuntimeState,
    phase: KeyPhase,
    key: KeyObservation,
    key_up: bool,
) -> TransitionResult {
    state.key_decisions.expire(key.now_ms);
    match phase {
        KeyPhase::Test => {
            if state.mode != RuntimeMode::Normal
                || state.focus != FocusState::Focused
                || state.active_session.is_none()
                || state.broker_generation.is_none()
                || state.pending_commit.is_some()
                || matches!(state.composition, CompositionState::Terminating { .. })
                || state.composition_epoch.0 == u64::MAX
                || state.last_request_seq.0 == u64::MAX
            {
                return TransitionResult::unchanged(
                    state,
                    Some(callback_reply(phase, key_up, EatDecision::Pass)),
                );
            }
            if state.key_decisions.len() >= LEDGER_CAPACITY {
                let effect_id = state.allocate_effect();
                return TransitionResult::one(
                    state,
                    Some(callback_reply(phase, key_up, EatDecision::Pass)),
                    Effect::RecordDiagnostic {
                        effect_id,
                        code: DiagnosticCode::KeyDecisionLedgerFull,
                    },
                );
            }
            if state.key_decisions.len()
                + usize::from(state.pending_request.is_some())
                + state.queue_len
                >= MUTATING_QUEUE_CAPACITY
            {
                return recover_queue_overflow(state, key_up);
            }
            let decision = EatDecision::Eat;
            // The capacity check above and this mutation are in the same pure transition, so a
            // free entry is guaranteed. Treat violation as an internal invariant failure rather
            // than keeping a second, unreachable overflow transition.
            state.key_decisions.insert(key, key_up, decision);
            TransitionResult::unchanged(state, Some(callback_reply(phase, key_up, decision)))
        }
        KeyPhase::Actual => {
            let decision = if state.mode == RuntimeMode::Normal
                && state.focus == FocusState::Focused
                && state.active_session.is_some()
                && state.broker_generation.is_some()
                && state.pending_commit.is_none()
                && !matches!(state.composition, CompositionState::Terminating { .. })
                && state.last_request_seq.0 < u64::MAX
            {
                state
                    .key_decisions
                    .consume(key, key_up)
                    .unwrap_or(EatDecision::Pass)
            } else {
                EatDecision::Pass
            };
            let reply = Some(callback_reply(phase, key_up, decision));
            if decision == EatDecision::Eat {
                if let Some(session) = state.active_session {
                    let request_seq = RequestSeq(state.last_request_seq.0 + 1);
                    state.last_request_seq = request_seq;
                    let identity = MessageIdentity {
                        client_instance_id: state.client_instance_id,
                        broker_generation: state.broker_generation.expect("broker checked above"),
                        session_id: session,
                        focus_epoch: state.focus_epoch,
                        composition_epoch: state.composition_epoch,
                        request_seq,
                    };
                    let enqueued = state.enqueue_key(QueuedKey {
                        identity,
                        action: MutatingAction::Key {
                            token: key.token,
                            key_up,
                        },
                    });
                    debug_assert!(enqueued, "test callback reserved input queue capacity");
                    if let Some(effect) = state.dispatch_next_key() {
                        return TransitionResult::one(state, reply, effect);
                    }
                }
            }
            TransitionResult::unchanged(state, reply)
        }
    }
}

fn identity_context_is_current(state: &RuntimeState, identity: MessageIdentity) -> bool {
    state.mode == RuntimeMode::Normal
        && state.client_instance_id == identity.client_instance_id
        && state.broker_generation == Some(identity.broker_generation)
        && state.active_session == Some(identity.session_id)
        && state.focus == FocusState::Focused
        && state.focus_epoch == identity.focus_epoch
}

fn encode_key(out: &mut Vec<u8>, key: KeyObservation) {
    put_u64(out, key.token);
    put_u16(out, key.virtual_key);
    put_u16(out, key.scan_code);
    put_u16(out, key.modifiers);
    out.push(u8::from(key.repeat));
    put_u64(out, key.now_ms);
}

fn encode_effect(out: &mut Vec<u8>, effect: Effect) {
    put_u64(out, effect.id().0);
    match effect {
        Effect::RecordDiagnostic { code, .. } => {
            out.push(0);
            out.push(code as u8);
        }
        Effect::SendKey {
            identity,
            token,
            key_up,
            ..
        } => {
            out.push(1);
            encode_identity(out, identity);
            put_u64(out, token);
            out.push(u8::from(key_up));
        }
        Effect::SelectCandidate {
            identity,
            candidate_index,
            ..
        } => {
            out.push(12);
            encode_identity(out, identity);
            put_u16(out, candidate_index);
        }
        Effect::CancelComposition { session, epoch, .. } => {
            out.push(2);
            put_u64(out, session.0);
            put_u64(out, epoch.0);
            match effect {
                Effect::CancelComposition {
                    reason: Some(code), ..
                } => {
                    out.push(1);
                    out.push(code as u8);
                }
                _ => out.push(0),
            }
        }
        Effect::BeginHostComposition {
            session,
            epoch,
            preedit_token_id,
            ..
        } => {
            out.push(8);
            put_u64(out, session.0);
            put_u64(out, epoch.0);
            put_u64(out, preedit_token_id);
        }
        Effect::UpdateHostComposition {
            session,
            epoch,
            preedit_token_id,
            ..
        } => {
            out.push(9);
            put_u64(out, session.0);
            put_u64(out, epoch.0);
            put_u64(out, preedit_token_id);
        }
        Effect::TerminateHostComposition { session, epoch, .. } => {
            out.push(10);
            put_u64(out, session.0);
            put_u64(out, epoch.0);
        }
        Effect::SendRequestAck { identity, .. } => {
            out.push(11);
            encode_identity(out, identity);
        }
        Effect::ApplyHostCommit {
            client_instance_id,
            session,
            epoch,
            commit_id,
            token_id,
            ..
        } => {
            out.push(4);
            put_u64(out, client_instance_id.0);
            put_u64(out, session.0);
            put_u64(out, epoch.0);
            put_u64(out, commit_id.0);
            put_u64(out, token_id);
        }
        Effect::SendCommitPending { commit_id, .. } => {
            out.push(5);
            put_u64(out, commit_id.0);
        }
        Effect::SendCommitApplied {
            commit_id,
            host_revision,
            ..
        } => {
            out.push(6);
            put_u64(out, commit_id.0);
            put_u64(out, host_revision.0);
        }
        Effect::SendCommitRejected { commit_id, .. } => {
            out.push(7);
            put_u64(out, commit_id.0);
        }
        Effect::EnterPassthrough { reason, .. } => {
            out.push(3);
            match reason {
                Some(code) => {
                    out.push(1);
                    out.push(code as u8);
                }
                None => out.push(0),
            }
        }
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

/// Pure state transition. Host callback decisions are always returned synchronously.
pub fn reduce(state: RuntimeState, event: Event) -> TransitionResult {
    let result = reduce_inner(state, event);
    debug_assert!(
        result.next_state.model_invariants_hold(),
        "runtime model invariant violated"
    );
    debug_assert!(
        validate_effect_batch(&result.effects),
        "S2 effect batch is not canonical"
    );
    debug_assert!(
        reply_matches_event(event, result.immediate),
        "S3 callback reply mismatch"
    );
    result
}

fn reduce_inner(mut state: RuntimeState, event: Event) -> TransitionResult {
    match event {
        Event::HostKey { phase, key } => reduce_key_callback(state, phase, key, false),
        Event::HostKeyUp { phase, key } => reduce_key_callback(state, phase, key, true),
        Event::PhysicalKey { virtual_key, down } => {
            let bit = match virtual_key {
                0xA0 => Some(0), // left shift
                0xA1 => Some(1), // right shift
                0xA2 => Some(2), // left control
                0xA3 => Some(3), // right control
                0xA4 => Some(4), // left alt
                0xA5 => Some(5), // right alt
                0x5B => Some(6), // left Windows
                0x5C => Some(7), // right Windows
                _ => None,
            };
            if let Some(bit) = bit {
                if down {
                    state.physical_modifier_down |= 1 << bit;
                } else {
                    state.physical_modifier_down &= !(1 << bit);
                }
            }
            TransitionResult::unchanged(state, None)
        }
        Event::DecisionTtlExpired { token, now_ms } => {
            if state.key_decisions.expire_token(token, now_ms) {
                let effect_id = state.allocate_effect();
                TransitionResult::one(
                    state,
                    None,
                    Effect::RecordDiagnostic {
                        effect_id,
                        code: DiagnosticCode::KeyDecisionExpired,
                    },
                )
            } else {
                TransitionResult::unchanged(state, None)
            }
        }
        Event::FocusGained { session }
            if state.focus == FocusState::Focused && state.active_session == Some(session) =>
        {
            TransitionResult::unchanged(state, None)
        }
        Event::FocusGained { session } | Event::ContextPushed { session } => {
            let old_session = state.active_session;
            state.focus_epoch = Epoch(state.focus_epoch.0.saturating_add(1));
            state.focus = FocusState::Focused;
            state.active_session = Some(session);
            if state.pending_commit.is_none() {
                match state.composition {
                    CompositionState::Active { epoch } if old_session.is_some() => {
                        state.composition_epoch =
                            Epoch(state.composition_epoch.0.saturating_add(1));
                        state.key_decisions = KeyDecisionLedger::new();
                        state.clear_queued_mutations();
                        let effect = begin_composition_cancel(
                            &mut state,
                            old_session.unwrap_or(session),
                            epoch,
                        );
                        return TransitionResult::one(state, None, effect);
                    }
                    CompositionState::Starting { .. } | CompositionState::Updating { .. } => {
                        state.composition_epoch =
                            Epoch(state.composition_epoch.0.saturating_add(1));
                    }
                    CompositionState::Terminating { .. } => {}
                    CompositionState::Active { .. } | CompositionState::Idle => {
                        state.composition = CompositionState::Idle;
                        state.composition_epoch =
                            Epoch(state.composition_epoch.0.saturating_add(1));
                    }
                }
            } else if !matches!(state.composition, CompositionState::Terminating { .. }) {
                state.composition_epoch = Epoch(state.composition_epoch.0.saturating_add(1));
            }
            state.key_decisions = KeyDecisionLedger::new();
            if state.pending_commit.is_none() {
                state.clear_queued_mutations();
            }
            TransitionResult::unchanged(state, None)
        }
        Event::FocusLost { session } | Event::ContextPopped { session }
            if state.active_session == Some(session) =>
        {
            state.focus = FocusState::Unfocused;
            state.focus_epoch = Epoch(state.focus_epoch.0.saturating_add(1));
            state.composition_epoch = Epoch(state.composition_epoch.0.saturating_add(1));
            state.active_session = None;
            state.key_decisions = KeyDecisionLedger::new();
            if state.pending_commit.is_some() {
                state.clear_queue_only();
            } else {
                state.clear_queued_mutations();
            }
            match state.composition {
                _ if state
                    .pending_commit
                    .is_some_and(|pending| pending.identity.session_id == session) =>
                {
                    TransitionResult::unchanged(state, None)
                }
                CompositionState::Active { epoch } => {
                    let effect = begin_composition_cancel(&mut state, session, epoch);
                    TransitionResult::one(state, None, effect)
                }
                _ => TransitionResult::unchanged(state, None),
            }
        }
        Event::BrokerDisconnected => {
            let old_session = state.active_session;
            state.mode = RuntimeMode::Passthrough;
            state.broker_generation = None;
            state.key_decisions = KeyDecisionLedger::new();
            state.focus = FocusState::Unfocused;
            state.focus_epoch = Epoch(state.focus_epoch.0.saturating_add(1));
            state.composition_epoch = Epoch(state.composition_epoch.0.saturating_add(1));
            state.active_session = None;
            if state.pending_commit.is_some() {
                state.clear_queue_only();
            } else {
                state.clear_queued_mutations();
            }
            if state.pending_commit.is_none() {
                if let (Some(session), CompositionState::Active { epoch }) =
                    (old_session, state.composition)
                {
                    let effect = begin_composition_cancel(&mut state, session, epoch);
                    return TransitionResult::one(state, None, effect);
                }
            }
            let effect_id = state.allocate_effect();
            TransitionResult::one(
                state,
                None,
                Effect::EnterPassthrough {
                    effect_id,
                    reason: None,
                },
            )
        }
        Event::HostClosing | Event::InputMethodDeactivated => {
            let old_session = state.active_session;
            state.mode = RuntimeMode::Passthrough;
            state.key_decisions = KeyDecisionLedger::new();
            state.focus = FocusState::Unfocused;
            state.focus_epoch = Epoch(state.focus_epoch.0.saturating_add(1));
            state.composition_epoch = Epoch(state.composition_epoch.0.saturating_add(1));
            state.active_session = None;
            if state.pending_commit.is_some() {
                state.clear_queue_only();
            } else {
                state.clear_queued_mutations();
            }
            if state.pending_commit.is_none() {
                if let (Some(session), CompositionState::Active { epoch }) =
                    (old_session, state.composition)
                {
                    let effect = begin_composition_cancel(&mut state, session, epoch);
                    return TransitionResult::one(state, None, effect);
                }
            }
            let effect_id = state.allocate_effect();
            TransitionResult::one(
                state,
                None,
                Effect::EnterPassthrough {
                    effect_id,
                    reason: None,
                },
            )
        }
        Event::BrokerReady { generation } => {
            state.broker_generation = Some(generation);
            if state.active_session.is_none() && state.focus == FocusState::Unfocused {
                state.mode = RuntimeMode::Normal;
            }
            TransitionResult::unchanged(state, None)
        }
        Event::RequestCompleted { identity } => {
            if !identity_is_current(&state, identity)
                || state
                    .pending_request
                    .map_or(true, |pending| pending.request_seq != identity.request_seq)
            {
                return diagnostic_transition(state, DiagnosticCode::StaleRequestCompletion);
            }
            state.pending_request = None;
            if let Some(effect) = state.dispatch_next_key() {
                return TransitionResult::one(state, None, effect);
            }
            TransitionResult::unchanged(state, None)
        }
        Event::DeadlineExpired { request_seq }
            if state
                .pending_request
                .is_some_and(|pending| pending.request_seq == request_seq) =>
        {
            if let Some(pending) = state.pending_commit.filter(|pending| {
                pending.identity.request_seq == request_seq
                    && matches!(pending.status, CommitStatus::Applying { .. })
            }) {
                if let CommitStatus::Applying { effect_id } = pending.status {
                    state.outstanding_effects.remove_effect(effect_id);
                }
                if let Some(index) = state.commits.iter().position(|entry| {
                    entry.is_some_and(|entry| {
                        entry.identity == pending.identity && entry.commit_id == pending.commit_id
                    })
                }) {
                    state.commits[index] = Some(CommitEntry {
                        status: CommitStatus::Indeterminate,
                        ..pending
                    });
                }
                state.pending_commit = None;
                state.composition = CompositionState::Idle;
                state.pending_composition_ack = None;
                state.mode = RuntimeMode::Passthrough;
                state.focus = FocusState::Unfocused;
                state.focus_epoch = Epoch(state.focus_epoch.0.saturating_add(1));
                state.composition_epoch = Epoch(state.composition_epoch.0.saturating_add(1));
                state.active_session = None;
                state.pending_request = None;
                state.key_decisions = KeyDecisionLedger::new();
                state.clear_queue_only();
                let effect_id = state.allocate_effect();
                return TransitionResult::one(
                    state,
                    None,
                    Effect::EnterPassthrough {
                        effect_id,
                        reason: None,
                    },
                );
            }
            let old_session = state.active_session;
            state.mode = RuntimeMode::Passthrough;
            state.focus = FocusState::Unfocused;
            state.focus_epoch = Epoch(state.focus_epoch.0.saturating_add(1));
            state.composition_epoch = Epoch(state.composition_epoch.0.saturating_add(1));
            state.active_session = None;
            state.pending_request = None;
            state.key_decisions = KeyDecisionLedger::new();
            state.clear_queue_only();
            if let (Some(session), CompositionState::Active { epoch }) =
                (old_session, state.composition)
            {
                let effect = begin_composition_cancel(&mut state, session, epoch);
                TransitionResult::one(state, None, effect)
            } else {
                let effect_id = state.allocate_effect();
                TransitionResult::one(
                    state,
                    None,
                    Effect::EnterPassthrough {
                        effect_id,
                        reason: None,
                    },
                )
            }
        }
        Event::DeadlineExpired { .. } => TransitionResult::unchanged(state, None),
        Event::PumpMutatingQueue => {
            if let Some(effect) = state.dispatch_next_key() {
                TransitionResult::one(state, None, effect)
            } else {
                TransitionResult::unchanged(state, None)
            }
        }
        Event::EngineUpdate {
            identity,
            preedit_token_id,
        } => {
            if !identity_context_is_current(&state, identity)
                || state.pending_request.map(|pending| pending.request_seq)
                    != Some(identity.request_seq)
                || state.pending_commit.is_some()
                || state.pending_composition_ack.is_some()
            {
                return TransitionResult::unchanged(state, None);
            }
            let (epoch, result_class, effect) = match (state.composition, preedit_token_id) {
                (CompositionState::Idle, Some(token_id)) => {
                    let next_epoch = state.composition_epoch.0.checked_add(1);
                    let Some(next_epoch) = next_epoch else {
                        return TransitionResult::unchanged(state, None);
                    };
                    let epoch = Epoch(next_epoch);
                    if identity.composition_epoch != epoch {
                        return TransitionResult::unchanged(state, None);
                    }
                    state.composition_epoch = epoch;
                    let effect_id = state.allocate_effect();
                    (
                        epoch,
                        EffectResultClass::CompositionStart,
                        Effect::BeginHostComposition {
                            effect_id,
                            session: identity.session_id,
                            epoch,
                            preedit_token_id: token_id,
                        },
                    )
                }
                (CompositionState::Active { epoch }, Some(token_id))
                    if identity.composition_epoch == epoch =>
                {
                    let effect_id = state.allocate_effect();
                    (
                        epoch,
                        EffectResultClass::CompositionUpdate,
                        Effect::UpdateHostComposition {
                            effect_id,
                            session: identity.session_id,
                            epoch,
                            preedit_token_id: token_id,
                        },
                    )
                }
                (CompositionState::Active { epoch }, None)
                    if identity.composition_epoch == epoch =>
                {
                    let effect_id = state.allocate_effect();
                    (
                        epoch,
                        EffectResultClass::CompositionTermination,
                        Effect::TerminateHostComposition {
                            effect_id,
                            session: identity.session_id,
                            epoch,
                        },
                    )
                }
                (CompositionState::Idle, None)
                    if identity.composition_epoch == state.composition_epoch =>
                {
                    state.pending_request = None;
                    let effect_id = state.allocate_effect();
                    return TransitionResult::one(
                        state,
                        None,
                        Effect::SendRequestAck {
                            effect_id,
                            identity,
                        },
                    );
                }
                _ => return TransitionResult::unchanged(state, None),
            };
            let effect_id = effect.id();
            let registered = state.outstanding_effects.register(EffectExpectation {
                effect_id,
                result_class,
                scope: EffectScope::Composition {
                    session: identity.session_id,
                    epoch,
                },
            });
            debug_assert!(registered, "outstanding effect capacity exhausted");
            state.composition = match result_class {
                EffectResultClass::CompositionStart => {
                    CompositionState::Starting { epoch, effect_id }
                }
                EffectResultClass::CompositionUpdate => {
                    CompositionState::Updating { epoch, effect_id }
                }
                EffectResultClass::CompositionTermination => {
                    CompositionState::Terminating { epoch, effect_id }
                }
                EffectResultClass::HostCommit => unreachable!(),
            };
            state.pending_composition_ack = Some(identity);
            TransitionResult::one(state, None, effect)
        }
        Event::CandidateViewUpdated {
            identity,
            revision,
            candidate_count,
        } if identity_is_current(&state, identity)
            && matches!(state.composition, CompositionState::Active { epoch } if epoch == identity.composition_epoch)
            && identity.request_seq <= state.last_request_seq
            && identity.request_seq >= state.candidate_view_request_seq
            && revision > state.candidate_view_revision =>
        {
            state.candidate_view_revision = revision;
            state.candidate_view_request_seq = identity.request_seq;
            state.candidate_count = candidate_count;
            TransitionResult::unchanged(state, None)
        }
        Event::CandidateViewUpdated { .. } => {
            diagnostic_transition(state, DiagnosticCode::StaleCandidateView)
        }
        Event::UIActionIntent {
            identity,
            revision,
            candidate_index,
        } if identity_is_current(&state, identity)
            && matches!(state.composition, CompositionState::Active { epoch } if epoch == identity.composition_epoch)
            && revision == state.candidate_view_revision
            && state.pending_commit.is_none()
            && candidate_index < state.candidate_count =>
        {
            if state.key_decisions.len()
                + state.queue_len
                + usize::from(state.pending_request.is_some())
                >= MUTATING_QUEUE_CAPACITY
            {
                let effect_id = state.allocate_effect();
                return TransitionResult::one(
                    state,
                    None,
                    Effect::RecordDiagnostic {
                        effect_id,
                        code: DiagnosticCode::MutatingQueueFull,
                    },
                );
            }
            let request_seq = RequestSeq(state.last_request_seq.0.saturating_add(1));
            state.last_request_seq = request_seq;
            let action_identity = MessageIdentity {
                request_seq,
                ..identity
            };
            let enqueued = state.enqueue_key(QueuedKey {
                identity: action_identity,
                action: MutatingAction::SelectCandidate {
                    index: candidate_index,
                },
            });
            debug_assert!(enqueued, "UI intent reserved mutating queue capacity");
            if let Some(effect) = state.dispatch_next_key() {
                return TransitionResult::one(state, None, effect);
            }
            TransitionResult::unchanged(state, None)
        }
        Event::UIActionIntent { .. } => diagnostic_transition(state, DiagnosticCode::StaleUiAction),
        Event::EffectResult {
            effect_id,
            result_class,
            scope,
            outcome,
            host_revision,
        } => {
            let Some(index) =
                state
                    .outstanding_effects
                    .matching_index(effect_id, result_class, scope)
            else {
                return diagnostic_transition(state, DiagnosticCode::StaleEffectResult);
            };
            state.outstanding_effects.entries[index] = None;
            let mut request_ack = None;
            let scope_session = match scope {
                EffectScope::Composition { session, .. } => Some(session),
                EffectScope::Commit { .. } => None,
            };
            match (result_class, state.composition) {
                (
                    EffectResultClass::CompositionStart,
                    CompositionState::Starting {
                        epoch,
                        effect_id: expected,
                    },
                )
                | (
                    EffectResultClass::CompositionUpdate,
                    CompositionState::Updating {
                        epoch,
                        effect_id: expected,
                    },
                ) if expected == effect_id => {
                    let identity = state.pending_composition_ack.take();
                    if outcome == EffectOutcome::Succeeded {
                        if let Some(identity) = identity
                            .filter(|identity| identity_context_is_current(&state, *identity))
                        {
                            state.composition = CompositionState::Active { epoch };
                            if state
                                .pending_request
                                .is_some_and(|pending| pending.request_seq == identity.request_seq)
                            {
                                state.pending_request = None;
                            }
                            request_ack = Some(identity);
                        } else if let Some(session) = scope_session {
                            state.mode = RuntimeMode::Passthrough;
                            state.pending_request = None;
                            let cancel = begin_composition_cancel(&mut state, session, epoch);
                            return TransitionResult::one(state, None, cancel);
                        }
                    } else {
                        state.mode = RuntimeMode::Passthrough;
                        state.pending_request = None;
                        state.focus = FocusState::Unfocused;
                        state.active_session = None;
                        state.composition = CompositionState::Idle;
                    }
                }
                (
                    EffectResultClass::CompositionTermination,
                    CompositionState::Terminating {
                        epoch,
                        effect_id: expected,
                    },
                ) if expected == effect_id => {
                    let identity = state.pending_composition_ack.take();
                    state.composition = CompositionState::Idle;
                    if outcome != EffectOutcome::Succeeded {
                        state.mode = RuntimeMode::Passthrough;
                        state.focus = FocusState::Unfocused;
                        state.active_session = None;
                        state.pending_request = None;
                    } else if let Some(identity) =
                        identity.filter(|identity| identity_context_is_current(&state, *identity))
                    {
                        state.pending_request = None;
                        request_ack = Some(identity);
                    }
                    let _ = epoch;
                }
                _ => {}
            }
            if let Some(identity) = request_ack {
                let ack_effect_id = state.allocate_effect();
                return TransitionResult::one(
                    state,
                    None,
                    Effect::SendRequestAck {
                        effect_id: ack_effect_id,
                        identity,
                    },
                );
            }
            if result_class == EffectResultClass::HostCommit {
                if let Some(pending) = state.pending_commit.filter(|pending| {
                    matches!(pending.status, CommitStatus::Applying { effect_id: id } if id == effect_id)
                }) {
                    if let Some(index) = state.commits.iter().position(|entry| entry.is_some_and(|entry| {
                        entry.identity == pending.identity && entry.commit_id == pending.commit_id
                    })) {
                        let (status, reply) = match (outcome, host_revision) {
                            (EffectOutcome::Succeeded, Some(host_revision)) => (
                                CommitStatus::Applied { host_revision },
                                Some(Effect::SendCommitApplied {
                                    effect_id: state.allocate_effect(),
                                    commit_id: pending.commit_id,
                                    host_revision,
                                }),
                            ),
                            (EffectOutcome::Rejected, _) => (
                                CommitStatus::Rejected,
                                Some(Effect::SendCommitRejected {
                                    effect_id: state.allocate_effect(),
                                    commit_id: pending.commit_id,
                                }),
                            ),
                            _ => (CommitStatus::Indeterminate, None),
                        };
                        state.commits[index] = Some(CommitEntry { status, ..pending });
                        state.pending_commit = None;
                        if state.pending_request.is_some_and(|request| request.request_seq == pending.identity.request_seq) {
                            state.pending_request = None;
                        }
                        match status {
                            CommitStatus::Applied { .. } | CommitStatus::Rejected => {
                                state.composition = CompositionState::Idle;
                                state.pending_composition_ack = None;
                            }
                            CommitStatus::Indeterminate => {
                                state.mode = RuntimeMode::Passthrough;
                                state.focus = FocusState::Unfocused;
                                state.active_session = None;
                                state.composition = CompositionState::Idle;
                                state.key_decisions = KeyDecisionLedger::new();
                                state.clear_queue_only();
                                state.pending_composition_ack = None;
                            }
                            CommitStatus::Applying { .. } => unreachable!(),
                        }
                        if let Some(reply) = reply {
                            return TransitionResult::one(state, None, reply);
                        }
                    }
                }
            }
            TransitionResult::unchanged(state, None)
        }
        Event::CommitIntent {
            identity,
            commit_id,
            token_id,
        } => {
            let session = identity.session_id;
            let epoch = identity.composition_epoch;
            if !identity_is_current(&state, identity) {
                return diagnostic_transition(state, DiagnosticCode::StaleCommitIntent);
            }
            if let Some(existing) = state
                .commits
                .iter()
                .flatten()
                .find(|entry| entry.identity == identity && entry.commit_id == commit_id)
                .copied()
            {
                let effect_id = state.allocate_effect();
                let effect = match existing.status {
                    CommitStatus::Applying { .. } => Effect::SendCommitPending {
                        effect_id,
                        commit_id,
                    },
                    CommitStatus::Applied { host_revision } => Effect::SendCommitApplied {
                        effect_id,
                        commit_id,
                        host_revision,
                    },
                    CommitStatus::Rejected => Effect::SendCommitRejected {
                        effect_id,
                        commit_id,
                    },
                    CommitStatus::Indeterminate => {
                        return diagnostic_transition(state, DiagnosticCode::StaleCommitIntent)
                    }
                };
                return TransitionResult::one(state, None, effect);
            }
            // identity_is_current above already gates mode, focus, client, broker,
            // session, and both epochs. Only commit-specific preconditions remain.
            if state.composition != (CompositionState::Active { epoch })
                || state.pending_commit.is_some()
                || state
                    .pending_request
                    .map_or(true, |request| request.request_seq != identity.request_seq)
            {
                return diagnostic_transition(state, DiagnosticCode::StaleCommitIntent);
            }
            let Some(slot) = state.commits.iter().position(Option::is_none) else {
                return diagnostic_transition(state, DiagnosticCode::StaleCommitIntent);
            };
            let effect_id = state.allocate_effect();
            let entry = CommitEntry {
                identity,
                commit_id,
                status: CommitStatus::Applying { effect_id },
            };
            let scope = EffectScope::Commit {
                client_instance_id: identity.client_instance_id,
                session,
                epoch,
                commit_id,
            };
            let registered = state.outstanding_effects.register(EffectExpectation {
                effect_id,
                result_class: EffectResultClass::HostCommit,
                scope,
            });
            debug_assert!(registered, "outstanding effect capacity exhausted");
            state.commits[slot] = Some(entry);
            state.pending_commit = Some(entry);
            state.last_request_seq = identity.request_seq;
            TransitionResult::one(
                state,
                None,
                Effect::ApplyHostCommit {
                    effect_id,
                    client_instance_id: identity.client_instance_id,
                    session,
                    epoch,
                    commit_id,
                    token_id,
                },
            )
        }
        _ => {
            let effect_id = state.allocate_effect();
            TransitionResult::one(
                state,
                None,
                Effect::RecordDiagnostic {
                    effect_id,
                    code: DiagnosticCode::InvalidTransition,
                },
            )
        }
    }
}

fn validate_effect_batch(effects: &[Option<Effect>; 2]) -> bool {
    if effects[1].is_some() && effects[0].is_none() {
        return false;
    }
    match (effects[0], effects[1]) {
        // The initial reducer intentionally emits at most one effect per
        // transition. Independent co-effects can be enabled with explicit
        // batch semantics once the corresponding adapters exist.
        (Some(_), Some(_)) => false,
        _ => true,
    }
}

fn reply_matches_event(event: Event, reply: Option<ImmediateReply>) -> bool {
    match (event, reply) {
        (
            Event::HostKey {
                phase: KeyPhase::Test,
                ..
            },
            Some(ImmediateReply::TestKey(_)),
        ) => true,
        (
            Event::HostKey {
                phase: KeyPhase::Actual,
                ..
            },
            Some(ImmediateReply::Key(_)),
        ) => true,
        (
            Event::HostKeyUp {
                phase: KeyPhase::Test,
                ..
            },
            Some(ImmediateReply::TestKeyUp(_)),
        ) => true,
        (
            Event::HostKeyUp {
                phase: KeyPhase::Actual,
                ..
            },
            Some(ImmediateReply::KeyUp(_)),
        ) => true,
        (Event::HostKey { .. }, _) => false,
        (Event::HostKeyUp { .. }, _) => false,
        (_, None) => true,
        (_, Some(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focused_state() -> RuntimeState {
        let initial = RuntimeState::new(ClientInstanceId(7));
        let connected = reduce(
            initial,
            Event::BrokerReady {
                generation: BrokerGeneration(2),
            },
        )
        .next_state;
        reduce(
            connected,
            Event::FocusGained {
                session: SessionId(3),
            },
        )
        .next_state
    }

    fn key(token: u64, virtual_key: u16, now_ms: u64) -> KeyObservation {
        KeyObservation {
            token,
            virtual_key,
            scan_code: 30,
            modifiers: 0,
            repeat: false,
            now_ms,
        }
    }

    #[test]
    fn test_and_actual_callbacks_return_the_same_synchronous_decision() {
        let tested = reduce(
            focused_state(),
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(1, 65, 10),
            },
        );
        assert_eq!(
            tested.immediate,
            Some(ImmediateReply::TestKey(EatDecision::Eat))
        );
        assert!(tested.effects.iter().all(Option::is_none));

        let actual = reduce(
            tested.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: key(2, 65, 11),
            },
        );
        assert_eq!(
            actual.immediate,
            Some(ImmediateReply::Key(EatDecision::Eat))
        );
        assert!(matches!(
            actual.effects[0],
            Some(Effect::SendKey { token: 2, .. })
        ));
    }

    #[test]
    fn key_up_callbacks_match_only_key_up_ledger_entries_and_preserve_direction() {
        let observation = key(31, 0xA0, 20);
        let tested_up = reduce(
            focused_state(),
            Event::HostKeyUp {
                phase: KeyPhase::Test,
                key: observation,
            },
        );
        assert_eq!(
            tested_up.immediate,
            Some(ImmediateReply::TestKeyUp(EatDecision::Eat))
        );
        let actual_up = reduce(
            tested_up.next_state,
            Event::HostKeyUp {
                phase: KeyPhase::Actual,
                key: observation,
            },
        );
        assert_eq!(
            actual_up.immediate,
            Some(ImmediateReply::KeyUp(EatDecision::Eat))
        );
        assert!(matches!(
            actual_up.effects[0],
            Some(Effect::SendKey {
                key_up: true,
                token: 31,
                ..
            })
        ));

        let tested_down = reduce(
            focused_state(),
            Event::HostKey {
                phase: KeyPhase::Test,
                key: observation,
            },
        );
        let mismatched_up = reduce(
            tested_down.next_state,
            Event::HostKeyUp {
                phase: KeyPhase::Actual,
                key: observation,
            },
        );
        assert_eq!(
            mismatched_up.immediate,
            Some(ImmediateReply::KeyUp(EatDecision::Pass))
        );
        assert_eq!(mismatched_up.effects, [None, None]);
        let actual_down = reduce(
            mismatched_up.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: observation,
            },
        );
        assert_eq!(
            actual_down.immediate,
            Some(ImmediateReply::Key(EatDecision::Eat))
        );
        assert!(matches!(
            actual_down.effects[0],
            Some(Effect::SendKey { key_up: false, .. })
        ));
    }

    #[test]
    fn decision_ttl_event_expires_only_the_matching_elapsed_entry() {
        let observation = key(41, 65, 10);
        let tested = reduce(
            focused_state(),
            Event::HostKey {
                phase: KeyPhase::Test,
                key: observation,
            },
        );
        let early = reduce(
            tested.next_state,
            Event::DecisionTtlExpired {
                token: observation.token,
                now_ms: 259,
            },
        );
        assert_eq!(early.next_state.key_decisions.len(), 1);
        assert!(early.effects.iter().all(Option::is_none));

        let expired = reduce(
            early.next_state,
            Event::DecisionTtlExpired {
                token: observation.token,
                now_ms: 260,
            },
        );
        assert_eq!(expired.next_state.key_decisions.len(), 0);
        assert!(matches!(
            expired.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::KeyDecisionExpired,
                ..
            })
        ));
        let duplicate_timer = reduce(
            expired.next_state,
            Event::DecisionTtlExpired {
                token: observation.token,
                now_ms: 261,
            },
        );
        assert!(duplicate_timer.effects.iter().all(Option::is_none));
        let actual = reduce(
            duplicate_timer.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: KeyObservation {
                    now_ms: 261,
                    ..observation
                },
            },
        );
        assert_eq!(
            actual.immediate,
            Some(ImmediateReply::Key(EatDecision::Pass))
        );
    }

    #[test]
    fn unmatched_or_expired_actual_callback_fails_open() {
        let tested = reduce(
            focused_state(),
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(1, 65, 10),
            },
        );
        let expired = reduce(
            tested.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: key(2, 65, 260),
            },
        );
        assert_eq!(
            expired.immediate,
            Some(ImmediateReply::Key(EatDecision::Pass))
        );
        assert!(expired.effects.iter().all(Option::is_none));

        let unmatched = reduce(
            focused_state(),
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: key(3, 66, 10),
            },
        );
        assert_eq!(
            unmatched.immediate,
            Some(ImmediateReply::Key(EatDecision::Pass))
        );
    }

    #[test]
    fn full_ledger_passes_without_eating_the_key() {
        let mut state = focused_state();
        for index in 0..LEDGER_CAPACITY {
            let transition = reduce(
                state,
                Event::HostKey {
                    phase: KeyPhase::Test,
                    key: key(index as u64, index as u16, 1),
                },
            );
            assert_eq!(
                transition.immediate,
                Some(ImmediateReply::TestKey(EatDecision::Eat))
            );
            state = transition.next_state;
        }
        let overflow = reduce(
            state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(99, 99, 1),
            },
        );
        assert_eq!(
            overflow.immediate,
            Some(ImmediateReply::TestKey(EatDecision::Pass))
        );
        assert!(matches!(
            overflow.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::KeyDecisionLedgerFull,
                ..
            })
        ));
    }

    #[test]
    fn ambiguous_physical_key_match_fails_open() {
        let state = focused_state();
        let first = reduce(
            state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(1, 65, 10),
            },
        );
        let second = reduce(
            first.next_state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(2, 65, 11),
            },
        );
        let actual = reduce(
            second.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: key(3, 65, 12),
            },
        );
        assert_eq!(
            actual.immediate,
            Some(ImmediateReply::Key(EatDecision::Pass))
        );
        assert!(!actual
            .effects
            .iter()
            .flatten()
            .any(|effect| matches!(effect, Effect::SendKey { .. })));
    }

    #[test]
    fn mutating_requests_are_queued_in_order_with_a_single_request_in_flight() {
        let mut state = focused_state();
        let mut first_identity = None;
        for index in 0..MUTATING_QUEUE_CAPACITY {
            let observation = key(index as u64, 30 + index as u16, index as u64 + 1);
            let tested = reduce(
                state,
                Event::HostKey {
                    phase: KeyPhase::Test,
                    key: observation,
                },
            );
            assert_eq!(
                tested.immediate,
                Some(ImmediateReply::TestKey(EatDecision::Eat))
            );
            let actual = reduce(
                tested.next_state,
                Event::HostKey {
                    phase: KeyPhase::Actual,
                    key: observation,
                },
            );
            assert_eq!(
                actual.immediate,
                Some(ImmediateReply::Key(EatDecision::Eat))
            );
            if index == 0 {
                let Some(Effect::SendKey { identity, .. }) = actual.effects[0] else {
                    panic!("first key should dispatch");
                };
                first_identity = Some(identity);
            } else {
                assert_eq!(actual.effects, [None, None]);
            }
            state = actual.next_state;
        }
        assert_eq!(state.pending_request.unwrap().request_seq, RequestSeq(1));
        assert_eq!(state.queue_len, MUTATING_QUEUE_CAPACITY - 1);

        let overflow = reduce(
            state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(99, 999, 100),
            },
        );
        assert_eq!(
            overflow.immediate,
            Some(ImmediateReply::TestKey(EatDecision::Pass))
        );
        assert_eq!(overflow.next_state.mode, RuntimeMode::Passthrough);
        assert_eq!(overflow.next_state.focus, FocusState::Unfocused);
        assert_eq!(overflow.next_state.active_session, None);
        assert_eq!(overflow.next_state.queue_len, 0);
        assert_eq!(overflow.next_state.pending_request, None);
        assert!(matches!(
            overflow.effects[0],
            Some(Effect::EnterPassthrough {
                reason: Some(DiagnosticCode::MutatingQueueFull),
                ..
            })
        ));

        let completed = reduce(
            state,
            Event::RequestCompleted {
                identity: first_identity.unwrap(),
            },
        );
        let Some(Effect::SendKey {
            identity: second_identity,
            ..
        }) = completed.effects[0]
        else {
            panic!("next key should dispatch");
        };
        assert_eq!(second_identity.request_seq, RequestSeq(2));
        assert_eq!(completed.next_state.pending_request, Some(second_identity));
        assert_eq!(completed.next_state.queue_len, MUTATING_QUEUE_CAPACITY - 2);

        let duplicate_completion = reduce(
            completed.next_state,
            Event::RequestCompleted {
                identity: first_identity.unwrap(),
            },
        );
        assert_eq!(
            duplicate_completion.next_state.pending_request,
            completed.next_state.pending_request
        );
        assert!(matches!(
            duplicate_completion.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleRequestCompletion,
                ..
            })
        ));
    }

    #[test]
    fn queue_overflow_terminates_active_composition_and_fails_open() {
        let mut state = focused_state();
        state.composition = CompositionState::Active { epoch: Epoch(7) };
        state.composition_epoch = Epoch(7);
        for index in 0..MUTATING_QUEUE_CAPACITY {
            let observation = key(index as u64, index as u16, index as u64 + 1);
            state = reduce(
                state,
                Event::HostKey {
                    phase: KeyPhase::Test,
                    key: observation,
                },
            )
            .next_state;
            state = reduce(
                state,
                Event::HostKey {
                    phase: KeyPhase::Actual,
                    key: observation,
                },
            )
            .next_state;
        }
        let overflow = reduce(
            state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(99, 999, 100),
            },
        );
        assert_eq!(
            overflow.immediate,
            Some(ImmediateReply::TestKey(EatDecision::Pass))
        );
        assert_eq!(overflow.next_state.mode, RuntimeMode::Passthrough);
        assert!(matches!(
            overflow.next_state.composition,
            CompositionState::Terminating {
                epoch: Epoch(7),
                ..
            }
        ));
        assert!(matches!(
            overflow.effects[0],
            Some(Effect::CancelComposition {
                reason: Some(DiagnosticCode::MutatingQueueFull),
                ..
            })
        ));
        let _ = overflow.canonical_bytes();
        assert!(overflow.next_state.structural_invariants_hold());
    }

    #[test]
    fn full_key_decision_ledger_returns_pass_with_diagnostic() {
        let mut state = focused_state();
        state.key_decisions.entries = std::array::from_fn(|index| {
            Some(KeyDecision {
                key: key(index as u64, index as u16, 1),
                key_up: false,
                decision: EatDecision::Eat,
                expires_at_ms: u64::MAX,
            })
        });
        let result = reduce(
            state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(500, 65, 2),
            },
        );
        assert_eq!(
            result.immediate,
            Some(ImmediateReply::TestKey(EatDecision::Pass))
        );
        assert!(matches!(
            result.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::KeyDecisionLedgerFull,
                ..
            })
        ));
    }

    #[test]
    fn candidate_intent_at_capacity_is_diagnostic_only() {
        let mut state = focused_state();
        state.composition_epoch = Epoch(9);
        state.composition = CompositionState::Active { epoch: Epoch(9) };
        let (mut state, identity) = dispatched_key_identity(state, 1);
        let view = reduce(
            state,
            Event::CandidateViewUpdated {
                identity,
                revision: 1,
                candidate_count: 1,
            },
        );
        state = view.next_state;
        state.queue_len = MUTATING_QUEUE_CAPACITY - 1;
        let full = reduce(
            state,
            Event::UIActionIntent {
                identity,
                revision: 1,
                candidate_index: 0,
            },
        );
        assert_eq!(full.next_state.queue_len, state.queue_len);
        assert_eq!(full.next_state.pending_request, state.pending_request);
        assert_eq!(full.next_state.mode, state.mode);
        assert!(matches!(
            full.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::MutatingQueueFull,
                ..
            })
        ));
    }

    #[test]
    fn empty_engine_update_acks_request_without_composition_effect() {
        let (state, identity) = dispatched_key_identity(focused_state(), 1);
        let empty = reduce(
            state,
            Event::EngineUpdate {
                identity,
                preedit_token_id: None,
            },
        );
        assert_eq!(empty.next_state.composition, CompositionState::Idle);
        assert_eq!(empty.next_state.pending_request, None);
        assert!(matches!(
            empty.effects[0],
            Some(Effect::SendRequestAck { identity: ack, .. }) if ack == identity
        ));
    }

    #[test]
    fn host_commit_cache_capacity_fails_closed_without_new_effect() {
        let mut state = composing_state();
        let Event::CommitIntent { identity, .. } = commit_intent() else {
            unreachable!()
        };
        for (index, entry) in state.commits.iter_mut().enumerate() {
            *entry = Some(CommitEntry {
                identity: if index == 0 {
                    MessageIdentity {
                        focus_epoch: Epoch(0),
                        ..identity
                    }
                } else {
                    identity
                },
                commit_id: CommitId(100 + index as u64),
                status: CommitStatus::Rejected,
            });
        }
        let full = reduce(state, commit_intent());
        assert_eq!(full.next_state.commits, state.commits);
        assert_eq!(full.next_state.pending_commit, state.pending_commit);
        assert!(matches!(
            full.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleCommitIntent,
                ..
            })
        ));
    }

    #[test]
    fn focus_loss_during_applying_commit_preserves_terminal_obligation() {
        let applying = reduce(composing_state(), commit_intent());
        let lost = reduce(
            applying.next_state,
            Event::FocusLost {
                session: SessionId(3),
            },
        );
        assert_eq!(
            lost.next_state.pending_commit,
            applying.next_state.pending_commit
        );
        assert!(lost.effects.iter().all(Option::is_none));
        assert!(lost.next_state.structural_invariants_hold());
    }

    #[test]
    fn new_focus_during_applying_commit_preserves_old_terminal_obligation() {
        let applying = reduce(composing_state(), commit_intent());
        let Some(Effect::ApplyHostCommit {
            effect_id,
            client_instance_id,
            session,
            epoch,
            commit_id,
            ..
        }) = applying.effects[0]
        else {
            panic!("commit intent must emit a host commit");
        };
        let switched = reduce(
            applying.next_state,
            Event::FocusGained {
                session: SessionId(4),
            },
        );
        assert_eq!(switched.next_state.active_session, Some(SessionId(4)));
        assert_eq!(
            switched.next_state.pending_commit,
            applying.next_state.pending_commit
        );
        assert!(switched.next_state.model_invariants_hold());

        let new_identity = MessageIdentity {
            client_instance_id,
            broker_generation: BrokerGeneration(2),
            session_id: SessionId(4),
            focus_epoch: switched.next_state.focus_epoch,
            composition_epoch: switched.next_state.composition_epoch,
            request_seq: switched.next_state.last_request_seq,
        };
        let stale_view = reduce(
            switched.next_state,
            Event::CandidateViewUpdated {
                identity: new_identity,
                revision: 1,
                candidate_count: 1,
            },
        );
        assert!(matches!(
            stale_view.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleCandidateView,
                ..
            })
        ));
        let stale_action = reduce(
            switched.next_state,
            Event::UIActionIntent {
                identity: new_identity,
                revision: 0,
                candidate_index: 0,
            },
        );
        assert!(matches!(
            stale_action.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleUiAction,
                ..
            })
        ));

        let completed = reduce(
            switched.next_state,
            Event::EffectResult {
                effect_id,
                result_class: EffectResultClass::HostCommit,
                scope: EffectScope::Commit {
                    client_instance_id,
                    session,
                    epoch,
                    commit_id,
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: Some(HostRevision(19)),
            },
        );
        assert_eq!(completed.next_state.active_session, Some(SessionId(4)));
        assert_eq!(completed.next_state.pending_commit, None);
        assert!(completed.next_state.commits.iter().flatten().any(|entry| {
            entry.commit_id == commit_id
                && entry.status
                    == CommitStatus::Applied {
                        host_revision: HostRevision(19),
                    }
        }));
        assert!(completed.next_state.model_invariants_hold());
    }

    #[test]
    fn host_close_during_applying_commit_keeps_commit_terminal_obligation() {
        let applying = reduce(composing_state(), commit_intent());
        let closing = reduce(applying.next_state, Event::HostClosing);
        assert_eq!(
            closing.next_state.pending_commit,
            applying.next_state.pending_commit
        );
        assert_eq!(closing.next_state.mode, RuntimeMode::Passthrough);
        assert!(matches!(
            closing.effects[0],
            Some(Effect::EnterPassthrough { .. })
        ));
        assert!(closing.next_state.structural_invariants_hold());
    }

    #[test]
    fn model_invariant_rejects_stable_active_composition_in_failed_states() {
        let mut passthrough_active = composing_state();
        passthrough_active.mode = RuntimeMode::Passthrough;
        assert!(!passthrough_active.model_invariants_hold());

        let mut unfocused_active = composing_state();
        unfocused_active.focus = FocusState::Unfocused;
        unfocused_active.active_session = None;
        assert!(!unfocused_active.model_invariants_hold());

        let applying = reduce(composing_state(), commit_intent());
        let closing = reduce(applying.next_state, Event::HostClosing);
        assert!(closing.next_state.model_invariants_hold());
    }

    #[test]
    fn broker_disconnect_cancels_active_host_composition() {
        let state = composing_state();
        let disconnected = reduce(state, Event::BrokerDisconnected);
        assert_eq!(disconnected.next_state.mode, RuntimeMode::Passthrough);
        assert_eq!(disconnected.next_state.broker_generation, None);
        assert_eq!(disconnected.next_state.active_session, None);
        assert!(matches!(
            disconnected.effects[0],
            Some(Effect::CancelComposition {
                session: SessionId(3),
                epoch: Epoch(9),
                ..
            })
        ));
        assert!(disconnected.next_state.structural_invariants_hold());
    }

    #[test]
    fn broker_disconnect_during_composition_start_cancels_after_start_result() {
        let (state, mut identity) = dispatched_key_identity(focused_state(), 51);
        identity.composition_epoch = Epoch(state.composition_epoch.0 + 1);
        let started = reduce(
            state,
            Event::EngineUpdate {
                identity,
                preedit_token_id: Some(7),
            },
        );
        let Some(Effect::BeginHostComposition {
            effect_id: start_effect,
            session,
            epoch,
            ..
        }) = started.effects[0]
        else {
            panic!("first preedit starts the host composition");
        };

        let disconnected = reduce(started.next_state, Event::BrokerDisconnected);
        assert_eq!(disconnected.next_state.mode, RuntimeMode::Passthrough);
        assert!(matches!(
            disconnected.next_state.composition,
            CompositionState::Starting { effect_id, .. } if effect_id == start_effect
        ));

        // Termination depends on knowing whether the in-flight start reached TSF.
        let start_result = reduce(
            disconnected.next_state,
            Event::EffectResult {
                effect_id: start_effect,
                result_class: EffectResultClass::CompositionStart,
                scope: EffectScope::Composition { session, epoch },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        let Some(Effect::CancelComposition {
            effect_id: cancel_effect,
            session: cancel_session,
            epoch: cancel_epoch,
            ..
        }) = start_result.effects[0]
        else {
            panic!("a late successful start must be cancelled");
        };
        assert_eq!((cancel_session, cancel_epoch), (session, epoch));

        let terminated = reduce(
            start_result.next_state,
            Event::EffectResult {
                effect_id: cancel_effect,
                result_class: EffectResultClass::CompositionTermination,
                scope: EffectScope::Composition { session, epoch },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(terminated.next_state.composition, CompositionState::Idle);
        assert_eq!(terminated.next_state.mode, RuntimeMode::Passthrough);
        assert!(terminated
            .next_state
            .outstanding_effects
            .entries
            .iter()
            .all(Option::is_none));
        assert!(terminated.next_state.structural_invariants_hold());
    }

    #[test]
    fn deadline_with_active_composition_emits_cancel_obligation() {
        let state = focused_state();
        let (mut state, identity) = dispatched_key_identity(state, 50);
        state.composition_epoch = Epoch(9);
        state.composition = CompositionState::Active { epoch: Epoch(9) };
        let expired = reduce(
            state,
            Event::DeadlineExpired {
                request_seq: identity.request_seq,
            },
        );
        assert_eq!(expired.next_state.mode, RuntimeMode::Passthrough);
        assert!(matches!(
            expired.next_state.composition,
            CompositionState::Terminating {
                epoch: Epoch(9),
                ..
            }
        ));
        assert!(matches!(
            expired.effects[0],
            Some(Effect::CancelComposition {
                session: SessionId(3),
                epoch: Epoch(9),
                ..
            })
        ));
        assert!(expired.next_state.structural_invariants_hold());
    }

    #[test]
    fn candidate_intent_dispatches_immediately_when_mutation_lane_is_idle() {
        let mut state = focused_state();
        state.composition_epoch = Epoch(9);
        state.composition = CompositionState::Active { epoch: Epoch(9) };
        let (state, identity) = dispatched_key_identity(state, 60);
        let ready = reduce(state, Event::RequestCompleted { identity });
        let view = reduce(
            ready.next_state,
            Event::CandidateViewUpdated {
                identity,
                revision: 7,
                candidate_count: 2,
            },
        );
        let selected = reduce(
            view.next_state,
            Event::UIActionIntent {
                identity,
                revision: 7,
                candidate_index: 1,
            },
        );
        assert!(matches!(
            selected.effects[0],
            Some(Effect::SelectCandidate {
                candidate_index: 1,
                ..
            })
        ));
        assert_eq!(
            selected
                .next_state
                .pending_request
                .map(|request| request.request_seq),
            Some(RequestSeq(2))
        );
    }

    #[test]
    fn pump_dispatches_a_queued_mutation_when_lane_is_idle() {
        let mut state = focused_state();
        let identity = MessageIdentity {
            client_instance_id: state.client_instance_id,
            broker_generation: state.broker_generation.unwrap(),
            session_id: state.active_session.unwrap(),
            focus_epoch: state.focus_epoch,
            composition_epoch: state.composition_epoch,
            request_seq: RequestSeq(1),
        };
        assert!(state.enqueue_key(QueuedKey {
            identity,
            action: MutatingAction::Key {
                token: 77,
                key_up: false,
            },
        }));
        let pumped = reduce(state, Event::PumpMutatingQueue);
        assert!(matches!(
            pumped.effects[0],
            Some(Effect::SendKey { token: 77, .. })
        ));
        assert_eq!(pumped.next_state.pending_request, Some(identity));
        assert_eq!(pumped.next_state.queue_len, 0);
    }

    #[test]
    fn empty_mutating_queue_pump_is_a_noop() {
        let state = focused_state();
        let pumped = reduce(state, Event::PumpMutatingQueue);
        assert_eq!(pumped.next_state, state);
        assert_eq!(pumped.effects, [None, None]);
    }

    #[test]
    fn engine_update_rejects_wrong_or_exhausted_composition_epoch() {
        let (state, identity) = dispatched_key_identity(focused_state(), 78);
        let stale_epoch = reduce(
            state,
            Event::EngineUpdate {
                identity,
                preedit_token_id: Some(1),
            },
        );
        assert_eq!(stale_epoch.next_state, state);
        assert!(stale_epoch.effects.iter().all(Option::is_none));

        let empty_wrong_epoch = reduce(
            state,
            Event::EngineUpdate {
                identity: MessageIdentity {
                    composition_epoch: Epoch(state.composition_epoch.0 + 1),
                    ..identity
                },
                preedit_token_id: None,
            },
        );
        assert_eq!(empty_wrong_epoch.next_state, state);
        assert_eq!(empty_wrong_epoch.effects, [None, None]);

        let active = composing_state();
        let Event::CommitIntent {
            identity: active_identity,
            ..
        } = commit_intent()
        else {
            unreachable!()
        };
        for preedit_token_id in [Some(1), None] {
            let wrong_epoch = reduce(
                active,
                Event::EngineUpdate {
                    identity: MessageIdentity {
                        composition_epoch: Epoch(8),
                        ..active_identity
                    },
                    preedit_token_id,
                },
            );
            assert_eq!(wrong_epoch.next_state, active);
            assert_eq!(wrong_epoch.effects, [None, None]);
        }

        let mut exhausted = state;
        exhausted.composition_epoch = Epoch(u64::MAX);
        let exhausted_update = reduce(
            exhausted,
            Event::EngineUpdate {
                identity,
                preedit_token_id: Some(1),
            },
        );
        assert_eq!(exhausted_update.next_state, exhausted);
        assert!(exhausted_update.effects.iter().all(Option::is_none));
    }

    #[test]
    fn engine_update_ignores_stale_request_and_mismatched_active_composition() {
        let (state, identity) = dispatched_key_identity(focused_state(), 78);
        let stale_request = reduce(
            state,
            Event::EngineUpdate {
                identity: MessageIdentity {
                    request_seq: RequestSeq(identity.request_seq.0.saturating_add(1)),
                    ..identity
                },
                preedit_token_id: Some(1),
            },
        );
        assert_eq!(stale_request.next_state, state);
        assert_eq!(stale_request.effects, [None, None]);

        let mut inconsistent = state;
        inconsistent.composition_epoch = Epoch(9);
        inconsistent.composition = CompositionState::Active { epoch: Epoch(8) };
        // This deliberately bypasses `reduce`'s prefix invariant assertion to cover the
        // reducer's defensive fallback for a malformed persisted/internal state.
        let ignored = reduce_inner(
            inconsistent,
            Event::EngineUpdate {
                identity: MessageIdentity {
                    composition_epoch: Epoch(9),
                    ..identity
                },
                preedit_token_id: Some(2),
            },
        );
        assert_eq!(ignored.next_state, inconsistent);
        assert_eq!(ignored.effects, [None, None]);
    }

    #[test]
    fn engine_update_waits_for_in_flight_composition_result() {
        let (state, mut identity) = dispatched_key_identity(focused_state(), 79);
        identity.composition_epoch = Epoch(state.composition_epoch.0 + 1);
        let started = reduce(
            state,
            Event::EngineUpdate {
                identity,
                preedit_token_id: Some(1),
            },
        );
        let duplicate = reduce(
            started.next_state,
            Event::EngineUpdate {
                identity,
                preedit_token_id: Some(2),
            },
        );
        assert_eq!(duplicate.next_state, started.next_state);
        assert_eq!(duplicate.effects, [None, None]);
        assert!(matches!(
            duplicate.next_state.composition,
            CompositionState::Starting { .. }
        ));

        let Some(Effect::BeginHostComposition {
            effect_id,
            session,
            epoch,
            ..
        }) = started.effects[0]
        else {
            panic!("first update must begin composition");
        };
        let key_completed = reduce(started.next_state, Event::RequestCompleted { identity });
        assert_eq!(key_completed.next_state.pending_request, None);
        let completed = reduce(
            key_completed.next_state,
            Event::EffectResult {
                effect_id,
                result_class: EffectResultClass::CompositionStart,
                scope: EffectScope::Composition { session, epoch },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(
            completed.next_state.composition,
            CompositionState::Active { epoch }
        );
        assert!(matches!(
            completed.effects[0],
            Some(Effect::SendRequestAck { .. })
        ));
    }

    #[test]
    fn focus_loss_cancels_composition_as_a_single_obligation_effect() {
        let mut state = focused_state();
        state.composition = CompositionState::Active { epoch: Epoch(9) };
        let lost = reduce(
            state,
            Event::FocusLost {
                session: SessionId(3),
            },
        );
        assert_eq!(
            lost.next_state.composition,
            CompositionState::Terminating {
                epoch: Epoch(9),
                effect_id: EffectId(1)
            }
        );
        assert!(matches!(
            lost.effects[0],
            Some(Effect::CancelComposition {
                session: SessionId(3),
                epoch: Epoch(9),
                ..
            })
        ));
        assert_eq!(lost.effects[1], None);
        assert!(lost.next_state.structural_invariants_hold());

        let wrong_scope = reduce(
            lost.next_state,
            Event::EffectResult {
                effect_id: EffectId(1),
                result_class: EffectResultClass::CompositionTermination,
                scope: EffectScope::Composition {
                    session: SessionId(4),
                    epoch: Epoch(9),
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(
            wrong_scope.next_state.composition,
            lost.next_state.composition
        );
        assert!(wrong_scope.next_state.structural_invariants_hold());

        let completed = reduce(
            wrong_scope.next_state,
            Event::EffectResult {
                effect_id: EffectId(1),
                result_class: EffectResultClass::CompositionTermination,
                scope: EffectScope::Composition {
                    session: SessionId(3),
                    epoch: Epoch(9),
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(completed.next_state.composition, CompositionState::Idle);
        assert!(completed.next_state.structural_invariants_hold());
    }

    #[test]
    fn context_push_terminates_old_context_without_orphaning_start_effects() {
        let composing = composing_state();
        let repeated_focus = reduce(
            composing,
            Event::FocusGained {
                session: SessionId(3),
            },
        );
        assert_eq!(repeated_focus.next_state, composing);
        assert_eq!(repeated_focus.effects, [None, None]);

        let same_session_context = reduce(
            composing,
            Event::ContextPushed {
                session: SessionId(3),
            },
        );
        assert!(matches!(
            same_session_context.effects[0],
            Some(Effect::CancelComposition {
                session: SessionId(3),
                epoch: Epoch(9),
                ..
            })
        ));
        assert!(matches!(
            same_session_context.next_state.composition,
            CompositionState::Terminating {
                epoch: Epoch(9),
                ..
            }
        ));
        assert!(same_session_context.next_state.model_invariants_hold());

        let pushed = reduce(
            composing_state(),
            Event::ContextPushed {
                session: SessionId(4),
            },
        );
        assert_eq!(pushed.next_state.active_session, Some(SessionId(4)));
        assert_eq!(pushed.next_state.focus, FocusState::Focused);
        assert_eq!(pushed.next_state.composition_epoch, Epoch(10));
        assert!(matches!(
            pushed.effects[0],
            Some(Effect::CancelComposition {
                session: SessionId(3),
                epoch: Epoch(9),
                ..
            })
        ));
        assert!(pushed.next_state.structural_invariants_hold());

        let (state, mut identity) = dispatched_key_identity(focused_state(), 52);
        identity.composition_epoch = Epoch(state.composition_epoch.0 + 1);
        let starting = reduce(
            state,
            Event::EngineUpdate {
                identity,
                preedit_token_id: Some(8),
            },
        );
        let Some(Effect::BeginHostComposition {
            effect_id,
            session,
            epoch,
            ..
        }) = starting.effects[0]
        else {
            panic!("composition start is in flight");
        };
        let changed = reduce(
            starting.next_state,
            Event::FocusGained {
                session: SessionId(4),
            },
        );
        assert!(matches!(
            changed.next_state.composition,
            CompositionState::Starting {
                effect_id: pending,
                ..
            } if pending == effect_id
        ));
        assert!(changed.next_state.structural_invariants_hold());

        let late_start = reduce(
            changed.next_state,
            Event::EffectResult {
                effect_id,
                result_class: EffectResultClass::CompositionStart,
                scope: EffectScope::Composition { session, epoch },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert!(matches!(
            late_start.effects[0],
            Some(Effect::CancelComposition {
                session: SessionId(3),
                epoch: actual_epoch,
                ..
            }) if actual_epoch == epoch
        ));
        assert_eq!(late_start.next_state.active_session, Some(SessionId(4)));
        assert!(late_start.next_state.structural_invariants_hold());
    }

    #[test]
    fn mismatched_focus_loss_is_diagnostic_and_new_focus_preserves_pending_cancel() {
        let initial = focused_state();
        let stale = reduce(
            initial,
            Event::FocusLost {
                session: SessionId(99),
            },
        );
        assert_eq!(stale.next_state.active_session, initial.active_session);
        assert_eq!(stale.next_state.focus, initial.focus);
        assert_eq!(stale.next_state.composition, initial.composition);
        assert!(matches!(
            stale.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::InvalidTransition,
                ..
            })
        ));

        let mut composing = focused_state();
        composing.composition = CompositionState::Active { epoch: Epoch(9) };
        let cancel = reduce(
            composing,
            Event::FocusLost {
                session: SessionId(3),
            },
        );
        let Some(Effect::CancelComposition {
            effect_id,
            session,
            epoch,
            ..
        }) = cancel.effects[0]
        else {
            panic!("focus loss schedules composition cancellation");
        };
        let refocused = reduce(
            cancel.next_state,
            Event::FocusGained {
                session: SessionId(4),
            },
        );
        assert_eq!(refocused.next_state.active_session, Some(SessionId(4)));
        assert_eq!(refocused.next_state.focus, FocusState::Focused);
        assert_eq!(
            refocused.next_state.composition,
            CompositionState::Terminating { epoch, effect_id }
        );

        let terminated = reduce(
            refocused.next_state,
            Event::EffectResult {
                effect_id,
                result_class: EffectResultClass::CompositionTermination,
                scope: EffectScope::Composition { session, epoch },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(terminated.next_state.composition, CompositionState::Idle);
        assert_eq!(terminated.next_state.active_session, Some(SessionId(4)));
        assert_eq!(terminated.next_state.focus, FocusState::Focused);
        assert!(terminated.next_state.structural_invariants_hold());
    }

    fn dispatched_key_identity(
        mut state: RuntimeState,
        token: u64,
    ) -> (RuntimeState, MessageIdentity) {
        let observation = key(token, 65, token);
        let tested = reduce(
            state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: observation,
            },
        );
        state = tested.next_state;
        let actual = reduce(
            state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: observation,
            },
        );
        let Some(Effect::SendKey { identity, .. }) = actual.effects[0] else {
            panic!("key should dispatch");
        };
        (actual.next_state, identity)
    }

    #[test]
    fn engine_update_starts_composition_and_stale_start_is_cancelled() {
        let (state, mut identity) = dispatched_key_identity(focused_state(), 1);
        identity.composition_epoch = Epoch(state.composition_epoch.0 + 1);
        let started = reduce(
            state,
            Event::EngineUpdate {
                identity,
                preedit_token_id: Some(42),
            },
        );
        let Some(Effect::BeginHostComposition {
            effect_id,
            session,
            epoch,
            ..
        }) = started.effects[0]
        else {
            panic!("first preedit should begin a host composition");
        };
        assert_eq!(
            started.next_state.composition,
            CompositionState::Starting { epoch, effect_id }
        );

        let lost = reduce(started.next_state, Event::FocusLost { session });
        let completed = reduce(
            lost.next_state,
            Event::EffectResult {
                effect_id,
                result_class: EffectResultClass::CompositionStart,
                scope: EffectScope::Composition { session, epoch },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(completed.next_state.mode, RuntimeMode::Passthrough);
        assert!(
            matches!(completed.effects[0], Some(Effect::CancelComposition { session: s, epoch: e, .. }) if s == session && e == epoch)
        );
        assert!(completed.next_state.structural_invariants_hold());
    }

    #[test]
    fn composition_can_update_and_terminate_through_effect_results() {
        let (state, mut start_identity) = dispatched_key_identity(focused_state(), 1);
        start_identity.composition_epoch = Epoch(state.composition_epoch.0 + 1);
        let started = reduce(
            state,
            Event::EngineUpdate {
                identity: start_identity,
                preedit_token_id: Some(42),
            },
        );
        let Some(Effect::BeginHostComposition {
            effect_id: start_effect,
            session,
            epoch,
            ..
        }) = started.effects[0]
        else {
            panic!("first preedit starts host composition");
        };
        let active = reduce(
            started.next_state,
            Event::EffectResult {
                effect_id: start_effect,
                result_class: EffectResultClass::CompositionStart,
                scope: EffectScope::Composition { session, epoch },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(
            active.next_state.composition,
            CompositionState::Active { epoch }
        );

        let (state, update_identity) = dispatched_key_identity(active.next_state, 2);
        let updated = reduce(
            state,
            Event::EngineUpdate {
                identity: update_identity,
                preedit_token_id: Some(84),
            },
        );
        let Some(Effect::UpdateHostComposition {
            effect_id: update_effect,
            session,
            epoch,
            preedit_token_id: 84,
        }) = updated.effects[0]
        else {
            panic!("active composition update emits host update");
        };
        assert!(matches!(
            updated.next_state.composition,
            CompositionState::Updating { .. }
        ));
        let active = reduce(
            updated.next_state,
            Event::EffectResult {
                effect_id: update_effect,
                result_class: EffectResultClass::CompositionUpdate,
                scope: EffectScope::Composition { session, epoch },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(
            active.next_state.composition,
            CompositionState::Active { epoch }
        );

        let (state, terminate_identity) = dispatched_key_identity(active.next_state, 3);
        let terminating = reduce(
            state,
            Event::EngineUpdate {
                identity: terminate_identity,
                preedit_token_id: None,
            },
        );
        let Some(Effect::TerminateHostComposition {
            effect_id: terminate_effect,
            session,
            epoch,
        }) = terminating.effects[0]
        else {
            panic!("empty preedit terminates host composition");
        };
        assert!(matches!(
            terminating.next_state.composition,
            CompositionState::Terminating { .. }
        ));
        let idle = reduce(
            terminating.next_state,
            Event::EffectResult {
                effect_id: terminate_effect,
                result_class: EffectResultClass::CompositionTermination,
                scope: EffectScope::Composition { session, epoch },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(idle.next_state.composition, CompositionState::Idle);
        assert!(idle.next_state.structural_invariants_hold());
    }

    #[test]
    fn failed_composition_update_or_termination_fails_open() {
        for outcome in [EffectOutcome::Rejected, EffectOutcome::Indeterminate] {
            let mut state = focused_state();
            state.composition_epoch = Epoch(9);
            state.composition = CompositionState::Active { epoch: Epoch(9) };
            let (state, identity) = dispatched_key_identity(state, 10);
            let update = reduce(
                state,
                Event::EngineUpdate {
                    identity,
                    preedit_token_id: Some(77),
                },
            );
            let Some(Effect::UpdateHostComposition {
                effect_id,
                session,
                epoch,
                ..
            }) = update.effects[0]
            else {
                panic!("active composition update emits an effect");
            };
            let failed = reduce(
                update.next_state,
                Event::EffectResult {
                    effect_id,
                    result_class: EffectResultClass::CompositionUpdate,
                    scope: EffectScope::Composition { session, epoch },
                    outcome,
                    host_revision: None,
                },
            );
            assert_eq!(failed.next_state.mode, RuntimeMode::Passthrough);
            assert_eq!(failed.next_state.active_session, None);
            assert_eq!(failed.next_state.composition, CompositionState::Idle);

            let mut state = focused_state();
            state.composition_epoch = Epoch(9);
            state.composition = CompositionState::Active { epoch: Epoch(9) };
            let (state, identity) = dispatched_key_identity(state, 11);
            let termination = reduce(
                state,
                Event::EngineUpdate {
                    identity,
                    preedit_token_id: None,
                },
            );
            let Some(Effect::TerminateHostComposition {
                effect_id,
                session,
                epoch,
            }) = termination.effects[0]
            else {
                panic!("empty preedit emits a termination effect");
            };
            let failed = reduce(
                termination.next_state,
                Event::EffectResult {
                    effect_id,
                    result_class: EffectResultClass::CompositionTermination,
                    scope: EffectScope::Composition { session, epoch },
                    outcome,
                    host_revision: None,
                },
            );
            assert_eq!(failed.next_state.mode, RuntimeMode::Passthrough);
            assert_eq!(failed.next_state.active_session, None);
            assert_eq!(failed.next_state.composition, CompositionState::Idle);
        }
    }

    #[test]
    fn mutating_deadline_fails_to_passthrough_and_ignores_late_reply() {
        let (state, identity) = dispatched_key_identity(focused_state(), 1);
        let expired = reduce(
            state,
            Event::DeadlineExpired {
                request_seq: identity.request_seq,
            },
        );
        assert_eq!(expired.next_state.mode, RuntimeMode::Passthrough);
        assert_eq!(expired.next_state.pending_request, None);
        assert!(matches!(
            expired.effects[0],
            Some(Effect::EnterPassthrough { .. })
        ));
        let late = reduce(expired.next_state, Event::RequestCompleted { identity });
        assert_eq!(late.next_state.mode, expired.next_state.mode);
        assert_eq!(
            late.next_state.active_session,
            expired.next_state.active_session
        );
        assert!(matches!(
            late.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleRequestCompletion,
                ..
            })
        ));
        let stale_timer = reduce(
            late.next_state,
            Event::DeadlineExpired {
                request_seq: identity.request_seq,
            },
        );
        assert_eq!(stale_timer.next_state, late.next_state);
    }

    #[test]
    fn commit_deadline_marks_result_indeterminate_and_drops_late_terminal() {
        let started = reduce(composing_state(), commit_intent());
        let Some(Effect::ApplyHostCommit {
            effect_id,
            client_instance_id,
            session,
            epoch,
            commit_id,
            ..
        }) = started.effects[0]
        else {
            panic!("commit intent starts one host commit effect");
        };
        let request_seq = started
            .next_state
            .pending_commit
            .unwrap()
            .identity
            .request_seq;
        let expired = reduce(started.next_state, Event::DeadlineExpired { request_seq });
        assert_eq!(expired.next_state.pending_commit, None);
        assert_eq!(expired.next_state.mode, RuntimeMode::Passthrough);
        assert!(expired.next_state.commits.iter().flatten().any(|entry| {
            entry.commit_id == commit_id && entry.status == CommitStatus::Indeterminate
        }));
        assert!(matches!(
            expired.effects[0],
            Some(Effect::EnterPassthrough { .. })
        ));

        let late = reduce(
            expired.next_state,
            Event::EffectResult {
                effect_id,
                result_class: EffectResultClass::HostCommit,
                scope: EffectScope::Commit {
                    client_instance_id,
                    session,
                    epoch,
                    commit_id,
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: Some(HostRevision(99)),
            },
        );
        assert!(matches!(
            late.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleEffectResult,
                ..
            })
        ));
        assert_eq!(late.next_state.mode, expired.next_state.mode);
        assert_eq!(late.next_state.commits, expired.next_state.commits);
        assert_eq!(late.next_state.pending_commit, None);
    }

    #[test]
    fn commit_terminal_survives_prior_key_request_completion() {
        let Event::CommitIntent { identity, .. } = commit_intent() else {
            unreachable!()
        };
        let queued_key = key(98, 66, 3);
        let tested = reduce(
            composing_state(),
            Event::HostKey {
                phase: KeyPhase::Test,
                key: queued_key,
            },
        );
        let queued = reduce(
            tested.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: queued_key,
            },
        );
        assert_eq!(queued.next_state.queue_len, 1);
        let applying = reduce(queued.next_state, commit_intent());
        let Some(Effect::ApplyHostCommit {
            effect_id,
            client_instance_id,
            session,
            epoch,
            commit_id,
            ..
        }) = applying.effects[0]
        else {
            panic!("commit intent must emit a host commit");
        };
        let key_completed = reduce(applying.next_state, Event::RequestCompleted { identity });
        assert_eq!(key_completed.next_state.pending_request, None);
        assert_eq!(
            key_completed.next_state.pending_commit,
            applying.next_state.pending_commit
        );
        assert_eq!(key_completed.next_state.queue_len, 1);
        assert!(key_completed.effects.iter().all(Option::is_none));
        let premature_pump = reduce(key_completed.next_state, Event::PumpMutatingQueue);
        assert_eq!(premature_pump.next_state.queue_len, 1);
        assert!(premature_pump.effects.iter().all(Option::is_none));
        assert!(key_completed.next_state.model_invariants_hold());

        let committed = reduce(
            key_completed.next_state,
            Event::EffectResult {
                effect_id,
                result_class: EffectResultClass::HostCommit,
                scope: EffectScope::Commit {
                    client_instance_id,
                    session,
                    epoch,
                    commit_id,
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: Some(HostRevision(20)),
            },
        );
        assert_eq!(committed.next_state.pending_commit, None);
        assert!(committed.next_state.commits.iter().flatten().any(|entry| {
            entry.commit_id == commit_id
                && entry.status
                    == CommitStatus::Applied {
                        host_revision: HostRevision(20),
                    }
        }));
        assert!(matches!(
            committed.effects[0],
            Some(Effect::SendCommitApplied { .. })
        ));
        let dispatched = reduce(committed.next_state, Event::PumpMutatingQueue);
        assert!(matches!(
            dispatched.effects[0],
            Some(Effect::SendKey {
                identity: MessageIdentity {
                    request_seq: RequestSeq(2),
                    ..
                },
                token: 98,
                ..
            })
        ));
        assert_eq!(dispatched.next_state.queue_len, 0);
    }

    #[test]
    fn candidate_intents_share_the_mutation_sequence_and_reject_stale_views() {
        let mut state = focused_state();
        state.composition_epoch = Epoch(9);
        state.composition = CompositionState::Active { epoch: Epoch(9) };
        let (state, identity) = dispatched_key_identity(state, 1);
        let view = reduce(
            state,
            Event::CandidateViewUpdated {
                identity,
                revision: 4,
                candidate_count: 3,
            },
        );
        let stale = reduce(
            view.next_state,
            Event::UIActionIntent {
                identity,
                revision: 3,
                candidate_index: 1,
            },
        );
        assert_eq!(
            stale.next_state.candidate_view_revision,
            view.next_state.candidate_view_revision
        );
        assert_eq!(stale.next_state.queue_len, view.next_state.queue_len);
        assert!(matches!(
            stale.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleUiAction,
                ..
            })
        ));
        let stale_response = reduce(
            view.next_state,
            Event::CandidateViewUpdated {
                identity: MessageIdentity {
                    request_seq: RequestSeq(0),
                    ..identity
                },
                revision: 5,
                candidate_count: 9,
            },
        );
        assert_eq!(
            stale_response.next_state.candidate_view_revision,
            view.next_state.candidate_view_revision
        );
        assert_eq!(
            stale_response.next_state.candidate_count,
            view.next_state.candidate_count
        );
        assert!(matches!(
            stale_response.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleCandidateView,
                ..
            })
        ));

        let queued = reduce(
            view.next_state,
            Event::UIActionIntent {
                identity,
                revision: 4,
                candidate_index: 2,
            },
        );
        assert_eq!(queued.next_state.queue_len, 1);
        assert!(queued.effects.iter().all(Option::is_none));
        let completed = reduce(queued.next_state, Event::RequestCompleted { identity });
        let Some(Effect::SelectCandidate {
            identity: selected,
            candidate_index: 2,
            ..
        }) = completed.effects[0]
        else {
            panic!("candidate intent should be ordered behind the in-flight key");
        };
        let _ = queued.next_state.canonical_bytes();
        let _ = completed.canonical_bytes();
        assert_eq!(selected.request_seq, RequestSeq(identity.request_seq.0 + 1));

        let invalid_index = reduce(
            completed.next_state,
            Event::UIActionIntent {
                identity: selected,
                revision: 4,
                candidate_index: 3,
            },
        );
        assert_eq!(
            invalid_index.next_state.queue_len,
            completed.next_state.queue_len
        );
        assert!(matches!(
            invalid_index.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleUiAction,
                ..
            })
        ));
    }

    #[test]
    fn candidate_and_ui_guards_reject_stale_or_out_of_range_intents() {
        let (idle, idle_identity) = dispatched_key_identity(focused_state(), 79);
        let idle_view = reduce(
            idle,
            Event::CandidateViewUpdated {
                identity: idle_identity,
                revision: 1,
                candidate_count: 1,
            },
        );
        assert!(matches!(
            idle_view.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleCandidateView,
                ..
            })
        ));
        let idle_action = reduce(
            idle,
            Event::UIActionIntent {
                identity: idle_identity,
                revision: 0,
                candidate_index: 0,
            },
        );
        assert!(matches!(
            idle_action.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleUiAction,
                ..
            })
        ));

        let mut state = focused_state();
        state.composition_epoch = Epoch(9);
        state.composition = CompositionState::Active { epoch: Epoch(9) };
        let (state, identity) = dispatched_key_identity(state, 80);
        let view = reduce(
            state,
            Event::CandidateViewUpdated {
                identity,
                revision: 4,
                candidate_count: 3,
            },
        )
        .next_state;

        for event in [
            Event::CandidateViewUpdated {
                identity,
                revision: 4,
                candidate_count: 9,
            },
            Event::CandidateViewUpdated {
                identity: MessageIdentity {
                    request_seq: RequestSeq(identity.request_seq.0 + 1),
                    ..identity
                },
                revision: 5,
                candidate_count: 9,
            },
        ] {
            let rejected = reduce(view, event);
            assert_eq!(rejected.next_state.candidate_view_revision, 4);
            assert_eq!(rejected.next_state.candidate_count, 3);
            assert!(matches!(
                rejected.effects[0],
                Some(Effect::RecordDiagnostic {
                    code: DiagnosticCode::StaleCandidateView,
                    ..
                })
            ));
        }

        let out_of_range = reduce(
            view,
            Event::UIActionIntent {
                identity,
                revision: 4,
                candidate_index: 3,
            },
        );
        assert_eq!(out_of_range.next_state.queue_len, view.queue_len);
        assert!(matches!(
            out_of_range.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleUiAction,
                ..
            })
        ));

        let committing = reduce(view, commit_intent());
        let blocked = reduce(
            committing.next_state,
            Event::UIActionIntent {
                identity,
                revision: 4,
                candidate_index: 1,
            },
        );
        assert_eq!(
            blocked.next_state.pending_commit,
            committing.next_state.pending_commit
        );
        assert_eq!(
            blocked.next_state.queue_len,
            committing.next_state.queue_len
        );
        assert!(matches!(
            blocked.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleUiAction,
                ..
            })
        ));
    }

    #[test]
    fn modifier_release_bookkeeping_is_independent_of_mutating_queue_pressure() {
        let mut state = reduce(
            focused_state(),
            Event::PhysicalKey {
                virtual_key: 0xA2,
                down: true,
            },
        )
        .next_state;
        assert_ne!(state.physical_modifier_down & (1 << 2), 0);
        for index in 0..MUTATING_QUEUE_CAPACITY {
            let observation = key(index as u64, index as u16, index as u64 + 1);
            state = reduce(
                state,
                Event::HostKey {
                    phase: KeyPhase::Test,
                    key: observation,
                },
            )
            .next_state;
            state = reduce(
                state,
                Event::HostKey {
                    phase: KeyPhase::Actual,
                    key: observation,
                },
            )
            .next_state;
        }
        assert_eq!(
            state.queue_len + usize::from(state.pending_request.is_some()),
            MUTATING_QUEUE_CAPACITY
        );
        let released = reduce(
            state,
            Event::PhysicalKey {
                virtual_key: 0xA2,
                down: false,
            },
        );
        assert_eq!(released.next_state.physical_modifier_down & (1 << 2), 0);
        assert_eq!(released.next_state.queue_len, state.queue_len);
    }

    #[test]
    fn physical_modifier_map_tracks_each_side_and_ignores_non_modifiers() {
        let modifier_keys = [
            (0xA0, 0), // left shift
            (0xA1, 1), // right shift
            (0xA2, 2), // left control
            (0xA3, 3), // right control
            (0xA4, 4), // left alt
            (0xA5, 5), // right alt
            (0x5B, 6), // left Windows
            (0x5C, 7), // right Windows
        ];
        let mut state = focused_state();
        for (virtual_key, bit) in modifier_keys {
            state = reduce(
                state,
                Event::PhysicalKey {
                    virtual_key,
                    down: true,
                },
            )
            .next_state;
            assert_ne!(state.physical_modifier_down & (1 << bit), 0);
        }
        for (virtual_key, bit) in modifier_keys {
            state = reduce(
                state,
                Event::PhysicalKey {
                    virtual_key,
                    down: false,
                },
            )
            .next_state;
            assert_eq!(state.physical_modifier_down & (1 << bit), 0);
        }
        let unchanged = reduce(
            state,
            Event::PhysicalKey {
                virtual_key: 0x41,
                down: true,
            },
        );
        assert_eq!(unchanged.next_state.physical_modifier_down, 0);
        assert!(unchanged.effects.iter().all(Option::is_none));
    }

    #[test]
    fn host_close_and_deactivation_never_wait_and_cancel_active_composition() {
        let mut state = focused_state();
        state.composition_epoch = Epoch(9);
        state.composition = CompositionState::Active { epoch: Epoch(9) };
        let closing = reduce(state, Event::HostClosing);
        assert_eq!(closing.next_state.mode, RuntimeMode::Passthrough);
        assert_eq!(closing.next_state.active_session, None);
        assert!(matches!(
            closing.effects[0],
            Some(Effect::CancelComposition { .. })
        ));

        let deactivated = reduce(state, Event::InputMethodDeactivated);
        assert_eq!(deactivated.next_state.mode, RuntimeMode::Passthrough);
        assert!(matches!(
            deactivated.effects[0],
            Some(Effect::CancelComposition { .. })
        ));
    }

    #[test]
    fn unknown_effect_result_cannot_mutate_host_state() {
        let initial = focused_state();
        let result = reduce(
            initial,
            Event::EffectResult {
                effect_id: EffectId(999),
                result_class: EffectResultClass::CompositionTermination,
                scope: EffectScope::Composition {
                    session: SessionId(3),
                    epoch: Epoch(1),
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: None,
            },
        );
        assert_eq!(result.next_state.mode, initial.mode);
        assert_eq!(result.next_state.focus, initial.focus);
        assert_eq!(result.next_state.active_session, initial.active_session);
        assert_eq!(result.next_state.composition, initial.composition);
        assert!(matches!(
            result.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleEffectResult,
                ..
            })
        ));
        assert!(result.next_state.model_invariants_hold());
    }

    #[test]
    fn reducer_is_deterministic_for_the_same_trace() {
        let events = [
            Event::FocusGained {
                session: SessionId(3),
            },
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(1, 65, 10),
            },
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: key(1, 65, 11),
            },
            Event::BrokerDisconnected,
        ];
        let initial = RuntimeState::new(ClientInstanceId(7));
        let expected: Vec<_> = events
            .iter()
            .fold((initial, Vec::new()), |(state, mut trace), event| {
                let transition = reduce(state, *event);
                trace.push(transition);
                (transition.next_state, trace)
            })
            .1;
        for _ in 0..100 {
            let actual: Vec<_> = events
                .iter()
                .fold((initial, Vec::new()), |(state, mut trace), event| {
                    let transition = reduce(state, *event);
                    trace.push(transition);
                    (transition.next_state, trace)
                })
                .1;
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn non_callback_events_have_no_immediate_reply() {
        let transition = reduce(
            RuntimeState::new(ClientInstanceId(7)),
            Event::FocusGained {
                session: SessionId(3),
            },
        );
        assert_eq!(transition.immediate, None);
    }

    #[test]
    fn key_callbacks_fail_open_at_counter_limits_and_during_termination() {
        let observation = key(97, 65, 1);
        let mut exhausted_epoch = focused_state();
        exhausted_epoch.composition_epoch = Epoch(u64::MAX);
        let epoch_reply = reduce(
            exhausted_epoch,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: observation,
            },
        );
        assert_eq!(
            epoch_reply.immediate,
            Some(ImmediateReply::TestKey(EatDecision::Pass))
        );

        let mut exhausted_request = focused_state();
        exhausted_request.last_request_seq = RequestSeq(u64::MAX);
        for phase in [KeyPhase::Test, KeyPhase::Actual] {
            let reply = reduce(
                exhausted_request,
                Event::HostKey {
                    phase,
                    key: observation,
                },
            );
            assert_eq!(
                reply.immediate,
                Some(callback_reply(phase, false, EatDecision::Pass))
            );
            assert!(reply.effects.iter().all(Option::is_none));
        }

        let lost = reduce(
            composing_state(),
            Event::FocusLost {
                session: SessionId(3),
            },
        );
        let refocused = reduce(
            lost.next_state,
            Event::FocusGained {
                session: SessionId(4),
            },
        );
        assert!(matches!(
            refocused.next_state.composition,
            CompositionState::Terminating { .. }
        ));
        let actual = reduce(
            refocused.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: observation,
            },
        );
        assert_eq!(
            actual.immediate,
            Some(ImmediateReply::Key(EatDecision::Pass))
        );
        assert!(actual.effects.iter().all(Option::is_none));
    }

    #[test]
    fn s2_s3_oracles_reject_malformed_effect_batches_and_replies() {
        let diagnostic = Some(Effect::RecordDiagnostic {
            effect_id: EffectId(1),
            code: DiagnosticCode::InvalidTransition,
        });
        assert!(validate_effect_batch(&[diagnostic, None]));
        assert!(!validate_effect_batch(&[None, diagnostic]));
        assert!(!validate_effect_batch(&[diagnostic, diagnostic]));

        let key_down = Event::HostKey {
            phase: KeyPhase::Test,
            key: key(1, 65, 1),
        };
        let key_up = Event::HostKeyUp {
            phase: KeyPhase::Actual,
            key: key(1, 65, 2),
        };
        let non_callback = Event::FocusGained {
            session: SessionId(3),
        };
        assert!(!reply_matches_event(key_down, None));
        assert!(!reply_matches_event(
            key_down,
            Some(ImmediateReply::Key(EatDecision::Pass))
        ));
        assert!(!reply_matches_event(key_up, None));
        assert!(!reply_matches_event(
            key_up,
            Some(ImmediateReply::TestKeyUp(EatDecision::Pass))
        ));
        assert!(!reply_matches_event(
            non_callback,
            Some(ImmediateReply::TestKey(EatDecision::Pass))
        ));
    }

    fn composing_state() -> RuntimeState {
        let mut state = focused_state();
        let tested = reduce(
            state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: key(90, 65, 1),
            },
        );
        state = reduce(
            tested.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: key(90, 65, 2),
            },
        )
        .next_state;
        state.composition = CompositionState::Active { epoch: Epoch(9) };
        state.composition_epoch = Epoch(9);
        state
    }

    fn commit_intent() -> Event {
        Event::CommitIntent {
            identity: MessageIdentity {
                client_instance_id: ClientInstanceId(7),
                broker_generation: BrokerGeneration(2),
                session_id: SessionId(3),
                focus_epoch: Epoch(1),
                composition_epoch: Epoch(9),
                request_seq: RequestSeq(1),
            },
            commit_id: CommitId(44),
            token_id: 1234,
        }
    }

    #[test]
    fn applying_duplicate_is_acknowledged_without_a_second_host_effect() {
        let initial = composing_state();
        let first = reduce(initial, commit_intent());
        assert!(matches!(
            first.effects[0],
            Some(Effect::ApplyHostCommit {
                effect_id: EffectId(2),
                ..
            })
        ));
        assert!(first.next_state.structural_invariants_hold());

        let duplicate = reduce(first.next_state, commit_intent());
        assert!(matches!(
            duplicate.effects[0],
            Some(Effect::SendCommitPending {
                commit_id: CommitId(44),
                ..
            })
        ));
        let _ = first.canonical_bytes();
        let _ = duplicate.canonical_bytes();
        assert!(!duplicate
            .effects
            .iter()
            .flatten()
            .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));
        assert!(duplicate.next_state.structural_invariants_hold());

        let Event::CommitIntent {
            identity, token_id, ..
        } = commit_intent()
        else {
            unreachable!()
        };
        let different_commit = reduce(
            first.next_state,
            Event::CommitIntent {
                identity,
                commit_id: CommitId(45),
                token_id,
            },
        );
        assert_eq!(
            different_commit.next_state.pending_commit,
            first.next_state.pending_commit
        );
        assert!(matches!(
            different_commit.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleCommitIntent,
                ..
            })
        ));

        for preedit_token_id in [Some(7), None] {
            let blocked_update = reduce(
                first.next_state,
                Event::EngineUpdate {
                    identity,
                    preedit_token_id,
                },
            );
            assert_eq!(blocked_update.next_state, first.next_state);
            assert_eq!(blocked_update.effects, [None, None]);
        }

        let observation = key(91, 66, 3);
        let tested = reduce(
            first.next_state,
            Event::HostKey {
                phase: KeyPhase::Test,
                key: observation,
            },
        );
        assert_eq!(
            tested.immediate,
            Some(ImmediateReply::TestKey(EatDecision::Pass))
        );
        let actual = reduce(
            tested.next_state,
            Event::HostKey {
                phase: KeyPhase::Actual,
                key: observation,
            },
        );
        assert_eq!(
            actual.immediate,
            Some(ImmediateReply::Key(EatDecision::Pass))
        );
        assert_eq!(
            actual.next_state.pending_commit,
            first.next_state.pending_commit
        );
        assert!(actual.effects.iter().all(Option::is_none));
    }

    #[test]
    fn successful_commit_is_terminal_and_duplicate_only_resends_ack() {
        let first = reduce(composing_state(), commit_intent());
        let forged_scope = reduce(
            first.next_state,
            Event::EffectResult {
                effect_id: EffectId(2),
                result_class: EffectResultClass::HostCommit,
                scope: EffectScope::Commit {
                    client_instance_id: ClientInstanceId(8),
                    session: SessionId(3),
                    epoch: Epoch(9),
                    commit_id: CommitId(44),
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: Some(HostRevision(80)),
            },
        );
        assert_eq!(
            forged_scope.next_state.pending_commit,
            first.next_state.pending_commit
        );
        assert!(matches!(
            forged_scope.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleEffectResult,
                ..
            })
        ));
        let committed = reduce(
            first.next_state,
            Event::EffectResult {
                effect_id: EffectId(2),
                result_class: EffectResultClass::HostCommit,
                scope: EffectScope::Commit {
                    client_instance_id: ClientInstanceId(7),
                    session: SessionId(3),
                    epoch: Epoch(9),
                    commit_id: CommitId(44),
                },
                outcome: EffectOutcome::Succeeded,
                host_revision: Some(HostRevision(81)),
            },
        );
        assert!(matches!(
            committed.effects[0],
            Some(Effect::SendCommitApplied {
                host_revision: HostRevision(81),
                ..
            })
        ));
        assert_eq!(committed.next_state.composition, CompositionState::Idle);
        assert!(committed.next_state.structural_invariants_hold());

        let duplicate = reduce(committed.next_state, commit_intent());
        assert!(matches!(
            duplicate.effects[0],
            Some(Effect::SendCommitApplied { .. })
        ));
        assert!(!duplicate
            .effects
            .iter()
            .flatten()
            .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));
    }

    #[test]
    fn stale_commit_identity_is_rejected_before_host_effect() {
        let initial = composing_state();
        let Event::CommitIntent {
            identity,
            commit_id,
            token_id,
        } = commit_intent()
        else {
            unreachable!()
        };
        let mut forged = [identity; 6];
        forged[0].client_instance_id = ClientInstanceId(8);
        forged[1].broker_generation = BrokerGeneration(99);
        forged[2].session_id = SessionId(99);
        forged[3].focus_epoch = Epoch(88);
        forged[4].composition_epoch = Epoch(88);
        forged[5].request_seq = RequestSeq(2);
        for identity in forged {
            let rejected = reduce(
                initial,
                Event::CommitIntent {
                    identity,
                    commit_id,
                    token_id,
                },
            );
            assert_eq!(rejected.next_state.mode, initial.mode);
            assert_eq!(rejected.next_state.focus, initial.focus);
            assert_eq!(rejected.next_state.active_session, initial.active_session);
            assert_eq!(rejected.next_state.composition, initial.composition);
            assert!(matches!(
                rejected.effects[0],
                Some(Effect::RecordDiagnostic {
                    code: DiagnosticCode::StaleCommitIntent,
                    ..
                })
            ));
            assert!(!rejected
                .effects
                .iter()
                .flatten()
                .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));
        }
    }

    #[test]
    fn rejected_and_indeterminate_commit_have_distinct_terminal_paths() {
        let first = reduce(composing_state(), commit_intent());
        let rejected = reduce(
            first.next_state,
            Event::EffectResult {
                effect_id: EffectId(2),
                result_class: EffectResultClass::HostCommit,
                scope: EffectScope::Commit {
                    client_instance_id: ClientInstanceId(7),
                    session: SessionId(3),
                    epoch: Epoch(9),
                    commit_id: CommitId(44),
                },
                outcome: EffectOutcome::Rejected,
                host_revision: None,
            },
        );
        assert!(matches!(
            rejected.effects[0],
            Some(Effect::SendCommitRejected { .. })
        ));
        let _ = rejected.canonical_bytes();
        let _ = rejected.next_state.canonical_bytes();
        assert_eq!(rejected.next_state.mode, RuntimeMode::Normal);
        let rejected_retry = reduce(rejected.next_state, commit_intent());
        assert!(matches!(
            rejected_retry.effects[0],
            Some(Effect::SendCommitRejected { .. })
        ));
        let _ = rejected_retry.canonical_bytes();
        assert!(!rejected_retry
            .effects
            .iter()
            .flatten()
            .any(|effect| matches!(effect, Effect::ApplyHostCommit { .. })));

        let first = reduce(composing_state(), commit_intent());
        let uncertain = reduce(
            first.next_state,
            Event::EffectResult {
                effect_id: EffectId(2),
                result_class: EffectResultClass::HostCommit,
                scope: EffectScope::Commit {
                    client_instance_id: ClientInstanceId(7),
                    session: SessionId(3),
                    epoch: Epoch(9),
                    commit_id: CommitId(44),
                },
                outcome: EffectOutcome::Indeterminate,
                host_revision: None,
            },
        );
        assert_eq!(uncertain.next_state.mode, RuntimeMode::Passthrough);
        assert_eq!(uncertain.next_state.active_session, None);
        let _ = uncertain.next_state.canonical_bytes();
        let retry = reduce(uncertain.next_state, commit_intent());
        assert!(matches!(
            retry.effects[0],
            Some(Effect::RecordDiagnostic {
                code: DiagnosticCode::StaleCommitIntent,
                ..
            })
        ));
    }

    #[test]
    fn deterministic_scenarios_cover_every_transition_return_site() {
        let scenarios: &[fn()] = &[
            ambiguous_physical_key_match_fails_open,
            applying_duplicate_is_acknowledged_without_a_second_host_effect,
            broker_disconnect_cancels_active_host_composition,
            broker_disconnect_during_composition_start_cancels_after_start_result,
            candidate_intent_at_capacity_is_diagnostic_only,
            candidate_intent_dispatches_immediately_when_mutation_lane_is_idle,
            candidate_intents_share_the_mutation_sequence_and_reject_stale_views,
            candidate_and_ui_guards_reject_stale_or_out_of_range_intents,
            composition_can_update_and_terminate_through_effect_results,
            commit_deadline_marks_result_indeterminate_and_drops_late_terminal,
            commit_terminal_survives_prior_key_request_completion,
            context_push_terminates_old_context_without_orphaning_start_effects,
            deadline_with_active_composition_emits_cancel_obligation,
            decision_ttl_event_expires_only_the_matching_elapsed_entry,
            empty_engine_update_acks_request_without_composition_effect,
            empty_mutating_queue_pump_is_a_noop,
            engine_update_ignores_stale_request_and_mismatched_active_composition,
            engine_update_rejects_wrong_or_exhausted_composition_epoch,
            engine_update_starts_composition_and_stale_start_is_cancelled,
            engine_update_waits_for_in_flight_composition_result,
            failed_composition_update_or_termination_fails_open,
            focus_loss_cancels_composition_as_a_single_obligation_effect,
            focus_loss_during_applying_commit_preserves_terminal_obligation,
            full_key_decision_ledger_returns_pass_with_diagnostic,
            full_ledger_passes_without_eating_the_key,
            host_close_and_deactivation_never_wait_and_cancel_active_composition,
            host_close_during_applying_commit_keeps_commit_terminal_obligation,
            host_commit_cache_capacity_fails_closed_without_new_effect,
            key_up_callbacks_match_only_key_up_ledger_entries_and_preserve_direction,
            key_callbacks_fail_open_at_counter_limits_and_during_termination,
            mismatched_focus_loss_is_diagnostic_and_new_focus_preserves_pending_cancel,
            model_invariant_rejects_stable_active_composition_in_failed_states,
            modifier_release_bookkeeping_is_independent_of_mutating_queue_pressure,
            mutating_deadline_fails_to_passthrough_and_ignores_late_reply,
            mutating_requests_are_queued_in_order_with_a_single_request_in_flight,
            new_focus_during_applying_commit_preserves_old_terminal_obligation,
            non_callback_events_have_no_immediate_reply,
            physical_modifier_map_tracks_each_side_and_ignores_non_modifiers,
            pump_dispatches_a_queued_mutation_when_lane_is_idle,
            queue_overflow_terminates_active_composition_and_fails_open,
            reducer_is_deterministic_for_the_same_trace,
            rejected_and_indeterminate_commit_have_distinct_terminal_paths,
            stale_commit_identity_is_rejected_before_host_effect,
            successful_commit_is_terminal_and_duplicate_only_resends_ack,
            test_and_actual_callbacks_return_the_same_synchronous_decision,
            unknown_effect_result_cannot_mutate_host_state,
            unmatched_or_expired_actual_callback_fails_open,
        ];

        TRANSITION_RETURN_SITES.with(|sites| {
            *sites.borrow_mut() = Some(std::collections::BTreeSet::new());
        });
        for scenario in scenarios {
            scenario();
        }
        let observed = TRANSITION_RETURN_SITES.with(|sites| {
            sites
                .borrow_mut()
                .take()
                .expect("transition site capture was installed")
        });

        let production_source = include_str!("lib.rs")
            .split("mod tests {")
            .next()
            .expect("test module boundary exists");
        let expected: std::collections::BTreeSet<u32> = production_source
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                (line.contains("TransitionResult::unchanged(")
                    || line.contains("TransitionResult::one("))
                .then_some(index as u32 + 1)
            })
            .collect();
        let uncovered: Vec<u32> = expected.difference(&observed).copied().collect();
        assert!(
            uncovered.is_empty(),
            "deterministic reducer scenarios missed transition return sites at lines {uncovered:?}"
        );
    }
}
