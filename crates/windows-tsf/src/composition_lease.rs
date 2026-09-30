//! A terminated TSF range must never be cleared by a delayed cancel session.

use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone)]
pub(super) struct CompositionLease<T> {
    pub(super) handle: T,
    pub(super) alive: Rc<Cell<bool>>,
    pub(super) preedit_written: bool,
}

impl<T> CompositionLease<T> {
    pub(super) fn is_live(&self) -> bool {
        self.alive.get()
    }

    pub(super) fn can_clear(&self) -> bool {
        self.is_live() && self.preedit_written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_first_preedit_write_preserves_original_selection() {
        let lease = CompositionLease {
            handle: "selected host text",
            alive: Rc::new(Cell::new(true)),
            preedit_written: false,
        };
        assert_eq!(lease.handle, "selected host text");
        assert!(!lease.can_clear());
        assert!(lease.is_live());
    }

    #[test]
    fn external_termination_disables_delayed_cancel_of_old_range() {
        let old = CompositionLease {
            handle: 1,
            alive: Rc::new(Cell::new(true)),
            preedit_written: true,
        };
        let delayed_cancel = old.clone();
        assert!(delayed_cancel.can_clear());
        // The host may reuse old range anchors for a replacement composition.
        old.alive.set(false);
        assert!(!delayed_cancel.can_clear());
        assert!(!delayed_cancel.is_live());
    }

    #[test]
    fn old_termination_does_not_invalidate_the_replacement_composition() {
        let old = CompositionLease {
            handle: 1,
            alive: Rc::new(Cell::new(true)),
            preedit_written: true,
        };
        let new = CompositionLease {
            handle: 2,
            alive: Rc::new(Cell::new(true)),
            preedit_written: true,
        };
        old.alive.set(false);
        assert!(!old.can_clear());
        assert!(new.can_clear());
    }
}
