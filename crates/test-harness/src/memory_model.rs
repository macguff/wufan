//! Independent, deterministic EventDot replication model for Slice 0.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EventDot {
    pub replica_id: u64,
    pub seq: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LearnDelta {
    pub item_id: u64,
    pub amount: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    pub events: BTreeMap<EventDot, LearnDelta>,
    pub version_vector: BTreeMap<u64, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryReplica {
    replica_id: u64,
    snapshot: Snapshot,
    applied: BTreeSet<EventDot>,
    totals: BTreeMap<u64, i64>,
    application_count: BTreeMap<EventDot, u32>,
}

impl MemoryReplica {
    pub fn new(replica_id: u64) -> Self {
        Self {
            replica_id,
            snapshot: Snapshot::default(),
            applied: BTreeSet::new(),
            totals: BTreeMap::new(),
            application_count: BTreeMap::new(),
        }
    }

    pub fn record(&mut self, delta: LearnDelta) -> EventDot {
        let seq = self
            .snapshot
            .version_vector
            .get(&self.replica_id)
            .copied()
            .unwrap_or(0)
            + 1;
        let dot = EventDot {
            replica_id: self.replica_id,
            seq,
        };
        self.snapshot.events.insert(dot, delta);
        self.apply_once(dot, delta);
        self.snapshot.version_vector.insert(self.replica_id, seq);
        dot
    }

    pub fn merge(&mut self, incoming: &Snapshot) -> usize {
        let mut applied = 0;
        for (dot, delta) in &incoming.events {
            let canonical_delta = match self.snapshot.events.get(dot).copied() {
                Some(existing) if existing != *delta => continue,
                Some(existing) => existing,
                None => {
                    self.snapshot.events.insert(*dot, *delta);
                    *delta
                }
            };
            if self.apply_once(*dot, canonical_delta) {
                applied += 1;
            }
        }
        for (replica_id, seq) in &incoming.version_vector {
            let current = self.snapshot.version_vector.entry(*replica_id).or_default();
            *current = (*current).max(*seq);
        }
        applied
    }

    fn apply_once(&mut self, dot: EventDot, delta: LearnDelta) -> bool {
        if !self.applied.insert(dot) {
            return false;
        }
        *self.totals.entry(delta.item_id).or_default() += delta.amount;
        *self.application_count.entry(dot).or_default() += 1;
        true
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }
    pub fn total(&self, item_id: u64) -> i64 {
        self.totals.get(&item_id).copied().unwrap_or(0)
    }
    pub fn application_count(&self, dot: EventDot) -> u32 {
        self.application_count.get(&dot).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_b_c_a_and_repeated_import_apply_each_dot_once() {
        let mut a = MemoryReplica::new(1);
        let dot_a = a.record(LearnDelta {
            item_id: 10,
            amount: 1,
        });

        let mut b = MemoryReplica::new(2);
        assert_eq!(b.merge(&a.snapshot()), 1);
        let dot_b = b.record(LearnDelta {
            item_id: 10,
            amount: 2,
        });

        let mut c = MemoryReplica::new(3);
        assert_eq!(c.merge(&b.snapshot()), 2);
        let dot_c = c.record(LearnDelta {
            item_id: 10,
            amount: 3,
        });

        let c_snapshot = c.snapshot();
        assert_eq!(a.merge(&c_snapshot), 2);
        assert_eq!(a.merge(&c_snapshot), 0);
        assert_eq!(b.merge(&c_snapshot), 1);
        assert_eq!(b.merge(&c_snapshot), 0);

        for replica in [&a, &b, &c] {
            assert_eq!(replica.total(10), 6);
            assert_eq!(replica.application_count(dot_a), 1);
            assert_eq!(replica.application_count(dot_b), 1);
            assert_eq!(replica.application_count(dot_c), 1);
        }
    }

    #[test]
    fn seeded_duplicate_and_reordered_snapshot_merges_are_idempotent() {
        for seed in 1..=32u64 {
            let mut random = seed;
            let mut replicas = [
                MemoryReplica::new(1),
                MemoryReplica::new(2),
                MemoryReplica::new(3),
            ];
            for _ in 0..1_000 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                let target = (random as usize) % 3;
                if random & 3 == 0 {
                    replicas[target].record(LearnDelta {
                        item_id: (random >> 8) % 7,
                        amount: 1,
                    });
                } else {
                    let source = ((random >> 4) as usize) % 3;
                    let snapshot = replicas[source].snapshot();
                    replicas[target].merge(&snapshot);
                    replicas[target].merge(&snapshot);
                }
                for replica in &replicas {
                    let snapshot = replica.snapshot();
                    let mut expected = BTreeMap::<u64, i64>::new();
                    for (dot, delta) in &snapshot.events {
                        *expected.entry(delta.item_id).or_default() += delta.amount;
                        assert_eq!(
                            replica.application_count(*dot),
                            1,
                            "seed={seed}, dot={dot:?}"
                        );
                    }
                    for item_id in 0..7 {
                        assert_eq!(
                            replica.total(item_id),
                            expected.get(&item_id).copied().unwrap_or(0),
                            "seed={seed}"
                        );
                    }
                }
            }
        }
    }
}
