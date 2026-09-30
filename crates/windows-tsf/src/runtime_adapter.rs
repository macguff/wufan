//! One apartment owner for actual keys, requests, preedit and commit results.
use crate::broker_client::ViewReceipt;
use ime_pinyin_engine::{EngineReply, Preedit};
use ime_protocol::{EffectId, HostRevision, MessageIdentity};
use ime_runtime_core::{
    input_lifecycle::{self, Action, Event, State, Transition},
    CommitStatus, Effect, EffectOutcome, EffectScope,
};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Coordinator {
    state: State,
    revision: u64,
}
impl Coordinator {
    fn event(&mut self, event: Event) -> Transition {
        let transition = input_lifecycle::reduce(self.state, event);
        self.state = transition.state;
        transition
    }
}
#[derive(Clone, Default)]
pub(super) struct RuntimeOwner(Rc<RefCell<Coordinator>>);
impl RuntimeOwner {
    pub(super) fn probe_state(&self) -> Option<State> {
        self.0.try_borrow().ok().map(|owner| owner.state)
    }
    pub(super) fn event(&self, event: Event) -> Transition {
        self.0.borrow_mut().event(event)
    }
    pub(super) fn invalidate(&self) {
        self.event(Event::Invalidated);
    }
    pub(super) fn admit(
        &self,
        mut receipt: ViewReceipt,
        ticket: u64,
        reply: &EngineReply,
    ) -> Option<HostEditGuard> {
        if receipt.commit_id().is_some() != reply.commit.is_some() {
            return None;
        }
        let identity = receipt.identity();
        let action = self
            .event(Event::View {
                identity,
                ticket,
                visible: matches!(reply.preedit, Preedit::Show(_)),
                commit_id: receipt.commit_id(),
            })
            .action;
        let kind = match action {
            Action::ApplyPreedit { .. } => Operation::Preedit,
            Action::ApplyCommit(Effect::ApplyHostCommit {
                effect_id,
                client_instance_id,
                session,
                epoch,
                commit_id,
                ..
            }) => Operation::Commit {
                effect_id,
                scope: EffectScope::Commit {
                    client_instance_id,
                    session,
                    epoch,
                    commit_id,
                },
            },
            Action::Pending => {
                receipt.disarm();
                return None;
            }
            _ => return None,
        };
        Some(HostEditGuard {
            owner: self.clone(),
            receipt: Some(receipt),
            identity,
            ticket,
            kind,
            outcome: EffectOutcome::Rejected,
            revision: None,
        })
    }
}
enum Operation {
    Preedit,
    Commit {
        effect_id: EffectId,
        scope: EffectScope,
    },
}
pub(super) struct HostEditGuard {
    owner: RuntimeOwner,
    receipt: Option<ViewReceipt>,
    identity: MessageIdentity,
    ticket: u64,
    kind: Operation,
    outcome: EffectOutcome,
    revision: Option<HostRevision>,
}
impl HostEditGuard {
    pub(super) fn attempting(&mut self) {
        self.outcome = EffectOutcome::Indeterminate;
    }
    pub(super) fn applied(&mut self) {
        if matches!(self.kind, Operation::Preedit) {
            self.outcome = EffectOutcome::Succeeded;
            return;
        }
        let mut coordinator = self.owner.0.borrow_mut();
        if let Some(revision) = coordinator.revision.checked_add(1) {
            coordinator.revision = revision;
            self.revision = Some(HostRevision(revision));
            self.outcome = EffectOutcome::Succeeded;
        }
    }
}
impl Drop for HostEditGuard {
    fn drop(&mut self) {
        let Some(mut receipt) = self.receipt.take() else {
            return;
        };
        let event = match self.kind {
            Operation::Preedit => Event::PreeditResult {
                identity: self.identity,
                ticket: self.ticket,
                outcome: self.outcome,
            },
            Operation::Commit { effect_id, scope } => Event::CommitResult {
                effect_id,
                scope,
                outcome: self.outcome,
                host_revision: self.revision,
            },
        };
        match self.owner.event(event).action {
            Action::CompleteView {
                identity,
                ticket,
                status,
            } if identity == self.identity
                && (ticket == self.ticket
                    || (ticket == 0 && matches!(self.kind, Operation::Commit { .. }))) =>
            {
                receipt.complete(status)
            }
            _ => receipt.complete(if self.outcome == EffectOutcome::Rejected {
                CommitStatus::Rejected
            } else {
                CommitStatus::Indeterminate
            }),
        }
        // Only report the result. Finished from the worker drives the next key.
    }
}
