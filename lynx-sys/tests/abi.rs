use std::ffi::{c_char, c_int, c_void};
use std::mem::{align_of, offset_of, size_of};

use lynx_sys::{
    LynxDataDestructor, LynxFetchResourceCallback, LynxGlClearCurrentCallback,
    LynxGlCreateFboCallback, LynxGlMakeCurrentCallback, LynxGlPresentCallback,
    LynxGlProcResolverCallback, LynxKeyEvent, LynxLogCallback, LynxPointerEvent,
    LynxRendererFinalizer, LynxRendererPostTaskCallback, LynxResourceFetcherFinalizer,
    LynxShowTextInputCallback, LynxTask, LynxUiPostTaskCallback, LynxUiRunsOnCurrentThreadCallback,
    LynxUiTaskRunnerConfig, LynxViewClientCallback, LynxViewClientErrorCallback,
    NapiAsyncCompleteCallback, NapiAsyncExecuteCallback, NapiCallback, NapiModuleCreator,
    LYNX_KEY_EVENT_TYPE_DOWN, LYNX_KEY_EVENT_TYPE_REPEAT, LYNX_KEY_EVENT_TYPE_UP, LYNX_LOG_INFO,
    LYNX_POINTER_BUTTON_BACK, LYNX_POINTER_BUTTON_FORWARD, LYNX_POINTER_BUTTON_MIDDLE,
    LYNX_POINTER_BUTTON_PRIMARY, LYNX_POINTER_BUTTON_SECONDARY, LYNX_POINTER_DEVICE_KIND_MOUSE,
    LYNX_POINTER_PHASE_ADD, LYNX_POINTER_PHASE_CANCEL, LYNX_POINTER_PHASE_DOWN,
    LYNX_POINTER_PHASE_HOVER, LYNX_POINTER_PHASE_MOVE, LYNX_POINTER_PHASE_REMOVE,
    LYNX_POINTER_PHASE_UP, LYNX_POINTER_SIGNAL_KIND_NONE, LYNX_POINTER_SIGNAL_KIND_SCROLL,
    LYNX_RENDERER_TYPE_GL_DIRECT, LYNX_RESOURCE_TYPE_LYNX_CORE_JS, NAPI_AUTO_LENGTH, NAPI_OK,
};

#[test]
fn task_layout_matches_the_verified_sdk() {
    assert_eq!(size_of::<LynxTask>(), 16);
    assert_eq!(align_of::<LynxTask>(), 8);
    assert_eq!(offset_of!(LynxTask, runner), 0);
    assert_eq!(offset_of!(LynxTask, task), 8);
}

#[test]
fn ui_runner_config_layout_matches_the_verified_sdk() {
    assert_eq!(size_of::<LynxUiTaskRunnerConfig>(), 32);
    assert_eq!(align_of::<LynxUiTaskRunnerConfig>(), 8);
    assert_eq!(offset_of!(LynxUiTaskRunnerConfig, struct_size), 0);
    assert_eq!(offset_of!(LynxUiTaskRunnerConfig, user_data), 8);
    assert_eq!(
        offset_of!(LynxUiTaskRunnerConfig, runs_on_current_thread_callback),
        16
    );
    assert_eq!(offset_of!(LynxUiTaskRunnerConfig, post_task_callback), 24);
}

#[test]
fn input_event_layouts_match_the_verified_sdk() {
    assert_eq!(size_of::<LynxPointerEvent>(), 120);
    assert_eq!(align_of::<LynxPointerEvent>(), 8);
    assert_eq!(offset_of!(LynxPointerEvent, struct_size), 0);
    assert_eq!(offset_of!(LynxPointerEvent, phase), 8);
    assert_eq!(offset_of!(LynxPointerEvent, timestamp), 16);
    assert_eq!(offset_of!(LynxPointerEvent, x), 24);
    assert_eq!(offset_of!(LynxPointerEvent, y), 32);
    assert_eq!(offset_of!(LynxPointerEvent, device), 40);
    assert_eq!(offset_of!(LynxPointerEvent, signal_kind), 44);
    assert_eq!(offset_of!(LynxPointerEvent, scroll_delta_x), 48);
    assert_eq!(offset_of!(LynxPointerEvent, scroll_delta_y), 56);
    assert_eq!(offset_of!(LynxPointerEvent, device_kind), 64);
    assert_eq!(offset_of!(LynxPointerEvent, buttons), 72);
    assert_eq!(offset_of!(LynxPointerEvent, pan_x), 80);
    assert_eq!(offset_of!(LynxPointerEvent, pan_y), 88);
    assert_eq!(offset_of!(LynxPointerEvent, scale), 96);
    assert_eq!(offset_of!(LynxPointerEvent, rotation), 104);
    assert_eq!(offset_of!(LynxPointerEvent, is_precise_scroll), 112);

    assert_eq!(size_of::<LynxKeyEvent>(), 56);
    assert_eq!(align_of::<LynxKeyEvent>(), 8);
    assert_eq!(offset_of!(LynxKeyEvent, struct_size), 0);
    assert_eq!(offset_of!(LynxKeyEvent, timestamp), 8);
    assert_eq!(offset_of!(LynxKeyEvent, event_type), 16);
    assert_eq!(offset_of!(LynxKeyEvent, physical), 24);
    assert_eq!(offset_of!(LynxKeyEvent, logical), 32);
    assert_eq!(offset_of!(LynxKeyEvent, character), 40);
    assert_eq!(offset_of!(LynxKeyEvent, synthesized), 48);
}

