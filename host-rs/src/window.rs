mod ffi;

use std::cell::{Cell, RefCell};
use std::ffi::{c_void, CStr, CString};
use std::io::{self, Write};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lynx_launcher_host::runtime::{DesktopApi, EventWake, GlApi, RuntimeCore, RuntimeViewOptions};
use lynx_launcher_host::support::{
    calculate_window_metrics, logical_key, physical_key, utf8_from_codepoint,
    xsettings_window_scale, InputState, PointerDispatch, PressedKey, WindowMetrics,
};
use lynx_launcher_host::{verify_linked_lynx, WindowRunOptions};
use lynx_sys::{
    LynxKeyEvent, LynxPointerEvent, LYNX_KEY_EVENT_TYPE_DOWN, LYNX_KEY_EVENT_TYPE_REPEAT,
    LYNX_KEY_EVENT_TYPE_UP, LYNX_POINTER_BUTTON_BACK, LYNX_POINTER_BUTTON_FORWARD,
    LYNX_POINTER_BUTTON_MIDDLE, LYNX_POINTER_BUTTON_PRIMARY, LYNX_POINTER_BUTTON_SECONDARY,
    LYNX_POINTER_DEVICE_KIND_MOUSE,
};

const INITIAL_WIDTH: f32 = 1120.0;
const INITIAL_HEIGHT: f32 = 760.0;
const MAXIMUM_EVENT_WAIT: Duration = Duration::from_millis(250);
const STRANDED_GL_LOG: &[u8] = b"[host-rs] fatal: renderer stranded an OpenGL context; \
skipping glfwDestroyWindow/glfwTerminate and exiting for OS cleanup\n";
const NATIVE_CALLBACKS_LOG: &[u8] = b"[host-rs] fatal: native callbacks did not quiesce; \
skipping glfwDestroyWindow/glfwTerminate and exiting for OS cleanup\n";

static CALLBACK_PANICKED: AtomicBool = AtomicBool::new(false);
static STARTUP_FOCUS_LOST: AtomicBool = AtomicBool::new(false);

struct GlfwRuntime {
    _thread_bound: PhantomData<Rc<()>>,
}

impl GlfwRuntime {
    fn initialize() -> io::Result<Self> {
        CALLBACK_PANICKED.store(false, Ordering::Release);
        STARTUP_FOCUS_LOST.store(false, Ordering::Release);
        unsafe { ffi::glfw_set_error_callback(Some(glfw_error)) };
        if unsafe { ffi::glfw_init() } == ffi::GLFW_FALSE {
            unsafe { ffi::glfw_set_error_callback(None) };
            return Err(io::Error::other("GLFW initialization failed"));
        }
        let runtime = Self {
            _thread_bound: PhantomData,
        };
        ensure_callback_did_not_panic()?;
        Ok(runtime)
    }
}

impl Drop for GlfwRuntime {
    fn drop(&mut self) {
        unsafe {
            ffi::glfw_terminate();
            ffi::glfw_set_error_callback(None);
        }
    }
}

