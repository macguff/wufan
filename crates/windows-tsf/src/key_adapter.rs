//! Synchronous, fail-open bridge from TSF key callbacks into RuntimeCore.
//!
//! TSF invokes key callbacks on its apartment thread. Keep this lane local to
//! that thread: no mutex, IPC, disk access, logging, or COM work is performed.

use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use ime_pinyin_engine::{EngineKey, EngineReply, PinyinEngine};
use ime_protocol::ClientInstanceId;
use ime_runtime_core::{
    reduce, EatDecision, Event, ImmediateReply, KeyObservation, KeyPhase, RuntimeState,
};
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, WPARAM};

const MOD_SHIFT: u16 = 1 << 0;
const MOD_CONTROL: u16 = 1 << 1;
const MOD_ALT: u16 = 1 << 2;

thread_local! {
    // No Broker generation/session is installed by the probe, so RuntimeCore
    // deliberately remains fail-open; the separate local pinyin path handles
    // only its explicitly recognized Chinese-mode keys.
    static STATE: RefCell<RuntimeState> = const {
        RefCell::new(RuntimeState::new(ClientInstanceId(1)))
    };
    static CALLBACK_SEQUENCE: Cell<u64> = const { Cell::new(0) };
    static STARTED_AT: Instant = Instant::now();
    static MODIFIERS: Cell<u16> = const { Cell::new(0) };
    static PINYIN: RefCell<PinyinEngine> = RefCell::new(PinyinEngine::new());
}

pub(super) struct KeyCallbackOutcome {
    pub handled: BOOL,
    pub engine_reply: Option<EngineReply>,
}

#[derive(Clone, Copy)]
enum CallbackKind {
    TestDown,
    ActualDown,
    TestUp,
    ActualUp,
}

impl CallbackKind {
    fn phase(self) -> KeyPhase {
        match self {
            Self::TestDown | Self::TestUp => KeyPhase::Test,
            Self::ActualDown | Self::ActualUp => KeyPhase::Actual,
        }
    }

    fn is_key_up(self) -> bool {
        matches!(self, Self::TestUp | Self::ActualUp)
    }
}

pub(super) fn test_key_down(wparam: WPARAM, lparam: LPARAM, chinese_mode: bool) -> BOOL {
    catch_unwind(AssertUnwindSafe(|| {
        if chinese_mode
            && no_control_alt()
            && engine_key(wparam.0 as u16).is_some_and(engine_key_eligible)
        {
            return BOOL(1);
        }
        dispatch(CallbackKind::TestDown, wparam, lparam)
    }))
    .unwrap_or(BOOL(0))
}

pub(super) fn key_down(wparam: WPARAM, lparam: LPARAM, chinese_mode: bool) -> KeyCallbackOutcome {
    catch_unwind(AssertUnwindSafe(|| {
        key_down_inner(wparam, lparam, chinese_mode)
    }))
    .unwrap_or(KeyCallbackOutcome {
        handled: BOOL(0),
        engine_reply: None,
    })
}

fn key_down_inner(wparam: WPARAM, lparam: LPARAM, chinese_mode: bool) -> KeyCallbackOutcome {
    let virtual_key = wparam.0 as u16;
    if chinese_mode && no_control_alt() {
        if let Some(key) = engine_key(virtual_key) {
            if engine_key_eligible(key) {
                if let Some(engine_reply) = PINYIN.with(|engine| {
                    engine
                        .try_borrow_mut()
                        .ok()
                        .map(|mut engine| engine.handle(key))
                }) {
                    if engine_reply.consumed {
                        return KeyCallbackOutcome {
                            handled: BOOL(1),
                            engine_reply: Some(engine_reply),
                        };
                    }
                }
            }
        }
    }
    KeyCallbackOutcome {
        handled: dispatch(CallbackKind::ActualDown, wparam, lparam),
        engine_reply: None,
    }
}

pub(super) fn test_key_up(wparam: WPARAM, lparam: LPARAM) -> BOOL {
    dispatch(CallbackKind::TestUp, wparam, lparam)
}

pub(super) fn key_up(wparam: WPARAM, lparam: LPARAM) -> BOOL {
    dispatch(CallbackKind::ActualUp, wparam, lparam)
}

fn no_control_alt() -> bool {
    MODIFIERS.with(|modifiers| modifiers.get() & (MOD_CONTROL | MOD_ALT) == 0)
}

