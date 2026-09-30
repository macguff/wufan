use core::ffi::c_void;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::composition::CompositionController;
use windows::core::{
    implement, Error, IUnknown, IUnknownImpl, Interface, Ref, Result, BOOL, BSTR, GUID, HRESULT,
    PCWSTR,
};
use windows::Win32::Foundation::{
    E_FAIL, E_NOINTERFACE, E_POINTER, HMODULE, LPARAM, RPC_E_CHANGED_MODE, S_FALSE, WPARAM,
};
use windows::Win32::Graphics::Gdi::HBITMAP;
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
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr, ITfCompartment,
    ITfCompartmentEventSink, ITfCompartmentEventSink_Impl, ITfCompartmentMgr, ITfContext,
    ITfInputProcessorProfiles, ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr,
    ITfLangBarItemButton, ITfLangBarItemButton_Impl, ITfLangBarItemMgr, ITfLangBarItemSink,
    ITfLangBarItem_Impl, ITfMenu, ITfSource, ITfSource_Impl, ITfTextInputProcessor,
    ITfTextInputProcessor_Impl, ITfThreadMgr, ITfThreadMgrEventSink,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    GUID_TFCAT_TIP_KEYBOARD, TF_CONVERSIONMODE_NATIVE, TF_LANGBARITEMINFO, TF_LBI_CLK_LEFT,
    TF_LBI_ICON, TF_LBI_STATUS, TF_LBI_STYLE_BTN_BUTTON, TF_LBI_STYLE_BTN_MENU, TF_LBI_TEXT,
    TF_MOD_CONTROL, TF_PRESERVEDKEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, LoadIconW, IDI_APPLICATION, IDI_INFORMATION,
};

/// Stable CLSID for the Windows probe. Registration will use this same value.
pub const CLSID_WUFAN: GUID = GUID::from_u128(0x58f68769_239a_4dca_854e_57aa887e979b);
const PROFILE_WUFAN: GUID = GUID::from_u128(0x9de41ec9_bcc0_4dd8_90c8_975f03c3ef73);
const CLSID_STRING: &str = "{58F68769-239A-4DCA-854E-57AA887E979B}";
const GUID_MODE_BUTTON: GUID = GUID::from_u128(0xa913a196_f35b_4b7f_9dcb_a093834e0e66);
const GUID_PRESERVED_TOGGLE: GUID = GUID::from_u128(0x9ad9bb91_62fa_42a1_b124_48f3cbf4bd70);

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

/// Count callbacks retained by TSF, including after the TextService is released.
pub(super) struct ComLifetime;

impl ComLifetime {
    pub(super) fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for ComLifetime {
    fn drop(&mut self) {
        LIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Clone)]
struct Compartments {
    open_close: ITfCompartment,
    conversion: ITfCompartment,
}

struct ModePersistenceWorker {
    stopped: Arc<AtomicBool>,
    wake: SyncSender<()>,
    thread: JoinHandle<()>,
}

impl ModePersistenceWorker {
    fn start(mode: Arc<AtomicBool>, identity: String, path: PathBuf) -> std::io::Result<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let (wake, receiver) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("wufan-mode-persist".to_owned())
            .spawn(move || {
                let mut last_written = None;
                loop {
                    let _ = receiver.recv_timeout(Duration::from_millis(500));
                    let current = mode.load(Ordering::Acquire);
                    if last_written != Some(current) {
                        let _ = write_app_mode(&path, &identity, current);
                        last_written = Some(current);
                    }
                    if worker_stopped.load(Ordering::Acquire) {
                        break;
                    }
                }
            })?;
        Ok(Self {
            stopped,
            wake,
            thread,
        })
    }

    fn signal(&self) {
        match self.wake.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) | Err(TrySendError::Disconnected(())) => {}
        }
    }

    fn stop(self) {
        self.stopped.store(true, Ordering::Release);
        self.signal();
        let _ = self.thread.join();
    }
}

