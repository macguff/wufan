//! Nonblocking TSF key adapter to the single RuntimeCore input lifecycle.
use crate::{
    broker_client::{BrokerClient, WorkerEvent},
    runtime_adapter::RuntimeOwner,
};
use ime_runtime_core::{
    input_lifecycle::{Action, Event},
    EatDecision, KeyObservation,
};
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;

pub(super) struct KeyAdapter {
    pub(super) runtime: RuntimeOwner,
    pub(super) broker: RefCell<Option<BrokerClient>>,
    started_at: Instant,
}
pub(super) struct KeyCallbackOutcome {
    pub handled: BOOL,
    pub reset: bool,
}
impl KeyAdapter {
    pub(super) fn probe_repeated(&self) -> u64 {
        self.broker
            .try_borrow()
            .ok()
            .and_then(|broker| broker.as_ref().map(BrokerClient::repeated_count))
            .unwrap_or(0)
    }
    pub(super) fn probe_accepted(&self) -> u64 {
        self.broker
            .try_borrow()
            .ok()
            .and_then(|broker| broker.as_ref().map(BrokerClient::accepted_count))
            .unwrap_or(0)
    }
    /// Read-only apartment probe; no COM calls, worker waits or reducer events.
    pub(super) fn probe_flags(&self) -> u32 {
        let Some(state) = self.runtime.probe_state() else {
            return 0;
        };
        let ready = state.is_active()
            && self
                .broker
                .try_borrow()
                .ok()
                .is_some_and(|broker| broker.as_ref().is_some_and(BrokerClient::ready));
        u32::from(ready) | (u32::from(state.is_idle()) << 1)
    }
    pub(super) fn new() -> Self {
        Self {
            runtime: RuntimeOwner::default(),
            broker: RefCell::new(None),
            started_at: Instant::now(),
        }
    }
    fn observation(&self, wparam: WPARAM, lparam: LPARAM) -> KeyObservation {
        let packed = lparam.0 as u32;
        KeyObservation {
            token: 0,
            virtual_key: wparam.0 as u16,
            scan_code: (((packed >> 16) & 0xff) as u16) | ((((packed >> 24) & 1) as u16) << 8),
            modifiers: current_modifiers(),
            repeat: (packed & 0xffff) > 1 || packed & (1 << 30) != 0,
            now_ms: self.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
        }
    }
    fn available(&self, chinese: bool) -> bool {
        chinese
            && self
                .broker
                .borrow()
                .as_ref()
                .is_some_and(BrokerClient::ready)
    }
    pub(super) fn test_key_down(&self, wparam: WPARAM, lparam: LPARAM, chinese: bool) -> BOOL {
        catch_unwind(AssertUnwindSafe(|| {
            if let Some(broker) = self.broker.borrow().as_ref() {
                broker.count(0);
                broker.feature(6, ((wparam.0 as u64) << 32) | lparam.0 as u32 as u64);
            }
            let transition = self.runtime.event(Event::Test {
                key: self.observation(wparam, lparam),
                available: self.available(chinese),
            });
            if let Some(broker) = self.broker.borrow().as_ref() {
                broker.feature(1, u64::from(transition.decision == EatDecision::Eat));
            }
            BOOL(i32::from(transition.decision == EatDecision::Eat))
        }))
        .unwrap_or(BOOL(0))
    }
    pub(super) fn key_down(
        &self,
        wparam: WPARAM,
        lparam: LPARAM,
        chinese: bool,
    ) -> KeyCallbackOutcome {
        catch_unwind(AssertUnwindSafe(|| {
            if let Some(broker) = self.broker.borrow().as_ref() {
                broker.count(2);
                broker.feature(7, ((wparam.0 as u64) << 32) | lparam.0 as u32 as u64);
            }
            let key = self.observation(wparam, lparam);
            let transition = self.runtime.event(Event::Actual {
                key,
                available: self.available(chinese),
            });
            let accepted = transition.decision == EatDecision::Eat;
            let sent = self.execute(transition.action);
            if accepted && sent {
                if let Some(broker) = self.broker.borrow().as_ref() {
                    broker.count(3);
                    broker.count(4);
                    if key.repeat {
                        broker.count(11);
                    }
                }
            }
            KeyCallbackOutcome {
                handled: BOOL(i32::from(accepted && sent)),
                reset: !sent,
            }
        }))
        .unwrap_or(KeyCallbackOutcome {
            handled: BOOL(0),
            reset: true,
        })
    }
    // Engine release events are not handled by this pinyin schema. Reading the
    // apartment modifier state on each down event avoids retaining stuck bits.
    pub(super) fn test_key_up(&self, _: WPARAM, _: LPARAM) -> BOOL {
        BOOL(0)
    }
    pub(super) fn key_up(&self, _: WPARAM, _: LPARAM) -> BOOL {
        BOOL(0)
    }
    pub(super) fn execute(&self, action: Action) -> bool {
        let succeeded = match action {
            Action::SendKey {
                identity,
                ticket,
                code,
            } => self
                .broker
                .borrow()
                .as_ref()
                .is_some_and(|broker| broker.submit(identity, ticket, code)),
            Action::Reset => false,
            _ => true,
        };
        if !succeeded {
            self.reset_input();
        }
        succeeded
    }
    pub(super) fn reset_input(&self) {
        self.runtime.invalidate();
        if let Some(broker) = self.broker.borrow().as_ref() {
            broker.count(5);
            broker.reset();
        }
    }
    pub(super) fn start_broker(&self) -> windows::core::Result<()> {
        crate::windows_impl::pin_for_worker()?;
        let executable = std::env::var_os("WUFAN_BROKER_EXE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(crate::windows_impl::dll_path().unwrap_or_default())
                    .with_file_name("ime-broker.exe")
            });
        let executable = executable
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase();
        let broker = BrokerClient::start(executable)
            .map_err(|_| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        *self.broker.borrow_mut() = Some(broker);
        Ok(())
    }
    pub(super) fn stop_broker(&self) {
        self.runtime.invalidate();
        self.broker.borrow_mut().take();
    }
    pub(super) fn poll_broker(&self) -> Option<WorkerEvent> {
        self.broker.borrow().as_ref().and_then(BrokerClient::poll)
    }
    pub(super) fn broker_epoch(&self) -> u64 {
        self.broker
            .borrow()
            .as_ref()
            .map(BrokerClient::epoch)
            .unwrap_or(0)
    }
}
fn current_modifiers() -> u16 {
    [(0x10, 1), (0x11, 2), (0x12, 4)]
        .into_iter()
        .fold(0, |mask, (key, flag)| {
            // SAFETY: GetKeyState accepts a virtual key and no pointer.
            mask | if unsafe { GetKeyState(key) } < 0 {
                flag
            } else {
                0
            }
        })
}
