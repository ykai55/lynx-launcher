use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::slice;
use std::str;

use crate::{Application, Error, Launcher};

pub const LYNX_STATUS_OK: u32 = 0;
pub const LYNX_STATUS_INVALID_ARGUMENT: u32 = 1;
pub const LYNX_STATUS_NOT_FOUND: u32 = 2;
pub const LYNX_STATUS_PLATFORM_ERROR: u32 = 3;
pub const LYNX_STATUS_PANIC: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LynxSlice {
    pub data: *const u8,
    pub len: usize,
}

impl Default for LynxSlice {
    fn default() -> Self {
        Self {
            data: ptr::null(),
            len: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LynxApplicationView {
    pub id: LynxSlice,
    pub name: LynxSlice,
    pub icon: LynxSlice,
}

pub struct LynxLauncher {
    launcher: Launcher,
}

pub struct LynxAppList {
    applications: Vec<FfiApplication>,
}

pub struct LynxIcon {
    path: Vec<u8>,
}

pub struct LynxError {
    message: Vec<u8>,
}

struct FfiApplication {
    id: Vec<u8>,
    name: Vec<u8>,
    icon: Option<Vec<u8>>,
}

impl From<&Application> for FfiApplication {
    fn from(application: &Application) -> Self {
        Self {
            id: application.id().as_bytes().to_vec(),
            name: application.name().as_bytes().to_vec(),
            icon: application.icon().map(|icon| icon.as_bytes().to_vec()),
        }
    }
}

struct FfiFailure {
    status: u32,
    message: String,
}

impl FfiFailure {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: LYNX_STATUS_INVALID_ARGUMENT,
            message: message.into(),
        }
    }
}

impl From<Error> for FfiFailure {
    fn from(error: Error) -> Self {
        let status = match error {
            Error::ApplicationNotFound(_) => LYNX_STATUS_NOT_FOUND,
            _ => LYNX_STATUS_PLATFORM_ERROR,
        };
        Self {
            status,
            message: error.to_string(),
        }
    }
}

fn bytes(value: &[u8]) -> LynxSlice {
    if value.is_empty() {
        LynxSlice::default()
    } else {
        LynxSlice {
            data: value.as_ptr(),
            len: value.len(),
        }
    }
}

unsafe fn input(slice: LynxSlice) -> Result<String, FfiFailure> {
    if slice.len == 0 {
        return Ok(String::new());
    }
    if slice.data.is_null() {
        return Err(FfiFailure::invalid(
            "slice data must not be null when length is non-zero",
        ));
    }
    let value = slice::from_raw_parts(slice.data, slice.len);
    str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| FfiFailure::invalid("input must be valid UTF-8"))
}

fn status_call(
    out_error: *mut *mut LynxError,
    operation: impl FnOnce() -> Result<(), FfiFailure>,
) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if !out_error.is_null() {
            unsafe { out_error.write(ptr::null_mut()) };
        }
        operation()
    }));
    match result {
        Ok(Ok(())) => LYNX_STATUS_OK,
        Ok(Err(failure)) => {
            if !out_error.is_null() {
                let error = Box::new(LynxError {
                    message: failure.message.into_bytes(),
                });
                unsafe { out_error.write(Box::into_raw(error)) };
            }
            failure.status
        }
        Err(_) => {
            if !out_error.is_null() {
                let error = Box::new(LynxError {
                    message: b"Rust panic was contained at the C ABI boundary".to_vec(),
                });
                unsafe { out_error.write(Box::into_raw(error)) };
            }
            LYNX_STATUS_PANIC
        }
    }
}

#[no_mangle]
pub extern "C" fn lynx_launcher_abi_version() -> u32 {
    1
}

#[no_mangle]
/// # Safety
/// `out_launcher` must be writable. `out_error` may be null or must be writable.
pub unsafe extern "C" fn lynx_launcher_create(
    out_launcher: *mut *mut LynxLauncher,
    out_error: *mut *mut LynxError,
) -> u32 {
    status_call(out_error, || {
        if out_launcher.is_null() {
            return Err(FfiFailure::invalid("out_launcher must not be null"));
        }
        unsafe { out_launcher.write(ptr::null_mut()) };
        let launcher = Launcher::discover().map_err(FfiFailure::from)?;
        unsafe { out_launcher.write(Box::into_raw(Box::new(LynxLauncher { launcher }))) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `launcher` must be null or a live handle returned by `lynx_launcher_create`.
pub unsafe extern "C" fn lynx_launcher_destroy(launcher: *mut LynxLauncher) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !launcher.is_null() {
            unsafe { drop(Box::from_raw(launcher)) };
        }
    }));
}