struct PopupWindow {
    raw: NonNull<ffi::GlfwWindow>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl PopupWindow {
    fn create(width: i32, height: i32) -> io::Result<Self> {
        unsafe {
            ffi::glfw_window_hint(ffi::GLFW_CONTEXT_VERSION_MAJOR, 3);
            ffi::glfw_window_hint(ffi::GLFW_CONTEXT_VERSION_MINOR, 3);
            ffi::glfw_window_hint(ffi::GLFW_OPENGL_PROFILE, ffi::GLFW_OPENGL_CORE_PROFILE);
            ffi::glfw_window_hint(ffi::GLFW_OPENGL_FORWARD_COMPAT, ffi::GLFW_TRUE);
            ffi::glfw_window_hint(ffi::GLFW_VISIBLE, ffi::GLFW_FALSE);
            ffi::glfw_window_hint(ffi::GLFW_DECORATED, ffi::GLFW_FALSE);
            ffi::glfw_window_hint(ffi::GLFW_RESIZABLE, ffi::GLFW_FALSE);
            ffi::glfw_window_hint(ffi::GLFW_FLOATING, ffi::GLFW_TRUE);
            ffi::glfw_window_hint(ffi::GLFW_FOCUS_ON_SHOW, ffi::GLFW_TRUE);
        }
        let raw = unsafe {
            ffi::glfw_create_window(
                width,
                height,
                c"Lynx Launcher".as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        let raw = NonNull::new(raw)
            .ok_or_else(|| io::Error::other("could not create an X11 OpenGL 3.3 GLFW window"))?;
        Ok(Self {
            raw,
            _thread_bound: PhantomData,
        })
    }

    fn raw(&self) -> *mut ffi::GlfwWindow {
        self.raw.as_ptr()
    }
}

impl Drop for PopupWindow {
    fn drop(&mut self) {
        unsafe {
            clear_window_callbacks(self.raw());
            if ffi::glfw_get_current_context() == self.raw() {
                ffi::glfw_make_context_current(std::ptr::null_mut());
            }
            ffi::glfw_destroy_window(self.raw());
        }
    }
}

struct WindowState {
    window: NonNull<ffi::GlfwWindow>,
    runtime: RefCell<RuntimeCore>,
    input: RefCell<InputState>,
    accepting_input: Cell<bool>,
    metrics: Cell<WindowMetrics>,
    system_scale: f32,
    callback_failed: AtomicBool,
    input_trace: bool,
    ignore_focus_loss: bool,
}

impl WindowState {
    fn new(
        window: NonNull<ffi::GlfwWindow>,
        runtime: RuntimeCore,
        metrics: WindowMetrics,
        system_scale: f32,
        ignore_focus_loss: bool,
    ) -> Self {
        Self {
            window,
            runtime: RefCell::new(runtime),
            input: RefCell::new(InputState::default()),
            accepting_input: Cell::new(true),
            metrics: Cell::new(metrics),
            system_scale,
            callback_failed: AtomicBool::new(false),
            input_trace: std::env::var_os("LYNX_LAUNCHER_E2E_SNAPSHOT_TRACE").as_deref()
                == Some(std::ffi::OsStr::new("1")),
            ignore_focus_loss,
        }
    }

    fn register_callbacks(&self) {
        let user_data = (self as *const Self).cast_mut().cast();
        unsafe {
            ffi::glfw_set_window_user_pointer(self.window.as_ptr(), user_data);
            ffi::glfw_set_cursor_position_callback(
                self.window.as_ptr(),
                Some(cursor_position_callback),
            );
            ffi::glfw_set_cursor_enter_callback(self.window.as_ptr(), Some(cursor_enter_callback));
            ffi::glfw_set_mouse_button_callback(self.window.as_ptr(), Some(mouse_button_callback));
            ffi::glfw_set_scroll_callback(self.window.as_ptr(), Some(scroll_callback));
            ffi::glfw_set_key_callback(self.window.as_ptr(), Some(key_callback));
            ffi::glfw_set_char_callback(self.window.as_ptr(), Some(character_callback));
            ffi::glfw_set_framebuffer_size_callback(
                self.window.as_ptr(),
                Some(framebuffer_size_callback),
            );
            ffi::glfw_set_window_size_callback(self.window.as_ptr(), Some(window_size_callback));
            ffi::glfw_set_window_content_scale_callback(
                self.window.as_ptr(),
                Some(content_scale_callback),
            );
            ffi::glfw_set_window_focus_callback(self.window.as_ptr(), Some(window_focus_callback));
        }
    }

    fn unregister_callbacks(&self) {
        unsafe { clear_window_callbacks(self.window.as_ptr()) };
    }

    fn ensure_healthy(&self) -> io::Result<()> {
        if self.callback_failed.load(Ordering::Acquire) {
            return Err(io::Error::other("a GLFW window callback failed"));
        }
        self.runtime
            .try_borrow()
            .map_err(|_| io::Error::other("the Lynx runtime is already borrowed"))?
            .ensure_healthy()
    }

    fn record_callback_failure(&self) {
        self.accepting_input.set(false);
        self.callback_failed.store(true, Ordering::Release);
        unsafe {
            ffi::glfw_set_window_should_close(self.window.as_ptr(), ffi::GLFW_TRUE);
            ffi::glfw_post_empty_event();
        }
    }

    fn update_metrics(&self) -> io::Result<()> {
        let Some(metrics) = window_metrics(self.window.as_ptr(), self.system_scale)? else {
            return Ok(());
        };
        self.runtime
            .try_borrow()
            .map_err(|_| io::Error::other("the Lynx runtime is already borrowed"))?
            .update_view_metrics(
                metrics.logical_width,
                metrics.logical_height,
                metrics.pixel_ratio,
            )?;
        self.metrics.set(metrics);
        log_metrics(metrics);
        Ok(())
    }

    fn send_pointer_events(&self, events: Vec<PointerDispatch>) -> io::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let (cursor_x, cursor_y) = self
            .input
            .try_borrow()
            .map_err(|_| io::Error::other("the input state is already borrowed"))?
            .cursor_position();
        let metrics = self.metrics.get();
        let runtime = self
            .runtime
            .try_borrow()
            .map_err(|_| io::Error::other("the Lynx runtime is already borrowed"))?;
        for dispatch in events {
            let mut event = LynxPointerEvent {
                struct_size: std::mem::size_of::<LynxPointerEvent>(),
                phase: dispatch.phase,
                timestamp: 0,
                x: cursor_x * f64::from(metrics.framebuffer_scale_x),
                y: cursor_y * f64::from(metrics.framebuffer_scale_y),
                device: 0,
                signal_kind: dispatch.signal_kind,
                scroll_delta_x: dispatch.scroll_delta_x * f64::from(metrics.pixel_ratio),
                scroll_delta_y: dispatch.scroll_delta_y * f64::from(metrics.pixel_ratio),
                device_kind: LYNX_POINTER_DEVICE_KIND_MOUSE,
                buttons: dispatch.buttons,
                pan_x: 0.0,
                pan_y: 0.0,
                scale: 1.0,
                rotation: 0.0,
                is_precise_scroll: 0,
            };
            runtime.send_pointer_event(&mut event)?;
            if self.input_trace {
                eprintln!(
                    "[host-rs] pointer dispatch phase={} signal={} x={} y={} scroll-logical-y={} scroll-physical-y={} buttons={}",
                    pointer_phase_name(dispatch.phase),
                    pointer_signal_name(dispatch.signal_kind),
                    event.x,
                    event.y,
                    dispatch.scroll_delta_y,
                    event.scroll_delta_y,
                    event.buttons
                );
            }
        }
        Ok(())
    }

    fn send_key(
        &self,
        event_type: i32,
        key: PressedKey,
        character: *const std::ffi::c_char,
        synthesized: bool,
    ) -> io::Result<()> {
        let mut event = LynxKeyEvent {
            struct_size: std::mem::size_of::<LynxKeyEvent>(),
            timestamp: 0.0,
            event_type,
            physical: key.physical,
            logical: key.logical,
            character,
            synthesized,
        };
        self.runtime
            .try_borrow()
            .map_err(|_| io::Error::other("the Lynx runtime is already borrowed"))?
            .send_key_event(&mut event)?;
        if self.input_trace {
            eprintln!(
                "[host-rs] key dispatch type={} physical={} logical={} synthesized={}",
                key_event_type_name(event_type),
                key.physical,
                key.logical,
                synthesized
            );
        }
        Ok(())
    }

    fn cancel_input(&self) -> io::Result<()> {
        let (keys, pointer_events) = self
            .input
            .try_borrow_mut()
            .map_err(|_| io::Error::other("the input state is already borrowed"))?
            .cancel();
        {
            let runtime = self
                .runtime
                .try_borrow()
                .map_err(|_| io::Error::other("the Lynx runtime is already borrowed"))?;
            runtime.cancel_text_input();
        }
        for key in keys {
            self.send_key(LYNX_KEY_EVENT_TYPE_UP, key, std::ptr::null(), true)?;
        }
        self.send_pointer_events(pointer_events)
    }

    fn focus_changed(&self, focused: bool) -> io::Result<()> {
        if focused {
            if !self.accepting_input.get() {
                return Ok(());
            }
            return self
                .runtime
                .try_borrow()
                .map_err(|_| io::Error::other("the Lynx runtime is already borrowed"))?
                .enter_foreground();
        }

        if self.ignore_focus_loss {
            return Ok(());
        }

        if !self.accepting_input.replace(false) {
            return Ok(());
        }
        let cancel_result = self.cancel_input();
        let background_result = self
            .runtime
            .try_borrow()
            .map_err(|_| io::Error::other("the Lynx runtime is already borrowed"))?
            .enter_background();
        eprintln!("[host-rs] window lost focus; exiting");
        unsafe { ffi::glfw_set_window_should_close(self.window.as_ptr(), ffi::GLFW_TRUE) };
        cancel_result.and(background_result)
    }
}

pub fn run(options: WindowRunOptions) -> io::Result<()> {
    verify_linked_lynx(&options.expected_lynx_library)?;
    if std::env::var_os("DISPLAY").is_none_or(|value| value.is_empty()) {
        return Err(io::Error::other(
            "DISPLAY is not set; the GLFW host requires XWayland/X11",
        ));
    }
    let ignore_focus_loss = match std::env::var_os("LYNX_LAUNCHER_E2E_IGNORE_FOCUS_LOSS") {
        None => false,
        Some(value) if value == std::ffi::OsStr::new("1") => true,
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "LYNX_LAUNCHER_E2E_IGNORE_FOCUS_LOSS must be 1 when set",
            ));
        }
    };

    let _glfw = GlfwRuntime::initialize()?;
    let system_scale = system_window_scale()?;
    let width = scaled_dimension(INITIAL_WIDTH, system_scale)?;
    let height = scaled_dimension(INITIAL_HEIGHT, system_scale)?;
    eprintln!("[host-rs] system window scale: {system_scale}");

    let window = PopupWindow::create(width, height)?;
    configure_popup_window(window.raw())?;
    unsafe {
        ffi::glfw_make_context_current(window.raw());
    }
    if unsafe { ffi::glfw_get_current_context() } != window.raw() {
        return Err(io::Error::other(
            "GLFW did not make the OpenGL context current",
        ));
    }
    unsafe { ffi::glfw_swap_interval(1) };

    let mut framebuffer_width = 0;
    let mut framebuffer_height = 0;
    unsafe {
        ffi::glfw_get_framebuffer_size(
            window.raw(),
            &mut framebuffer_width,
            &mut framebuffer_height,
        );
    }
    if framebuffer_width <= 0 || framebuffer_height <= 0 {
        return Err(io::Error::other("GLFW returned an empty framebuffer"));
    }
    let initial_metrics = window_metrics(window.raw(), system_scale)?
        .ok_or_else(|| io::Error::other("GLFW returned invalid initial window metrics"))?;
    log_metrics(initial_metrics);

    let version = unsafe { ffi::gl_get_string(ffi::GL_VERSION) };
    if version.is_null() {
        return Err(io::Error::other("OpenGL did not report a version"));
    }
    let version = unsafe { CStr::from_ptr(version.cast()) }
        .to_string_lossy()
        .into_owned();
    unsafe { ffi::glfw_make_context_current(std::ptr::null_mut()) };
    present_shell_frame(&window, framebuffer_width, framebuffer_height)?;
    unsafe {
        ffi::glfw_set_window_focus_callback(window.raw(), Some(startup_window_focus_callback));
        ffi::glfw_show_window(window.raw());
        ffi::glfw_focus_window(window.raw());
        if std::env::var_os("LYNX_LAUNCHER_E2E_STARTUP_FOCUS_WAIT").as_deref()
            == Some(std::ffi::OsStr::new("1"))
            && std::env::var_os("LYNX_LAUNCHER_E2E_SNAPSHOT_TRACE").as_deref()
                == Some(std::ffi::OsStr::new("1"))
        {
            let deadline = Instant::now() + Duration::from_secs(1);
            while !STARTUP_FOCUS_LOST.load(Ordering::Acquire) {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                ffi::glfw_wait_events_timeout(deadline.duration_since(now).as_secs_f64());
            }
        } else {
            ffi::glfw_wait_events_timeout(0.25);
        }
    }
    present_shell_frame(&window, framebuffer_width, framebuffer_height)?;
    ensure_callback_did_not_panic()?;
    eprintln!("[host-rs] OpenGL: {version} via pinned GLFW X11");
    eprintln!("[host-rs] first shell GL frame presented");

    let deadline = options
        .run_for_seconds
        .map(|seconds| {
            let duration = Duration::try_from_secs_f64(seconds).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "--run-for is too large")
            })?;
            Instant::now().checked_add(duration).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "--run-for is too large")
            })
        })
        .transpose()?;

    let view_options = RuntimeViewOptions {
        bundle: options.bundle.clone(),
        lynx_core: options.lynx_core.clone(),
        icu: options.icu.clone(),
        logical_width: initial_metrics.logical_width,
        logical_height: initial_metrics.logical_height,
        pixel_ratio: initial_metrics.pixel_ratio,
    };
    // SAFETY: `runtime` shuts down before `window` and `_glfw` are dropped;
    // the function table is static and the wake callback has no userdata.
    let mut runtime = unsafe {
        RuntimeCore::initialize(
            window.raw().cast(),
            GlApi::new(
                runtime_make_context_current,
                runtime_get_current_context,
                runtime_swap_buffers,
                runtime_get_proc_address,
            ),
            DesktopApi::new(
                runtime_get_clipboard,
                runtime_set_clipboard,
                runtime_create_cursor,
                runtime_destroy_cursor,
                runtime_set_cursor,
                runtime_set_cursor_mode,
            ),
            EventWake::new(std::ptr::null_mut(), runtime_wake_event_loop),
            &view_options,
        )
    }?;

    let run_result = match runtime.initialize_view(&view_options) {
        Ok(()) => {
            eprintln!("[host-rs] runtime core initialized");
            let state = Box::new(WindowState::new(
                window.raw,
                runtime,
                initial_metrics,
                system_scale,
                ignore_focus_loss,
            ));
            state.register_callbacks();
            let startup_focus_result = if STARTUP_FOCUS_LOST.swap(false, Ordering::AcqRel)
                || unsafe { ffi::glfw_window_should_close(window.raw()) } != ffi::GLFW_FALSE
                || unsafe { ffi::glfw_get_window_attrib(window.raw(), ffi::GLFW_FOCUSED) }
                    != ffi::GLFW_TRUE
            {
                state.focus_changed(false)
            } else {
                Ok(())
            };
            let loop_result = (|| {
                startup_focus_result?;
                state.runtime.borrow().run_due_tasks()?;
                state.ensure_healthy()?;
                while unsafe { ffi::glfw_window_should_close(window.raw()) } == ffi::GLFW_FALSE {
                    ensure_callback_did_not_panic()?;
                    state.runtime.borrow().run_due_tasks()?;
                    if options.exit_after_first_frame && state.runtime.borrow().first_frame_ready()
                    {
                        eprintln!("[host-rs] auto-exit after first rendered frame");
                        break;
                    }
                    let mut timeout = state.runtime.borrow().wait_duration(MAXIMUM_EVENT_WAIT)?;
                    if let Some(deadline) = deadline {
                        let now = Instant::now();
                        if now >= deadline {
                            eprintln!("[host-rs] auto-exit after bounded run");
                            break;
                        }
                        timeout = timeout.min(deadline.duration_since(now));
                    }
                    unsafe { ffi::glfw_wait_events_timeout(timeout.as_secs_f64().max(0.0005)) };
                    ensure_callback_did_not_panic()?;
                    state.ensure_healthy()?;
                }
                ensure_callback_did_not_panic()
            })();

            // Stop accepting GLFW input before cancellation so synthetic
            // releases cannot recreate state while the view is shutting down.
            state.accepting_input.set(false);
            let cancel_result = state.cancel_input();
            state.unregister_callbacks();
            let shutdown_result = state.runtime.borrow_mut().shutdown();
            let requires_process_exit = state
                .runtime
                .borrow()
                .requires_process_exit_without_glfw_cleanup();
            let native_callbacks_not_quiesced =
                state.runtime.borrow().native_callbacks_not_quiesced();
            if requires_process_exit {
                let fatal_log = if native_callbacks_not_quiesced {
                    NATIVE_CALLBACKS_LOG
                } else {
                    STRANDED_GL_LOG
                };
                let fatal_error = io::Error::from(io::ErrorKind::Other);
                std::mem::forget(state);
                std::mem::forget(window);
                std::mem::forget(_glfw);
                let _ = io::stderr().write_all(fatal_log);
                return Err(fatal_error);
            }
            if shutdown_result.is_ok() {
                eprintln!("[host-rs] runtime core shutdown complete");
            }
            cancel_result?;
            loop_result?;
            shutdown_result
        }
        Err(error) => {
            let shutdown_result = runtime.shutdown();
            if runtime.requires_process_exit_without_glfw_cleanup() {
                let fatal_log = if runtime.native_callbacks_not_quiesced() {
                    NATIVE_CALLBACKS_LOG
                } else {
                    STRANDED_GL_LOG
                };
                let fatal_error = io::Error::from(io::ErrorKind::Other);
                std::mem::forget(runtime);
                std::mem::forget(window);
                std::mem::forget(_glfw);
                let _ = io::stderr().write_all(fatal_log);
                return Err(fatal_error);
            }
            shutdown_result?;
            Err(error)
        }
    };
    run_result
}

