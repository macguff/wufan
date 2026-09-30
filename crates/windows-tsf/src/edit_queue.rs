//! One outstanding TSF write session per context, with bounded FIFO edits.

use std::collections::VecDeque;

const CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EditTicket(u64);

pub(super) struct EditQueue<T> {
    pending: VecDeque<T>,
    scheduled: Option<EditTicket>,
    next_ticket: u64,
}

impl<T> Default for EditQueue<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::with_capacity(CAPACITY),
            scheduled: None,
            next_ticket: 1,
        }
    }
}

impl<T> EditQueue<T> {
    /// A returned ticket requests a new TSF session. None reuses the queued one.
    pub(super) fn push(&mut self, edit: T) -> Result<Option<EditTicket>, ()> {
        if self.pending.len() == CAPACITY {
            return Err(());
        }
        self.pending.push_back(edit);
        if self.scheduled.is_some() {
            return Ok(None);
        }
        let ticket = EditTicket(self.next_ticket);
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .expect("edit ticket exhausted");
        self.scheduled = Some(ticket);
        Ok(Some(ticket))
    }

    pub(super) fn pop(&mut self, ticket: EditTicket) -> Option<T> {
        if self.scheduled != Some(ticket) {
            return None;
        }
        let next = self.pending.pop_front();
        if next.is_none() {
            self.scheduled = None;
        }
        next
    }

    pub(super) fn is_current(&self, ticket: EditTicket) -> bool {
        self.scheduled == Some(ticket)
    }

    pub(super) fn reject(&mut self, ticket: EditTicket) {
        if self.is_current(ticket) {
            self.invalidate();
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.pending.clear();
        self.scheduled = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preedit_commit_and_new_preedit_keep_fifo_order() {
        let mut queue = EditQueue::default();
        let ticket = queue.push("preedit").unwrap().unwrap();
        assert_eq!(queue.push("commit"), Ok(None));
        assert_eq!(queue.push("new preedit"), Ok(None));
        assert_eq!(queue.pop(ticket), Some("preedit"));
        assert_eq!(queue.pop(ticket), Some("commit"));
        assert_eq!(queue.pop(ticket), Some("new preedit"));
        assert_eq!(queue.pop(ticket), None);
        assert!(!queue.is_current(ticket));
    }

    #[test]
    fn old_context_session_cannot_apply_or_reject_new_context_edits() {
        let mut queue = EditQueue::default();
        let old = queue.push("old context").unwrap().unwrap();
        queue.invalidate();
        let new = queue.push("new context").unwrap().unwrap();
        assert_ne!(old, new);
        assert_eq!(queue.pop(old), None);
        queue.reject(old);
        assert_eq!(queue.pop(new), Some("new context"));
    }

    #[test]
    fn completed_ticket_cannot_drain_a_later_session() {
        let mut queue = EditQueue::default();
        let old = queue.push(1).unwrap().unwrap();
        assert_eq!(queue.pop(old), Some(1));
        assert_eq!(queue.pop(old), None);
        let new = queue.push(2).unwrap().unwrap();
        assert_ne!(old, new);
        assert_eq!(queue.pop(old), None);
        assert_eq!(queue.pop(new), Some(2));
    }

    #[test]
    fn queue_capacity_and_rejected_session_are_bounded() {
        let mut queue = EditQueue::default();
        let ticket = queue.push(0).unwrap().unwrap();
        for edit in 1..CAPACITY {
            assert_eq!(queue.push(edit), Ok(None));
        }
        assert_eq!(queue.push(CAPACITY), Err(()));
        queue.reject(ticket);
        assert_eq!(queue.pop(ticket), None);
        assert!(queue.push(99).unwrap().is_some());
    }

    #[test]
    fn reentrant_edits_share_the_active_session_and_cancel_invalidates_it() {
        let mut queue = EditQueue::default();
        let ticket = queue.push(1).unwrap().unwrap();
        assert_eq!(queue.pop(ticket), Some(1));
        assert_eq!(queue.push(2), Ok(None));
        assert_eq!(queue.pop(ticket), Some(2));
        queue.invalidate();
        assert!(!queue.is_current(ticket));
        assert_eq!(queue.pop(ticket), None);
    }
}
