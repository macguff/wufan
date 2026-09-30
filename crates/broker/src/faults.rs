//! Explicit, one-shot acceptance faults. Ordinary builds reject the switches.
use ime_protocol::wire::{Reply, Request};

#[cfg(not(feature = "fault-injection"))]
pub(super) struct Faults;
#[cfg(not(feature = "fault-injection"))]
impl Faults {
    pub(super) fn from_args(args: &[String]) -> Result<Self, String> {
        if args.iter().any(|arg| arg.starts_with("--dev-fault")) {
            return Err("fault injection is not enabled in this build".into());
        }
        Ok(Self)
    }
    pub(super) async fn before_reply(&self, _: &Request, _: &Reply) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(feature = "fault-injection")]
pub(super) struct Faults {
    stage: Option<String>,
    directory: std::path::PathBuf,
    fired: std::sync::atomic::AtomicBool,
}
#[cfg(feature = "fault-injection")]
impl Faults {
    pub(super) fn from_args(args: &[String]) -> Result<Self, String> {
        let value = |flag: &str| -> Result<Option<&str>, String> {
            let positions: Vec<_> = args
                .iter()
                .enumerate()
                .filter(|(_, a)| *a == flag)
                .collect();
            if positions.len() > 1 {
                return Err(format!("duplicate {flag}"));
            }
            positions
                .first()
                .map(|(index, _)| {
                    args.get(index + 1)
                        .map(String::as_str)
                        .ok_or_else(|| format!("missing {flag} value"))
                })
                .transpose()
        };
        let stage = value("--dev-fault")?;
        let directory = value("--dev-fault-dir")?;
        if stage.is_some() != directory.is_some() {
            return Err("--dev-fault and --dev-fault-dir must be provided together".into());
        }
        if let Some(stage) = stage {
            if !args.iter().any(|a| a == "--dev-client") {
                return Err("fault injection requires an explicit development client".into());
            }
            if !matches!(stage, "key-reply-stall" | "commit-ack-stall") {
                return Err("unknown fault stage".into());
            }
        }
        let directory = directory
            .map(std::fs::canonicalize)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        if stage.is_some() && !directory.is_dir() {
            return Err("fault directory must already exist".into());
        }
        Ok(Self {
            stage: stage.map(str::to_owned),
            directory,
            fired: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub(super) async fn before_reply(
        &self,
        request: &Request,
        reply: &Reply,
    ) -> Result<(), String> {
        use ime_protocol::wire::{Action, ReplyResult};
        use std::sync::atomic::Ordering;
        let Some(stage) = self.stage.as_deref() else {
            return Ok(());
        };
        let matches = match stage {
            "key-reply-stall" => {
                matches!(request.action, Action::Key { .. })
                    && matches!(reply.result, ReplyResult::View { .. })
            }
            "commit-ack-stall" => {
                matches!(request.action, Action::CommitAck { .. })
                    && matches!(reply.result, ReplyResult::Acknowledged)
            }
            _ => false,
        };
        if matches && !self.fired.swap(true, Ordering::AcqRel) {
            // Marker proves the real engine/ACK reducer ran before the stall.
            // Never record text or user dictionary contents.
            std::fs::write(self.directory.join(format!("{stage}.entered")), b"ENTERED")
                .map_err(|e| e.to_string())?;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
        Ok(())
    }
}