#[test]
fn callbacks_and_constants_match_the_verified_sdk() {
    let _: LynxLogCallback = None::<unsafe extern "C" fn(c_int, *const c_char, *const c_char)>;
    let _: LynxUiRunsOnCurrentThreadCallback = None::<unsafe extern "C" fn(*mut c_void) -> bool>;
    let _: LynxUiPostTaskCallback = None::<unsafe extern "C" fn(LynxTask, u64, *mut c_void)>;
    let _: LynxRendererFinalizer =
        None::<unsafe extern "C" fn(*mut lynx_sys::LynxWindowlessRenderer, *mut c_void)>;
    let _: LynxGlMakeCurrentCallback =
        None::<unsafe extern "C" fn(*mut lynx_sys::LynxWindowlessRenderer) -> bool>;
    let _: LynxGlClearCurrentCallback =
        None::<unsafe extern "C" fn(*mut lynx_sys::LynxWindowlessRenderer) -> bool>;
    let _: LynxGlPresentCallback =
        None::<unsafe extern "C" fn(*mut lynx_sys::LynxWindowlessRenderer) -> bool>;
    let _: LynxGlCreateFboCallback =
        None::<unsafe extern "C" fn(*mut lynx_sys::LynxWindowlessRenderer, c_int, c_int) -> u32>;
    let _: LynxGlProcResolverCallback = None::<
        unsafe extern "C" fn(*mut lynx_sys::LynxWindowlessRenderer, *const c_char) -> *mut c_void,
    >;
    let _: LynxRendererPostTaskCallback =
        None::<unsafe extern "C" fn(*mut lynx_sys::LynxWindowlessRenderer, LynxTask, u64)>;
    let _: LynxShowTextInputCallback =
        None::<unsafe extern "C" fn(*mut lynx_sys::LynxWindowlessRenderer, bool)>;
    let _: LynxResourceFetcherFinalizer =
        None::<unsafe extern "C" fn(*mut lynx_sys::LynxGenericResourceFetcher, *mut c_void)>;
    let _: LynxFetchResourceCallback = None::<
        unsafe extern "C" fn(
            *mut lynx_sys::LynxGenericResourceFetcher,
            *mut lynx_sys::LynxResourceRequest,
            *mut lynx_sys::LynxResourceResponse,
        ),
    >;
    let _: LynxDataDestructor = None::<unsafe extern "C" fn(*mut u8, usize, *mut c_void)>;
    let _: LynxViewClientCallback = None::<unsafe extern "C" fn(*mut lynx_sys::LynxViewClient)>;
    let _: LynxViewClientErrorCallback =
        None::<unsafe extern "C" fn(*mut lynx_sys::LynxViewClient, c_int, *const c_char)>;
    let _: NapiCallback = None::<
        unsafe extern "C" fn(lynx_sys::NapiEnv, lynx_sys::NapiCallbackInfo) -> lynx_sys::NapiValue,
    >;
    let _: NapiModuleCreator = None::<
        unsafe extern "C" fn(
            lynx_sys::NapiEnv,
            lynx_sys::NapiValue,
            *const c_char,
            *mut c_void,
        ) -> lynx_sys::NapiValue,
    >;
    let _: NapiAsyncExecuteCallback = None::<unsafe extern "C" fn(lynx_sys::NapiEnv, *mut c_void)>;
    let _: NapiAsyncCompleteCallback =
        None::<unsafe extern "C" fn(lynx_sys::NapiEnv, c_int, *mut c_void)>;

    for callback_size in [
        size_of::<LynxLogCallback>(),
        size_of::<LynxUiRunsOnCurrentThreadCallback>(),
        size_of::<LynxUiPostTaskCallback>(),
        size_of::<LynxRendererFinalizer>(),
        size_of::<LynxGlMakeCurrentCallback>(),
        size_of::<LynxGlClearCurrentCallback>(),
        size_of::<LynxGlPresentCallback>(),
        size_of::<LynxGlCreateFboCallback>(),
        size_of::<LynxGlProcResolverCallback>(),
        size_of::<LynxRendererPostTaskCallback>(),
        size_of::<LynxShowTextInputCallback>(),
        size_of::<LynxResourceFetcherFinalizer>(),
        size_of::<LynxFetchResourceCallback>(),
        size_of::<LynxDataDestructor>(),
        size_of::<LynxViewClientCallback>(),
        size_of::<LynxViewClientErrorCallback>(),
        size_of::<NapiCallback>(),
        size_of::<NapiModuleCreator>(),
        size_of::<NapiAsyncExecuteCallback>(),
        size_of::<NapiAsyncCompleteCallback>(),
    ] {
        assert_eq!(callback_size, size_of::<*mut c_void>());
    }
    assert_eq!(size_of::<c_int>(), 4);
    assert_eq!(LYNX_LOG_INFO, 2);
    assert_eq!(LYNX_RENDERER_TYPE_GL_DIRECT, 2);
    assert_eq!(LYNX_RESOURCE_TYPE_LYNX_CORE_JS, 7);
    assert_eq!(LYNX_POINTER_PHASE_CANCEL, 0);
    assert_eq!(LYNX_POINTER_PHASE_UP, 1);
    assert_eq!(LYNX_POINTER_PHASE_DOWN, 2);
    assert_eq!(LYNX_POINTER_PHASE_MOVE, 3);
    assert_eq!(LYNX_POINTER_PHASE_ADD, 4);
    assert_eq!(LYNX_POINTER_PHASE_REMOVE, 5);
    assert_eq!(LYNX_POINTER_PHASE_HOVER, 6);
    assert_eq!(LYNX_POINTER_SIGNAL_KIND_NONE, 0);
    assert_eq!(LYNX_POINTER_SIGNAL_KIND_SCROLL, 1);
    assert_eq!(LYNX_POINTER_DEVICE_KIND_MOUSE, 1);
    assert_eq!(LYNX_POINTER_BUTTON_PRIMARY, 1);
    assert_eq!(LYNX_POINTER_BUTTON_SECONDARY, 2);
    assert_eq!(LYNX_POINTER_BUTTON_MIDDLE, 4);
    assert_eq!(LYNX_POINTER_BUTTON_BACK, 8);
    assert_eq!(LYNX_POINTER_BUTTON_FORWARD, 16);
    assert_eq!(LYNX_KEY_EVENT_TYPE_UP, 1);
    assert_eq!(LYNX_KEY_EVENT_TYPE_DOWN, 2);
    assert_eq!(LYNX_KEY_EVENT_TYPE_REPEAT, 3);
    assert_eq!(NAPI_OK, 0);
    assert_eq!(NAPI_AUTO_LENGTH, usize::MAX);
}

