#![forbid(unsafe_code)]

//! Platform-independent protocol identities shared by the runtime and adapters.

use core::fmt;

pub mod wire;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            serde::Serialize,
            serde::Deserialize,
        )]
        pub struct $name(pub u64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(ClientInstanceId);
id_type!(BrokerGeneration);
id_type!(SessionId);
id_type!(EffectId);
id_type!(TimerId);
id_type!(CommitId);
id_type!(DeploymentId);
id_type!(RequestSeq);
id_type!(HostRevision);

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Epoch(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageIdentity {
    pub client_instance_id: ClientInstanceId,
    pub broker_generation: BrokerGeneration,
    pub session_id: SessionId,
    pub focus_epoch: Epoch,
    pub composition_epoch: Epoch,
    pub request_seq: RequestSeq,
}

/// Stable wire reason codes. Values are append-only and must not be renumbered.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u16)]
pub enum ReasonCode {
    Unknown = 0,
    StaleIdentity = 1,
    InvalidState = 2,
    EffectMismatch = 3,
    BrokerUnavailable = 4,
    QueueCapacityExceeded = 5,
}