unsafe fn runtime_make_context_current(window: *mut c_void) {
    unsafe { ffi::glfw_make_context_current(window.cast()) };
}

unsafe fn runtime_get_current_context() -> *mut c_void {
    unsafe { ffi::glfw_get_current_context().cast() }
}

unsafe fn runtime_swap_buffers(window: *mut c_void) {
    unsafe { ffi::glfw_swap_buffers(window.cast()) };
}

unsafe fn runtime_get_proc_address(name: *const std::ffi::c_char) -> *mut c_void {
    unsafe { ffi::glfw_get_proc_address(name) }
        .map(|function| function as *const () as *mut c_void)
        .unwrap_or(std::ptr::null_mut())
}

unsafe fn runtime_get_clipboard(window: *mut c_void) -> *const std::ffi::c_char {
    unsafe { ffi::glfw_get_clipboard_string(window.cast()) }
}

unsafe fn runtime_set_clipboard(window: *mut c_void, value: *const std::ffi::c_char) {
    unsafe { ffi::glfw_set_clipboard_string(window.cast(), value) };
}

unsafe fn runtime_create_cursor(shape: i32) -> *mut c_void {
    let shape = match shape {
        1 => ffi::GLFW_HAND_CURSOR,
        2 => ffi::GLFW_IBEAM_CURSOR,
        3 => ffi::GLFW_CROSSHAIR_CURSOR,
        4 => ffi::GLFW_HRESIZE_CURSOR,
        5 => ffi::GLFW_VRESIZE_CURSOR,
        _ => ffi::GLFW_ARROW_CURSOR,
    };
    unsafe { ffi::glfw_create_standard_cursor(shape).cast() }
}