#[test]
fn runtime_function_declarations_match_the_reviewed_signatures() {
    use lynx_sys::*;

    let _: unsafe extern "C" fn(LynxLogCallback) = lynx_log_init;
    let _: unsafe extern "C" fn(c_int) = lynx_log_set_minimum_level;
    let _: unsafe extern "C" fn(*const LynxUiTaskRunnerConfig) -> bool =
        lynx_windowless_set_global_ui_task_runner;
    let _: unsafe extern "C" fn(LynxTask) -> bool = lynx_windowless_run_ui_task;
    let _: unsafe extern "C" fn(
        c_int,
        *mut c_void,
        LynxRendererFinalizer,
    ) -> *mut LynxWindowlessRenderer = lynx_windowless_renderer_create_with_finalizer;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer) -> *mut c_void =
        lynx_windowless_renderer_get_user_data;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, LynxGlMakeCurrentCallback) =
        lynx_windowless_renderer_bind_on_gl_make_current;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, LynxGlClearCurrentCallback) =
        lynx_windowless_renderer_bind_on_gl_clear_current;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, LynxGlPresentCallback) =
        lynx_windowless_renderer_bind_on_gl_present;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, LynxGlCreateFboCallback) =
        lynx_windowless_renderer_bind_on_gl_create_fbo;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, LynxGlProcResolverCallback) =
        lynx_windowless_renderer_bind_on_gl_proc_resolver;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, LynxRendererPostTaskCallback) =
        lynx_windowless_renderer_bind_on_post_task;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, LynxTask) =
        lynx_windowless_renderer_run_task;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, *mut LynxPointerEvent) =
        lynx_windowless_renderer_send_pointer_event;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, *mut LynxKeyEvent) =
        lynx_windowless_renderer_send_key_event;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer, LynxShowTextInputCallback) =
        lynx_windowless_renderer_bind_show_text_input;
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer) = lynx_windowless_renderer_release;

    let _: unsafe extern "C" fn(
        *mut c_void,
        LynxResourceFetcherFinalizer,
    ) -> *mut LynxGenericResourceFetcher = lynx_generic_resource_fetcher_create_with_finalizer;
    let _: unsafe extern "C" fn(*mut LynxGenericResourceFetcher) -> *mut c_void =
        lynx_generic_resource_fetcher_get_user_data;
    let _: unsafe extern "C" fn(*mut LynxGenericResourceFetcher, LynxFetchResourceCallback) =
        lynx_generic_resource_fetcher_bind_fetch_resource;
    let _: unsafe extern "C" fn(*mut LynxGenericResourceFetcher, LynxFetchResourceCallback) =
        lynx_generic_resource_fetcher_bind_fetch_resource_path;
    let _: unsafe extern "C" fn(*mut LynxGenericResourceFetcher) =
        lynx_generic_resource_fetcher_release;
    let _: unsafe extern "C" fn(*mut LynxResourceRequest) -> c_int = lynx_resource_request_get_type;
    let _: unsafe extern "C" fn(*mut LynxResourceRequest) -> *const c_char =
        lynx_resource_request_get_url;
    let _: unsafe extern "C" fn(*mut LynxResourceRequest) = lynx_resource_request_release;
    let _: unsafe extern "C" fn(*mut LynxResourceResponse, c_int) = lynx_resource_response_set_code;
    let _: unsafe extern "C" fn(*mut LynxResourceResponse, *const c_char) =
        lynx_resource_response_set_error_message;
    let _: unsafe extern "C" fn(
        *mut LynxResourceResponse,
        *mut u8,
        usize,
        LynxDataDestructor,
        *mut c_void,
    ) = lynx_resource_response_set_data;
    let _: unsafe extern "C" fn(*mut LynxResourceResponse) = lynx_resource_response_callback;
    let _: unsafe extern "C" fn(*mut LynxResourceResponse) = lynx_resource_response_release;

    let _: unsafe extern "C" fn() -> *mut LynxViewBuilder = lynx_view_builder_create;
    let _: unsafe extern "C" fn(*mut LynxViewBuilder, bool) =
        lynx_view_builder_set_enable_js_runtime;
    let _: unsafe extern "C" fn(*mut LynxViewBuilder, *const c_char) =
        lynx_view_builder_set_icu_data_path;
    let _: unsafe extern "C" fn(*mut LynxViewBuilder, *mut LynxWindowlessRenderer) =
        lynx_view_builder_set_windowless_renderer;
    let _: unsafe extern "C" fn(*mut LynxViewBuilder, *mut LynxGenericResourceFetcher) =
        lynx_view_builder_set_generic_resource_fetcher;
    let _: unsafe extern "C" fn(
        *mut LynxViewBuilder,
        *const c_char,
        NapiModuleCreator,
        *mut c_void,
    ) = lynx_view_builder_register_native_module;
    let _: unsafe extern "C" fn(*mut LynxViewBuilder) = lynx_view_builder_release;
    let _: unsafe extern "C" fn(*mut LynxViewBuilder, *mut c_void) -> *mut LynxView =
        lynx_view_create;
    let _: unsafe extern "C" fn(*mut LynxView, *mut LynxViewClient) = lynx_view_add_client;
    let _: unsafe extern "C" fn(*mut LynxView) = lynx_view_enter_foreground;
    let _: unsafe extern "C" fn(*mut LynxView) = lynx_view_enter_background;
    let _: unsafe extern "C" fn(*mut LynxView, *mut LynxLoadMeta) = lynx_view_load_template;
    let _: unsafe extern "C" fn(*mut LynxView) = lynx_view_release;
    let _: unsafe extern "C" fn(*mut c_void) -> *mut LynxViewClient = lynx_view_client_create;
    let _: unsafe extern "C" fn(*mut LynxViewClient) -> *mut c_void =
        lynx_view_client_get_user_data;
    let _: unsafe extern "C" fn(*mut LynxViewClient, LynxViewClientCallback) =
        lynx_view_client_bind_on_first_screen;
    let _: unsafe extern "C" fn(*mut LynxViewClient, LynxViewClientErrorCallback) =
        lynx_view_client_bind_on_received_error;
    let _: unsafe extern "C" fn(*mut LynxViewClient) = lynx_view_client_release;
    let _: unsafe extern "C" fn() -> *mut LynxLoadMeta = lynx_load_meta_create;
    let _: unsafe extern "C" fn(*mut LynxLoadMeta, *const c_char) = lynx_load_meta_set_url;
    let _: unsafe extern "C" fn(
        *mut LynxLoadMeta,
        *mut u8,
        usize,
        LynxDataDestructor,
        *mut c_void,
    ) = lynx_load_meta_set_binary_data;
    let _: unsafe extern "C" fn(*mut LynxLoadMeta) = lynx_load_meta_release;

    let _: unsafe extern "C" fn(NapiEnv, *mut NapiValue) -> c_int = napi_get_undefined_weak;
    let _: unsafe extern "C" fn(NapiEnv, *mut NapiValue) -> c_int = napi_create_object_weak;
    let _: unsafe extern "C" fn(NapiEnv, usize, *mut NapiValue) -> c_int =
        napi_create_array_with_length_weak;
    let _: unsafe extern "C" fn(NapiEnv, *const c_char, usize, *mut NapiValue) -> c_int =
        napi_create_string_utf8_weak;
    let _: unsafe extern "C" fn(NapiEnv, NapiValue, *mut c_char, usize, *mut usize) -> c_int =
        napi_get_value_string_utf8_weak;
    let _: unsafe extern "C" fn(
        NapiEnv,
        *const c_char,
        usize,
        NapiCallback,
        *mut c_void,
        *mut NapiValue,
    ) -> c_int = napi_create_function_weak;
    let _: unsafe extern "C" fn(NapiEnv, NapiValue, NapiValue, *mut NapiValue) -> c_int =
        napi_create_error_weak;
    let _: unsafe extern "C" fn(NapiEnv, NapiValue, *const c_char, NapiValue) -> c_int =
        napi_set_named_property_weak;
    let _: unsafe extern "C" fn(NapiEnv, NapiValue, u32, NapiValue) -> c_int =
        napi_set_element_weak;
    let _: unsafe extern "C" fn(
        NapiEnv,
        NapiCallbackInfo,
        *mut usize,
        *mut NapiValue,
        *mut NapiValue,
        *mut *mut c_void,
    ) -> c_int = napi_get_cb_info_weak;
    let _: unsafe extern "C" fn(NapiEnv, *const c_char, *const c_char) -> c_int =
        napi_throw_error_weak;
    let _: unsafe extern "C" fn(NapiEnv, *mut NapiDeferred, *mut NapiValue) -> c_int =
        napi_create_promise_weak;
    let _: unsafe extern "C" fn(NapiEnv, NapiDeferred, NapiValue) -> c_int =
        napi_resolve_deferred_weak;
    let _: unsafe extern "C" fn(NapiEnv, NapiDeferred, NapiValue) -> c_int =
        napi_reject_deferred_weak;
    let _: unsafe extern "C" fn(
        NapiEnv,
        NapiValue,
        NapiValue,
        NapiAsyncExecuteCallback,
        NapiAsyncCompleteCallback,
        *mut c_void,
        *mut NapiAsyncWork,
    ) -> c_int = napi_create_async_work_weak;
    let _: unsafe extern "C" fn(NapiEnv, NapiAsyncWork) -> c_int = napi_delete_async_work_weak;
    let _: unsafe extern "C" fn(NapiEnv, NapiAsyncWork) -> c_int = napi_queue_async_work_weak;
}

