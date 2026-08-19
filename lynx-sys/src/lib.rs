#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("lynx-sys supports only the verified Linux x86_64 SDK");

use std::ffi::{c_char, c_int, c_void, CStr, OsStr};
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

type LynxLogCallback = unsafe extern "C" fn(c_int, *const c_char, *const c_char);

#[repr(C)]
struct DlInfo {
    filename: *const c_char,
    base: *mut c_void,
    symbol_name: *const c_char,
    symbol_address: *mut c_void,
}

#[link(name = "lynx")]
unsafe extern "C" {
    fn lynx_log_init(callback: LynxLogCallback);
}

#[link(name = "dl")]
unsafe extern "C" {
    fn dladdr(address: *const c_void, info: *mut DlInfo) -> c_int;
}

pub fn loaded_library_path() -> io::Result<PathBuf> {
    let mut info = MaybeUninit::<DlInfo>::zeroed();
    let address = lynx_log_init as *const () as *const c_void;

    // SAFETY: `address` names a linked function and `info` points to writable
    // storage with the platform `Dl_info` layout.
    if unsafe { dladdr(address, info.as_mut_ptr()) } == 0 {
        return Err(io::Error::other(
            "dladdr could not locate the linked lynx_log_init symbol",
        ));
    }

    // SAFETY: A successful `dladdr` initializes `info`. `filename` is either
    // NULL or a NUL-terminated string owned by the dynamic loader.
    let info = unsafe { info.assume_init() };
    if info.filename.is_null() {
        return Err(io::Error::other(
            "dladdr returned no library path for lynx_log_init",
        ));
    }
    let bytes = unsafe { CStr::from_ptr(info.filename) }.to_bytes();
    Ok(PathBuf::from(OsStr::from_bytes(bytes)))
}