fn app_mode_path() -> Option<(String, PathBuf)> {
    let mut image = vec![0u16; 32768];
    let length = unsafe { GetModuleFileNameW(None, &mut image) } as usize;
    if length == 0 || length >= image.len() {
        return None;
    }
    let identity = String::from_utf16_lossy(&image[..length]);
    let key = identity
        .to_lowercase()
        .encode_utf16()
        .fold(0xcbf29ce484222325_u64, |hash, unit| {
            (hash ^ u64::from(unit)).wrapping_mul(0x100000001b3)
        });
    let root = std::env::var_os("LOCALAPPDATA")?;
    Some((
        identity,
        PathBuf::from(root)
            .join("Wufan")
            .join("mode-memory")
            .join(format!("{key:016x}.state")),
    ))
}

fn read_app_mode(path: &Path, identity: &str) -> Option<bool> {
    let contents = std::fs::read_to_string(path).ok()?;
    let (stored_identity, stored_mode) = contents.split_once('\n')?;
    if !stored_identity.eq_ignore_ascii_case(identity) {
        return None;
    }
    match stored_mode.trim() {
        "chinese" => Some(true),
        "english" => Some(false),
        _ => None,
    }
}

fn write_app_mode(path: &Path, identity: &str, chinese: bool) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("mode path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let value = if chinese { "chinese" } else { "english" };
    std::fs::write(&temporary, format!("{identity}\n{value}\n"))?;
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) if path.exists() => {
            std::fs::remove_file(path)?;
            std::fs::rename(temporary, path).map_err(|_| error)
        }
        Err(error) => Err(error),
    }
}

struct ModeController {
    input: CompositionController,
    chinese: Arc<AtomicBool>,
    client_id: AtomicU32,
    compartments: Mutex<Option<Compartments>>,
    compartment_sources: Mutex<Vec<(ITfSource, u32)>>,
    sink: Mutex<Option<ITfLangBarItemSink>>,
    worker: Mutex<Option<ModePersistenceWorker>>,
}

impl ModeController {
    fn new(input: CompositionController) -> Self {
        Self {
            input,
            chinese: Arc::new(AtomicBool::new(true)),
            client_id: AtomicU32::new(0),
            compartments: Mutex::new(None),
            compartment_sources: Mutex::new(Vec::new()),
            sink: Mutex::new(None),
            worker: Mutex::new(None),
        }
    }

    fn is_chinese(&self) -> bool {
        self.chinese.load(Ordering::Acquire)
    }

    fn start_persistence(&self, identity: String, path: PathBuf) -> Result<()> {
        let initial = read_app_mode(&path, &identity).unwrap_or(true);
        self.chinese.store(initial, Ordering::Release);
        let worker = ModePersistenceWorker::start(Arc::clone(&self.chinese), identity, path)
            .map_err(|_| Error::from(E_FAIL))?;
        let mut current = self.worker.lock().map_err(|_| Error::from(E_NOINTERFACE))?;
        *current = Some(worker);
        Ok(())
    }

    fn attach_compartments(
        self: &Arc<Self>,
        client_id: u32,
        compartments: Compartments,
    ) -> Result<()> {
        let event_sink: ITfCompartmentEventSink = ModeCompartmentSink::new(Arc::clone(self)).into();
        let unknown: IUnknown = event_sink.cast()?;
        let open_source: ITfSource = compartments.open_close.cast()?;
        let conversion_source: ITfSource = compartments.conversion.cast()?;
        let open_cookie =
            unsafe { open_source.AdviseSink(&ITfCompartmentEventSink::IID, &unknown)? };
        let conversion_cookie = match unsafe {
            conversion_source.AdviseSink(&ITfCompartmentEventSink::IID, &unknown)
        } {
            Ok(cookie) => cookie,
            Err(error) => {
                let _ = unsafe { open_source.UnadviseSink(open_cookie) };
                return Err(error);
            }
        };
        let Ok(mut sources) = self.compartment_sources.lock() else {
            let _ = unsafe { open_source.UnadviseSink(open_cookie) };
            let _ = unsafe { conversion_source.UnadviseSink(conversion_cookie) };
            return Err(Error::from(E_NOINTERFACE));
        };
        sources.push((open_source, open_cookie));
        sources.push((conversion_source, conversion_cookie));
        self.client_id.store(client_id, Ordering::Release);
        if let Ok(mut current) = self.compartments.lock() {
            *current = Some(compartments);
        } else {
            return Err(Error::from(E_NOINTERFACE));
        }
        self.apply_compartment_mode(self.is_chinese())
    }