fn engine_key(virtual_key: u16) -> Option<EngineKey> {
    match virtual_key {
        0x41..=0x5A => Some(EngineKey::Character(char::from_u32(virtual_key as u32)?)),
        0x08 => Some(EngineKey::Backspace),
        0x20 => Some(EngineKey::Space),
        0x0D => Some(EngineKey::Enter),
        0x1B => Some(EngineKey::Escape),
        0x31..=0x39 => Some(EngineKey::Candidate((virtual_key - 0x31) as usize)),
        0x30 => Some(EngineKey::Candidate(9)),
        _ => None,
    }
}

fn engine_key_eligible(key: EngineKey) -> bool {
    match key {
        EngineKey::Character(_) => true,
        _ => PINYIN.with(|engine| {
            engine
                .try_borrow()
                .map(|engine| engine.has_composition())
                .unwrap_or(false)
        }),
    }
}

pub(super) fn reset_pinyin() -> Option<EngineReply> {
    PINYIN.with(|engine| {
        engine
            .try_borrow_mut()
            .ok()
            .and_then(|mut engine| engine.has_composition().then(|| engine.reset()))
    })
}

fn dispatch(kind: CallbackKind, wparam: WPARAM, lparam: LPARAM) -> BOOL {
    // Keep Rust unwinding inside the implementation boundary. Release builds
    // use panic=abort, so this primarily protects unwind-enabled dev builds.
    catch_unwind(AssertUnwindSafe(|| dispatch_inner(kind, wparam, lparam))).unwrap_or(BOOL(0))
}

fn dispatch_inner(kind: CallbackKind, wparam: WPARAM, lparam: LPARAM) -> BOOL {
    let packed = lparam.0 as u32;
    let key_up = kind.is_key_up();
    let virtual_key = wparam.0 as u16;
    let modifiers = MODIFIERS.with(Cell::get);
    let key = KeyObservation {
        token: CALLBACK_SEQUENCE.with(|sequence| {
            let next = sequence.get().saturating_add(1);
            sequence.set(next);
            next
        }),
        virtual_key,
        // LPARAM bits 16..23 carry the hardware scan code. Preserve the
        // extended-key bit in the upper bit of this field to avoid collisions.
        scan_code: (((packed >> 16) & 0xff) as u16) | ((((packed >> 24) & 1) as u16) << 8),
        modifiers,
        repeat: (packed & 0xffff) > 1 || (packed & (1 << 30)) != 0,
        now_ms: STARTED_AT
            .with(|started| started.elapsed().as_millis().min(u64::MAX as u128) as u64),
    };

    let reply = STATE
        .with(|state| {
            // Reentrancy is not expected from the reducer, but never let a
            // RefCell borrow panic escape a COM callback: the caller maps None to
            // Pass below.
            let Ok(mut state) = state.try_borrow_mut() else {
                return None;
            };
            let event = if key_up {
                Event::HostKeyUp {
                    phase: kind.phase(),
                    key,
                }
            } else {
                Event::HostKey {
                    phase: kind.phase(),
                    key,
                }
            };
            let transition = reduce(*state, event);
            let immediate = transition.immediate;
            *state = transition.next_state;
            Some(immediate)
        })
        .flatten();

    if matches!(kind, CallbackKind::ActualDown | CallbackKind::ActualUp) {
        update_modifier_state(virtual_key, key_up);
    }

    // A mismatched/missing RuntimeCore reply is always fail-open at the TSF
    // boundary, so an adapter bug cannot suppress host text input.
    let decision = match (kind, reply) {
        (CallbackKind::TestDown, Some(ImmediateReply::TestKey(decision)))
        | (CallbackKind::ActualDown, Some(ImmediateReply::Key(decision)))
        | (CallbackKind::TestUp, Some(ImmediateReply::TestKeyUp(decision)))
        | (CallbackKind::ActualUp, Some(ImmediateReply::KeyUp(decision))) => decision,
        _ => EatDecision::Pass,
    };
    BOOL(if decision == EatDecision::Eat { 1 } else { 0 })
}

fn update_modifier_state(virtual_key: u16, key_up: bool) {
    let modifier = match virtual_key {
        0x10 | 0xA0 | 0xA1 => MOD_SHIFT,
        0x11 | 0xA2 | 0xA3 => MOD_CONTROL,
        0x12 | 0xA4 | 0xA5 => MOD_ALT,
        _ => return,
    };
    MODIFIERS.with(|modifiers| {
        let state = modifiers.get();
        modifiers.set(if key_up {
            state & !modifier
        } else {
            state | modifier
        });
    });
}
