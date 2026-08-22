use std::ffi::{c_char, c_double, c_float, c_int, c_long, c_uchar, c_uint, c_ulong, c_void};

#[repr(C)]
pub struct GlfwWindow {
    _private: [u8; 0],
}

#[repr(C)]
pub struct GlfwMonitor {
    _private: [u8; 0],
}

#[repr(C)]
pub struct GlfwCursor {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Display {
    _private: [u8; 0],
}

pub type GlfwErrorCallback = Option<unsafe extern "C" fn(c_int, *const c_char)>;
pub type GlfwCursorPositionCallback =
    Option<unsafe extern "C" fn(*mut GlfwWindow, c_double, c_double)>;
pub type GlfwCursorEnterCallback = Option<unsafe extern "C" fn(*mut GlfwWindow, c_int)>;
pub type GlfwMouseButtonCallback =
    Option<unsafe extern "C" fn(*mut GlfwWindow, c_int, c_int, c_int)>;
pub type GlfwScrollCallback = Option<unsafe extern "C" fn(*mut GlfwWindow, c_double, c_double)>;
pub type GlfwKeyCallback =
    Option<unsafe extern "C" fn(*mut GlfwWindow, c_int, c_int, c_int, c_int)>;
pub type GlfwCharCallback = Option<unsafe extern "C" fn(*mut GlfwWindow, c_uint)>;
pub type GlfwWindowSizeCallback = Option<unsafe extern "C" fn(*mut GlfwWindow, c_int, c_int)>;
pub type GlfwWindowContentScaleCallback =
    Option<unsafe extern "C" fn(*mut GlfwWindow, c_float, c_float)>;
pub type GlfwWindowFocusCallback = Option<unsafe extern "C" fn(*mut GlfwWindow, c_int)>;
pub type GlfwGlProc = Option<unsafe extern "C" fn()>;
pub type Atom = c_ulong;
pub type XWindow = c_ulong;

pub const GLFW_TRUE: c_int = 1;
pub const GLFW_FALSE: c_int = 0;
pub const GLFW_RELEASE: c_int = 0;
pub const GLFW_PRESS: c_int = 1;
pub const GLFW_REPEAT: c_int = 2;
pub const GLFW_MOUSE_BUTTON_LEFT: c_int = 0;
pub const GLFW_MOUSE_BUTTON_RIGHT: c_int = 1;
pub const GLFW_MOUSE_BUTTON_MIDDLE: c_int = 2;
pub const GLFW_MOUSE_BUTTON_4: c_int = 3;
pub const GLFW_MOUSE_BUTTON_5: c_int = 4;
pub const GLFW_FOCUSED: c_int = 0x0002_0001;
pub const GLFW_RESIZABLE: c_int = 0x0002_0003;
pub const GLFW_VISIBLE: c_int = 0x0002_0004;
pub const GLFW_DECORATED: c_int = 0x0002_0005;
pub const GLFW_FLOATING: c_int = 0x0002_0007;
pub const GLFW_FOCUS_ON_SHOW: c_int = 0x0002_000c;
pub const GLFW_CURSOR: c_int = 0x0003_3001;
pub const GLFW_CURSOR_NORMAL: c_int = 0x0003_4001;
pub const GLFW_CURSOR_HIDDEN: c_int = 0x0003_4002;
pub const GLFW_ARROW_CURSOR: c_int = 0x0003_6001;
pub const GLFW_IBEAM_CURSOR: c_int = 0x0003_6002;
pub const GLFW_CROSSHAIR_CURSOR: c_int = 0x0003_6003;
pub const GLFW_HAND_CURSOR: c_int = 0x0003_6004;
pub const GLFW_HRESIZE_CURSOR: c_int = 0x0003_6005;
pub const GLFW_VRESIZE_CURSOR: c_int = 0x0003_6006;
pub const GLFW_CONTEXT_VERSION_MAJOR: c_int = 0x0002_2002;
pub const GLFW_CONTEXT_VERSION_MINOR: c_int = 0x0002_2003;
pub const GLFW_OPENGL_FORWARD_COMPAT: c_int = 0x0002_2006;
pub const GLFW_OPENGL_PROFILE: c_int = 0x0002_2008;
pub const GLFW_OPENGL_CORE_PROFILE: c_int = 0x0003_2001;

pub const X_NONE: Atom = 0;
pub const X_FALSE: c_int = 0;
pub const X_TRUE: c_int = 1;
pub const X_SUCCESS: c_int = 0;
pub const X_PROP_MODE_REPLACE: c_int = 0;
pub const XA_ATOM: Atom = 4;

pub const GL_VERSION: c_uint = 0x1f02;
pub const GL_COLOR_BUFFER_BIT: c_uint = 0x0000_4000;

unsafe extern "C" {
    #[link_name = "glfwSetErrorCallback"]
    pub fn glfw_set_error_callback(callback: GlfwErrorCallback) -> GlfwErrorCallback;
    #[link_name = "glfwInit"]
    pub fn glfw_init() -> c_int;
    #[link_name = "glfwTerminate"]
    pub fn glfw_terminate();
    #[link_name = "glfwGetPrimaryMonitor"]
    pub fn glfw_get_primary_monitor() -> *mut GlfwMonitor;
    #[link_name = "glfwGetMonitorContentScale"]
    pub fn glfw_get_monitor_content_scale(
        monitor: *mut GlfwMonitor,
        x_scale: *mut c_float,
        y_scale: *mut c_float,
    );
    #[link_name = "glfwWindowHint"]
    pub fn glfw_window_hint(hint: c_int, value: c_int);
    #[link_name = "glfwCreateWindow"]
    pub fn glfw_create_window(
        width: c_int,
        height: c_int,
        title: *const c_char,
        monitor: *mut GlfwMonitor,
        share: *mut GlfwWindow,
    ) -> *mut GlfwWindow;
    #[link_name = "glfwDestroyWindow"]
    pub fn glfw_destroy_window(window: *mut GlfwWindow);
    #[link_name = "glfwWindowShouldClose"]
    pub fn glfw_window_should_close(window: *mut GlfwWindow) -> c_int;
    #[link_name = "glfwSetWindowShouldClose"]
    pub fn glfw_set_window_should_close(window: *mut GlfwWindow, value: c_int);
    #[link_name = "glfwGetWindowAttrib"]
    pub fn glfw_get_window_attrib(window: *mut GlfwWindow, attribute: c_int) -> c_int;
    #[link_name = "glfwSetWindowUserPointer"]
    pub fn glfw_set_window_user_pointer(window: *mut GlfwWindow, pointer: *mut c_void);
    #[link_name = "glfwGetWindowUserPointer"]
    pub fn glfw_get_window_user_pointer(window: *mut GlfwWindow) -> *mut c_void;
    #[link_name = "glfwGetWindowSize"]
    pub fn glfw_get_window_size(window: *mut GlfwWindow, width: *mut c_int, height: *mut c_int);
    #[link_name = "glfwGetFramebufferSize"]
    pub fn glfw_get_framebuffer_size(
        window: *mut GlfwWindow,
        width: *mut c_int,
        height: *mut c_int,
    );
    #[link_name = "glfwGetWindowContentScale"]
    pub fn glfw_get_window_content_scale(
        window: *mut GlfwWindow,
        x_scale: *mut c_float,
        y_scale: *mut c_float,
    );
    #[link_name = "glfwShowWindow"]
    pub fn glfw_show_window(window: *mut GlfwWindow);
    #[link_name = "glfwFocusWindow"]
    pub fn glfw_focus_window(window: *mut GlfwWindow);
    #[link_name = "glfwGetClipboardString"]
    pub fn glfw_get_clipboard_string(window: *mut GlfwWindow) -> *const c_char;
    #[link_name = "glfwSetClipboardString"]
    pub fn glfw_set_clipboard_string(window: *mut GlfwWindow, value: *const c_char);
    #[link_name = "glfwCreateStandardCursor"]
    pub fn glfw_create_standard_cursor(shape: c_int) -> *mut GlfwCursor;
    #[link_name = "glfwDestroyCursor"]
    pub fn glfw_destroy_cursor(cursor: *mut GlfwCursor);
    #[link_name = "glfwSetCursor"]
    pub fn glfw_set_cursor(window: *mut GlfwWindow, cursor: *mut GlfwCursor);
    #[link_name = "glfwSetInputMode"]
    pub fn glfw_set_input_mode(window: *mut GlfwWindow, mode: c_int, value: c_int);
    #[link_name = "glfwSetCursorPosCallback"]
    pub fn glfw_set_cursor_position_callback(
        window: *mut GlfwWindow,
        callback: GlfwCursorPositionCallback,
    ) -> GlfwCursorPositionCallback;
    #[link_name = "glfwSetCursorEnterCallback"]
    pub fn glfw_set_cursor_enter_callback(
        window: *mut GlfwWindow,
        callback: GlfwCursorEnterCallback,
    ) -> GlfwCursorEnterCallback;
    #[link_name = "glfwSetMouseButtonCallback"]
    pub fn glfw_set_mouse_button_callback(
        window: *mut GlfwWindow,
        callback: GlfwMouseButtonCallback,
    ) -> GlfwMouseButtonCallback;
    #[link_name = "glfwSetScrollCallback"]
    pub fn glfw_set_scroll_callback(
        window: *mut GlfwWindow,
        callback: GlfwScrollCallback,
    ) -> GlfwScrollCallback;
    #[link_name = "glfwSetKeyCallback"]
    pub fn glfw_set_key_callback(
        window: *mut GlfwWindow,
        callback: GlfwKeyCallback,
    ) -> GlfwKeyCallback;
    #[link_name = "glfwSetCharCallback"]
    pub fn glfw_set_char_callback(
        window: *mut GlfwWindow,
        callback: GlfwCharCallback,
    ) -> GlfwCharCallback;
    #[link_name = "glfwSetFramebufferSizeCallback"]
    pub fn glfw_set_framebuffer_size_callback(
        window: *mut GlfwWindow,
        callback: GlfwWindowSizeCallback,
    ) -> GlfwWindowSizeCallback;
    #[link_name = "glfwSetWindowSizeCallback"]
    pub fn glfw_set_window_size_callback(
        window: *mut GlfwWindow,
        callback: GlfwWindowSizeCallback,
    ) -> GlfwWindowSizeCallback;
    #[link_name = "glfwSetWindowContentScaleCallback"]
    pub fn glfw_set_window_content_scale_callback(
        window: *mut GlfwWindow,
        callback: GlfwWindowContentScaleCallback,
    ) -> GlfwWindowContentScaleCallback;
    #[link_name = "glfwSetWindowFocusCallback"]
    pub fn glfw_set_window_focus_callback(
        window: *mut GlfwWindow,
        callback: GlfwWindowFocusCallback,
    ) -> GlfwWindowFocusCallback;
    #[link_name = "glfwWaitEventsTimeout"]
    pub fn glfw_wait_events_timeout(timeout: c_double);
    #[link_name = "glfwPostEmptyEvent"]
    pub fn glfw_post_empty_event();
    #[link_name = "glfwMakeContextCurrent"]
    pub fn glfw_make_context_current(window: *mut GlfwWindow);
    #[link_name = "glfwGetCurrentContext"]
    pub fn glfw_get_current_context() -> *mut GlfwWindow;
    #[link_name = "glfwSwapBuffers"]
    pub fn glfw_swap_buffers(window: *mut GlfwWindow);
    #[link_name = "glfwSwapInterval"]
    pub fn glfw_swap_interval(interval: c_int);
    #[link_name = "glfwGetProcAddress"]
    pub fn glfw_get_proc_address(name: *const c_char) -> GlfwGlProc;

    #[link_name = "glfwGetX11Display"]
    pub fn glfw_get_x11_display() -> *mut Display;
    #[link_name = "glfwGetX11Window"]
    pub fn glfw_get_x11_window(window: *mut GlfwWindow) -> XWindow;

    #[link_name = "XDefaultScreen"]
    pub fn x_default_screen(display: *mut Display) -> c_int;
    #[link_name = "XInternAtom"]
    pub fn x_intern_atom(display: *mut Display, name: *const c_char, only_if_exists: c_int)
        -> Atom;
    #[link_name = "XGetSelectionOwner"]
    pub fn x_get_selection_owner(display: *mut Display, selection: Atom) -> XWindow;
    #[link_name = "XGetWindowProperty"]
    pub fn x_get_window_property(
        display: *mut Display,
        window: XWindow,
        property: Atom,
        offset: c_long,
        length: c_long,
        delete: c_int,
        requested_type: Atom,
        actual_type: *mut Atom,
        actual_format: *mut c_int,
        item_count: *mut c_ulong,
        remaining: *mut c_ulong,
        data: *mut *mut c_uchar,
    ) -> c_int;
    #[link_name = "XChangeProperty"]
    pub fn x_change_property(
        display: *mut Display,
        window: XWindow,
        property: Atom,
        property_type: Atom,
        format: c_int,
        mode: c_int,
        data: *const c_uchar,
        element_count: c_int,
    ) -> c_int;
    #[link_name = "XSetWindowBackground"]
    pub fn x_set_window_background(
        display: *mut Display,
        window: XWindow,
        background_pixel: c_ulong,
    ) -> c_int;
    #[link_name = "XFlush"]
    pub fn x_flush(display: *mut Display) -> c_int;
    #[link_name = "XFree"]
    pub fn x_free(data: *mut c_void) -> c_int;

    #[link_name = "glViewport"]
    pub fn gl_viewport(x: c_int, y: c_int, width: c_int, height: c_int);
    #[link_name = "glClearColor"]
    pub fn gl_clear_color(red: c_float, green: c_float, blue: c_float, alpha: c_float);
    #[link_name = "glClear"]
    pub fn gl_clear(mask: c_uint);
    #[link_name = "glFinish"]
    pub fn gl_finish();
    #[link_name = "glGetString"]
    pub fn gl_get_string(name: c_uint) -> *const c_uchar;
}