    fn apply_compartment_mode(&self, chinese: bool) -> Result<()> {
        let compartments = self
            .compartments
            .lock()
            .map_err(|_| Error::from(E_NOINTERFACE))?
            .clone();
        if let Some(compartments) = compartments {
            let client_id = self.client_id.load(Ordering::Acquire);
            let open = VARIANT::from(i32::from(chinese));
            // TSF compartments require VT_I4, including conversion bit masks.
            let conversion = VARIANT::from(if chinese {
                TF_CONVERSIONMODE_NATIVE as i32
            } else {
                0i32
            });
            unsafe { compartments.open_close.SetValue(client_id, &open)? };
            if let Err(error) = unsafe { compartments.conversion.SetValue(client_id, &conversion) }
            {
                let rollback = VARIANT::from(i32::from(!chinese));
                let _ = unsafe { compartments.open_close.SetValue(client_id, &rollback) };
                return Err(error);
            }
        }
        Ok(())
    }

    fn set(&self, chinese: bool) -> Result<()> {
        if self.is_chinese() == chinese {
            return Ok(());
        }
        self.apply_compartment_mode(chinese)?;
        if self.chinese.swap(chinese, Ordering::AcqRel) != chinese {
            self.notify_and_persist();
        }
        Ok(())
    }

    fn sync_from_compartments(&self) -> Result<()> {
        let compartments = self
            .compartments
            .lock()
            .map_err(|_| Error::from(E_NOINTERFACE))?
            .clone()
            .ok_or_else(|| Error::from(E_NOINTERFACE))?;
        let open = unsafe { compartments.open_close.GetValue()? };
        let conversion = unsafe { compartments.conversion.GetValue()? };
        let open = i32::try_from(&open).map_err(|_| Error::from(E_NOINTERFACE))? != 0;
        let conversion = u32::try_from(&conversion).map_err(|_| Error::from(E_NOINTERFACE))?;
        let chinese = open && conversion & TF_CONVERSIONMODE_NATIVE != 0;
        if self.chinese.swap(chinese, Ordering::AcqRel) != chinese {
            self.notify_and_persist();
        }
        Ok(())
    }

    fn notify_and_persist(&self) {
        self.input.reset(self.client_id.load(Ordering::Acquire));
        let sink = self.sink.lock().ok().and_then(|sink| sink.clone());
        if let Some(sink) = sink {
            let _ = unsafe { sink.OnUpdate(TF_LBI_TEXT | TF_LBI_ICON | TF_LBI_STATUS) };
        }
        if let Ok(worker) = self.worker.lock() {
            if let Some(worker) = worker.as_ref() {
                worker.signal();
            }
        }
    }

    fn toggle(&self) -> Result<()> {
        self.set(!self.is_chinese())
    }

    fn stop_persistence(&self) {
        if let Ok(mut worker) = self.worker.lock() {
            if let Some(worker) = worker.take() {
                worker.stop();
            }
        }
    }

    fn detach_compartments(&self) -> Result<()> {
        let subscriptions = {
            let mut current = self
                .compartment_sources
                .lock()
                .map_err(|_| Error::from(E_NOINTERFACE))?;
            std::mem::take(&mut *current)
        };
        let mut first_error = None;
        for (source, cookie) in subscriptions {
            if let Err(error) = unsafe { source.UnadviseSink(cookie) } {
                first_error.get_or_insert(error);
            }
        }
        if let Ok(mut compartments) = self.compartments.lock() {
            *compartments = None;
        }
        self.client_id.store(0, Ordering::Release);
        first_error.map_or(Ok(()), Err)
    }
}

#[implement(ITfCompartmentEventSink)]
struct ModeCompartmentSink {
    mode: Arc<ModeController>,
}

impl ModeCompartmentSink {
    fn new(mode: Arc<ModeController>) -> Self {
        Self { mode }
    }
}

impl ITfCompartmentEventSink_Impl for ModeCompartmentSink_Impl {
    fn OnChange(&self, guid: *const GUID) -> Result<()> {
        if guid.is_null() {
            return Err(Error::from(E_POINTER));
        }
        let changed = unsafe { *guid };
        if changed == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE
            || changed == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION
        {
            self.mode.sync_from_compartments()?;
        }
        Ok(())
    }
}

