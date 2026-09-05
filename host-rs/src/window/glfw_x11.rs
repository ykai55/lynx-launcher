mod ffi;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{c_void, CStr, CString};
use std::io;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lynx_launcher_host::runtime::{DesktopApi, EventWake, GlApi, GlContextBinding};
use lynx_launcher_host::support::{
    calculate_window_metrics, logical_key, physical_key, utf8_from_codepoint,
    xsettings_window_scale, WindowMetrics,
};

use super::{
    log_metrics, BackendRuntimeAdapters, KeyAction, PointerButton, WindowBackend, WindowEvent,
};

const INITIAL_WIDTH: f32 = 1120.0;
const INITIAL_HEIGHT: f32 = 760.0;
const STRANDED_GL_LOG: &[u8] = b"[host-rs] fatal: renderer stranded an OpenGL context; \
skipping glfwDestroyWindow/glfwTerminate and exiting for OS cleanup\n";
const NATIVE_CALLBACKS_LOG: &[u8] = b"[host-rs] fatal: native callbacks did not quiesce; \
skipping glfwDestroyWindow/glfwTerminate and exiting for OS cleanup\n";

static CALLBACK_PANICKED: AtomicBool = AtomicBool::new(false);

struct GlfwRuntime {
    _thread_bound: PhantomData<Rc<()>>,
}