unsafe fn runtime_destroy_cursor(cursor: *mut c_void) {
    unsafe { ffi::glfw_destroy_cursor(cursor.cast()) };
}

unsafe fn runtime_set_cursor(window: *mut c_void, cursor: *mut c_void) {
    unsafe { ffi::glfw_set_cursor(window.cast(), cursor.cast()) };
}

unsafe fn runtime_set_cursor_mode(window: *mut c_void, hidden: i32) {
    let mode = if hidden == 0 {
        ffi::GLFW_CURSOR_NORMAL
    } else {
        ffi::GLFW_CURSOR_HIDDEN
    };
    unsafe { ffi::glfw_set_input_mode(window.cast(), ffi::GLFW_CURSOR, mode) };
}

unsafe fn runtime_wake_event_loop(_: *mut c_void) {
    unsafe { ffi::glfw_post_empty_event() };
}

fn present_shell_frame(window: &PopupWindow, width: i32, height: i32) -> io::Result<()> {
    unsafe { ffi::glfw_make_context_current(window.raw()) };
    if unsafe { ffi::glfw_get_current_context() } != window.raw() {
        return Err(io::Error::other(
            "GLFW did not make the OpenGL context current",
        ));
    }
    unsafe {
        ffi::gl_viewport(0, 0, width, height);
        ffi::gl_clear_color(0.035, 0.039, 0.043, 1.0);
        ffi::gl_clear(ffi::GL_COLOR_BUFFER_BIT);
        ffi::gl_finish();
        ffi::glfw_swap_buffers(window.raw());
        ffi::glfw_make_context_current(std::ptr::null_mut());
    }
    Ok(())
}

