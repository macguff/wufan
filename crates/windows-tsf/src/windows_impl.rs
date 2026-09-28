use core::ffi::c_void;
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use windows::core::{
    implement, Error, IUnknown, IUnknownImpl, Interface, Ref, Result, BOOL, GUID, HRESULT, PCWSTR,
};
use windows::Win32::Foundation::{
    E_NOINTERFACE, E_POINTER, HMODULE, LPARAM, RPC_E_CHANGED_MODE, S_FALSE, WPARAM,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IClassFactory, IClassFactory_Impl,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr, ITfContext,
    ITfInputProcessorProfiles, ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr,
    ITfTextInputProcessor, ITfTextInputProcessor_Impl, ITfThreadMgr, GUID_TFCAT_TIP_KEYBOARD,
};

/// Stable CLSID for the Windows probe. Registration will use this same value.
pub const CLSID_WUFAN: GUID = GUID::from_u128(0x58f68769_239a_4dca_854e_57aa887e979b);
const PROFILE_WUFAN: GUID = GUID::from_u128(0x9de41ec9_bcc0_4dd8_90c8_975f03c3ef73);
const CLSID_STRING: &str = "{58F68769-239A-4DCA-854E-57AA887E979B}";

fn registration_trace(stage: &str) {
    if let Some(path) = std::env::var_os("WUFAN_TSF_PROBE_TRACE") {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{stage}");
        }
    }
}

static SERVER_LOCKS: AtomicU32 = AtomicU32::new(0);
static LIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);

#[implement(IClassFactory)]
struct ClassFactory;

impl ClassFactory {
    fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for ClassFactory {
    fn drop(&mut self) {
        LIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        output: *mut *mut c_void,
    ) -> Result<()> {
        if output.is_null() || riid.is_null() {
            return Err(Error::from(E_POINTER));
        }
        // SAFETY: COM supplies a writable output pointer. It must be cleared even on failure.
        unsafe { *output = core::ptr::null_mut() };
        if !outer.is_null() {
            return Err(Error::from(
                windows::Win32::Foundation::CLASS_E_NOAGGREGATION,
            ));
        }
        let processor: ITfTextInputProcessor = TextService::new().into();
        // SAFETY: `processor` is a live COM object and COM supplies a GUID/output pointer.
        let hr = unsafe { processor.query(riid, output) };
        hr.ok()
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            SERVER_LOCKS.fetch_add(1, Ordering::Relaxed);
        } else {
            SERVER_LOCKS
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                .ok();
        }
        Ok(())
    }
}

#[derive(Default)]
struct Activation {
    thread_manager: Option<ITfThreadMgr>,
    client_id: u32,
}

#[implement(ITfTextInputProcessor, ITfKeyEventSink)]
struct TextService {
    activation: Mutex<Activation>,
}

impl TextService {
    fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
        Self {
            activation: Mutex::new(Activation::default()),
        }
    }
}

impl Drop for TextService {
    fn drop(&mut self) {
        LIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, manager: Ref<ITfThreadMgr>, client_id: u32) -> Result<()> {
        let manager = manager.as_ref().ok_or_else(|| Error::from(E_POINTER))?;
        let keystrokes: ITfKeystrokeMgr = manager.cast()?;
        let sink: ITfKeyEventSink = self.to_interface();
        // SAFETY: TSF owns the manager and callback contract; no host operation is performed here.
        unsafe { keystrokes.AdviseKeyEventSink(client_id, &sink, true)? };
        let mut state = self
            .activation
            .lock()
            .map_err(|_| Error::from(E_NOINTERFACE))?;
        state.thread_manager = Some(manager.clone());
        state.client_id = client_id;
        Ok(())
    }

    fn Deactivate(&self) -> Result<()> {
        let mut state = self
            .activation
            .lock()
            .map_err(|_| Error::from(E_NOINTERFACE))?;
        if let Some(manager) = state.thread_manager.take() {
            let keystrokes: ITfKeystrokeMgr = manager.cast()?;
            // SAFETY: this undoes the subscription made during activation.
            unsafe { keystrokes.UnadviseKeyEventSink(state.client_id)? };
        }
        Ok(())
    }
}

#[allow(non_snake_case)]
impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, _foreground: BOOL) -> Result<()> {
        Ok(())
    }
    fn OnTestKeyDown(
        &self,
        _context: Ref<ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(BOOL(0))
    }
    fn OnTestKeyUp(
        &self,
        _context: Ref<ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(BOOL(0))
    }
    fn OnKeyDown(
        &self,
        _context: Ref<ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(BOOL(0))
    }
    fn OnKeyUp(&self, _context: Ref<ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        Ok(BOOL(0))
    }
    fn OnPreservedKey(&self, _context: Ref<ITfContext>, _guid: *const GUID) -> Result<BOOL> {
        Ok(BOOL(0))
    }
}