#[implement(ITfLangBarItemButton, ITfSource)]
struct ModeButton {
    mode: Arc<ModeController>,
}

impl ModeButton {
    fn new(mode: Arc<ModeController>) -> Self {
        Self { mode }
    }
}

impl ITfLangBarItem_Impl for ModeButton_Impl {
    fn GetInfo(&self, info: *mut TF_LANGBARITEMINFO) -> Result<()> {
        if info.is_null() {
            return Err(Error::from(E_POINTER));
        }
        let mut value = TF_LANGBARITEMINFO {
            clsidService: CLSID_WUFAN,
            guidItem: GUID_MODE_BUTTON,
            dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_BTN_MENU,
            ..Default::default()
        };
        let description: Vec<u16> = "Wufan mode".encode_utf16().collect();
        value.szDescription[..description.len()].copy_from_slice(&description);
        unsafe { info.write(value) };
        Ok(())
    }

    fn GetStatus(&self) -> Result<u32> {
        Ok(0)
    }

    fn Show(&self, _show: BOOL) -> Result<()> {
        Ok(())
    }

    fn GetTooltipString(&self) -> Result<BSTR> {
        Ok(BSTR::from(if self.mode.is_chinese() {
            "Wufan：中文模式"
        } else {
            "Wufan: English mode"
        }))
    }
}

impl ITfLangBarItemButton_Impl for ModeButton_Impl {
    fn OnClick(
        &self,
        click: windows::Win32::UI::TextServices::TfLBIClick,
        _point: &windows::Win32::Foundation::POINT,
        _area: *const windows::Win32::Foundation::RECT,
    ) -> Result<()> {
        if click == TF_LBI_CLK_LEFT {
            self.mode.toggle()?;
        }
        Ok(())
    }

    fn InitMenu(&self, menu: Ref<ITfMenu>) -> Result<()> {
        let menu = menu.as_ref().ok_or_else(|| Error::from(E_POINTER))?;
        let chinese: Vec<u16> = "中文模式".encode_utf16().collect();
        let english: Vec<u16> = "英文模式".encode_utf16().collect();
        let mut submenu = None;
        unsafe {
            menu.AddMenuItem(
                1,
                0,
                HBITMAP::default(),
                HBITMAP::default(),
                &chinese,
                &mut submenu,
            )?;
            menu.AddMenuItem(
                2,
                0,
                HBITMAP::default(),
                HBITMAP::default(),
                &english,
                &mut submenu,
            )?;
        }
        Ok(())
    }

    fn OnMenuSelect(&self, id: u32) -> Result<()> {
        match id {
            1 => self.mode.set(true),
            2 => self.mode.set(false),
            _ => Ok(()),
        }
    }

    fn GetIcon(&self) -> Result<windows::Win32::UI::WindowsAndMessaging::HICON> {
        let icon = unsafe {
            LoadIconW(
                None,
                if self.mode.is_chinese() {
                    IDI_INFORMATION
                } else {
                    IDI_APPLICATION
                },
            )?
        };
        unsafe { CopyIcon(icon) }
    }

    fn GetText(&self) -> Result<BSTR> {
        Ok(BSTR::from(if self.mode.is_chinese() {
            "中"
        } else {
            "英"
        }))
    }
}

impl ITfSource_Impl for ModeButton_Impl {
    fn AdviseSink(&self, iid: *const GUID, unknown: Ref<IUnknown>) -> Result<u32> {
        if iid.is_null() || unsafe { *iid } != ITfLangBarItemSink::IID {
            return Err(Error::from(E_NOINTERFACE));
        }
        let unknown = unknown.as_ref().ok_or_else(|| Error::from(E_POINTER))?;
        let sink: ITfLangBarItemSink = unknown.cast()?;
        if let Ok(mut current) = self.mode.sink.lock() {
            *current = Some(sink);
            Ok(1)
        } else {
            Err(Error::from(E_NOINTERFACE))
        }
    }