fn scaled_dimension(logical: f32, scale: f32) -> io::Result<i32> {
    let physical = (logical * scale).round();
    if !physical.is_finite() || physical <= 0.0 || physical > i32::MAX as f32 {
        return Err(io::Error::other(
            "system scale produced an invalid window size",
        ));
    }
    Ok(physical as i32)
}

fn system_window_scale() -> io::Result<f32> {
    let mut content_scale_x = 1.0;
    let mut content_scale_y = 1.0;
    let monitor = unsafe { ffi::glfw_get_primary_monitor() };
    if !monitor.is_null() {
        unsafe {
            ffi::glfw_get_monitor_content_scale(
                monitor,
                &mut content_scale_x,
                &mut content_scale_y,
            );
        }
    }
    let mut scale = content_scale_x.max(content_scale_y);
    let display = unsafe { ffi::glfw_get_x11_display() };
    if !display.is_null() {
        if let Some(xsettings_scale) = read_xsettings_scale(display) {
            scale = scale.max(xsettings_scale);
        }
    }
    scale = scale.max(1.0);
    if std::env::var_os("LYNX_LAUNCHER_E2E_SNAPSHOT_TRACE").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        if let Some(value) = std::env::var_os("LYNX_LAUNCHER_E2E_SYSTEM_SCALE") {
            let value = value
                .to_str()
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && *value >= 1.0)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "LYNX_LAUNCHER_E2E_SYSTEM_SCALE must be a finite number at least 1",
                    )
                })?;
            scale = value;
        }
    }
    Ok(scale)
}

fn window_metrics(
    window: *mut ffi::GlfwWindow,
    system_scale: f32,
) -> io::Result<Option<WindowMetrics>> {
    let mut window_width = 0;
    let mut window_height = 0;
    let mut framebuffer_width = 0;
    let mut framebuffer_height = 0;
    let mut content_scale_x = 1.0;
    let mut content_scale_y = 1.0;
    unsafe {
        ffi::glfw_get_window_size(window, &mut window_width, &mut window_height);
        ffi::glfw_get_framebuffer_size(window, &mut framebuffer_width, &mut framebuffer_height);
        ffi::glfw_get_window_content_scale(window, &mut content_scale_x, &mut content_scale_y);
    }
    ensure_callback_did_not_panic()?;
    Ok(calculate_window_metrics(
        window_width,
        window_height,
        framebuffer_width,
        framebuffer_height,
        system_scale.max(content_scale_x).max(content_scale_y),
    ))
}