impl GlfwRuntime {
    fn initialize() -> io::Result<Self> {
        CALLBACK_PANICKED.store(false, Ordering::Release);
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

struct CallbackQueue {
    events: RefCell<VecDeque<WindowEvent>>,
    startup_focus_lost: AtomicBool,
    failed: AtomicBool,
    system_scale: f32,
}

impl CallbackQueue {
    fn push(&self, event: WindowEvent) -> io::Result<()> {
        self.events
            .try_borrow_mut()
            .map_err(|_| io::Error::other("the window event queue is already borrowed"))?
            .push_back(event);
        Ok(())
    }
}

pub(super) struct GlfwX11Backend {
    callbacks: Box<CallbackQueue>,
    window: PopupWindow,
    _glfw: GlfwRuntime,
    initial_metrics: WindowMetrics,
    framebuffer_width: i32,
    framebuffer_height: i32,
    gl_version: String,
}

pub(super) fn default_backend() -> io::Result<impl WindowBackend> {
    GlfwX11Backend::initialize()
}

impl GlfwX11Backend {
    pub(super) fn initialize() -> io::Result<Self> {
        if std::env::var_os("DISPLAY").is_none_or(|value| value.is_empty()) {
            return Err(io::Error::other(
                "DISPLAY is not set; the GLFW host requires XWayland/X11",
            ));
        }
        let glfw = GlfwRuntime::initialize()?;
        let system_scale = system_window_scale()?;
        let width = scaled_dimension(INITIAL_WIDTH, system_scale)?;
        let height = scaled_dimension(INITIAL_HEIGHT, system_scale)?;
        eprintln!("[host-rs] system window scale: {system_scale}");

        let window = PopupWindow::create(width, height)?;
        configure_popup_window(window.raw())?;
        unsafe { ffi::glfw_make_context_current(window.raw()) };
        if unsafe { ffi::glfw_get_current_context() } != window.raw() {
            return Err(io::Error::other(
                "GLFW did not make the OpenGL context current",
            ));
        }
        unsafe { ffi::glfw_swap_interval(1) };
        let (framebuffer_width, framebuffer_height) = framebuffer_size(window.raw())?;
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

        let callbacks = Box::new(CallbackQueue {
            events: RefCell::new(VecDeque::new()),
            startup_focus_lost: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            system_scale,
        });
        Ok(Self {
            callbacks,
            window,
            _glfw: glfw,
            initial_metrics,
            framebuffer_width,
            framebuffer_height,
            gl_version: version,
        })
    }
}

impl WindowBackend for GlfwX11Backend {
    fn show(&mut self) -> io::Result<()> {
        unsafe {
            ffi::glfw_set_window_user_pointer(
                self.window.raw(),
                (&*self.callbacks as *const CallbackQueue).cast_mut().cast(),
            );
            ffi::glfw_set_window_focus_callback(
                self.window.raw(),
                Some(startup_window_focus_callback),
            );
            ffi::glfw_show_window(self.window.raw());
            ffi::glfw_focus_window(self.window.raw());
            if std::env::var_os("LYNX_LAUNCHER_E2E_STARTUP_FOCUS_WAIT").as_deref()
                == Some(std::ffi::OsStr::new("1"))
                && trace_enabled()
            {
                let deadline = Instant::now() + Duration::from_secs(1);
                while !self.callbacks.startup_focus_lost.load(Ordering::Acquire) {
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
        present_shell_frame(
            &self.window,
            self.framebuffer_width,
            self.framebuffer_height,
        )?;
        ensure_callback_did_not_panic()?;
        eprintln!("[host-rs] OpenGL: {} via pinned GLFW X11", self.gl_version);
        eprintln!("[host-rs] first shell GL frame presented");
        Ok(())
    }
    fn runtime_adapters(&self) -> BackendRuntimeAdapters {
        BackendRuntimeAdapters {
            render_target: self.window.raw().cast(),
            gl: GlApi::new(
                runtime_make_render_target_current,
                runtime_capture_current_binding,
                runtime_is_render_target_current,
                runtime_restore_binding,
                runtime_swap_buffers,
                runtime_get_proc_address,
            ),
            desktop: DesktopApi::new(
                runtime_get_clipboard,
                runtime_set_clipboard,
                runtime_create_cursor,
                runtime_destroy_cursor,
                runtime_set_cursor,
                runtime_set_cursor_mode,
            ),
            wake: EventWake::new(std::ptr::null_mut(), runtime_wake_event_loop),
            initial_metrics: self.initial_metrics,
        }
    }

    fn activate_event_delivery(&mut self) -> io::Result<()> {
        unsafe {
            ffi::glfw_set_cursor_position_callback(
                self.window.raw(),
                Some(cursor_position_callback),
            );
            ffi::glfw_set_cursor_enter_callback(self.window.raw(), Some(cursor_enter_callback));
            ffi::glfw_set_mouse_button_callback(self.window.raw(), Some(mouse_button_callback));
            ffi::glfw_set_scroll_callback(self.window.raw(), Some(scroll_callback));
            ffi::glfw_set_key_callback(self.window.raw(), Some(key_callback));
            ffi::glfw_set_char_callback(self.window.raw(), Some(character_callback));
            ffi::glfw_set_framebuffer_size_callback(
                self.window.raw(),
                Some(framebuffer_size_callback),
            );
            ffi::glfw_set_window_size_callback(self.window.raw(), Some(window_size_callback));
            ffi::glfw_set_window_content_scale_callback(
                self.window.raw(),
                Some(content_scale_callback),
            );
            ffi::glfw_set_window_focus_callback(self.window.raw(), Some(window_focus_callback));
        }
        if self
            .callbacks
            .startup_focus_lost
            .swap(false, Ordering::AcqRel)
            || self.should_close()
            || unsafe { ffi::glfw_get_window_attrib(self.window.raw(), ffi::GLFW_FOCUSED) }
                != ffi::GLFW_TRUE
        {
            self.callbacks.push(WindowEvent::Focused(false))?;
        }
        Ok(())
    }

    fn drain_events(&self) -> io::Result<Vec<WindowEvent>> {
        Ok(self
            .callbacks
            .events
            .try_borrow_mut()
            .map_err(|_| io::Error::other("the window event queue is already borrowed"))?
            .drain(..)
            .collect())
    }

    fn wait_for_events(&self, timeout: Duration) {
        unsafe { ffi::glfw_wait_events_timeout(timeout.as_secs_f64()) };
    }

    fn should_close(&self) -> bool {
        (unsafe { ffi::glfw_window_should_close(self.window.raw()) }) != ffi::GLFW_FALSE
    }

    fn request_close(&self) {
        unsafe {
            ffi::glfw_set_window_should_close(self.window.raw(), ffi::GLFW_TRUE);
            ffi::glfw_post_empty_event();
        }
    }

    fn set_text_input_active(&self, _active: bool) -> io::Result<()> {
        Ok(())
    }

    fn stop_event_delivery(&mut self) {
        unsafe { clear_window_callbacks(self.window.raw()) };
    }

    fn ensure_healthy(&self) -> io::Result<()> {
        ensure_callback_did_not_panic()?;
        if self.callbacks.failed.load(Ordering::Acquire) {
            Err(io::Error::other("a GLFW window callback failed"))
        } else {
            Ok(())
        }
    }

    fn fatal_cleanup_log(&self, native_callbacks_not_quiesced: bool) -> &'static [u8] {
        if native_callbacks_not_quiesced {
            NATIVE_CALLBACKS_LOG
        } else {
            STRANDED_GL_LOG
        }
    }
}

impl Drop for GlfwX11Backend {
    fn drop(&mut self) {
        self.stop_event_delivery();
    }
}

unsafe fn runtime_make_render_target_current(window: *mut c_void) {
    unsafe { ffi::glfw_make_context_current(window.cast()) };
}

unsafe fn runtime_capture_current_binding() -> GlContextBinding {
    GlContextBinding::new(
        std::ptr::null_mut(),
        unsafe { ffi::glfw_get_current_context().cast() },
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    )
}

unsafe fn runtime_is_render_target_current(window: *mut c_void) -> bool {
    unsafe { ffi::glfw_get_current_context() == window.cast() }
}

unsafe fn runtime_restore_binding(binding: GlContextBinding) -> bool {
    if !binding.display().is_null()
        || !binding.draw_surface().is_null()
        || !binding.read_surface().is_null()
    {
        return false;
    }
    unsafe { ffi::glfw_make_context_current(binding.context().cast()) };
    unsafe { ffi::glfw_get_current_context().cast::<c_void>() == binding.context() }
}

unsafe fn runtime_swap_buffers(window: *mut c_void) -> bool {
    unsafe { ffi::glfw_swap_buffers(window.cast()) };
    !CALLBACK_PANICKED.load(Ordering::Acquire)
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

unsafe fn runtime_wake_event_loop(_: *mut c_void) -> bool {
    unsafe { ffi::glfw_post_empty_event() };
    !CALLBACK_PANICKED.load(Ordering::Acquire)
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

fn framebuffer_size(window: *mut ffi::GlfwWindow) -> io::Result<(i32, i32)> {
    let mut width = 0;
    let mut height = 0;
    unsafe { ffi::glfw_get_framebuffer_size(window, &mut width, &mut height) };
    ensure_callback_did_not_panic()?;
    if width <= 0 || height <= 0 {
        Err(io::Error::other("GLFW returned an empty framebuffer"))
    } else {
        Ok((width, height))
    }
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
    if trace_enabled() {
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

fn trace_enabled() -> bool {
    std::env::var_os("LYNX_LAUNCHER_E2E_SNAPSHOT_TRACE").as_deref()
        == Some(std::ffi::OsStr::new("1"))
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

unsafe fn callback_queue(window: *mut ffi::GlfwWindow) -> io::Result<NonNull<CallbackQueue>> {
    let pointer = unsafe { ffi::glfw_get_window_user_pointer(window) };
    NonNull::new(pointer.cast::<CallbackQueue>())
        .ok_or_else(|| io::Error::other("GLFW window userdata is unavailable"))
}

fn with_callback_queue(
    window: *mut ffi::GlfwWindow,
    callback: impl FnOnce(&CallbackQueue) -> io::Result<()>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let pointer = unsafe { callback_queue(window) }?;
        // SAFETY: The backend owns a stable Box and clears every callback and
        // the userdata pointer before releasing it.
        callback(unsafe { pointer.as_ref() })
    }));
    if result.as_ref().is_ok_and(|result| result.is_ok()) {
        return;
    }
    if result.is_err() {
        CALLBACK_PANICKED.store(true, Ordering::Release);
    }
    let recorded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let pointer = unsafe { callback_queue(window) }?;
        unsafe { pointer.as_ref() }
            .failed
            .store(true, Ordering::Release);
        Ok::<_, io::Error>(())
    }))
    .is_ok_and(|result| result.is_ok());
    if !recorded {
        CALLBACK_PANICKED.store(true, Ordering::Release);
    }
    unsafe {
        ffi::glfw_set_window_should_close(window, ffi::GLFW_TRUE);
        ffi::glfw_post_empty_event();
    }
}

unsafe extern "C" fn startup_window_focus_callback(window: *mut ffi::GlfwWindow, focused: i32) {
    with_callback_queue(window, |queue| {
        if focused != ffi::GLFW_TRUE {
            queue.startup_focus_lost.store(true, Ordering::Release);
            if trace_enabled() {
                eprintln!("[host-rs] startup window lost focus; closing");
            }
            unsafe {
                ffi::glfw_set_window_should_close(window, ffi::GLFW_TRUE);
                ffi::glfw_post_empty_event();
            }
        }
        Ok(())
    });
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

unsafe extern "C" fn cursor_position_callback(window: *mut ffi::GlfwWindow, x: f64, y: f64) {
    with_callback_queue(window, |queue| queue.push(WindowEvent::CursorMoved(x, y)));
}

unsafe extern "C" fn cursor_enter_callback(window: *mut ffi::GlfwWindow, entered: i32) {
    with_callback_queue(window, |queue| {
        queue.push(WindowEvent::CursorEntered(entered == ffi::GLFW_TRUE))
    });
}

unsafe extern "C" fn mouse_button_callback(
    window: *mut ffi::GlfwWindow,
    button: i32,
    action: i32,
    _modifiers: i32,
) {
    with_callback_queue(window, |queue| {
        let button = match button {
            ffi::GLFW_MOUSE_BUTTON_LEFT => PointerButton::Primary,
            ffi::GLFW_MOUSE_BUTTON_RIGHT => PointerButton::Secondary,
            ffi::GLFW_MOUSE_BUTTON_MIDDLE => PointerButton::Middle,
            ffi::GLFW_MOUSE_BUTTON_4 => PointerButton::Back,
            ffi::GLFW_MOUSE_BUTTON_5 => PointerButton::Forward,
            _ => return Ok(()),
        };
        let pressed = match action {
            ffi::GLFW_PRESS => true,
            ffi::GLFW_RELEASE => false,
            _ => return Ok(()),
        };
        queue.push(WindowEvent::PointerButton(button, pressed))
    });
}

unsafe extern "C" fn scroll_callback(window: *mut ffi::GlfwWindow, x_offset: f64, y_offset: f64) {
    with_callback_queue(window, |queue| {
        queue.push(WindowEvent::Scroll(x_offset, y_offset))
    });
}

unsafe extern "C" fn key_callback(
    window: *mut ffi::GlfwWindow,
    key: i32,
    _scan_code: i32,
    action: i32,
    _modifiers: i32,
) {
    with_callback_queue(window, |queue| {
        let physical = physical_key(key);
        if physical == 0 {
            return Ok(());
        }
        let action = match action {
            ffi::GLFW_PRESS => KeyAction::Press,
            ffi::GLFW_REPEAT => KeyAction::Repeat,
            ffi::GLFW_RELEASE => KeyAction::Release,
            _ => return Ok(()),
        };
        queue.push(WindowEvent::Key {
            id: key,
            physical,
            logical: logical_key(key),
            action,
        })
    });
}

unsafe extern "C" fn character_callback(window: *mut ffi::GlfwWindow, codepoint: u32) {
    with_callback_queue(window, |queue| {
        let text = utf8_from_codepoint(codepoint);
        if text.is_empty() {
            Ok(())
        } else {
            queue.push(WindowEvent::Text { codepoint, text })
        }
    });
}

fn queue_metrics(window: *mut ffi::GlfwWindow, queue: &CallbackQueue) -> io::Result<()> {
    if let Some(metrics) = window_metrics(window, queue.system_scale)? {
        queue.push(WindowEvent::Metrics(metrics))?;
    }
    Ok(())
}

unsafe extern "C" fn framebuffer_size_callback(
    window: *mut ffi::GlfwWindow,
    _width: i32,
    _height: i32,
) {
    with_callback_queue(window, |queue| queue_metrics(window, queue));
}

unsafe extern "C" fn window_size_callback(window: *mut ffi::GlfwWindow, _width: i32, _height: i32) {
    with_callback_queue(window, |queue| queue_metrics(window, queue));
}

unsafe extern "C" fn content_scale_callback(
    window: *mut ffi::GlfwWindow,
    _x_scale: f32,
    _y_scale: f32,
) {
    with_callback_queue(window, |queue| queue_metrics(window, queue));
}

unsafe extern "C" fn window_focus_callback(window: *mut ffi::GlfwWindow, focused: i32) {
    with_callback_queue(window, |queue| {
        queue.push(WindowEvent::Focused(focused == ffi::GLFW_TRUE))
    });
}