    fn UnadviseSink(&self, cookie: u32) -> Result<()> {
        if cookie != 1 {
            return Err(Error::from(E_NOINTERFACE));
        }
        if let Ok(mut current) = self.mode.sink.lock() {
            *current = None;
            Ok(())
        } else {
            Err(Error::from(E_NOINTERFACE))
        }
    }
}

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
        registration_trace("CreateInstance: enter");
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
    broker_window: Option<crate::broker_window::BrokerWindow>,
    thread_manager: Option<ITfThreadMgr>,
    keystroke_manager: Option<ITfKeystrokeMgr>,
    lang_bar_manager: Option<ITfLangBarItemMgr>,
    lang_bar_item: Option<ITfLangBarItemButton>,
    context_source: Option<ITfSource>,
    context_sink: Option<ITfThreadMgrEventSink>,
    context_cookie: Option<u32>,
    client_id: u32,
}

#[implement(ITfTextInputProcessor, ITfKeyEventSink)]
struct TextService {
    activation: Mutex<Activation>,
    mode: Arc<ModeController>,
    client_id: AtomicU32,
    composition: CompositionController,
}

impl TextService {
    #[allow(clippy::arc_with_non_send_sync)]
    fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
        let composition = CompositionController::new();
        Self {
            activation: Mutex::new(Activation::default()),
            mode: Arc::new(ModeController::new(composition.clone())),
            client_id: AtomicU32::new(0),
            composition,
        }
    }
}

impl Drop for TextService {
    fn drop(&mut self) {
        // A host can release the service without a completed Deactivate. Pending
        // sessions may retain their controller, so invalidate their tickets now.
        self.composition.reset(0);
        self.composition.keys.stop_broker();
        LIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, manager: Ref<ITfThreadMgr>, client_id: u32) -> Result<()> {
        registration_trace("Activate: enter");
        if self.client_id.load(Ordering::Acquire) != 0 {
            return Err(Error::from(E_FAIL));
        }
        let manager = manager.as_ref().ok_or_else(|| Error::from(E_POINTER))?;
        let keystrokes: ITfKeystrokeMgr = manager.cast()?;
        registration_trace("Activate: keystrokes ready");
        let sink: ITfKeyEventSink = self.to_interface();
        let mut advised = false;
        let mut preserved = false;
        let mut added_item = None;
        let setup = (|| {
            // SAFETY: TSF owns the manager and callback contract.
            unsafe { keystrokes.AdviseKeyEventSink(client_id, &sink, true)? };
            advised = true;
            registration_trace("Activate: key sink advised");

            let key = TF_PRESERVEDKEY {
                uVKey: 0x20,
                uModifiers: TF_MOD_CONTROL,
            };
            let description: Vec<u16> =
                "Toggle Wufan Chinese/English mode".encode_utf16().collect();
            unsafe {
                keystrokes.PreserveKey(client_id, &GUID_PRESERVED_TOGGLE, &key, &description)?
            };
            preserved = true;
            registration_trace("Activate: preserved key ready");

            let manager_compartments: ITfCompartmentMgr = manager.cast()?;
            let compartments = Compartments {
                open_close: unsafe {
                    manager_compartments.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)?
                },
                conversion: unsafe {
                    manager_compartments
                        .GetCompartment(&GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION)?
                },
            };
            if std::env::var_os("WUFAN_TSF_SMOKE_CHINESE").is_some() {
                self.mode.chinese.store(true, Ordering::Release);
            } else if let Some((identity, path)) = app_mode_path() {
                self.mode.start_persistence(identity, path)?;
            }
            self.mode.attach_compartments(client_id, compartments)?;
            registration_trace("Activate: compartments attached");

            let lang_bar_manager: ITfLangBarItemMgr = manager.cast()?;
            let lang_bar_item: ITfLangBarItemButton =
                ModeButton::new(Arc::clone(&self.mode)).into();
            unsafe { lang_bar_manager.AddItem(&lang_bar_item)? };
            added_item = Some((lang_bar_manager.clone(), lang_bar_item.clone()));
            registration_trace("Activate: language bar ready");
            let context_source: ITfSource = manager.cast()?;
            let context_sink = self.composition.context_sink(client_id);
            let context_cookie =
                unsafe { context_source.AdviseSink(&ITfThreadMgrEventSink::IID, &context_sink)? };
            let backend_setup = (|| {
                self.composition.keys.start_broker()?;
                crate::broker_window::BrokerWindow::new(self.composition.clone(), client_id)
            })();
            let broker_window = match backend_setup {
                Ok(window) => window,
                Err(error) => {
                    registration_trace(&format!("Activate setup failed: {:?}", error.code()));
                    self.composition.keys.stop_broker();
                    let _ = unsafe { context_source.UnadviseSink(context_cookie) };
                    return Err(error);
                }
            };
            Ok((
                lang_bar_manager,
                lang_bar_item,
                context_source,
                context_sink,
                context_cookie,
                broker_window,
            ))
        })();