/// # Safety
/// Called by COM with a valid CLSID/IID and a writable output pointer.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    if output.is_null() || clsid.is_null() || iid.is_null() {
        return E_POINTER;
    }
    *output = core::ptr::null_mut();
    if *clsid != CLSID_WUFAN {
        return windows::Win32::Foundation::CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = ClassFactory::new().into();
    factory.query(iid, output)
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if SERVER_LOCKS.load(Ordering::Relaxed) == 0 && LIVE_OBJECTS.load(Ordering::Relaxed) == 0 {
        windows::Win32::Foundation::S_OK
    } else {
        S_FALSE
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(core::iter::once(0)).collect()
}

fn set_registry_value(key: HKEY, name: &str, value: &str) -> Result<()> {
    let name = wide(name);
    let value = wide(value);
    // SAFETY: u16 is plain data and `value` remains alive through RegSetValueExW.
    let bytes =
        unsafe { core::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
    // SAFETY: key is an open registry handle and both strings are NUL terminated.
    unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) }.ok()
}

fn dll_path() -> Result<String> {
    registration_trace("dll_path: begin");
    let mut module = HMODULE::default();
    // SAFETY: the address belongs to this module; the flag asks for its handle without changing
    // its reference count. The pointer is used as an address, not dereferenced as UTF-16.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(DllGetClassObject as *const () as *const u16),
            &mut module,
        )?;
    }
    let mut buffer = vec![0u16; 32768];
    // SAFETY: module is live and the buffer is writable.
    let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    registration_trace("dll_path: module path read");
    if length == 0 || length >= buffer.len() {
        return Err(Error::from(E_POINTER));
    }
    Ok(String::from_utf16_lossy(&buffer[..length]))
}

fn register_com() -> Result<()> {
    registration_trace("register_com: begin");
    let path = format!("Software\\Classes\\CLSID\\{CLSID_STRING}\\InprocServer32");
    let mut key = HKEY::default();
    let path = wide(&path);
    // SAFETY: HKCU is valid, path is NUL terminated, and key points to writable storage.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    }
    .ok()?;
    registration_trace("register_com: key created");
    let result = (|| {
        set_registry_value(key, "", &dll_path()?)?;
        registration_trace("register_com: default value set");
        set_registry_value(key, "ThreadingModel", "Apartment")
    })();
    // SAFETY: key came from RegCreateKeyExW.
    unsafe { RegCloseKey(key) }.ok()?;
    registration_trace("register_com: key closed");
    result
}

fn unregister_com() -> Result<()> {
    let path = wide(&format!("Software\\Classes\\CLSID\\{CLSID_STRING}"));
    // SAFETY: HKCU is valid and path is NUL terminated.
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) }.ok()
}

fn with_com<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    // SAFETY: COM initialization is scoped to this thread and balanced on successful calls.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr == RPC_E_CHANGED_MODE {
        // The caller already selected another apartment. COM is usable, but we do not own its
        // initialization count and must not call CoUninitialize.
        return f();
    }
    if hr.is_err() {
        return Err(Error::from(hr));
    }
    let result = f();
    unsafe { CoUninitialize() };
    result
}

fn register_tsf() -> Result<()> {
    registration_trace("register_tsf: begin");
    with_com(|| {
        registration_trace("register_tsf: COM initialized");
        // SAFETY: class identifiers refer to Windows TSF services.
        let profiles: ITfInputProcessorProfiles = unsafe {
            CoCreateInstance(
                &CLSID_TF_InputProcessorProfiles,
                None::<&IUnknown>,
                CLSCTX_INPROC_SERVER,
            )?
        };
        let categories: ITfCategoryMgr = unsafe {
            CoCreateInstance(
                &CLSID_TF_CategoryMgr,
                None::<&IUnknown>,
                CLSCTX_INPROC_SERVER,
            )?
        };
        registration_trace("register_tsf: services created");
        unsafe {
            profiles.Register(&CLSID_WUFAN)?;
            registration_trace("register_tsf: profile registered");
            let label: Vec<u16> = "Wufan TSF Probe".encode_utf16().collect();
            profiles.AddLanguageProfile(&CLSID_WUFAN, 0x0804, &PROFILE_WUFAN, &label, &[], 0)?;
            registration_trace("register_tsf: language added");
            categories.RegisterCategory(&CLSID_WUFAN, &GUID_TFCAT_TIP_KEYBOARD, &CLSID_WUFAN)?;
            registration_trace("register_tsf: category registered");
        }
        Ok(())
    })
}

fn unregister_tsf() -> Result<()> {
    with_com(|| {
        let profiles: ITfInputProcessorProfiles = unsafe {
            CoCreateInstance(
                &CLSID_TF_InputProcessorProfiles,
                None::<&IUnknown>,
                CLSCTX_INPROC_SERVER,
            )?
        };
        // SAFETY: unregister removes this CLSID's profiles and categories.
        unsafe { profiles.Unregister(&CLSID_WUFAN) }
    })
}

#[no_mangle]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    registration_trace("DllRegisterServer: entry");
    if let Err(error) = register_com() {
        let _ = unregister_com();
        return error.code();
    }
    match register_tsf() {
        Ok(()) => windows::Win32::Foundation::S_OK,
        Err(error) => {
            let _ = unregister_tsf();
            let _ = unregister_com();
            error.code()
        }
    }
}

#[no_mangle]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    let tsf = unregister_tsf();
    let com = unregister_com();
    tsf.and(com)
        .map_or_else(|error| error.code(), |_| windows::Win32::Foundation::S_OK)
}
