//! Apartment message pump: bounded try_recv only, never waits for IPC.
use crate::{composition::CompositionController, windows_impl::ComLifetime};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use windows::{
    core::{Result, PCWSTR},
    Win32::{Foundation::*, System::LibraryLoader::*, UI::WindowsAndMessaging::*},
};
static NEXT_WINDOW: AtomicU64 = AtomicU64::new(1);
struct WindowState {
    controller: CompositionController,
    client_id: u32,
    _lifetime: ComLifetime,
}
pub(super) struct BrokerWindow {
    window: HWND,
    class: Vec<u16>,
    module: HMODULE,
}
impl BrokerWindow {
    pub(super) fn new(controller: CompositionController, client_id: u32) -> Result<Self> {
        let mut module = HMODULE::default();
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                PCWSTR(window_proc as *const () as *const u16),
                &mut module,
            )?;
        }
        let class: Vec<u16> = format!(
            "Wufan.BrokerPump.{}",
            NEXT_WINDOW.fetch_add(1, Ordering::Relaxed)
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let descriptor = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&descriptor) } == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let window = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(HINSTANCE(module.0)),
                None,
            )
        } {
            Ok(window) => window,
            Err(error) => {
                unsafe {
                    let _ = UnregisterClassW(PCWSTR(class.as_ptr()), Some(HINSTANCE(module.0)));
                }
                return Err(error);
            }
        };
        let state = Box::new(WindowState {
            controller,
            client_id,
            _lifetime: ComLifetime::new(),
        });
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, Box::into_raw(state) as isize);
        }
        let pump = Self {
            window,
            class,
            module,
        };
        if unsafe { SetTimer(Some(window), 1, 10, None) } == 0 {
            return Err(windows::core::Error::from_thread());
        }
        Ok(pump)
    }
}
impl Drop for BrokerWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.window), 1);
            let _ = DestroyWindow(self.window);
            let _ = UnregisterClassW(PCWSTR(self.class.as_ptr()), Some(HINSTANCE(self.module.0)));
        }
    }
}
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Local acceptance probe: readiness=bit 0, idle=bit 1. Query only;
    // it never pumps events, touches COM or performs an input operation.
    if (WM_APP + 0x574..=WM_APP + 0x577).contains(&message) {
        return LRESULT(
            catch_unwind(AssertUnwindSafe(|| {
                let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut WindowState;
                if state.is_null() {
                    0
                } else if message == WM_APP + 0x575 {
                    (*state).controller.keys.probe_accepted() as isize
                } else if message == WM_APP + 0x576 {
                    (*state).controller.keys.probe_repeated() as isize
                } else if message == WM_APP + 0x577 {
                    (*state).controller.keys.broker_epoch() as isize
                } else {
                    (*state).controller.keys.probe_flags() as isize
                }
            }))
            .unwrap_or(0),
        );
    }
    if message == WM_TIMER {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut WindowState;
            if !state.is_null() {
                // Retain before calling COM: a reentrant Deactivate may destroy the window.
                let controller = (*state).controller.clone();
                let client_id = (*state).client_id;
                controller.poll_broker(client_id);
            }
        }));
        return LRESULT(0);
    }
    if message == WM_NCDESTROY {
        let state = SetWindowLongPtrW(window, GWLP_USERDATA, 0) as *mut WindowState;
        if !state.is_null() {
            drop(Box::from_raw(state));
        }
    }
    DefWindowProcW(window, message, wparam, lparam)
}
