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

#[repr(C)]
pub struct LynxGenericResourceFetcher {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LynxResourceRequest {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LynxResourceResponse {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LynxViewClient {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LynxLoadMeta {
    _private: [u8; 0],
}

#[repr(C)]
pub struct NapiEnvWeak {
    _private: [u8; 0],
}

#[repr(C)]
pub struct NapiValueWeak {
    _private: [u8; 0],
}

#[repr(C)]
pub struct NapiCallbackInfoWeak {
    _private: [u8; 0],
}

#[repr(C)]
pub struct NapiDeferredWeak {
    _private: [u8; 0],
}

pub type NapiEnv = *mut NapiEnvWeak;
pub type NapiValue = *mut NapiValueWeak;
pub type NapiCallbackInfo = *mut NapiCallbackInfoWeak;
pub type NapiDeferred = *mut NapiDeferredWeak;

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
pub type LynxResourceFetcherFinalizer =
    Option<unsafe extern "C" fn(*mut LynxGenericResourceFetcher, *mut c_void)>;
pub type LynxFetchResourceCallback = Option<
    unsafe extern "C" fn(
        *mut LynxGenericResourceFetcher,
        *mut LynxResourceRequest,
        *mut LynxResourceResponse,
    ),
>;
pub type LynxDataDestructor = Option<unsafe extern "C" fn(*mut u8, usize, *mut c_void)>;
pub type LynxViewClientCallback = Option<unsafe extern "C" fn(*mut LynxViewClient)>;
pub type LynxViewClientErrorCallback =
    Option<unsafe extern "C" fn(*mut LynxViewClient, c_int, *const c_char)>;
pub type NapiCallback = Option<unsafe extern "C" fn(NapiEnv, NapiCallbackInfo) -> NapiValue>;
pub type NapiModuleCreator =
    Option<unsafe extern "C" fn(NapiEnv, NapiValue, *const c_char, *mut c_void) -> NapiValue>;

pub const LYNX_LOG_INFO: c_int = 2;
pub const LYNX_RENDERER_TYPE_GL_DIRECT: c_int = 2;
pub const LYNX_RESOURCE_TYPE_LYNX_CORE_JS: c_int = 7;
pub const NAPI_OK: c_int = 0;
pub const NAPI_AUTO_LENGTH: usize = usize::MAX;

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

    pub fn lynx_generic_resource_fetcher_create_with_finalizer(
        user_data: *mut c_void,
        finalizer: LynxResourceFetcherFinalizer,
    ) -> *mut LynxGenericResourceFetcher;
    pub fn lynx_generic_resource_fetcher_get_user_data(
        fetcher: *mut LynxGenericResourceFetcher,
    ) -> *mut c_void;
    pub fn lynx_generic_resource_fetcher_bind_fetch_resource(
        fetcher: *mut LynxGenericResourceFetcher,
        callback: LynxFetchResourceCallback,
    );
    pub fn lynx_generic_resource_fetcher_bind_fetch_resource_path(
        fetcher: *mut LynxGenericResourceFetcher,
        callback: LynxFetchResourceCallback,
    );
    pub fn lynx_generic_resource_fetcher_release(fetcher: *mut LynxGenericResourceFetcher);

    pub fn lynx_resource_request_get_type(request: *mut LynxResourceRequest) -> c_int;
    pub fn lynx_resource_request_get_url(request: *mut LynxResourceRequest) -> *const c_char;
    pub fn lynx_resource_request_release(request: *mut LynxResourceRequest);

    pub fn lynx_resource_response_set_code(response: *mut LynxResourceResponse, code: c_int);
    pub fn lynx_resource_response_set_error_message(
        response: *mut LynxResourceResponse,
        message: *const c_char,
    );
    pub fn lynx_resource_response_set_data(
        response: *mut LynxResourceResponse,
        content: *mut u8,
        length: usize,
        destructor: LynxDataDestructor,
        opaque: *mut c_void,
    );
    pub fn lynx_resource_response_callback(response: *mut LynxResourceResponse);
    pub fn lynx_resource_response_release(response: *mut LynxResourceResponse);

