//! Model-only Broker authentication policy. Facts here represent OS-observed identity,
//! never claims supplied inside an IPC payload.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerObservation {
    pub sid: u64,
    pub logon_session: u64,
    pub signer: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthPolicy {
    pub expected_sid: u64,
    pub expected_logon_session: u64,
    pub pinned_signer: u64,
    pub dev_trust_mode: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthDecision {
    Official,
    Development,
    Rejected,
}

impl AuthPolicy {
    pub fn authenticate(self, peer: PeerObservation) -> AuthDecision {
        if peer.sid != self.expected_sid || peer.logon_session != self.expected_logon_session {
            return AuthDecision::Rejected;
        }
        if peer.signer == self.pinned_signer {
            AuthDecision::Official
        } else if self.dev_trust_mode {
            AuthDecision::Development
        } else {
            AuthDecision::Rejected
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(dev_trust_mode: bool) -> AuthPolicy {
        AuthPolicy {
            expected_sid: 10,
            expected_logon_session: 20,
            pinned_signer: 30,
            dev_trust_mode,
        }
    }

    #[test]
    fn same_sid_untrusted_client_is_not_a_broker() {
        let peer = PeerObservation {
            sid: 10,
            logon_session: 20,
            signer: 99,
        };
        assert_eq!(policy(false).authenticate(peer), AuthDecision::Rejected);
    }

    #[test]
    fn developer_build_requires_explicit_dev_trust_mode() {
        let peer = PeerObservation {
            sid: 10,
            logon_session: 20,
            signer: 99,
        };
        assert_eq!(policy(false).authenticate(peer), AuthDecision::Rejected);
        assert_eq!(policy(true).authenticate(peer), AuthDecision::Development);
    }

    #[test]
    fn official_signer_and_logon_context_are_both_checked() {
        let official = PeerObservation {
            sid: 10,
            logon_session: 20,
            signer: 30,
        };
        assert_eq!(policy(false).authenticate(official), AuthDecision::Official);
        assert_eq!(
            policy(true).authenticate(PeerObservation {
                logon_session: 21,
                ..official
            }),
            AuthDecision::Rejected
        );
    }
}