#[cfg(lynx_sys_cmake_link)]
#[test]
fn declarations_link_against_the_shim_and_verified_sdk() {
    use std::hint::black_box;

    use lynx_sys::*;

    let _: unsafe extern "C" fn(*mut LynxViewBuilder, f32, f32, f32) =
        lynx_sys_view_builder_set_screen_size;
    let _: unsafe extern "C" fn(*mut LynxViewBuilder, f32, f32, f32, f32) =
        lynx_sys_view_builder_set_frame;
    let _: unsafe extern "C" fn(*mut LynxViewBuilder, f32) = lynx_sys_view_builder_set_font_scale;
    let _: unsafe extern "C" fn(*mut LynxView, f32, f32, f32) = lynx_sys_view_update_screen_metrics;
    let _: unsafe extern "C" fn(*mut LynxView, f32, f32, f32, f32) = lynx_sys_view_set_frame;

    let symbols = [
        lynx_log_init as *const (),
        lynx_log_set_minimum_level as *const (),
        lynx_windowless_set_global_ui_task_runner as *const (),
        lynx_windowless_run_ui_task as *const (),
        lynx_windowless_renderer_create_with_finalizer as *const (),
        lynx_windowless_renderer_get_user_data as *const (),
        lynx_windowless_renderer_bind_on_gl_make_current as *const (),
        lynx_windowless_renderer_bind_on_gl_clear_current as *const (),
        lynx_windowless_renderer_bind_on_gl_present as *const (),
        lynx_windowless_renderer_bind_on_gl_create_fbo as *const (),
        lynx_windowless_renderer_bind_on_gl_proc_resolver as *const (),
        lynx_windowless_renderer_bind_on_post_task as *const (),
        lynx_windowless_renderer_run_task as *const (),
        lynx_windowless_renderer_send_pointer_event as *const (),
        lynx_windowless_renderer_send_key_event as *const (),
        lynx_windowless_renderer_bind_show_text_input as *const (),
        lynx_windowless_renderer_release as *const (),
        lynx_generic_resource_fetcher_create_with_finalizer as *const (),
        lynx_generic_resource_fetcher_get_user_data as *const (),
        lynx_generic_resource_fetcher_bind_fetch_resource as *const (),
        lynx_generic_resource_fetcher_bind_fetch_resource_path as *const (),
        lynx_generic_resource_fetcher_release as *const (),
        lynx_resource_request_get_type as *const (),
        lynx_resource_request_get_url as *const (),
        lynx_resource_request_release as *const (),
        lynx_resource_response_set_code as *const (),
        lynx_resource_response_set_error_message as *const (),
        lynx_resource_response_set_data as *const (),
        lynx_resource_response_callback as *const (),
        lynx_resource_response_release as *const (),
        lynx_view_builder_create as *const (),
        lynx_view_builder_set_enable_js_runtime as *const (),
        lynx_view_builder_set_icu_data_path as *const (),
        lynx_view_builder_set_windowless_renderer as *const (),
        lynx_view_builder_set_generic_resource_fetcher as *const (),
        lynx_view_builder_register_native_module as *const (),
        lynx_view_builder_release as *const (),
        lynx_view_create as *const (),
        lynx_view_add_client as *const (),
        lynx_view_enter_foreground as *const (),
        lynx_view_enter_background as *const (),
        lynx_view_load_template as *const (),
        lynx_view_release as *const (),
        lynx_view_client_create as *const (),
        lynx_view_client_get_user_data as *const (),
        lynx_view_client_bind_on_first_screen as *const (),
        lynx_view_client_bind_on_received_error as *const (),
        lynx_view_client_release as *const (),
        lynx_load_meta_create as *const (),
        lynx_load_meta_set_url as *const (),
        lynx_load_meta_set_binary_data as *const (),
        lynx_load_meta_release as *const (),
        napi_get_undefined_weak as *const (),
        napi_create_object_weak as *const (),
        napi_create_array_with_length_weak as *const (),
        napi_create_string_utf8_weak as *const (),
        napi_get_value_string_utf8_weak as *const (),
        napi_create_function_weak as *const (),
        napi_create_error_weak as *const (),
        napi_set_named_property_weak as *const (),
        napi_set_element_weak as *const (),
        napi_get_cb_info_weak as *const (),
        napi_throw_error_weak as *const (),
        napi_create_promise_weak as *const (),
        napi_resolve_deferred_weak as *const (),
        napi_reject_deferred_weak as *const (),
        napi_create_async_work_weak as *const (),
        napi_delete_async_work_weak as *const (),
        napi_queue_async_work_weak as *const (),
        lynx_sys_view_builder_set_screen_size as *const (),
        lynx_sys_view_builder_set_frame as *const (),
        lynx_sys_view_builder_set_font_scale as *const (),
        lynx_sys_view_update_screen_metrics as *const (),
        lynx_sys_view_set_frame as *const (),
    ];

    assert!(symbols
        .into_iter()
        .map(black_box)
        .all(|symbol| !symbol.is_null()));
}