        let (
            lang_bar_manager,
            lang_bar_item,
            context_source,
            context_sink,
            context_cookie,
            broker_window,
        ) = match setup {
            Ok(value) => value,
            Err(error) => {
                if let Some((manager, item)) = added_item {
                    let _ = unsafe { manager.RemoveItem(&item) };
                }
                if preserved {
                    let key = TF_PRESERVEDKEY {
                        uVKey: 0x20,
                        uModifiers: TF_MOD_CONTROL,
                    };
                    let _ = unsafe { keystrokes.UnpreserveKey(&GUID_PRESERVED_TOGGLE, &key) };
                }
                if advised {
                    let _ = unsafe { keystrokes.UnadviseKeyEventSink(client_id) };
                }
                let _ = self.mode.detach_compartments();
                self.mode.stop_persistence();
                return Err(error);
            }
        };

        if let Ok(document) = unsafe { manager.GetFocus() } {
            if let Ok(context) = unsafe { document.GetTop() } {
                self.composition.bind_context(Some(&context), client_id);
            }
        }
        let mut state = self
            .activation
            .lock()
            .map_err(|_| Error::from(E_NOINTERFACE))?;
        state.thread_manager = Some(manager.clone());
        state.keystroke_manager = Some(keystrokes);
        state.lang_bar_manager = Some(lang_bar_manager);
        state.lang_bar_item = Some(lang_bar_item);
        state.context_source = Some(context_source);
        state.context_sink = Some(context_sink);
        state.context_cookie = Some(context_cookie);
        state.client_id = client_id;
        state.broker_window = Some(broker_window);
        self.client_id.store(client_id, Ordering::Release);
        registration_trace("Activate: success");
        Ok(())
    }

    fn Deactivate(&self) -> Result<()> {
        let mut state = self
            .activation
            .lock()
            .map_err(|_| Error::from(E_NOINTERFACE))?;
        let activation = std::mem::take(&mut *state);
        drop(state);
        self.composition.reset(activation.client_id);
        self.composition.keys.stop_broker();
        self.client_id.store(0, Ordering::Release);

        let mut first_error = None;
        if let (Some(source), Some(cookie)) =
            (&activation.context_source, activation.context_cookie)
        {
            if let Err(error) = unsafe { source.UnadviseSink(cookie) } {
                first_error = Some(error);
            }
        }
        if let (Some(manager), Some(item)) =
            (&activation.lang_bar_manager, &activation.lang_bar_item)
        {
            if let Err(error) = unsafe { manager.RemoveItem(item) } {
                first_error = Some(error);
            }
        }
        if let Some(keystrokes) = &activation.keystroke_manager {
            let key = TF_PRESERVEDKEY {
                uVKey: 0x20,
                uModifiers: TF_MOD_CONTROL,
            };
            if let Err(error) = unsafe { keystrokes.UnpreserveKey(&GUID_PRESERVED_TOGGLE, &key) } {
                first_error.get_or_insert(error);
            }
            if let Err(error) = unsafe { keystrokes.UnadviseKeyEventSink(activation.client_id) } {
                first_error.get_or_insert(error);
            }
        }
        if let Err(error) = self.mode.detach_compartments() {
            first_error.get_or_insert(error);
        }
        self.mode.stop_persistence();
        first_error.map_or(Ok(()), Err)
    }
}