fn log_metrics(metrics: WindowMetrics) {
    eprintln!(
        "[host-rs] metrics logical={}x{} dpr={} framebuffer-scale={}x{}",
        metrics.logical_width,
        metrics.logical_height,
        metrics.pixel_ratio,
        metrics.framebuffer_scale_x,
        metrics.framebuffer_scale_y
    );
}

fn read_xsettings_scale(display: *mut ffi::Display) -> Option<f32> {
    let screen = unsafe { ffi::x_default_screen(display) };
    let selection_name = CString::new(format!("_XSETTINGS_S{screen}")).ok()?;
    let selection = unsafe { ffi::x_intern_atom(display, selection_name.as_ptr(), ffi::X_TRUE) };
    let property =
        unsafe { ffi::x_intern_atom(display, c"_XSETTINGS_SETTINGS".as_ptr(), ffi::X_TRUE) };
    if selection == ffi::X_NONE || property == ffi::X_NONE {
        return None;
    }
    let owner = unsafe { ffi::x_get_selection_owner(display, selection) };
    if owner == ffi::X_NONE {
        return None;
    }

    let mut actual_type = ffi::X_NONE;
    let mut actual_format = 0;
    let mut item_count = 0;
    let mut remaining = 0;
    let mut data = std::ptr::null_mut();
    let status = unsafe {
        ffi::x_get_window_property(
            display,
            owner,
            property,
            0,
            65_536,
            ffi::X_FALSE,
            property,
            &mut actual_type,
            &mut actual_format,
            &mut item_count,
            &mut remaining,
            &mut data,
        )
    };
    let scale = if status == ffi::X_SUCCESS
        && actual_type == property
        && actual_format == 8
        && remaining == 0
        && !data.is_null()
    {
        usize::try_from(item_count).ok().and_then(|length| unsafe {
            xsettings_window_scale(std::slice::from_raw_parts(data, length))
        })
    } else {
        None
    };
    if !data.is_null() {
        unsafe { ffi::x_free(data.cast::<c_void>()) };
    }
    scale
}

fn configure_popup_window(window: *mut ffi::GlfwWindow) -> io::Result<()> {
    let display = unsafe { ffi::glfw_get_x11_display() };
    let native_window = unsafe { ffi::glfw_get_x11_window(window) };
    if display.is_null() || native_window == ffi::X_NONE {
        return Err(io::Error::other("could not access the launcher X11 window"));
    }

    // Preserve the verified shell color on later X11 Expose clears without
    // reacquiring the GL context after the Lynx renderer starts.
    unsafe { ffi::x_set_window_background(display, native_window, 0x0009_0a0b) };

    let window_type = intern_atom(display, c"_NET_WM_WINDOW_TYPE")?;
    let normal_type = intern_atom(display, c"_NET_WM_WINDOW_TYPE_NORMAL")?;
    unsafe {
        ffi::x_change_property(
            display,
            native_window,
            window_type,
            ffi::XA_ATOM,
            32,
            ffi::X_PROP_MODE_REPLACE,
            (&normal_type as *const ffi::Atom).cast(),
            1,
        );
    }

    let window_state = intern_atom(display, c"_NET_WM_STATE")?;
    let states = [
        intern_atom(display, c"_NET_WM_STATE_ABOVE")?,
        intern_atom(display, c"_NET_WM_STATE_SKIP_TASKBAR")?,
        intern_atom(display, c"_NET_WM_STATE_SKIP_PAGER")?,
    ];
    unsafe {
        ffi::x_change_property(
            display,
            native_window,
            window_state,
            ffi::XA_ATOM,
            32,
            ffi::X_PROP_MODE_REPLACE,
            states.as_ptr().cast(),
            states.len() as i32,
        );
        ffi::x_flush(display);
    }
    Ok(())
}

fn intern_atom(display: *mut ffi::Display, name: &CStr) -> io::Result<ffi::Atom> {
    let atom = unsafe { ffi::x_intern_atom(display, name.as_ptr(), ffi::X_FALSE) };
    if atom == ffi::X_NONE {
        Err(io::Error::other(format!(
            "X11 could not intern atom {}",
            name.to_string_lossy()
        )))
    } else {
        Ok(atom)
    }
}

fn ensure_callback_did_not_panic() -> io::Result<()> {
    if CALLBACK_PANICKED.load(Ordering::Acquire) {
        Err(io::Error::other("a GLFW callback panicked"))
    } else {
        Ok(())
    }
}

unsafe extern "C" fn glfw_error(code: i32, description: *const std::ffi::c_char) {
    if std::panic::catch_unwind(|| {
        let description = if description.is_null() {
            "unknown GLFW error".into()
        } else {
            unsafe { CStr::from_ptr(description) }.to_string_lossy()
        };
        eprintln!("[glfw {code}] {description}");
    })
    .is_err()
    {
        CALLBACK_PANICKED.store(true, Ordering::Release);
    }
}

