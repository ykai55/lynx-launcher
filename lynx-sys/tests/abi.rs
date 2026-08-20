use std::ffi::{c_char, c_int, c_void};
use std::mem::{align_of, offset_of, size_of};

use lynx_sys::{
    LynxGlClearCurrentCallback, LynxGlCreateFboCallback, LynxGlMakeCurrentCallback,
    LynxGlPresentCallback, LynxGlProcResolverCallback, LynxLogCallback, LynxRendererFinalizer,
    LynxRendererPostTaskCallback, LynxTask, LynxUiPostTaskCallback,
    LynxUiRunsOnCurrentThreadCallback, LynxUiTaskRunnerConfig, LYNX_LOG_INFO,
    LYNX_RENDERER_TYPE_GL_DIRECT,
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
    ] {
        assert_eq!(callback_size, size_of::<*mut c_void>());
    }
    assert_eq!(size_of::<c_int>(), 4);
    assert_eq!(LYNX_LOG_INFO, 2);
    assert_eq!(LYNX_RENDERER_TYPE_GL_DIRECT, 2);
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
    let _: unsafe extern "C" fn(*mut LynxWindowlessRenderer) = lynx_windowless_renderer_release;
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
        lynx_windowless_renderer_release as *const (),
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