#[allow(non_snake_case)]
impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, foreground: BOOL) -> Result<()> {
        let result = catch_unwind(AssertUnwindSafe(|| {
            if foreground.as_bool() {
                self.mode.apply_compartment_mode(self.mode.is_chinese())?;
            } else {
                self.composition
                    .reset(self.client_id.load(Ordering::Acquire));
            }
            Ok(())
        }));
        match result {
            Ok(result) => result,
            Err(_) => Ok(()),
        }
    }
    fn OnTestKeyDown(
        &self,
        context: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Result<BOOL> {
        let handled = catch_unwind(AssertUnwindSafe(|| {
            if !self
                .composition
                .bind_context(context.as_ref(), self.client_id.load(Ordering::Acquire))
            {
                return BOOL(0);
            }
            if !self.mode.is_chinese() {
                self.composition
                    .reset(self.client_id.load(Ordering::Acquire));
            }
            self.composition
                .keys
                .test_key_down(wparam, lparam, self.mode.is_chinese())
        }))
        .unwrap_or(BOOL(0));
        Ok(handled)
    }
    fn OnTestKeyUp(
        &self,
        _context: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(self.composition.keys.test_key_up(wparam, lparam))
    }
    fn OnKeyDown(&self, context: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let handled = catch_unwind(AssertUnwindSafe(|| {
            let client_id = self.client_id.load(Ordering::Acquire);
            if !self.composition.bind_context(context.as_ref(), client_id) {
                return BOOL(0);
            }
            let chinese_mode = self.mode.is_chinese();
            if !chinese_mode {
                self.composition.reset(client_id);
            }
            let outcome = self.composition.keys.key_down(wparam, lparam, chinese_mode);
            if outcome.reset {
                self.composition.reset_transport(client_id);
                return BOOL(0);
            }
            outcome.handled
        }))
        .unwrap_or(BOOL(0));
        Ok(handled)
    }
    fn OnKeyUp(&self, _context: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        Ok(self.composition.keys.key_up(wparam, lparam))
    }
    fn OnPreservedKey(&self, _context: Ref<ITfContext>, guid: *const GUID) -> Result<BOOL> {
        let result = catch_unwind(AssertUnwindSafe(|| {
            if guid.is_null() {
                return Err(Error::from(E_POINTER));
            }
            if unsafe { *guid } == GUID_PRESERVED_TOGGLE {
                self.mode.toggle()?;
                self.composition
                    .reset(self.client_id.load(Ordering::Acquire));
                Ok(BOOL(1))
            } else {
                Ok(BOOL(0))
            }
        }));
        match result {
            Ok(result) => result,
            Err(_) => Ok(BOOL(0)),
        }
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

pub(super) fn pin_for_worker() -> Result<()> {
    let mut module = HMODULE::default();
    // IPC shutdown is nonblocking. Pin code while detached workers unwind;
    // the Windows loader releases the image when the host process exits.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                | windows::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR(DllGetClassObject as *const () as *const u16),
            &mut module,
        )
    }
}

pub(super) fn dll_path() -> Result<String> {
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
            let profile_manager: windows::Win32::UI::TextServices::ITfInputProcessorProfileMgr =
                profiles.cast()?;
            let label: Vec<u16> = "Wufan 技术内测".encode_utf16().collect();
            // The icon path is optional in TSF, but windows-rs's slice wrapper cannot pass a
            // null pointer: an empty slice still has a non-null dangling pointer. Use the raw
            // vtable for this call so TSF receives the documented NULL icon path.
            (Interface::vtable(&profile_manager).RegisterProfile)(
                Interface::as_raw(&profile_manager),
                &CLSID_WUFAN,
                0x0804,
                &PROFILE_WUFAN,
                PCWSTR(label.as_ptr()),
                label.len() as u32,
                PCWSTR::null(),
                0,
                0,
                windows::Win32::UI::Input::KeyboardAndMouse::HKL::default(),
                0,
                false.into(),
                0,
            )
            .ok()?;
            registration_trace("register_tsf: RegisterProfile succeeded");
            profiles.EnableLanguageProfile(&CLSID_WUFAN, 0x0804, &PROFILE_WUFAN, true)?;
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
        let categories: ITfCategoryMgr = unsafe {
            CoCreateInstance(
                &CLSID_TF_CategoryMgr,
                None::<&IUnknown>,
                CLSCTX_INPROC_SERVER,
            )?
        };
        // Both registrations belong to this CLSID. Attempt both removals even if the first
        // fails, so a partial registration does not leave the other half behind.
        let category_result = unsafe {
            categories.UnregisterCategory(&CLSID_WUFAN, &GUID_TFCAT_TIP_KEYBOARD, &CLSID_WUFAN)
        };
        let profile_result = unsafe { profiles.Unregister(&CLSID_WUFAN) };
        category_result.and(profile_result)
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
