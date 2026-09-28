//! Stale host geometry must never be used to position a candidate view.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeometrySnapshot {
    pub focus_epoch: u64,
    pub revision: u64,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidatePlacement {
    pub focus_epoch: u64,
    pub geometry_revision: u64,
    pub visible: bool,
}

impl CandidatePlacement {
    pub fn apply_geometry(&mut self, current_focus_epoch: u64, geometry: GeometrySnapshot) -> bool {
        if geometry.focus_epoch != current_focus_epoch
            || geometry.focus_epoch != self.focus_epoch
            || geometry.revision != self.geometry_revision
        {
            self.visible = false;
            return false;
        }
        self.visible = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_focus_or_geometry_revision_never_repositions_current_candidate() {
        let mut placement = CandidatePlacement {
            focus_epoch: 8,
            geometry_revision: 4,
            visible: true,
        };
        let stale = GeometrySnapshot {
            focus_epoch: 7,
            revision: 4,
            x: 10,
            y: 20,
        };
        assert!(!placement.apply_geometry(8, stale));
        assert!(!placement.visible);

        placement.visible = true;
        let old_revision = GeometrySnapshot {
            focus_epoch: 8,
            revision: 3,
            x: 10,
            y: 20,
        };
        assert!(!placement.apply_geometry(8, old_revision));
        assert!(!placement.visible);

        let current = GeometrySnapshot {
            focus_epoch: 8,
            revision: 4,
            x: 10,
            y: 20,
        };
        assert!(placement.apply_geometry(8, current));
        assert!(placement.visible);
    }
}