unsafe extern "C" fn startup_window_focus_callback(window: *mut ffi::GlfwWindow, focused: i32) {
    if std::panic::catch_unwind(|| {
        if focused != ffi::GLFW_TRUE {
            STARTUP_FOCUS_LOST.store(true, Ordering::Release);
            if std::env::var_os("LYNX_LAUNCHER_E2E_SNAPSHOT_TRACE").as_deref()
                == Some(std::ffi::OsStr::new("1"))
            {
                eprintln!("[host-rs] startup window lost focus; closing");
            }
            unsafe {
                ffi::glfw_set_window_should_close(window, ffi::GLFW_TRUE);
                ffi::glfw_post_empty_event();
            }
        }
    })
    .is_err()
    {
        CALLBACK_PANICKED.store(true, Ordering::Release);
        STARTUP_FOCUS_LOST.store(true, Ordering::Release);
        unsafe {
            ffi::glfw_set_window_should_close(window, ffi::GLFW_TRUE);
            ffi::glfw_post_empty_event();
        }
    }
}

unsafe fn clear_window_callbacks(window: *mut ffi::GlfwWindow) {
    unsafe {
        ffi::glfw_set_cursor_position_callback(window, None);
        ffi::glfw_set_cursor_enter_callback(window, None);
        ffi::glfw_set_mouse_button_callback(window, None);
        ffi::glfw_set_scroll_callback(window, None);
        ffi::glfw_set_key_callback(window, None);
        ffi::glfw_set_char_callback(window, None);
        ffi::glfw_set_framebuffer_size_callback(window, None);
        ffi::glfw_set_window_size_callback(window, None);
        ffi::glfw_set_window_content_scale_callback(window, None);
        ffi::glfw_set_window_focus_callback(window, None);
        ffi::glfw_set_window_user_pointer(window, std::ptr::null_mut());
    }
}

fn with_window_state(
    window: *mut ffi::GlfwWindow,
    callback: impl FnOnce(&WindowState) -> io::Result<()>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let pointer = unsafe { ffi::glfw_get_window_user_pointer(window) };
        let state = NonNull::new(pointer.cast::<WindowState>())
            .ok_or_else(|| io::Error::other("GLFW window userdata is unavailable"))?;
        // SAFETY: Callback registration stores a stable Box address and clears
        // every callback before that Box is released.
        callback(unsafe { state.as_ref() })
    }));
    if matches!(result, Ok(Ok(()))) {
        return;
    }

    let recorded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let pointer = unsafe { ffi::glfw_get_window_user_pointer(window) };
        let Some(state) = NonNull::new(pointer.cast::<WindowState>()) else {
            return false;
        };
        // SAFETY: The pointer has the same callback-scoped lifetime described
        // above and is only used through a shared reference.
        unsafe { state.as_ref() }.record_callback_failure();
        true
    }))
    .unwrap_or(false);
    if !recorded {
        CALLBACK_PANICKED.store(true, Ordering::Release);
        unsafe {
            ffi::glfw_set_window_should_close(window, ffi::GLFW_TRUE);
            ffi::glfw_post_empty_event();
        }
    }
}

unsafe extern "C" fn cursor_position_callback(window: *mut ffi::GlfwWindow, x: f64, y: f64) {
    with_window_state(window, |state| {
        if !state.accepting_input.get() {
            return Ok(());
        }
        let events = state
            .input
            .try_borrow_mut()
            .map_err(|_| io::Error::other("the input state is already borrowed"))?
            .move_cursor(x, y);
        state.send_pointer_events(events)
    });
}

unsafe extern "C" fn cursor_enter_callback(window: *mut ffi::GlfwWindow, entered: i32) {
    with_window_state(window, |state| {
        if !state.accepting_input.get() {
            return Ok(());
        }
        let events = state
            .input
            .try_borrow_mut()
            .map_err(|_| io::Error::other("the input state is already borrowed"))?
            .set_pointer_inside(entered == ffi::GLFW_TRUE);
        state.send_pointer_events(events)
    });
}

fn mouse_button_mask(button: i32) -> i64 {
    match button {
        ffi::GLFW_MOUSE_BUTTON_LEFT => LYNX_POINTER_BUTTON_PRIMARY,
        ffi::GLFW_MOUSE_BUTTON_RIGHT => LYNX_POINTER_BUTTON_SECONDARY,
        ffi::GLFW_MOUSE_BUTTON_MIDDLE => LYNX_POINTER_BUTTON_MIDDLE,
        ffi::GLFW_MOUSE_BUTTON_4 => LYNX_POINTER_BUTTON_BACK,
        ffi::GLFW_MOUSE_BUTTON_5 => LYNX_POINTER_BUTTON_FORWARD,
        _ => 0,
    }
}

fn pointer_phase_name(phase: i32) -> &'static str {
    match phase {
        lynx_sys::LYNX_POINTER_PHASE_CANCEL => "cancel",
        lynx_sys::LYNX_POINTER_PHASE_UP => "up",
        lynx_sys::LYNX_POINTER_PHASE_DOWN => "down",
        lynx_sys::LYNX_POINTER_PHASE_MOVE => "move",
        lynx_sys::LYNX_POINTER_PHASE_ADD => "add",
        lynx_sys::LYNX_POINTER_PHASE_REMOVE => "remove",
        lynx_sys::LYNX_POINTER_PHASE_HOVER => "hover",
        _ => "unknown",
    }
}

fn pointer_signal_name(signal: i32) -> &'static str {
    match signal {
        lynx_sys::LYNX_POINTER_SIGNAL_KIND_NONE => "none",
        lynx_sys::LYNX_POINTER_SIGNAL_KIND_SCROLL => "scroll",
        _ => "unknown",
    }
}