    pub fn lynx_view_builder_create() -> *mut LynxViewBuilder;
    pub fn lynx_view_builder_set_enable_js_runtime(builder: *mut LynxViewBuilder, enable: bool);
    pub fn lynx_view_builder_set_icu_data_path(builder: *mut LynxViewBuilder, path: *const c_char);
    pub fn lynx_view_builder_set_windowless_renderer(
        builder: *mut LynxViewBuilder,
        renderer: *mut LynxWindowlessRenderer,
    );
    pub fn lynx_view_builder_set_generic_resource_fetcher(
        builder: *mut LynxViewBuilder,
        fetcher: *mut LynxGenericResourceFetcher,
    );
    pub fn lynx_view_builder_register_native_module(
        builder: *mut LynxViewBuilder,
        name: *const c_char,
        creator: NapiModuleCreator,
        opaque: *mut c_void,
    );
    pub fn lynx_view_builder_release(builder: *mut LynxViewBuilder);

    pub fn lynx_view_create(builder: *mut LynxViewBuilder, user_data: *mut c_void)
        -> *mut LynxView;
    pub fn lynx_view_add_client(view: *mut LynxView, client: *mut LynxViewClient);
    pub fn lynx_view_enter_foreground(view: *mut LynxView);
    pub fn lynx_view_enter_background(view: *mut LynxView);
    pub fn lynx_view_load_template(view: *mut LynxView, load_meta: *mut LynxLoadMeta);
    pub fn lynx_view_release(view: *mut LynxView);

    pub fn lynx_view_client_create(user_data: *mut c_void) -> *mut LynxViewClient;
    pub fn lynx_view_client_get_user_data(client: *mut LynxViewClient) -> *mut c_void;
    pub fn lynx_view_client_bind_on_first_screen(
        client: *mut LynxViewClient,
        callback: LynxViewClientCallback,
    );
    pub fn lynx_view_client_bind_on_received_error(
        client: *mut LynxViewClient,
        callback: LynxViewClientErrorCallback,
    );
    pub fn lynx_view_client_release(client: *mut LynxViewClient);

    pub fn lynx_load_meta_create() -> *mut LynxLoadMeta;
    pub fn lynx_load_meta_set_url(load_meta: *mut LynxLoadMeta, url: *const c_char);
    pub fn lynx_load_meta_set_binary_data(
        load_meta: *mut LynxLoadMeta,
        content: *mut u8,
        length: usize,
        destructor: LynxDataDestructor,
        opaque: *mut c_void,
    );
    pub fn lynx_load_meta_release(load_meta: *mut LynxLoadMeta);

    pub fn napi_get_undefined_weak(env: NapiEnv, result: *mut NapiValue) -> c_int;
    pub fn napi_create_object_weak(env: NapiEnv, result: *mut NapiValue) -> c_int;
    pub fn napi_create_array_with_length_weak(
        env: NapiEnv,
        length: usize,
        result: *mut NapiValue,
    ) -> c_int;
    pub fn napi_create_string_utf8_weak(
        env: NapiEnv,
        value: *const c_char,
        length: usize,
        result: *mut NapiValue,
    ) -> c_int;
    pub fn napi_create_function_weak(
        env: NapiEnv,
        name: *const c_char,
        length: usize,
        callback: NapiCallback,
        data: *mut c_void,
        result: *mut NapiValue,
    ) -> c_int;
    pub fn napi_create_error_weak(
        env: NapiEnv,
        code: NapiValue,
        message: NapiValue,
        result: *mut NapiValue,
    ) -> c_int;
    pub fn napi_set_named_property_weak(
        env: NapiEnv,
        object: NapiValue,
        name: *const c_char,
        value: NapiValue,
    ) -> c_int;
    pub fn napi_set_element_weak(
        env: NapiEnv,
        object: NapiValue,
        index: u32,
        value: NapiValue,
    ) -> c_int;
    pub fn napi_get_cb_info_weak(
        env: NapiEnv,
        info: NapiCallbackInfo,
        argument_count: *mut usize,
        arguments: *mut NapiValue,
        this_argument: *mut NapiValue,
        data: *mut *mut c_void,
    ) -> c_int;
    pub fn napi_throw_error_weak(
        env: NapiEnv,
        code: *const c_char,
        message: *const c_char,
    ) -> c_int;
    pub fn napi_create_promise_weak(
        env: NapiEnv,
        deferred: *mut NapiDeferred,
        promise: *mut NapiValue,
    ) -> c_int;
    pub fn napi_resolve_deferred_weak(
        env: NapiEnv,
        deferred: NapiDeferred,
        resolution: NapiValue,
    ) -> c_int;
    pub fn napi_reject_deferred_weak(
        env: NapiEnv,
        deferred: NapiDeferred,
        rejection: NapiValue,
    ) -> c_int;

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
