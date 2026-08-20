#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("lynx-sys supports only the verified Linux x86_64 SDK");

use std::ffi::{c_char, c_int, c_void, CStr, OsStr};
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

pub type LynxLogCallback = Option<unsafe extern "C" fn(c_int, *const c_char, *const c_char)>;

#[repr(C)]
pub struct LynxTaskRunner {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LynxWindowlessRenderer {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LynxViewBuilder {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LynxView {
    _private: [u8; 0],
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct LynxTask {
    pub runner: *mut LynxTaskRunner,
    pub task: u64,
}

pub type LynxUiRunsOnCurrentThreadCallback = Option<unsafe extern "C" fn(*mut c_void) -> bool>;
pub type LynxUiPostTaskCallback = Option<unsafe extern "C" fn(LynxTask, u64, *mut c_void)>;

#[repr(C)]
pub struct LynxUiTaskRunnerConfig {
    pub struct_size: usize,
    pub user_data: *mut c_void,
    pub runs_on_current_thread_callback: LynxUiRunsOnCurrentThreadCallback,
    pub post_task_callback: LynxUiPostTaskCallback,
}

pub type LynxRendererFinalizer =
    Option<unsafe extern "C" fn(*mut LynxWindowlessRenderer, *mut c_void)>;
pub type LynxGlMakeCurrentCallback =
    Option<unsafe extern "C" fn(*mut LynxWindowlessRenderer) -> bool>;
pub type LynxGlClearCurrentCallback =
    Option<unsafe extern "C" fn(*mut LynxWindowlessRenderer) -> bool>;
pub type LynxGlPresentCallback = Option<unsafe extern "C" fn(*mut LynxWindowlessRenderer) -> bool>;
pub type LynxGlCreateFboCallback =
    Option<unsafe extern "C" fn(*mut LynxWindowlessRenderer, c_int, c_int) -> u32>;
pub type LynxGlProcResolverCallback =
    Option<unsafe extern "C" fn(*mut LynxWindowlessRenderer, *const c_char) -> *mut c_void>;
pub type LynxRendererPostTaskCallback =
    Option<unsafe extern "C" fn(*mut LynxWindowlessRenderer, LynxTask, u64)>;

pub const LYNX_LOG_INFO: c_int = 2;
pub const LYNX_RENDERER_TYPE_GL_DIRECT: c_int = 2;

#[repr(C)]
struct DlInfo {
    filename: *const c_char,
    base: *mut c_void,
    symbol_name: *const c_char,
    symbol_address: *mut c_void,
}

unsafe extern "C" {
    pub fn lynx_log_init(callback: LynxLogCallback);
    pub fn lynx_log_set_minimum_level(level: c_int);

    pub fn lynx_windowless_set_global_ui_task_runner(config: *const LynxUiTaskRunnerConfig)
        -> bool;
    pub fn lynx_windowless_run_ui_task(task: LynxTask) -> bool;

    pub fn lynx_windowless_renderer_create_with_finalizer(
        renderer_type: c_int,
        user_data: *mut c_void,
        finalizer: LynxRendererFinalizer,
    ) -> *mut LynxWindowlessRenderer;
    pub fn lynx_windowless_renderer_get_user_data(
        renderer: *mut LynxWindowlessRenderer,
    ) -> *mut c_void;
    pub fn lynx_windowless_renderer_bind_on_gl_make_current(
        renderer: *mut LynxWindowlessRenderer,
        callback: LynxGlMakeCurrentCallback,
    );
    pub fn lynx_windowless_renderer_bind_on_gl_clear_current(
        renderer: *mut LynxWindowlessRenderer,
        callback: LynxGlClearCurrentCallback,
    );
    pub fn lynx_windowless_renderer_bind_on_gl_present(
        renderer: *mut LynxWindowlessRenderer,
        callback: LynxGlPresentCallback,
    );
    pub fn lynx_windowless_renderer_bind_on_gl_create_fbo(
        renderer: *mut LynxWindowlessRenderer,
        callback: LynxGlCreateFboCallback,
    );
    pub fn lynx_windowless_renderer_bind_on_gl_proc_resolver(
        renderer: *mut LynxWindowlessRenderer,
        callback: LynxGlProcResolverCallback,
    );
    pub fn lynx_windowless_renderer_bind_on_post_task(
        renderer: *mut LynxWindowlessRenderer,
        callback: LynxRendererPostTaskCallback,
    );
    pub fn lynx_windowless_renderer_run_task(renderer: *mut LynxWindowlessRenderer, task: LynxTask);
    pub fn lynx_windowless_renderer_release(renderer: *mut LynxWindowlessRenderer);

    pub fn lynx_sys_view_builder_set_screen_size(
        builder: *mut LynxViewBuilder,
        width: f32,
        height: f32,
        pixel_ratio: f32,
    );
    pub fn lynx_sys_view_builder_set_frame(
        builder: *mut LynxViewBuilder,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    );
    pub fn lynx_sys_view_builder_set_font_scale(builder: *mut LynxViewBuilder, scale: f32);
    pub fn lynx_sys_view_update_screen_metrics(
        view: *mut LynxView,
        width: f32,
        height: f32,
        pixel_ratio: f32,
    );
    pub fn lynx_sys_view_set_frame(view: *mut LynxView, x: f32, y: f32, width: f32, height: f32);
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
