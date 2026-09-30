//! Deadline-bound Win32 overlapped I/O. Call only from an IPC worker.
use ime_protocol::wire::{decode, encode, frame_len};
use std::{
    os::windows::io::AsRawHandle,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::*,
        Storage::FileSystem::*,
        System::{Threading::*, IO::*},
    },
};
struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub struct Pipe {
    handle: Owned,
}
impl AsRawHandle for Pipe {
    fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        self.handle.0 .0
    }
}
impl Pipe {
    /// Worker-only liveness check while no reply is outstanding. The handle is
    /// overlapped; Peek does not read or wait for a frame. Extra idle data is a
    /// protocol violation because the Broker only sends solicited replies.
    pub fn check_idle(&self) -> Result<(), String> {
        let mut available = 0;
        unsafe {
            windows::Win32::System::Pipes::PeekNamedPipe(
                self.handle.0,
                None,
                0,
                None,
                Some(&mut available),
                None,
            )
            .map_err(|e| format!("idle pipe disconnected: {e}"))?;
        }
        if available != 0 {
            return Err("unsolicited idle Broker data".into());
        }
        Ok(())
    }
    pub fn open(name: &str) -> Result<Self, String> {
        let path: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                0xc0000000,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | FILE_FLAGS_AND_ATTRIBUTES(0x00110000),
                None,
            )
            .map_err(|e| e.to_string())?
        };
        Ok(Self {
            handle: Owned(handle),
        })
    }
    pub fn send<T: serde::Serialize>(&self, value: &T, stopped: &AtomicBool) -> Result<(), String> {
        let mut bytes = encode(value)?;
        self.transfer(&mut bytes, true, stopped)
    }
    pub fn receive<T: for<'a> serde::Deserialize<'a>>(
        &self,
        stopped: &AtomicBool,
    ) -> Result<T, String> {
        let mut header = [0; 4];
        self.transfer(&mut header, false, stopped)?;
        let mut body = vec![0; frame_len(header)?];
        self.transfer(&mut body, false, stopped)?;
        decode(&body)
    }
    fn transfer(
        &self,
        buffer: &mut [u8],
        writing: bool,
        stopped: &AtomicBool,
    ) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut offset = 0;
        while offset < buffer.len() {
            let event = Owned(unsafe {
                CreateEventW(None, true, false, PCWSTR::null()).map_err(|e| e.to_string())?
            });
            let mut overlapped = OVERLAPPED {
                hEvent: event.0,
                ..Default::default()
            };
            let mut transferred = 0;
            let result = unsafe {
                if writing {
                    WriteFile(
                        self.handle.0,
                        Some(&buffer[offset..]),
                        None,
                        Some(&mut overlapped),
                    )
                } else {
                    ReadFile(
                        self.handle.0,
                        Some(&mut buffer[offset..]),
                        None,
                        Some(&mut overlapped),
                    )
                }
            };
            if let Err(error) = result {
                if error.code() != windows::core::HRESULT::from_win32(ERROR_IO_PENDING.0) {
                    return Err(error.to_string());
                }
            }
            loop {
                let wait = unsafe { WaitForSingleObject(event.0, 10) };
                if wait == WAIT_OBJECT_0 {
                    break;
                }
                if wait != WAIT_TIMEOUT
                    || stopped.load(Ordering::Acquire)
                    || Instant::now() >= deadline
                {
                    unsafe {
                        let _ = CancelIoEx(self.handle.0, Some(&overlapped));
                        let _ =
                            GetOverlappedResult(self.handle.0, &overlapped, &mut transferred, true);
                    }
                    return Err("pipe I/O deadline or shutdown".into());
                }
            }
            unsafe {
                GetOverlappedResult(self.handle.0, &overlapped, &mut transferred, false)
                    .map_err(|e| e.to_string())?;
            }
            if transferred == 0 {
                return Err("pipe EOF".into());
            }
            offset += transferred as usize;
        }
        Ok(())
    }
}
