//! Development policy: same OS logon/session and exact Broker executable.
use std::{ffi::c_void, os::windows::io::AsRawHandle};
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        System::{Pipes::*, Threading::*},
    },
};
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct Local(HLOCAL);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(Some(self.0));
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerContext {
    pub session: u32,
    pub logon_high: i32,
    pub logon_low: u32,
    pub logon_sid: String,
    pub executable: String,
}
impl PeerContext {
    pub fn current() -> Result<Self, String> {
        inspect(std::process::id())
    }
    pub fn pipe_name(&self) -> String {
        format!(
            r"\\.\pipe\wufan-dev-{}-{:08x}-{:08x}",
            self.session, self.logon_high, self.logon_low
        )
    }
}
/// Allocated descriptor stays alive through CreateNamedPipeW.
pub struct PipeSecurity {
    descriptor: Local,
}
impl PipeSecurity {
    pub fn for_logon(context: &PeerContext) -> Result<Self, String> {
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})", context.logon_sid)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(Self {
            descriptor: Local(HLOCAL(descriptor.0)),
        })
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.0 .0,
            bInheritHandle: false.into(),
        }
    }
}
pub fn authorize_client(pipe: &impl AsRawHandle, expected: &PeerContext) -> Result<(), String> {
    let mut pid = 0;
    unsafe {
        GetNamedPipeClientProcessId(HANDLE(pipe.as_raw_handle()), &mut pid)
            .map_err(|e| e.to_string())?;
    }
    authorize(pid, expected)
}
pub fn authorize_development_client(
    pipe: &impl AsRawHandle,
    expected: &PeerContext,
    allowed: &[String],
) -> Result<(), String> {
    let mut pid = 0;
    unsafe {
        GetNamedPipeClientProcessId(HANDLE(pipe.as_raw_handle()), &mut pid)
            .map_err(|e| e.to_string())?;
    }
    let peer = inspect(pid)?;
    if peer.session == expected.session
        && peer.logon_sid == expected.logon_sid
        && peer.logon_high == expected.logon_high
        && peer.logon_low == expected.logon_low
        && (peer.executable == expected.executable || allowed.contains(&peer.executable))
    {
        Ok(())
    } else {
        Err("peer is outside the explicit development host/logon policy".into())
    }
}
pub fn authorize_server(pipe: &impl AsRawHandle, expected: &PeerContext) -> Result<(), String> {
    let mut pid = 0;
    unsafe {
        GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid)
            .map_err(|e| e.to_string())?;
    }
    authorize(pid, expected)
}
fn authorize(pid: u32, expected: &PeerContext) -> Result<(), String> {
    if &inspect(pid)? == expected {
        Ok(())
    } else {
        Err("peer is outside the development executable/logon policy".into())
    }
}
fn inspect(pid: u32) -> Result<PeerContext, String> {
    unsafe {
        // Hold one process handle throughout executable and token queries.
        let process = Handle(
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                .map_err(|e| e.to_string())?,
        );
        let mut token = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY, &mut token).map_err(|e| e.to_string())?;
        let token = Handle(token);
        let mut size = 32768;
        let mut path = vec![0u16; size as usize];
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut size,
        )
        .map_err(|e| e.to_string())?;
        let executable = String::from_utf16(&path[..size as usize])
            .map_err(|e| e.to_string())?
            .to_lowercase();
        let session: u32 = token_info(token.0, TokenSessionId)?;
        let stats: TOKEN_STATISTICS = token_info(token.0, TokenStatistics)?;
        let mut bytes = 0;
        let _ = GetTokenInformation(token.0, TokenLogonSid, None, 0, &mut bytes);
        if bytes == 0 || bytes > 64 * 1024 {
            return Err("invalid logon SID buffer size".into());
        }
        // Word alignment is needed before casting TOKEN_GROUPS.
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            token.0,
            TokenLogonSid,
            Some(buffer.as_mut_ptr().cast()),
            bytes,
            &mut bytes,
        )
        .map_err(|e| e.to_string())?;
        let groups = &*buffer.as_ptr().cast::<TOKEN_GROUPS>();
        if groups.GroupCount != 1 {
            return Err("expected one OS logon SID".into());
        }
        let mut sid = PWSTR::null();
        ConvertSidToStringSidW(groups.Groups[0].Sid, &mut sid).map_err(|e| e.to_string())?;
        let allocation = Local(HLOCAL(sid.0.cast::<c_void>()));
        let logon_sid = sid.to_string().map_err(|e| e.to_string())?;
        drop(allocation);
        Ok(PeerContext {
            session,
            logon_high: stats.AuthenticationId.HighPart,
            logon_low: stats.AuthenticationId.LowPart,
            logon_sid,
            executable,
        })
    }
}
unsafe fn token_info<T: Default>(
    token: HANDLE,
    class: TOKEN_INFORMATION_CLASS,
) -> Result<T, String> {
    let mut result = T::default();
    let mut bytes = 0;
    GetTokenInformation(
        token,
        class,
        Some((&mut result as *mut T).cast()),
        std::mem::size_of::<T>() as u32,
        &mut bytes,
    )
    .map_err(|e| e.to_string())?;
    Ok(result)
}
