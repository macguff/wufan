//! Safe, single-threaded engine boundary. No librime pointers escape this crate.
use ime_rime_sys as ffi;
use std::{
    collections::BTreeSet,
    ffi::{CStr, CString},
    marker::PhantomData,
    path::Path,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_TEXT: usize = 16 * 1024;
const MAX_CANDIDATES: usize = 10;
static ACTIVE: AtomicBool = AtomicBool::new(false);

struct RuntimeLease;
impl Drop for RuntimeLease {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineSession(pub usize);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Key {
    pub code: i32,
    pub modifiers: i32,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Candidate {
    pub text: String,
    pub comment: String,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EngineView {
    pub consumed: bool,
    pub preedit: String,
    pub cursor_utf16: u32,
    pub candidates: Vec<Candidate>,
    pub highlighted: u32,
    pub page: u32,
    pub last_page: bool,
    pub commit: Option<String>,
}

pub trait RimeEngine {
    fn open(&mut self) -> Result<EngineSession, String>;
    fn close(&mut self, session: EngineSession);
    fn key(&mut self, session: EngineSession, key: Key) -> Result<EngineView, String>;
    fn clear(&mut self, session: EngineSession) -> Result<(), String>;
    /// Called only after the host reports that the commit was applied.
    fn commit_applied(&mut self, session: EngineSession, text: &str) -> Result<(), String>;
}

/// The startup schema disables learning: librime otherwise learns before ACK.
pub struct NativeRime {
    library: ffi::Library,
    _strings: Vec<CString>,
    sessions: BTreeSet<usize>,
    _thread: PhantomData<Rc<()>>,
    _lease: RuntimeLease,
}

impl NativeRime {
    /// Loads a trusted, explicitly selected librime 1.17.0 distribution.
    /// Deployment is a startup operation and must never run in a TSF callback.
    pub fn start(dll: &Path, shared: &Path, user: &Path, deploy: bool) -> Result<Self, String> {
        if !cfg!(windows) {
            return Err("native startup currently requires the pinned Windows x64 runtime".into());
        }
        std::fs::create_dir_all(user).map_err(|e| e.to_string())?;
        let shared = shared.canonicalize().map_err(|e| e.to_string())?;
        let user = user.canonicalize().map_err(|e| e.to_string())?;
        let strings = [
            shared.to_str().ok_or("non-UTF8 shared path")?,
            user.to_str().ok_or("non-UTF8 user path")?,
            "Wufan",
            "wufan",
            "0.1.0",
            "wufan.broker",
        ]
        .into_iter()
        .map(CString::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
        if ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("only one librime runtime is permitted per process".into());
        }
        Self::initialize(dll, strings, deploy, RuntimeLease)
    }

    fn initialize(
        dll: &Path,
        strings: Vec<CString>,
        deploy: bool,
        lease: RuntimeLease,
    ) -> Result<Self, String> {
        let dll = dll.canonicalize().map_err(|e| e.to_string())?;
        // Verify the pinned DLL before executing native code. The read handle
        // denies replacement/writes until LoadLibrary has acquired its mapping.
        #[cfg(windows)]
        let _verified_dll = verify_dll(&dll)?;
        let library = unsafe { ffi::Library::load(&dll)? };
        let api = &library.api;
        macro_rules! require { ($($f:ident),* $(,)?) => { $(if api.$f.is_none() {
            return Err(concat!("missing librime function: ", stringify!($f)).into());
        })* }; }
        require!(
            get_version,
            setup,
            initialize,
            finalize,
            start_maintenance,
            join_maintenance_thread,
            create_session,
            destroy_session,
            select_schema,
            process_key,
            clear_composition,
            get_context,
            free_context,
            get_commit,
            free_commit,
            schema_open,
            config_close,
            config_get_bool,
            config_list_size,
            config_get_cstring
        );
        let version = unsafe { copy_text(api.get_version.unwrap()())? };
        if version != "1.17.0" {
            return Err(format!("expected librime 1.17.0, got {version}"));
        }
        let mut traits = ffi::RimeTraits {
            data_size: data_size::<ffi::RimeTraits>(),
            shared_data_dir: strings[0].as_ptr(),
            user_data_dir: strings[1].as_ptr(),
            distribution_name: strings[2].as_ptr(),
            distribution_code_name: strings[3].as_ptr(),
            distribution_version: strings[4].as_ptr(),
            app_name: strings[5].as_ptr(),
            min_log_level: 2,
            log_dir: strings[1].as_ptr(),
            ..Default::default()
        };
        unsafe {
            api.setup.unwrap()(&mut traits);
            api.initialize.unwrap()(&mut traits);
        }
        let engine = Self {
            library,
            _strings: strings,
            sessions: BTreeSet::new(),
            _thread: PhantomData,
            _lease: lease,
        };
        if deploy {
            unsafe {
                engine.library.api.start_maintenance.unwrap()(1);
                engine.library.api.join_maintenance_thread.unwrap()();
            }
        }
        engine.check_schema()?;
        Ok(engine)
    }

    fn check_schema(&self) -> Result<(), String> {
        let api = &self.library.api;
        let mut config = ffi::RimeConfig::default();
        unsafe {
            if api.schema_open.unwrap()(c"wufan_pinyin".as_ptr(), &mut config) == 0 {
                return Err("wufan_pinyin is not deployed; run with --deploy".into());
            }
            let guard = ConfigGuard { api, config };
            let mut enabled = 1;
            if api.config_get_bool.unwrap()(
                &mut config,
                c"translator/enable_user_dict".as_ptr(),
                &mut enabled,
            ) == 0
                || enabled != 0
            {
                return Err(
                    "startup schema must explicitly disable user dictionary learning".into(),
                );
            }
            if api.config_list_size.unwrap()(&mut config, c"engine/translators".as_ptr()) != 2 {
                return Err("unexpected translator list".into());
            }
            for (path, expected) in [
                (c"engine/translators/@0", "punct_translator"),
                (c"engine/translators/@1", "script_translator"),
            ] {
                if copy_text(api.config_get_cstring.unwrap()(&mut config, path.as_ptr()))?
                    != expected
                {
                    return Err("unexpected translator in startup schema".into());
                }
            }
            drop(guard);
        }
        Ok(())
    }

    fn valid(&self, session: EngineSession) -> Result<(), String> {
        if self.sessions.contains(&session.0) {
            Ok(())
        } else {
            Err("unknown engine session".into())
        }
    }

    fn snapshot(&self, session: EngineSession, consumed: bool) -> Result<EngineView, String> {
        let api = &self.library.api;
        let mut view = EngineView {
            consumed,
            ..Default::default()
        };
        // Native allocations are always freed, including conversion-error paths.
        unsafe {
            let mut commit = ffi::RimeCommit {
                data_size: data_size::<ffi::RimeCommit>(),
                ..Default::default()
            };
            if api.get_commit.unwrap()(session.0, &mut commit) != 0 {
                let guard = CommitGuard { api, commit };
                view.commit = Some(copy_text(commit.text)?);
                drop(guard);
            }
            let mut context = ffi::RimeContext {
                data_size: data_size::<ffi::RimeContext>(),
                ..Default::default()
            };
            if api.get_context.unwrap()(session.0, &mut context) != 0 {
                let guard = ContextGuard { api, context };
                view.preedit = copy_text(context.composition.preedit)?;
                let byte_cursor = usize::try_from(context.composition.cursor_pos)
                    .map_err(|_| "negative cursor")?;
                if !view.preedit.is_char_boundary(byte_cursor) {
                    return Err("invalid UTF-8 cursor".into());
                }
                view.cursor_utf16 = view.preedit[..byte_cursor].encode_utf16().count() as u32;
                let count = usize::try_from(context.menu.num_candidates)
                    .map_err(|_| "negative candidate count")?;
                if count > MAX_CANDIDATES || (count > 0 && context.menu.candidates.is_null()) {
                    return Err("invalid candidate array".into());
                }
                for i in 0..count {
                    let item = &*context.menu.candidates.add(i);
                    view.candidates.push(Candidate {
                        text: copy_text(item.text)?,
                        comment: copy_text(item.comment)?,
                    });
                }
                view.highlighted = u32::try_from(context.menu.highlighted_candidate_index)
                    .map_err(|_| "negative highlight")?;
                view.page = u32::try_from(context.menu.page_no).map_err(|_| "negative page")?;
                view.last_page = context.menu.is_last_page != 0;
                drop(guard);
            }
        }
        Ok(view)
    }
}

#[cfg(windows)]
fn verify_dll(path: &Path) -> Result<std::fs::File, String> {
    use sha2::{Digest, Sha256};
    use std::{io::Read, os::windows::fs::OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    if format!("{:x}", hasher.finalize())
        != "86b4c7357d4c6d293ce5589b234d8859ca2ac30923a03bedfa3926eeaf97fb0b"
    {
        return Err("rime.dll does not match the pinned Windows x64 librime 1.17.0 binary".into());
    }
    Ok(file)
}

impl RimeEngine for NativeRime {
    fn open(&mut self) -> Result<EngineSession, String> {
        let api = &self.library.api;
        unsafe {
            let id = api.create_session.unwrap()();
            if id == 0 {
                return Err("librime failed to create session".into());
            }
            if api.select_schema.unwrap()(id, c"wufan_pinyin".as_ptr()) == 0 {
                api.destroy_session.unwrap()(id);
                return Err("cannot select wufan_pinyin".into());
            }
            self.sessions.insert(id);
            Ok(EngineSession(id))
        }
    }
    fn close(&mut self, session: EngineSession) {
        if self.sessions.remove(&session.0) {
            unsafe {
                self.library.api.destroy_session.unwrap()(session.0);
            }
        }
    }
    fn key(&mut self, session: EngineSession, key: Key) -> Result<EngineView, String> {
        self.valid(session)?;
        let consumed = unsafe {
            self.library.api.process_key.unwrap()(session.0, key.code, key.modifiers) != 0
        };
        self.snapshot(session, consumed)
    }
    fn clear(&mut self, session: EngineSession) -> Result<(), String> {
        self.valid(session)?;
        unsafe {
            self.library.api.clear_composition.unwrap()(session.0);
        }
        Ok(())
    }
    fn commit_applied(&mut self, session: EngineSession, _text: &str) -> Result<(), String> {
        // Explicit no-op until an ACK-driven learning API is implemented.
        self.valid(session)
    }
}

impl Drop for NativeRime {
    fn drop(&mut self) {
        unsafe {
            for id in &self.sessions {
                self.library.api.destroy_session.unwrap()(*id);
            }
            self.library.api.finalize.unwrap()();
        }
    }
}

fn data_size<T>() -> i32 {
    (std::mem::size_of::<T>() - std::mem::size_of::<i32>()) as i32
}

/// Trusts the native ABI's allocation validity, but bounds the scan and validates UTF-8.
unsafe fn copy_text(ptr: *const std::ffi::c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Ok(String::new());
    }
    for len in 0..=MAX_TEXT {
        if *ptr.add(len) == 0 {
            return CStr::from_ptr(ptr)
                .to_str()
                .map(str::to_owned)
                .map_err(|e| e.to_string());
        }
    }
    Err("native text exceeds limit".into())
}

struct ContextGuard<'a> {
    api: &'a ffi::RimeApi,
    context: ffi::RimeContext,
}
impl Drop for ContextGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.api.free_context.unwrap()(&mut self.context);
        }
    }
}
struct CommitGuard<'a> {
    api: &'a ffi::RimeApi,
    commit: ffi::RimeCommit,
}
impl Drop for CommitGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.api.free_commit.unwrap()(&mut self.commit);
        }
    }
}
struct ConfigGuard<'a> {
    api: &'a ffi::RimeApi,
    config: ffi::RimeConfig,
}
impl Drop for ConfigGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.api.config_close.unwrap()(&mut self.config);
        }
    }
}