#[no_mangle]
/// # Safety
/// `launcher` must be live, `out_list` writable, and `out_error` null or writable.
pub unsafe extern "C" fn lynx_launcher_get_applications(
    launcher: *const LynxLauncher,
    out_list: *mut *mut LynxAppList,
    out_error: *mut *mut LynxError,
) -> u32 {
    status_call(out_error, || {
        if launcher.is_null() || out_list.is_null() {
            return Err(FfiFailure::invalid(
                "launcher and out_list must not be null",
            ));
        }
        unsafe { out_list.write(ptr::null_mut()) };
        let launcher = unsafe { &*launcher };
        let applications = launcher
            .launcher
            .applications()
            .iter()
            .map(FfiApplication::from)
            .collect();
        unsafe { out_list.write(Box::into_raw(Box::new(LynxAppList { applications }))) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `list` must be null or a live handle returned by `lynx_launcher_get_applications`.
pub unsafe extern "C" fn lynx_app_list_destroy(list: *mut LynxAppList) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !list.is_null() {
            unsafe { drop(Box::from_raw(list)) };
        }
    }));
}

#[no_mangle]
/// # Safety
/// `list` must be null or a live `LynxAppList` handle.
pub unsafe extern "C" fn lynx_app_list_len(list: *const LynxAppList) -> usize {
    catch_unwind(AssertUnwindSafe(|| {
        if list.is_null() {
            0
        } else {
            unsafe { &*list }.applications.len()
        }
    }))
    .unwrap_or(0)
}

#[no_mangle]
/// # Safety
/// `list` must be live, `out_application` writable, and `out_error` null or writable.
pub unsafe extern "C" fn lynx_app_list_get(
    list: *const LynxAppList,
    index: usize,
    out_application: *mut LynxApplicationView,
    out_error: *mut *mut LynxError,
) -> u32 {
    status_call(out_error, || {
        if list.is_null() || out_application.is_null() {
            return Err(FfiFailure::invalid(
                "list and out_application must not be null",
            ));
        }
        unsafe { out_application.write(LynxApplicationView::default()) };
        let application = unsafe { &*list }
            .applications
            .get(index)
            .ok_or_else(|| FfiFailure::invalid("application index is out of range"))?;
        let view = LynxApplicationView {
            id: bytes(&application.id),
            name: bytes(&application.name),
            icon: application.icon.as_deref().map(bytes).unwrap_or_default(),
        };
        unsafe { out_application.write(view) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// All handles must be live, outputs must be writable, and the input slice must
/// reference readable memory for the duration of the call.
pub unsafe extern "C" fn lynx_launcher_resolve_icon(
    launcher: *const LynxLauncher,
    application_id: LynxSlice,
    size: u32,
    out_icon: *mut *mut LynxIcon,
    out_error: *mut *mut LynxError,
) -> u32 {
    status_call(out_error, || {
        if launcher.is_null() || out_icon.is_null() {
            return Err(FfiFailure::invalid(
                "launcher and out_icon must not be null",
            ));
        }
        unsafe { out_icon.write(ptr::null_mut()) };
        let id = unsafe { input(application_id) }?;
        let launcher = unsafe { &*launcher };
        if let Some(path) = launcher
            .launcher
            .resolve_icon(&id, size)
            .map_err(FfiFailure::from)?
        {
            let icon = LynxIcon {
                path: path.to_string_lossy().as_bytes().to_vec(),
            };
            unsafe { out_icon.write(Box::into_raw(Box::new(icon))) };
        }
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `icon` must be null or a live `LynxIcon` handle.
pub unsafe extern "C" fn lynx_icon_path(icon: *const LynxIcon) -> LynxSlice {
    catch_unwind(AssertUnwindSafe(|| {
        if icon.is_null() {
            LynxSlice::default()
        } else {
            bytes(&unsafe { &*icon }.path)
        }
    }))
    .unwrap_or_default()
}

#[no_mangle]
/// # Safety
/// `icon` must be null or a live handle returned by `lynx_launcher_resolve_icon`.
pub unsafe extern "C" fn lynx_icon_destroy(icon: *mut LynxIcon) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !icon.is_null() {
            unsafe { drop(Box::from_raw(icon)) };
        }
    }));
}

#[no_mangle]
/// # Safety
/// `launcher` must be live, `out_error` null or writable, and the input slice
/// must reference readable memory for the duration of the call.
pub unsafe extern "C" fn lynx_launcher_launch(
    launcher: *const LynxLauncher,
    application_id: LynxSlice,
    out_error: *mut *mut LynxError,
) -> u32 {
    status_call(out_error, || {
        if launcher.is_null() {
            return Err(FfiFailure::invalid("launcher must not be null"));
        }
        let id = unsafe { input(application_id) }?;
        let launcher = unsafe { &*launcher };
        launcher.launcher.launch(&id).map_err(FfiFailure::from)?;
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `error` must be null or a live `LynxError` handle.
pub unsafe extern "C" fn lynx_error_message(error: *const LynxError) -> LynxSlice {
    catch_unwind(AssertUnwindSafe(|| {
        if error.is_null() {
            LynxSlice::default()
        } else {
            bytes(&unsafe { &*error }.message)
        }
    }))
    .unwrap_or_default()
}

#[no_mangle]
/// # Safety
/// `error` must be null or a live error returned through an `out_error` argument.
pub unsafe extern "C" fn lynx_error_destroy(error: *mut LynxError) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !error.is_null() {
            unsafe { drop(Box::from_raw(error)) };
        }
    }));
}
