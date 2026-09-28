//! D6 deployment commit-point model with deterministic crash recovery.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeploymentPhase {
    Stable,
    Draining,
    Activating,
    PersistingCommit,
    RollingBack,
    Passthrough,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeploymentSnapshot {
    pub source: u64,
    pub generated: u64,
    pub active_pointer: u64,
    pub journal_committed: u64,
    pub broker_runtime: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeploymentAction {
    None,
    CancelSessions,
    ActivateRuntime(u64),
    PersistCommit(u64),
    RestoreActivePointer(u64),
    DiscardCandidate(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeploymentMachine {
    pub phase: DeploymentPhase,
    pub snapshot: DeploymentSnapshot,
    candidate: Option<u64>,
    runtime_activated: bool,
    active_sessions: u32,
}

impl DeploymentMachine {
    pub fn stable(deployment: u64) -> Self {
        Self {
            phase: DeploymentPhase::Stable,
            snapshot: DeploymentSnapshot {
                source: deployment,
                generated: deployment,
                active_pointer: deployment,
                journal_committed: deployment,
                broker_runtime: Some(deployment),
            },
            candidate: None,
            runtime_activated: false,
            active_sessions: 0,
        }
    }

    pub fn set_active_sessions(&mut self, count: u32) -> bool {
        if self.phase != DeploymentPhase::Stable {
            return false;
        }
        self.active_sessions = count;
        true
    }

    pub fn session_cancelled(&mut self) -> bool {
        if self.phase != DeploymentPhase::Draining || self.active_sessions == 0 {
            return false;
        }
        self.active_sessions -= 1;
        true
    }

    pub fn prepare(&mut self, candidate: u64) -> DeploymentAction {
        if self.phase != DeploymentPhase::Stable || candidate == self.snapshot.journal_committed {
            return DeploymentAction::None;
        }
        self.snapshot.source = candidate;
        self.snapshot.generated = candidate;
        self.candidate = Some(candidate);
        self.phase = DeploymentPhase::Draining;
        self.snapshot.broker_runtime = None;
        DeploymentAction::CancelSessions
    }

    pub fn sessions_drained(&mut self) -> DeploymentAction {
        if self.phase != DeploymentPhase::Draining
            || self.candidate.is_none()
            || self.active_sessions != 0
        {
            return DeploymentAction::None;
        }
        self.phase = DeploymentPhase::Activating;
        self.snapshot.active_pointer = self.candidate.unwrap();
        DeploymentAction::ActivateRuntime(self.candidate.unwrap())
    }

    pub fn runtime_activated(&mut self, deployment: u64) -> DeploymentAction {
        if self.phase != DeploymentPhase::Activating || self.candidate != Some(deployment) {
            return DeploymentAction::None;
        }
        self.runtime_activated = true;
        self.snapshot.broker_runtime = Some(deployment);
        self.phase = DeploymentPhase::PersistingCommit;
        DeploymentAction::PersistCommit(deployment)
    }

    /// This is the only transition that updates the durable committed version.
    pub fn commit_persisted(&mut self, deployment: u64) -> DeploymentAction {
        if self.phase != DeploymentPhase::PersistingCommit
            || self.candidate != Some(deployment)
            || !self.runtime_activated
        {
            return DeploymentAction::None;
        }
        self.snapshot.journal_committed = deployment;
        self.snapshot.active_pointer = deployment;
        self.snapshot.source = deployment;
        self.snapshot.generated = deployment;
        self.snapshot.broker_runtime = Some(deployment);
        self.candidate = None;
        self.runtime_activated = false;
        self.phase = DeploymentPhase::Stable;
        self.active_sessions = 0;
        DeploymentAction::None
    }

    pub fn persist_failed(&mut self) -> DeploymentAction {
        if self.phase != DeploymentPhase::PersistingCommit {
            return DeploymentAction::None;
        }
        self.phase = DeploymentPhase::RollingBack;
        self.snapshot.broker_runtime = None;
        DeploymentAction::RestoreActivePointer(self.snapshot.journal_committed)
    }

    pub fn rollback_restored(&mut self, deployment: u64) -> DeploymentAction {
        if self.phase != DeploymentPhase::RollingBack
            || deployment != self.snapshot.journal_committed
        {
            self.phase = DeploymentPhase::Passthrough;
            self.snapshot.broker_runtime = None;
            return DeploymentAction::None;
        }
        let candidate = self.candidate.take();
        self.snapshot.source = deployment;
        self.snapshot.generated = deployment;
        self.snapshot.active_pointer = deployment;
        self.snapshot.broker_runtime = Some(deployment);
        self.runtime_activated = false;
        self.phase = DeploymentPhase::Stable;
        self.active_sessions = 0;
        candidate.map_or(DeploymentAction::None, DeploymentAction::DiscardCandidate)
    }

    /// Recovery trusts only the D6 journal, never the active pointer alone.
    pub fn recover_after_crash(&mut self, pointer_restore_succeeds: bool) -> DeploymentAction {
        let committed = self.snapshot.journal_committed;
        self.candidate = None;
        self.runtime_activated = false;
        self.active_sessions = 0;
        if self.snapshot.active_pointer != committed {
            if !pointer_restore_succeeds {
                self.phase = DeploymentPhase::Passthrough;
                self.snapshot.broker_runtime = None;
                return DeploymentAction::None;
            }
            self.snapshot.active_pointer = committed;
        }
        self.snapshot.source = committed;
        self.snapshot.generated = committed;
        self.snapshot.broker_runtime = Some(committed);
        self.phase = DeploymentPhase::Stable;
        DeploymentAction::None
    }

    pub fn sessions_may_start(&self) -> bool {
        self.phase == DeploymentPhase::Stable
    }

    /// Check phase-local facts after every transition, not only after recovery.
    pub fn invariants_hold(&self) -> bool {
        let snapshot = self.snapshot;
        match self.phase {
            DeploymentPhase::Stable => {
                self.candidate.is_none()
                    && !self.runtime_activated
                    && snapshot.source == snapshot.journal_committed
                    && snapshot.generated == snapshot.journal_committed
                    && snapshot.active_pointer == snapshot.journal_committed
                    && snapshot.broker_runtime == Some(snapshot.journal_committed)
            }
            DeploymentPhase::Draining => self.candidate.is_some_and(|candidate| {
                !self.runtime_activated
                    && snapshot.source == candidate
                    && snapshot.generated == candidate
                    && snapshot.active_pointer == snapshot.journal_committed
                    && snapshot.broker_runtime.is_none()
            }),
            DeploymentPhase::Activating => self.candidate.is_some_and(|candidate| {
                !self.runtime_activated
                    && snapshot.source == candidate
                    && snapshot.generated == candidate
                    && snapshot.active_pointer == candidate
                    && snapshot.broker_runtime.is_none()
            }),
            DeploymentPhase::PersistingCommit => self.candidate.is_some_and(|candidate| {
                self.runtime_activated
                    && candidate != snapshot.journal_committed
                    && snapshot.source == candidate
                    && snapshot.generated == candidate
                    && snapshot.active_pointer == candidate
                    && snapshot.broker_runtime == Some(candidate)
            }),
            DeploymentPhase::RollingBack => self.candidate.is_some_and(|candidate| {
                self.runtime_activated
                    && snapshot.source == candidate
                    && snapshot.generated == candidate
                    && snapshot.active_pointer == candidate
                    && snapshot.broker_runtime.is_none()
            }),
            DeploymentPhase::Passthrough => snapshot.broker_runtime.is_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d5_without_d6_recovers_committed_deployment() {
        let mut machine = DeploymentMachine::stable(10);
        assert_eq!(machine.prepare(11), DeploymentAction::CancelSessions);
        assert!(!machine.sessions_may_start());
        assert_eq!(
            machine.sessions_drained(),
            DeploymentAction::ActivateRuntime(11)
        );
        assert_eq!(
            machine.runtime_activated(11),
            DeploymentAction::PersistCommit(11)
        );
        assert_eq!(machine.phase, DeploymentPhase::PersistingCommit);
        assert_eq!(machine.snapshot.journal_committed, 10);

        machine.recover_after_crash(true);
        assert_eq!(machine.phase, DeploymentPhase::Stable);
        assert_eq!(machine.snapshot.active_pointer, 10);
        assert_eq!(machine.snapshot.broker_runtime, Some(10));
    }

    #[test]
    fn d6_persisted_deployment_survives_restart() {
        let mut machine = DeploymentMachine::stable(20);
        machine.prepare(21);
        machine.sessions_drained();
        machine.runtime_activated(21);
        machine.commit_persisted(21);
        assert_eq!(machine.snapshot.journal_committed, 21);

        machine.phase = DeploymentPhase::Passthrough;
        machine.snapshot.active_pointer = 20;
        assert_eq!(machine.recover_after_crash(true), DeploymentAction::None);
        assert_eq!(machine.snapshot.active_pointer, 21);
        assert_eq!(machine.snapshot.broker_runtime, Some(21));
    }

    #[test]
    fn failed_d6_rolls_back_or_stays_in_passthrough() {
        let mut machine = DeploymentMachine::stable(30);
        machine.prepare(31);
        machine.sessions_drained();
        machine.runtime_activated(31);
        assert_eq!(
            machine.persist_failed(),
            DeploymentAction::RestoreActivePointer(30)
        );
        assert_eq!(
            machine.rollback_restored(30),
            DeploymentAction::DiscardCandidate(31)
        );
        assert_eq!(machine.snapshot.journal_committed, 30);

        machine.prepare(32);
        machine.sessions_drained();
        machine.runtime_activated(32);
        machine.persist_failed();
        machine.rollback_restored(999);
        assert_eq!(machine.phase, DeploymentPhase::Passthrough);
        assert!(!machine.sessions_may_start());
    }

    #[test]
    fn activation_waits_until_every_old_session_is_cancelled() {
        let mut machine = DeploymentMachine::stable(40);
        assert!(machine.set_active_sessions(2));
        assert_eq!(machine.prepare(41), DeploymentAction::CancelSessions);
        assert_eq!(machine.sessions_drained(), DeploymentAction::None);
        assert_eq!(machine.phase, DeploymentPhase::Draining);
        assert!(machine.session_cancelled());
        assert_eq!(machine.sessions_drained(), DeploymentAction::None);
        assert!(machine.session_cancelled());
        assert_eq!(
            machine.sessions_drained(),
            DeploymentAction::ActivateRuntime(41)
        );
        assert_eq!(machine.phase, DeploymentPhase::Activating);
    }

    #[test]
    fn seeded_deployment_fault_interleavings_preserve_d6_recovery_rules() {
        for seed in 1..=32u64 {
            let mut random = seed;
            let mut machine = DeploymentMachine::stable(1);
            let mut candidate = 2u64;
            for _ in 0..1_000 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                match random % 8 {
                    0 if machine.phase == DeploymentPhase::Stable => {
                        candidate = machine.snapshot.journal_committed.saturating_add(1);
                        machine.prepare(candidate);
                    }
                    1 => {
                        machine.set_active_sessions((random % 3) as u32);
                    }
                    2 => {
                        machine.session_cancelled();
                    }
                    3 => {
                        machine.sessions_drained();
                    }
                    4 => {
                        machine.runtime_activated(candidate);
                    }
                    5 => {
                        machine.commit_persisted(candidate);
                    }
                    6 => {
                        machine.persist_failed();
                    }
                    _ => {
                        if machine.phase == DeploymentPhase::RollingBack {
                            let restore = machine.snapshot.journal_committed;
                            machine.rollback_restored(if random & 1 == 0 {
                                restore
                            } else {
                                restore + 99
                            });
                        } else {
                            machine.recover_after_crash(random & 1 == 0);
                        }
                    }
                }
                assert_eq!(
                    machine.sessions_may_start(),
                    machine.phase == DeploymentPhase::Stable,
                    "seed={seed}"
                );
                if machine.phase == DeploymentPhase::Stable {
                    assert_eq!(
                        machine.snapshot.active_pointer, machine.snapshot.journal_committed,
                        "seed={seed}"
                    );
                    assert_eq!(
                        machine.snapshot.broker_runtime,
                        Some(machine.snapshot.journal_committed),
                        "seed={seed}"
                    );
                }
                if machine.phase == DeploymentPhase::Passthrough {
                    assert_eq!(machine.snapshot.broker_runtime, None, "seed={seed}");
                }
            }
            assert!(
                machine.invariants_hold(),
                "D6 phase invariant violated for seed={seed}, phase={:?}, snapshot={:?}",
                machine.phase,
                machine.snapshot
            );
        }
    }
}
