mod ffi;

use std::ffi::{c_void, CStr, CString};
use std::io;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lynx_launcher_host::support::xsettings_window_scale;
use lynx_launcher_host::{verify_linked_lynx, WindowRunOptions};

const INITIAL_WIDTH: f32 = 1120.0;
const INITIAL_HEIGHT: f32 = 760.0;
const MAXIMUM_EVENT_WAIT: Duration = Duration::from_millis(250);

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
            ffi::glfw_set_window_focus_callback(self.raw(), None);
            if ffi::glfw_get_current_context() == self.raw() {
                ffi::glfw_make_context_current(std::ptr::null_mut());
            }
            ffi::glfw_destroy_window(self.raw());
        }
    }
}

pub fn run(options: WindowRunOptions) -> io::Result<()> {
    verify_linked_lynx(&options.expected_lynx_library)?;
    if std::env::var_os("DISPLAY").is_none_or(|value| value.is_empty()) {
        return Err(io::Error::other(
            "DISPLAY is not set; the GLFW host requires XWayland/X11",
        ));
    }

    let _glfw = GlfwRuntime::initialize()?;
    let system_scale = system_window_scale();
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
        ffi::glfw_show_window(window.raw());
        ffi::glfw_focus_window(window.raw());
        ffi::glfw_set_window_focus_callback(window.raw(), Some(window_focus));
        ffi::glfw_wait_events_timeout(0.25);
    }
    present_shell_frame(&window, framebuffer_width, framebuffer_height)?;
    ensure_callback_did_not_panic()?;
    eprintln!("[host-rs] OpenGL: {version} via pinned GLFW X11");
    eprintln!("[host-rs] first shell GL frame presented");

    if options.exit_after_first_frame {
        eprintln!("[host-rs] auto-exit after first shell frame");
        return Ok(());
    }

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

    while unsafe { ffi::glfw_window_should_close(window.raw()) } == ffi::GLFW_FALSE {
        ensure_callback_did_not_panic()?;
        let timeout = if let Some(deadline) = deadline {
            let now = Instant::now();
            if now >= deadline {
                eprintln!("[host-rs] auto-exit after bounded run");
                break;
            }
            MAXIMUM_EVENT_WAIT.min(deadline.duration_since(now))
        } else {
            MAXIMUM_EVENT_WAIT
        };
        unsafe { ffi::glfw_wait_events_timeout(timeout.as_secs_f64()) };
        ensure_callback_did_not_panic()?;
        present_shell_frame(&window, framebuffer_width, framebuffer_height)?;
    }
    ensure_callback_did_not_panic()
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

fn system_window_scale() -> f32 {
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
    scale.max(1.0)
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

    let window_type = intern_atom(display, c"_NET_WM_WINDOW_TYPE")?;
    let utility_type = intern_atom(display, c"_NET_WM_WINDOW_TYPE_UTILITY")?;
    unsafe {
        ffi::x_change_property(
            display,
            native_window,
            window_type,
            ffi::XA_ATOM,
            32,
            ffi::X_PROP_MODE_REPLACE,
            (&utility_type as *const ffi::Atom).cast(),
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

unsafe extern "C" fn window_focus(window: *mut ffi::GlfwWindow, focused: i32) {
    if focused == ffi::GLFW_TRUE {
        return;
    }
    if std::panic::catch_unwind(|| {
        eprintln!("[host-rs] window lost focus; exiting");
        unsafe { ffi::glfw_set_window_should_close(window, ffi::GLFW_TRUE) };
    })
    .is_err()
    {
        CALLBACK_PANICKED.store(true, Ordering::Release);
        unsafe { ffi::glfw_set_window_should_close(window, ffi::GLFW_TRUE) };
    }
}