fn key_event_type_name(event_type: i32) -> &'static str {
    match event_type {
        LYNX_KEY_EVENT_TYPE_UP => "up",
        LYNX_KEY_EVENT_TYPE_DOWN => "down",
        LYNX_KEY_EVENT_TYPE_REPEAT => "repeat",
        _ => "unknown",
    }
}

unsafe extern "C" fn mouse_button_callback(
    window: *mut ffi::GlfwWindow,
    button: i32,
    action: i32,
    _modifiers: i32,
) {
    with_window_state(window, |state| {
        if !state.accepting_input.get() {
            return Ok(());
        }
        let mask = mouse_button_mask(button);
        if mask == 0 {
            return Ok(());
        }
        let mut input = state
            .input
            .try_borrow_mut()
            .map_err(|_| io::Error::other("the input state is already borrowed"))?;
        let events = if action == ffi::GLFW_PRESS {
            input.press_button(mask)
        } else if action == ffi::GLFW_RELEASE {
            input.release_button(mask)
        } else {
            return Ok(());
        };
        drop(input);
        state.send_pointer_events(events)
    });
}

unsafe extern "C" fn scroll_callback(window: *mut ffi::GlfwWindow, x_offset: f64, y_offset: f64) {
    with_window_state(window, |state| {
        if !state.accepting_input.get() {
            return Ok(());
        }
        let events = state
            .input
            .try_borrow_mut()
            .map_err(|_| io::Error::other("the input state is already borrowed"))?
            .scroll(x_offset, y_offset);
        state.send_pointer_events(events)
    });
}

unsafe extern "C" fn key_callback(
    window: *mut ffi::GlfwWindow,
    key: i32,
    _scan_code: i32,
    action: i32,
    _modifiers: i32,
) {
    with_window_state(window, |state| {
        if !state.accepting_input.get() {
            return Ok(());
        }
        let physical = physical_key(key);
        if physical == 0 {
            return Ok(());
        }
        let logical = logical_key(key);
        if state.input_trace {
            eprintln!(
                "[host-rs] key key={key} action={action} physical={physical} logical={logical}"
            );
        }
        let pressed = if action == ffi::GLFW_PRESS {
            Some((
                LYNX_KEY_EVENT_TYPE_DOWN,
                state
                    .input
                    .try_borrow_mut()
                    .map_err(|_| io::Error::other("the input state is already borrowed"))?
                    .press_key(key, physical, logical),
            ))
        } else if action == ffi::GLFW_REPEAT {
            state
                .input
                .try_borrow()
                .map_err(|_| io::Error::other("the input state is already borrowed"))?
                .repeated_key(key)
                .map(|pressed| (LYNX_KEY_EVENT_TYPE_REPEAT, pressed))
        } else if action == ffi::GLFW_RELEASE {
            state
                .input
                .try_borrow_mut()
                .map_err(|_| io::Error::other("the input state is already borrowed"))?
                .release_key(key)
                .map(|pressed| (LYNX_KEY_EVENT_TYPE_UP, pressed))
        } else {
            None
        };
        let Some((event_type, pressed)) = pressed else {
            return Ok(());
        };
        state.send_key(
            event_type,
            pressed,
            if event_type == LYNX_KEY_EVENT_TYPE_UP {
                std::ptr::null()
            } else {
                c"".as_ptr()
            },
            false,
        )
    });
}

unsafe extern "C" fn character_callback(window: *mut ffi::GlfwWindow, codepoint: u32) {
    with_window_state(window, |state| {
        if !state.accepting_input.get() {
            return Ok(());
        }
        let text_input_active = state
            .runtime
            .try_borrow()
            .map_err(|_| io::Error::other("the Lynx runtime is already borrowed"))?
            .text_input_active();
        if state.input_trace {
            eprintln!("[host-rs] character codepoint={codepoint} active={text_input_active}");
        }
        if !text_input_active {
            return Ok(());
        }
        let character = utf8_from_codepoint(codepoint);
        if character.is_empty() || character.as_bytes().contains(&0) {
            return Ok(());
        }
        let character = CString::new(character)
            .map_err(|_| io::Error::other("character input unexpectedly contained NUL"))?;
        let pressed = PressedKey {
            physical: 0,
            logical: u64::from(codepoint),
        };
        state.send_key(LYNX_KEY_EVENT_TYPE_DOWN, pressed, character.as_ptr(), true)?;
        state.send_key(LYNX_KEY_EVENT_TYPE_UP, pressed, std::ptr::null(), true)
    });
}

unsafe extern "C" fn framebuffer_size_callback(
    window: *mut ffi::GlfwWindow,
    _width: i32,
    _height: i32,
) {
    with_window_state(window, WindowState::update_metrics);
}

unsafe extern "C" fn window_size_callback(window: *mut ffi::GlfwWindow, _width: i32, _height: i32) {
    with_window_state(window, WindowState::update_metrics);
}

unsafe extern "C" fn content_scale_callback(
    window: *mut ffi::GlfwWindow,
    _x_scale: f32,
    _y_scale: f32,
) {
    with_window_state(window, WindowState::update_metrics);
}

unsafe extern "C" fn window_focus_callback(window: *mut ffi::GlfwWindow, focused: i32) {
    with_window_state(window, |state| {
        state.focus_changed(focused == ffi::GLFW_TRUE)
    });
}
