//! Checked-in librime 1.17.0 ABI and an explicit-path dynamic loader.
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("The checked-in librime ABI currently supports 64-bit targets only.");

include!("../bindings.rs");

use std::{marker::PhantomData, path::Path, rc::Rc};

/// Owns the library for at least as long as its copied function table.
/// Deliberately apartment-bound; the adapter creates it on its serial worker.
pub struct Library {
    pub api: RimeApi,
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HMODULE,
    #[cfg(not(windows))]
    _handle: libloading::Library,
    _thread: PhantomData<Rc<()>>,
}

impl Library {
    /// # Safety
    /// `path` must identify a trusted librime binary implementing the vendored
    /// ABI. A dynamic library can execute arbitrary native code when loaded.
    pub unsafe fn load(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::{
                core::{PCSTR, PCWSTR},
                Win32::System::LibraryLoader::*,
            };
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            // Search dependencies alongside the pinned DLL and in system dirs.
            let handle = LoadLibraryExW(
                PCWSTR(wide.as_ptr()),
                None,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
            .map_err(|e| e.to_string())?;
            let result = (|| {
                let symbol = GetProcAddress(handle, PCSTR(c"rime_get_api".as_ptr().cast()))
                    .ok_or("missing rime_get_api")?;
                let get_api: unsafe extern "C" fn() -> *mut RimeApi = std::mem::transmute(symbol);
                copy_api(get_api())
            })();
            match result {
                Ok(api) => Ok(Self {
                    api,
                    handle,
                    _thread: PhantomData,
                }),
                Err(e) => {
                    let _ = windows::Win32::Foundation::FreeLibrary(handle);
                    Err(e)
                }
            }
        }
        #[cfg(not(windows))]
        {
            let handle = libloading::Library::new(path).map_err(|e| e.to_string())?;
            let get_api: libloading::Symbol<unsafe extern "C" fn() -> *mut RimeApi> =
                handle.get(b"rime_get_api\0").map_err(|e| e.to_string())?;
            let api = copy_api(get_api())?;
            Ok(Self {
                api,
                _handle: handle,
                _thread: PhantomData,
            })
        }
    }
}

unsafe fn copy_api(ptr: *mut RimeApi) -> Result<RimeApi, String> {
    if ptr.is_null() {
        return Err("null RimeApi".into());
    }
    // data_size excludes the first int, including its trailing ABI padding.
    let size = ptr.cast::<i32>().read();
    if size < (std::mem::size_of::<RimeApi>() - std::mem::size_of::<i32>()) as i32 {
        return Err("librime function table is shorter than the pinned ABI".into());
    }
    Ok(ptr.read())
}

#[cfg(windows)]
impl Drop for Library {
    fn drop(&mut self) {
        // All sessions and the global runtime have been finalized by the adapter.
        unsafe {
            let _ = windows::Win32::Foundation::FreeLibrary(self.handle);
        }
    }
}
