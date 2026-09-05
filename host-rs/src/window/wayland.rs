use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::io;
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

use khronos_egl as egl;
use lynx_launcher_host::runtime::{DesktopApi, EventWake, GlApi, GlContextBinding};
use lynx_launcher_host::support::{logical_key, physical_key, utf8_from_codepoint, WindowMetrics};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::{
    wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_egl::WlEglSurface;
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{self, WpCursorShapeDeviceV1},
    wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

use super::{
    log_metrics, BackendRuntimeAdapters, KeyAction, PointerButton, WindowBackend, WindowEvent,
};

const LOGICAL_WIDTH: u32 = 1120;
const LOGICAL_HEIGHT: u32 = 760;
const MINIMUM_SEAT_VERSION: u32 = 3;
const KEYBOARD_RELEASE_VERSION: u32 = 3;
const SEAT_RELEASE_VERSION: u32 = 5;
const POINTER_RELEASE_VERSION: u32 = 3;
const PLACEMENT_PROBE_HOLD: Duration = Duration::from_secs(5);
const STRANDED_GL_LOG: &[u8] = b"[host-rs] fatal: renderer stranded an EGL context; \
skipping native Wayland/EGL cleanup and exiting for OS cleanup\n";
const NATIVE_CALLBACKS_LOG: &[u8] = b"[host-rs] fatal: native callbacks did not quiesce; \
skipping native Wayland/EGL cleanup and exiting for OS cleanup\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LayerIntent {
    width: u32,
    height: u32,
    layer: zwlr_layer_shell_v1::Layer,
    keyboard: zwlr_layer_surface_v1::KeyboardInteractivity,
    exclusive_zone: i32,
}

struct OutputRecord {
    proxy: wl_output::WlOutput,
    identity: u32,
    name: Option<String>,
}

#[repr(C)]
struct XkbContext(c_void);
#[repr(C)]
struct XkbKeymap(c_void);
#[repr(C)]
struct XkbStateHandle(c_void);

struct XkbKeyboard {
    context: NonNull<XkbContext>,
    keymap: NonNull<XkbKeymap>,
    state: NonNull<XkbStateHandle>,
}

impl XkbKeyboard {
    fn from_keymap(fd: OwnedFd, size: u32) -> io::Result<Self> {
        let size =
            usize::try_from(size).map_err(|_| io::Error::other("XKB keymap is too large"))?;
        if size == 0 || size > 16 * 1024 * 1024 {
            return Err(io::Error::other("XKB keymap has an invalid size"));
        }
        let mapping = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                fd.as_raw_fd(),
                0,
            )
        };
        if mapping == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let context = NonNull::new(unsafe { xkb_context_new(0) })
                .ok_or_else(|| io::Error::other("xkbcommon could not create a context"))?;
            let keymap = NonNull::new(unsafe {
                xkb_keymap_new_from_string(context.as_ptr(), mapping.cast(), 1, 0)
            });
            let Some(keymap) = keymap else {
                unsafe { xkb_context_unref(context.as_ptr()) };
                return Err(io::Error::other("xkbcommon rejected the compositor keymap"));
            };
            let state = NonNull::new(unsafe { xkb_state_new(keymap.as_ptr()) });
            let Some(state) = state else {
                unsafe {
                    xkb_keymap_unref(keymap.as_ptr());
                    xkb_context_unref(context.as_ptr());
                }
                return Err(io::Error::other(
                    "xkbcommon could not create keyboard state",
                ));
            };
            Ok(Self {
                context,
                keymap,
                state,
            })
        })();
        unsafe { libc::munmap(mapping, size) };
        result
    }

    fn update_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        unsafe {
            xkb_state_update_mask(self.state.as_ptr(), depressed, latched, locked, 0, 0, group);
        }
    }

    fn translate_key(&self, key: u32) -> Option<TranslatedKey> {
        let keycode = key.checked_add(8)?;
        let glfw_key = evdev_to_glfw(key).or_else(|| {
            keysym_to_glfw(unsafe { xkb_state_key_get_one_sym(self.state.as_ptr(), keycode) })
        });
        let physical = evdev_to_glfw(key).map(physical_key).unwrap_or(0);
        glfw_key.and_then(|glfw_key| {
            (physical != 0).then(|| {
                let codepoint = unsafe { xkb_state_key_get_utf32(self.state.as_ptr(), keycode) };
                TranslatedKey {
                    id: i32::try_from(key).unwrap_or(i32::MAX),
                    physical,
                    logical: logical_key(glfw_key),
                    text: utf8_from_codepoint(codepoint),
                    repeats: unsafe { xkb_keymap_key_repeats(self.keymap.as_ptr(), keycode) != 0 },
                }
            })
        })
    }
}

impl Drop for XkbKeyboard {
    fn drop(&mut self) {
        unsafe {
            xkb_state_unref(self.state.as_ptr());
            xkb_keymap_unref(self.keymap.as_ptr());
            xkb_context_unref(self.context.as_ptr());
        }
    }
}

struct TranslatedKey {
    id: i32,
    physical: u64,
    logical: u64,
    text: String,
    repeats: bool,
}

#[derive(Clone)]
struct RepeatingKey {
    key: u32,
    next: Instant,
    interval: Duration,
}

#[derive(Default)]
struct PendingScroll {
    x: f64,
    y: f64,
    discrete_x: Option<f64>,
    discrete_y: Option<f64>,
}

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_context_new(flags: u32) -> *mut XkbContext;
    fn xkb_context_unref(context: *mut XkbContext);
    fn xkb_keymap_new_from_string(
        context: *mut XkbContext,
        string: *const c_char,
        format: u32,
        flags: u32,
    ) -> *mut XkbKeymap;
    fn xkb_keymap_unref(keymap: *mut XkbKeymap);
    fn xkb_keymap_key_repeats(keymap: *mut XkbKeymap, key: u32) -> c_int;
    fn xkb_state_new(keymap: *mut XkbKeymap) -> *mut XkbStateHandle;
    fn xkb_state_unref(state: *mut XkbStateHandle);
    fn xkb_state_update_mask(
        state: *mut XkbStateHandle,
        depressed_mods: u32,
        latched_mods: u32,
        locked_mods: u32,
        depressed_layout: u32,
        latched_layout: u32,
        locked_layout: u32,
    ) -> u32;
    fn xkb_state_key_get_one_sym(state: *mut XkbStateHandle, key: u32) -> u32;
    fn xkb_state_key_get_utf32(state: *mut XkbStateHandle, key: u32) -> u32;
}

const fn layer_intent() -> LayerIntent {
    LayerIntent {
        width: LOGICAL_WIDTH,
        height: LOGICAL_HEIGHT,
        layer: zwlr_layer_shell_v1::Layer::Overlay,
        keyboard: zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
        exclusive_zone: -1,
    }
}

struct WaylandState {
    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<ZwlrLayerShellV1>,
    cursor_shape_manager: Option<WpCursorShapeManagerV1>,
    outputs: HashMap<u32, OutputRecord>,
    surface_outputs: HashSet<u32>,
    seat_global_name: Option<u32>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    cursor_shape_device: Option<WpCursorShapeDeviceV1>,
    xkb: Option<XkbKeyboard>,
    repeat_rate: u32,
    repeat_delay: Duration,
    repeating_key: Option<RepeatingKey>,
    pending_scroll: PendingScroll,
    pointer_enter_serial: Option<u32>,
    applied_cursor: Option<(bool, i32, u32)>,
    configured: bool,
    configured_size: (u32, u32),
    scale: i32,
    focused: bool,
    startup_focus_lost: bool,
    close: bool,
    failed: bool,
    delivering_events: bool,
    events: VecDeque<WindowEvent>,
}

impl Default for WaylandState {
    fn default() -> Self {
        Self {
            compositor: None,
            layer_shell: None,
            cursor_shape_manager: None,
            outputs: HashMap::new(),
            surface_outputs: HashSet::new(),
            seat_global_name: None,
            seat: None,
            keyboard: None,
            pointer: None,
            cursor_shape_device: None,
            xkb: None,
            repeat_rate: 0,
            repeat_delay: Duration::ZERO,
            repeating_key: None,
            pending_scroll: PendingScroll::default(),
            pointer_enter_serial: None,
            applied_cursor: None,
            configured: false,
            configured_size: (LOGICAL_WIDTH, LOGICAL_HEIGHT),
            scale: 1,
            focused: false,
            startup_focus_lost: false,
            close: false,
            failed: false,
            delivering_events: false,
            events: VecDeque::new(),
        }
    }
}

impl WaylandState {
    fn set_keyboard_focus(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        let lost_focus = self.focused && !focused;
        self.focused = focused;
        if self.delivering_events {
            self.events.push_back(WindowEvent::Focused(focused));
        } else if lost_focus {
            self.startup_focus_lost = true;
        }
    }

    fn activate_event_delivery(&mut self) {
        self.delivering_events = true;
        if self.startup_focus_lost {
            self.events.push_back(WindowEvent::Focused(false));
        } else if self.focused {
            self.events.push_back(WindowEvent::Focused(true));
        }
    }

    fn remove_keyboard_capability(&mut self) {
        self.repeating_key = None;
        self.xkb = None;
        self.set_keyboard_focus(false);
    }

    fn remove_pointer_capability(&mut self, had_pointer: bool) {
        self.clear_pointer_capability();
        if self.delivering_events && had_pointer {
            self.events.push_back(WindowEvent::PointerCancelled);
        }
    }

    fn clear_pointer_capability(&mut self) {
        self.cursor_shape_device = None;
        self.pending_scroll = PendingScroll::default();
        self.pointer_enter_serial = None;
        self.applied_cursor = None;
    }

    fn finish_seat_removal(&mut self) {
        self.seat_global_name = None;
        self.remove_keyboard_capability();
        self.clear_pointer_capability();
        if self.delivering_events {
            self.events.push_back(WindowEvent::InputCancelled);
        }
    }

    fn apply_layer_configure(&mut self, width: u32, height: u32) {
        if self.configured && self.delivering_events {
            self.failed = true;
            self.close = true;
            return;
        }
        self.configured = true;
        self.configured_size = (width.max(LOGICAL_WIDTH), height.max(LOGICAL_HEIGHT));
    }

    fn surface_entered_output(&mut self, identity: u32) {
        if self.delivering_events && !self.surface_outputs.contains(&identity) {
            self.fail_output_change();
        } else {
            self.surface_outputs.insert(identity);
        }
    }

    fn surface_left_output(&mut self, identity: u32) {
        if self.delivering_events {
            self.fail_output_change();
        } else {
            self.surface_outputs.remove(&identity);
        }
    }

    fn output_global_removed(&mut self, global_name: u32) {
        let Some(output) = self.outputs.remove(&global_name) else {
            return;
        };
        if self.surface_outputs.remove(&output.identity) && self.delivering_events {
            self.fail_output_change();
        }
    }

    fn fail_output_change(&mut self) {
        self.failed = true;
        self.close = true;
    }

    fn handle_key(&mut self, key: u32, pressed: bool) {
        if !pressed
            && self
                .repeating_key
                .as_ref()
                .is_some_and(|repeat| repeat.key == key)
        {
            self.repeating_key = None;
        }
        let Some(translated) = self
            .xkb
            .as_ref()
            .and_then(|keyboard| keyboard.translate_key(key))
        else {
            return;
        };
        if !self.delivering_events {
            return;
        }
        self.events.push_back(WindowEvent::Key {
            id: translated.id,
            physical: translated.physical,
            logical: translated.logical,
            action: if pressed {
                KeyAction::Press
            } else {
                KeyAction::Release
            },
        });
        if pressed {
            self.queue_key_text(&translated.text);
            if translated.repeats && self.repeat_rate > 0 {
                self.repeating_key = Some(RepeatingKey {
                    key,
                    next: Instant::now() + self.repeat_delay,
                    interval: Duration::from_nanos(
                        (1_000_000_000 / u64::from(self.repeat_rate)).max(1),
                    ),
                });
            }
        }
    }

    fn queue_key_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.events.push_back(WindowEvent::Text {
                codepoint: text.chars().next().map(u32::from).unwrap_or(0),
                text: text.to_owned(),
            });
        }
    }

    fn queue_due_repeats(&mut self, now: Instant) {
        for _ in 0..32 {
            let Some(repeat) = self.repeating_key.as_mut() else {
                break;
            };
            if repeat.next > now {
                break;
            }
            let key = repeat.key;
            repeat.next += repeat.interval;
            let Some(key) = self
                .xkb
                .as_ref()
                .and_then(|keyboard| keyboard.translate_key(key))
                .filter(|key| key.repeats)
            else {
                self.repeating_key = None;
                break;
            };
            self.events.push_back(WindowEvent::Key {
                id: key.id,
                physical: key.physical,
                logical: key.logical,
                action: KeyAction::Repeat,
            });
            self.queue_key_text(&key.text);
        }
    }

    fn next_repeat_deadline(&self) -> Option<Instant> {
        self.repeating_key.as_ref().map(|repeat| repeat.next)
    }

    fn update_repeat_info(&mut self, rate: i32, delay: i32, now: Instant) {
        if rate <= 0 || delay < 0 {
            self.repeat_rate = 0;
            self.repeating_key = None;
            return;
        }
        self.repeat_rate = u32::try_from(rate).unwrap_or(0);
        self.repeat_delay = Duration::from_millis(u64::try_from(delay).unwrap_or(0));
        if let Some(repeat) = self.repeating_key.as_mut() {
            repeat.interval =
                Duration::from_nanos((1_000_000_000 / u64::from(self.repeat_rate)).max(1));
            repeat.next = now + self.repeat_delay;
        }
    }

    fn flush_scroll(&mut self) {
        let x = self
            .pending_scroll
            .discrete_x
            .unwrap_or(self.pending_scroll.x / 10.0);
        let y = self
            .pending_scroll
            .discrete_y
            .unwrap_or(self.pending_scroll.y / 10.0);
        self.pending_scroll = PendingScroll::default();
        if (x != 0.0 || y != 0.0) && self.delivering_events {
            self.events.push_back(WindowEvent::Scroll(x, y));
        }
    }

    fn apply_cursor(&mut self, hidden: bool, shape: i32) {
        let Some(serial) = self.pointer_enter_serial else {
            return;
        };
        if self.applied_cursor == Some((hidden, shape, serial)) {
            return;
        }
        if hidden {
            if let Some(pointer) = self.pointer.as_ref() {
                pointer.set_cursor(serial, None, 0, 0);
            }
            self.applied_cursor = Some((hidden, shape, serial));
            return;
        }
        let requested_shape = shape;
        let shape = match requested_shape {
            1 => wp_cursor_shape_device_v1::Shape::Pointer,
            2 => wp_cursor_shape_device_v1::Shape::Text,
            3 => wp_cursor_shape_device_v1::Shape::Crosshair,
            4 => wp_cursor_shape_device_v1::Shape::EwResize,
            5 => wp_cursor_shape_device_v1::Shape::NsResize,
            _ => wp_cursor_shape_device_v1::Shape::Default,
        };
        if let Some(cursor) = self.cursor_shape_device.as_ref() {
            cursor.set_shape(serial, shape);
            self.applied_cursor = Some((hidden, requested_shape, serial));
        }
    }
}

const fn seat_binding_version(advertised: u32) -> Option<u32> {
    if advertised < MINIMUM_SEAT_VERSION {
        None
    } else {
        Some(if advertised < 9 { advertised } else { 9 })
    }
}

const fn pointer_release_supported(version: u32) -> bool {
    version >= POINTER_RELEASE_VERSION
}

fn evdev_to_glfw(key: u32) -> Option<i32> {
    Some(match key {
        1 => 256,
        2..=10 => 49 + i32::try_from(key - 2).ok()?,
        11 => 48,
        12 => 45,
        13 => 61,
        14 => 259,
        15 => 258,
        16..=25 => [81, 87, 69, 82, 84, 89, 85, 73, 79, 80][usize::try_from(key - 16).ok()?],
        26 => 91,
        27 => 93,
        28 => 257,
        29 => 341,
        30..=38 => [65, 83, 68, 70, 71, 72, 74, 75, 76][usize::try_from(key - 30).ok()?],
        39 => 59,
        40 => 39,
        41 => 96,
        42 => 340,
        43 => 92,
        44..=50 => [90, 88, 67, 86, 66, 78, 77][usize::try_from(key - 44).ok()?],
        51 => 44,
        52 => 46,
        53 => 47,
        54 => 344,
        56 => 342,
        57 => 32,
        58 => 280,
        59..=68 => 290 + i32::try_from(key - 59).ok()?,
        87 => 300,
        88 => 301,
        97 => 345,
        100 => 346,
        102 => 268,
        103 => 265,
        104 => 266,
        105 => 263,
        106 => 262,
        107 => 269,
        108 => 264,
        109 => 267,
        110 => 260,
        111 => 261,
        119 => 284,
        125 => 343,
        126 => 347,
        127 => 348,
        _ => return None,
    })
}

fn keysym_to_glfw(keysym: u32) -> Option<i32> {
    Some(match keysym {
        0x20..=0x7e => {
            let character = char::from_u32(keysym)?;
            if character.is_ascii_alphabetic() {
                i32::from(character.to_ascii_uppercase() as u8)
            } else {
                i32::from(character as u8)
            }
        }
        0xff08 => 259,
        0xff09 => 258,
        0xff0d => 257,
        0xff1b => 256,
        0xff50 => 268,
        0xff51 => 263,
        0xff52 => 265,
        0xff53 => 262,
        0xff54 => 264,
        0xff55 => 266,
        0xff56 => 267,
        0xff57 => 269,
        0xff63 => 260,
        0xffff => 261,
        0xffbe..=0xffc9 => 290 + i32::try_from(keysym - 0xffbe).ok()?,
        0xffe1 => 340,
        0xffe2 => 344,
        0xffe3 => 341,
        0xffe4 => 345,
        0xffe7 => 343,
        0xffe8 => 347,
        0xffe9 => 342,
        0xffea => 346,
        0xffeb => 343,
        0xffec => 347,
        _ => return None,
    })
}

const fn keyboard_release_supported(version: u32) -> bool {
    version >= KEYBOARD_RELEASE_VERSION
}

const fn seat_release_supported(version: u32) -> bool {
    version >= SEAT_RELEASE_VERSION
}

struct WakeFd {
    fd: RawFd,
    failed: AtomicBool,
}

impl WakeFd {
    fn new() -> io::Result<Self> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self {
                fd,
                failed: AtomicBool::new(false),
            })
        }
    }

    fn drain(&self) {
        let mut value = 0_u64;
        while unsafe {
            libc::read(
                self.fd,
                (&mut value as *mut u64).cast(),
                std::mem::size_of::<u64>(),
            )
        } > 0
        {}
    }
}

impl Drop for WakeFd {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

struct EglRenderTarget {
    display: egl::Display,
    context: egl::Context,
    surface: egl::Surface,
    window: WlEglSurface,
    failed: AtomicBool,
    cursor_shape: AtomicI32,
    cursor_hidden: AtomicBool,
    wake: NonNull<WakeFd>,
}

unsafe impl Send for EglRenderTarget {}
// Renderer callbacks only read immutable EGL handles and the atomic failure
// latch. The non-Sync wl_egl_window is touched only by the platform thread
// before callback registration and after runtime callbacks have quiesced.
unsafe impl Sync for EglRenderTarget {}

impl EglRenderTarget {
    fn create(
        connection: &Connection,
        surface: &wl_surface::WlSurface,
        scale: i32,
        wake: NonNull<WakeFd>,
    ) -> io::Result<Self> {
        let width = i32::try_from(LOGICAL_WIDTH).unwrap() * scale;
        let height = i32::try_from(LOGICAL_HEIGHT).unwrap() * scale;
        surface.set_buffer_scale(scale);
        let window = WlEglSurface::new(surface.id(), width, height).map_err(io::Error::other)?;
        let native_display = connection.display().id().as_ptr().cast();
        let display = unsafe { egl::API.get_display(native_display) }
            .ok_or_else(|| io::Error::other("EGL could not open the Wayland display"))?;
        egl::API.initialize(display).map_err(egl_error)?;
        let initialized = (|| {
            egl::API.bind_api(egl::OPENGL_API).map_err(egl_error)?;
            let config = egl::API
                .choose_first_config(
                    display,
                    &[
                        egl::SURFACE_TYPE,
                        egl::WINDOW_BIT,
                        egl::RENDERABLE_TYPE,
                        egl::OPENGL_BIT,
                        egl::RED_SIZE,
                        8,
                        egl::GREEN_SIZE,
                        8,
                        egl::BLUE_SIZE,
                        8,
                        egl::ALPHA_SIZE,
                        8,
                        egl::NONE,
                    ],
                )
                .map_err(egl_error)?
                .ok_or_else(|| io::Error::other("EGL found no OpenGL window configuration"))?;
            let context = egl::API
                .create_context(
                    display,
                    config,
                    None,
                    &[
                        egl::CONTEXT_MAJOR_VERSION,
                        3,
                        egl::CONTEXT_MINOR_VERSION,
                        3,
                        egl::CONTEXT_OPENGL_PROFILE_MASK,
                        egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
                        egl::NONE,
                    ],
                )
                .map_err(egl_error)?;
            let egl_surface = match unsafe {
                egl::API.create_window_surface(display, config, window.ptr().cast_mut(), None)
            } {
                Ok(surface) => surface,
                Err(error) => {
                    let _ = egl::API.destroy_context(display, context);
                    return Err(egl_error(error));
                }
            };
            Ok((context, egl_surface))
        })();
        let (context, egl_surface) = match initialized {
            Ok(resources) => resources,
            Err(error) => {
                let _ = egl::API.terminate(display);
                return Err(error);
            }
        };
        Ok(Self {
            display,
            context,
            surface: egl_surface,
            window,
            failed: AtomicBool::new(false),
            cursor_shape: AtomicI32::new(0),
            cursor_hidden: AtomicBool::new(false),
            wake,
        })
    }

    fn binding(&self) -> GlContextBinding {
        egl_target_binding(
            self.display.as_ptr(),
            self.context.as_ptr(),
            self.surface.as_ptr(),
        )
    }

    fn is_current_binding(&self, binding: GlContextBinding) -> bool {
        binding == self.binding()
    }

    fn present_shell_frame(&self, color: [f32; 3]) -> io::Result<()> {
        egl::API
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .map_err(egl_error)?;
        unsafe {
            glViewport(0, 0, self.window.get_size().0, self.window.get_size().1);
            glClearColor(color[0], color[1], color[2], 1.0);
            glClear(0x0000_4000);
            glFinish();
        }
        egl::API
            .swap_buffers(self.display, self.surface)
            .map_err(egl_error)?;
        egl::API
            .make_current(self.display, None, None, None)
            .map_err(egl_error)
    }

    fn resize(&self, scale: i32) {
        self.window.resize(
            i32::try_from(LOGICAL_WIDTH).unwrap() * scale,
            i32::try_from(LOGICAL_HEIGHT).unwrap() * scale,
            0,
            0,
        );
    }
}

impl Drop for EglRenderTarget {
    fn drop(&mut self) {
        if self.is_current_binding(capture_egl_binding()) {
            let _ = egl::API.make_current(self.display, None, None, None);
        }
        let _ = egl::API.destroy_surface(self.display, self.surface);
        let _ = egl::API.destroy_context(self.display, self.context);
        let _ = egl::API.terminate(self.display);
    }
}

pub(super) struct WaylandBackend {
    connection: Connection,
    queue: RefCell<EventQueue<WaylandState>>,
    state: RefCell<WaylandState>,
    surface: wl_surface::WlSurface,
    layer_surface: ZwlrLayerSurfaceV1,
    layer_shell: ZwlrLayerShellV1,
    wake: Box<WakeFd>,
    target: Option<Box<EglRenderTarget>>,
    metrics: WindowMetrics,
}

impl WaylandBackend {
    pub(super) fn initialize(requested_output: Option<&str>) -> io::Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_none_or(|value| value.is_empty()) {
            return Err(io::Error::other(
                "WAYLAND_DISPLAY is not set; native Wayland was explicitly requested",
            ));
        }
        let connection = Connection::connect_to_env()
            .map_err(|error| io::Error::other(format!("could not connect to Wayland: {error}")))?;
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut state = WaylandState::default();
        queue.roundtrip(&mut state).map_err(wayland_error)?;
        if requested_output.is_some() {
            queue.roundtrip(&mut state).map_err(wayland_error)?;
        }
        if state.failed {
            return Err(io::Error::other(
                "the compositor advertises wl_seat below required version 3",
            ));
        }
        let compositor = state
            .compositor
            .as_ref()
            .ok_or_else(|| io::Error::other("Wayland compositor global is unavailable"))?;
        let layer_shell = state
            .layer_shell
            .clone()
            .ok_or_else(|| io::Error::other("wlr-layer-shell-v1 is unavailable"))?;
        let surface = compositor.create_surface(&qh, ());
        let intent = layer_intent();
        let output = requested_output
            .map(|requested| {
                state
                    .outputs
                    .values()
                    .find(|output| output.name.as_deref() == Some(requested))
                    .map(|output| &output.proxy)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("Wayland placement output is unavailable: {requested}"),
                        )
                    })
            })
            .transpose()?;
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            output,
            intent.layer,
            "lynx-launcher".into(),
            &qh,
            (),
        );
        layer_surface.set_size(intent.width, intent.height);
        layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::empty());
        layer_surface.set_keyboard_interactivity(intent.keyboard);
        layer_surface.set_exclusive_zone(intent.exclusive_zone);
        surface.commit();
        while !state.configured && !state.close {
            queue.blocking_dispatch(&mut state).map_err(wayland_error)?;
        }
        if state.close {
            return Err(io::Error::other(
                "the compositor closed the layer surface before its initial configure",
            ));
        }
        eprintln!(
            "[host-rs] native Wayland layer-shell configured logical={}x{} output={} anchor=none layer=overlay keyboard=on-demand exclusive-zone={}",
            state.configured_size.0,
            state.configured_size.1,
            requested_output.unwrap_or("compositor-selected"),
            intent.exclusive_zone
        );
        let scale = state.scale.max(1);
        Ok(Self {
            connection,
            queue: RefCell::new(queue),
            state: RefCell::new(state),
            surface,
            layer_surface,
            layer_shell,
            wake: Box::new(WakeFd::new()?),
            target: None,
            metrics: metrics(scale),
        })
    }

    fn dispatch_pending(&self) -> io::Result<()> {
        self.queue
            .borrow_mut()
            .dispatch_pending(&mut self.state.borrow_mut())
            .map_err(wayland_error)?;
        Ok(())
    }

    fn show_frame(&mut self, color: [f32; 3], marker: &str) -> io::Result<()> {
        let scale = self.state.borrow().scale.max(1);
        let target = Box::new(EglRenderTarget::create(
            &self.connection,
            &self.surface,
            scale,
            NonNull::from(&*self.wake),
        )?);
        target.present_shell_frame(color)?;
        self.queue
            .borrow_mut()
            .roundtrip(&mut self.state.borrow_mut())
            .map_err(wayland_error)?;
        let preferred_scale = self.state.borrow().scale.max(1);
        if preferred_scale != scale {
            self.surface.set_buffer_scale(preferred_scale);
            target.resize(preferred_scale);
            target.present_shell_frame(color)?;
        }
        self.target = Some(target);
        self.metrics = metrics(preferred_scale);
        log_metrics(self.metrics);
        eprintln!("[host-rs] OpenGL 3.3 via native Wayland layer-shell + EGL");
        eprintln!("{marker}");
        Ok(())
    }
}

pub(super) fn run_placement_probe(output: &str) -> io::Result<()> {
    let mut backend = WaylandBackend::initialize(Some(output))?;
    backend.show_frame(
        [1.0, 0.0, 1.0],
        "[host-rs] E2E Wayland placement probe frame presented",
    )?;
    backend.activate_event_delivery()?;
    let deadline = std::time::Instant::now() + PLACEMENT_PROBE_HOLD;
    while std::time::Instant::now() < deadline && !backend.should_close() {
        backend.ensure_healthy()?;
        backend.wait_for_events(
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(Duration::from_millis(250)),
        );
        backend.drain_events()?;
    }
    backend.ensure_healthy()?;
    eprintln!("[host-rs] E2E Wayland placement probe shutdown complete");
    Ok(())
}

impl WindowBackend for WaylandBackend {
    fn show(&mut self) -> io::Result<()> {
        self.show_frame(
            [0.035, 0.039, 0.043],
            "[host-rs] first shell GL frame presented",
        )?;
        let state = self.state.borrow();
        eprintln!(
            "[host-rs] native Wayland input: pointer+keyboard+xkb-repeat+UTF-8; cursor-shape-v1={} clipboard=unsupported IME-composition=unsupported",
            state.cursor_shape_manager.is_some()
        );
        Ok(())
    }

    fn runtime_adapters(&self) -> BackendRuntimeAdapters {
        let target = self
            .target
            .as_deref()
            .expect("Wayland backend was not shown");
        let desktop = if self.state.borrow().cursor_shape_manager.is_some() {
            DesktopApi::cursor_only(
                runtime_create_cursor,
                runtime_destroy_cursor,
                runtime_set_cursor,
                runtime_set_cursor_mode,
            )
        } else {
            DesktopApi::unsupported()
        };
        BackendRuntimeAdapters {
            render_target: (target as *const EglRenderTarget).cast_mut().cast(),
            gl: GlApi::new(
                runtime_make_current,
                runtime_capture_binding,
                runtime_is_current,
                runtime_restore_binding,
                runtime_swap_buffers,
                runtime_get_proc_address,
            ),
            desktop,
            wake: EventWake::new(
                (&*self.wake as *const WakeFd).cast_mut().cast(),
                runtime_wake_event_loop,
            ),
            initial_metrics: self.metrics,
        }
    }

    fn activate_event_delivery(&mut self) -> io::Result<()> {
        let mut state = self.state.borrow_mut();
        state.activate_event_delivery();
        Ok(())
    }

    fn drain_events(&self) -> io::Result<Vec<WindowEvent>> {
        self.dispatch_pending()?;
        let mut state = self.state.borrow_mut();
        state.queue_due_repeats(Instant::now());
        if let Some(target) = self.target.as_deref() {
            state.apply_cursor(
                target.cursor_hidden.load(Ordering::Acquire),
                target.cursor_shape.load(Ordering::Acquire),
            );
        }
        Ok(state.events.drain(..).collect())
    }

    fn wait_for_events(&self, timeout: Duration) {
        if self.dispatch_pending().is_err() {
            let mut state = self.state.borrow_mut();
            state.failed = true;
            state.close = true;
            return;
        }
        let queue = self.queue.borrow();
        let needs_write = match flush_for_poll(&queue) {
            Ok(needs_write) => needs_write,
            Err(()) => {
                let mut state = self.state.borrow_mut();
                state.failed = true;
                state.close = true;
                return;
            }
        };
        let Some(read_guard) = queue.prepare_read() else {
            return;
        };
        let timeout = self
            .state
            .borrow()
            .next_repeat_deadline()
            .map(|deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(timeout)
            })
            .unwrap_or(timeout);
        let timeout_ms = timeout.as_millis().max(1).min(i32::MAX as u128) as i32;
        let mut fds = [
            libc::pollfd {
                fd: queue.as_fd().as_raw_fd(),
                events: libc::POLLIN | if needs_write { libc::POLLOUT } else { 0 },
                revents: 0,
            },
            libc::pollfd {
                fd: self.wake.fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let status = loop {
            let status =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
            if status >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                break status;
            }
        };
        if status < 0 {
            let mut state = self.state.borrow_mut();
            state.failed = true;
            state.close = true;
            return;
        }
        if fds[1].revents & libc::POLLIN != 0 {
            self.wake.drain();
        }
        if fds[0].revents & libc::POLLOUT != 0 && flush_for_poll(&queue).is_err() {
            let mut state = self.state.borrow_mut();
            state.failed = true;
            state.close = true;
            return;
        }
        if fds[0].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0
            && read_guard.read().is_err()
        {
            let mut state = self.state.borrow_mut();
            state.failed = true;
            state.close = true;
        }
    }

    fn should_close(&self) -> bool {
        self.state.borrow().close
    }

    fn request_close(&self) {
        self.state.borrow_mut().close = true;
        if !unsafe { runtime_wake_event_loop((&*self.wake as *const WakeFd).cast_mut().cast()) } {
            self.state.borrow_mut().failed = true;
        }
    }

    fn set_text_input_active(&self, _active: bool) -> io::Result<()> {
        Ok(())
    }

    fn stop_event_delivery(&mut self) {
        self.state.borrow_mut().delivering_events = false;
    }

    fn ensure_healthy(&self) -> io::Result<()> {
        if self.state.borrow().failed
            || self.wake.failed.load(Ordering::Acquire)
            || self
                .target
                .as_deref()
                .is_some_and(|target| target.failed.load(Ordering::Acquire))
        {
            Err(io::Error::other("a native Wayland EGL callback failed"))
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

impl Drop for WaylandBackend {
    fn drop(&mut self) {
        self.stop_event_delivery();
        drop(self.target.take());
        let mut state = self.state.borrow_mut();
        if let Some(cursor_shape_device) = state.cursor_shape_device.take() {
            cursor_shape_device.destroy();
        }
        if let Some(pointer) = state.pointer.take() {
            if pointer_release_supported(pointer.version()) {
                pointer.release();
            }
        }
        if let Some(keyboard) = state.keyboard.take() {
            if keyboard_release_supported(keyboard.version()) {
                keyboard.release();
            }
        }
        if let Some(seat) = state.seat.take() {
            if seat_release_supported(seat.version()) {
                seat.release();
            }
        }
        if let Some(manager) = state.cursor_shape_manager.take() {
            manager.destroy();
        }
        drop(state);
        self.layer_surface.destroy();
        self.surface.destroy();
        self.layer_shell.destroy();
    }
}

fn metrics(scale: i32) -> WindowMetrics {
    let scale = scale.max(1) as f32;
    WindowMetrics {
        logical_width: LOGICAL_WIDTH as f32,
        logical_height: LOGICAL_HEIGHT as f32,
        pixel_ratio: scale,
        framebuffer_scale_x: scale,
        framebuffer_scale_y: scale,
    }
}

fn egl_error(error: egl::Error) -> io::Error {
    io::Error::other(format!("EGL error: {error}"))
}

fn wayland_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("Wayland error: {error}"))
}

fn flush_for_poll(queue: &EventQueue<WaylandState>) -> Result<bool, ()> {
    classify_flush_result(queue.flush())
}

fn classify_flush_result(result: Result<(), WaylandError>) -> Result<bool, ()> {
    match result {
        Ok(()) => Ok(false),
        Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(true),
        Err(_) => Err(()),
    }
}

fn capture_egl_binding() -> GlContextBinding {
    GlContextBinding::new(
        egl::API
            .get_current_display()
            .map_or(std::ptr::null_mut(), |value| value.as_ptr()),
        egl::API
            .get_current_context()
            .map_or(std::ptr::null_mut(), |value| value.as_ptr()),
        egl::API
            .get_current_surface(egl::DRAW)
            .map_or(std::ptr::null_mut(), |value| value.as_ptr()),
        egl::API
            .get_current_surface(egl::READ)
            .map_or(std::ptr::null_mut(), |value| value.as_ptr()),
    )
}

fn egl_target_binding(
    display: *mut c_void,
    context: *mut c_void,
    surface: *mut c_void,
) -> GlContextBinding {
    GlContextBinding::new(display, context, surface, surface)
}

unsafe fn target<'a>(render_target: *mut c_void) -> Option<&'a EglRenderTarget> {
    NonNull::new(render_target.cast::<EglRenderTarget>()).map(|target| unsafe { target.as_ref() })
}

unsafe fn runtime_make_current(render_target: *mut c_void) {
    let Some(target) = (unsafe { target(render_target) }) else {
        return;
    };
    if egl::API
        .make_current(
            target.display,
            Some(target.surface),
            Some(target.surface),
            Some(target.context),
        )
        .is_err()
    {
        target.failed.store(true, Ordering::Release);
    }
}

unsafe fn runtime_capture_binding() -> GlContextBinding {
    capture_egl_binding()
}

unsafe fn runtime_is_current(render_target: *mut c_void) -> bool {
    unsafe { target(render_target) }
        .is_some_and(|target| target.is_current_binding(capture_egl_binding()))
}

unsafe fn runtime_restore_binding(binding: GlContextBinding) -> bool {
    let display = if binding.display().is_null() {
        let Some(display) = egl::API.get_current_display() else {
            return binding == GlContextBinding::empty();
        };
        display
    } else {
        unsafe { egl::Display::from_ptr(binding.display()) }
    };
    let draw = (!binding.draw_surface().is_null())
        .then(|| unsafe { egl::Surface::from_ptr(binding.draw_surface()) });
    let read = (!binding.read_surface().is_null())
        .then(|| unsafe { egl::Surface::from_ptr(binding.read_surface()) });
    let context = (!binding.context().is_null())
        .then(|| unsafe { egl::Context::from_ptr(binding.context()) });
    egl::API.make_current(display, draw, read, context).is_ok() && capture_egl_binding() == binding
}

unsafe fn runtime_swap_buffers(render_target: *mut c_void) -> bool {
    let Some(target) = (unsafe { target(render_target) }) else {
        return false;
    };
    let swapped = egl::API
        .swap_buffers(target.display, target.surface)
        .is_ok();
    if !swapped {
        target.failed.store(true, Ordering::Release);
    }
    swapped
}

unsafe fn runtime_get_proc_address(name: *const c_char) -> *mut c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let Ok(name) = unsafe { CStr::from_ptr(name) }.to_str() else {
        return std::ptr::null_mut();
    };
    egl::API
        .get_proc_address(name)
        .map_or(std::ptr::null_mut(), |function| {
            function as *const () as *mut c_void
        })
}

unsafe fn runtime_create_cursor(shape: c_int) -> *mut c_void {
    Box::into_raw(Box::new(shape)).cast()
}

unsafe fn runtime_destroy_cursor(cursor: *mut c_void) {
    if !cursor.is_null() {
        drop(unsafe { Box::from_raw(cursor.cast::<c_int>()) });
    }
}

unsafe fn runtime_set_cursor(render_target: *mut c_void, cursor: *mut c_void) {
    let Some(target) = (unsafe { target(render_target) }) else {
        return;
    };
    let shape = if cursor.is_null() {
        0
    } else {
        unsafe { *cursor.cast::<c_int>() }
    };
    target.cursor_shape.store(shape, Ordering::Release);
    let _ = unsafe { runtime_wake_event_loop(target.wake.as_ptr().cast()) };
}

unsafe fn runtime_set_cursor_mode(render_target: *mut c_void, hidden: c_int) {
    let Some(target) = (unsafe { target(render_target) }) else {
        return;
    };
    target.cursor_hidden.store(hidden != 0, Ordering::Release);
    let _ = unsafe { runtime_wake_event_loop(target.wake.as_ptr().cast()) };
}

fn write_wake(mut write: impl FnMut() -> io::Result<usize>) -> bool {
    loop {
        match write() {
            Ok(size) => return size == std::mem::size_of::<u64>(),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return true,
            Err(_) => return false,
        }
    }
}

unsafe fn runtime_wake_event_loop(user_data: *mut c_void) -> bool {
    let Some(wake) = NonNull::new(user_data.cast::<WakeFd>()) else {
        return false;
    };
    let wake = unsafe { wake.as_ref() };
    let value = 1_u64;
    let written = write_wake(|| {
        let status = unsafe {
            libc::write(
                wake.fd,
                (&value as *const u64).cast(),
                std::mem::size_of::<u64>(),
            )
        };
        if status < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(status as usize)
        }
    });
    if !written {
        wake.failed.store(true, Ordering::Release);
    }
    written
}

impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" if state.compositor.is_none() => {
                    state.compositor = Some(registry.bind(name, version.min(6), qh, ()))
                }
                "zwlr_layer_shell_v1" if state.layer_shell.is_none() => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()))
                }
                "wp_cursor_shape_manager_v1" if state.cursor_shape_manager.is_none() => {
                    let manager: WpCursorShapeManagerV1 = registry.bind(name, 1, qh, ());
                    if let Some(pointer) = state.pointer.as_ref() {
                        state.cursor_shape_device = Some(manager.get_pointer(pointer, qh, ()));
                    }
                    state.cursor_shape_manager = Some(manager);
                }
                "wl_seat" if state.seat.is_none() => {
                    if let Some(version) = seat_binding_version(version) {
                        state.seat_global_name = Some(name);
                        let seat = registry.bind(name, version, qh, ());
                        state.seat = Some(seat);
                    } else {
                        state.failed = true;
                        state.close = true;
                    }
                }
                "wl_output" if version >= 4 => {
                    let output = registry.bind(name, version.min(4), qh, name);
                    state.outputs.insert(
                        name,
                        OutputRecord {
                            identity: output.id().protocol_id(),
                            proxy: output,
                            name: None,
                        },
                    );
                }
                _ => {}
            }
        } else if let wl_registry::Event::GlobalRemove { name } = event {
            state.output_global_removed(name);
            if state.seat_global_name == Some(name) {
                if let Some(cursor_shape_device) = state.cursor_shape_device.take() {
                    cursor_shape_device.destroy();
                }
                if let Some(pointer) = state.pointer.take() {
                    if pointer_release_supported(pointer.version()) {
                        pointer.release();
                    }
                }
                if let Some(keyboard) = state.keyboard.take() {
                    if keyboard_release_supported(keyboard.version()) {
                        keyboard.release();
                    }
                }
                if let Some(seat) = state.seat.take() {
                    if seat_release_supported(seat.version()) {
                        seat.release();
                    }
                }
                state.finish_seat_removal();
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        global_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(output) = state.outputs.get_mut(global_name) {
                output.name = Some(name);
            }
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_surface::Event::Enter { output } => {
                state.surface_entered_output(output.id().protocol_id());
            }
            wl_surface::Event::Leave { output } => {
                state.surface_left_output(output.id().protocol_id());
            }
            wl_surface::Event::PreferredBufferScale { factor } => {
                let factor = factor.max(1);
                if factor != state.scale {
                    if state.delivering_events {
                        state.failed = true;
                        state.close = true;
                    } else {
                        state.scale = factor;
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                state.apply_layer_configure(width, height);
            }
            zwlr_layer_surface_v1::Event::Closed => state.close = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for WaylandState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            } else if !capabilities.contains(wl_seat::Capability::Keyboard) {
                if let Some(keyboard) = state.keyboard.take() {
                    if keyboard_release_supported(keyboard.version()) {
                        keyboard.release();
                    }
                }
                state.remove_keyboard_capability();
            }
            if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                let pointer = seat.get_pointer(qh, ());
                if let Some(manager) = state.cursor_shape_manager.as_ref() {
                    state.cursor_shape_device = Some(manager.get_pointer(&pointer, qh, ()));
                }
                state.pointer = Some(pointer);
            } else if !capabilities.contains(wl_seat::Capability::Pointer) {
                let had_pointer = state.pointer.is_some();
                if let Some(cursor_shape_device) = state.cursor_shape_device.take() {
                    cursor_shape_device.destroy();
                }
                if let Some(pointer) = state.pointer.take() {
                    if pointer_release_supported(pointer.version()) {
                        pointer.release();
                    }
                }
                state.remove_pointer_capability(had_pointer);
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => match XkbKeyboard::from_keymap(fd, size) {
                Ok(keyboard) => {
                    state.repeating_key = None;
                    state.xkb = Some(keyboard);
                }
                Err(error) => {
                    eprintln!("[host-rs] native Wayland keymap error: {error}");
                    state.failed = true;
                    state.close = true;
                }
            },
            wl_keyboard::Event::Keymap { .. } => {
                state.repeating_key = None;
                state.failed = true;
                state.close = true;
            }
            wl_keyboard::Event::Enter { keys, .. } => {
                let key_state_ready =
                    state.xkb.is_some() && keys.len().is_multiple_of(std::mem::size_of::<u32>());
                if !key_state_ready {
                    state.failed = true;
                    state.close = true;
                    return;
                }
                state.set_keyboard_focus(true);
            }
            wl_keyboard::Event::Leave { .. } => {
                state.repeating_key = None;
                state.set_keyboard_focus(false);
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(key_state),
                ..
            } => match key_state {
                wl_keyboard::KeyState::Pressed => state.handle_key(key, true),
                wl_keyboard::KeyState::Released => state.handle_key(key, false),
                _ => {}
            },
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(keyboard) = state.xkb.as_mut() {
                    keyboard.update_modifiers(mods_depressed, mods_latched, mods_locked, group);
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                state.update_repeat_info(rate, delay, Instant::now());
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for WaylandState {
    fn event(
        state: &mut Self,
        pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_enter_serial = Some(serial);
                state.applied_cursor = None;
                if let Some(cursor) = state.cursor_shape_device.as_ref() {
                    cursor.set_shape(serial, wp_cursor_shape_device_v1::Shape::Default);
                    state.applied_cursor = Some((false, 0, serial));
                }
                if state.delivering_events {
                    state
                        .events
                        .push_back(WindowEvent::CursorMoved(surface_x, surface_y));
                    state.events.push_back(WindowEvent::CursorEntered(true));
                }
            }
            wl_pointer::Event::Leave { .. } => {
                state.pending_scroll = PendingScroll::default();
                state.pointer_enter_serial = None;
                state.applied_cursor = None;
                if state.delivering_events {
                    state.events.push_back(WindowEvent::CursorEntered(false));
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                if state.delivering_events {
                    state
                        .events
                        .push_back(WindowEvent::CursorMoved(surface_x, surface_y));
                }
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(button_state),
                ..
            } => {
                let button = match button {
                    0x110 => PointerButton::Primary,
                    0x111 => PointerButton::Secondary,
                    0x112 => PointerButton::Middle,
                    0x113 => PointerButton::Back,
                    0x114 => PointerButton::Forward,
                    _ => return,
                };
                if state.delivering_events {
                    state.events.push_back(WindowEvent::PointerButton(
                        button,
                        button_state == wl_pointer::ButtonState::Pressed,
                    ));
                }
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => {
                match axis {
                    wl_pointer::Axis::HorizontalScroll => state.pending_scroll.x += value,
                    wl_pointer::Axis::VerticalScroll => state.pending_scroll.y -= value,
                    _ => {}
                }
                if pointer.version() < 5 {
                    state.flush_scroll();
                }
            }
            wl_pointer::Event::AxisDiscrete {
                axis: WEnum::Value(axis),
                discrete,
            } => match axis {
                wl_pointer::Axis::HorizontalScroll => {
                    state.pending_scroll.discrete_x = Some(f64::from(discrete));
                }
                wl_pointer::Axis::VerticalScroll => {
                    state.pending_scroll.discrete_y = Some(-f64::from(discrete));
                }
                _ => {}
            },
            wl_pointer::Event::AxisValue120 {
                axis: WEnum::Value(axis),
                value120,
            } => match axis {
                wl_pointer::Axis::HorizontalScroll => {
                    state.pending_scroll.discrete_x = Some(f64::from(value120) / 120.0);
                }
                wl_pointer::Axis::VerticalScroll => {
                    state.pending_scroll.discrete_y = Some(-f64::from(value120) / 120.0);
                }
                _ => {}
            },
            wl_pointer::Event::Frame => state.flush_scroll(),
            _ => {}
        }
    }
}

delegate_noop!(WaylandState: ignore WpCursorShapeDeviceV1);
delegate_noop!(WaylandState: ignore WpCursorShapeManagerV1);

delegate_noop!(WaylandState: ignore wl_compositor::WlCompositor);
delegate_noop!(WaylandState: ignore ZwlrLayerShellV1);

#[link(name = "OpenGL")]
unsafe extern "C" {
    fn glViewport(x: i32, y: i32, width: i32, height: i32);
    fn glClearColor(red: f32, green: f32, blue: f32, alpha: f32);
    fn glClear(mask: u32);
    fn glFinish();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn layer_configuration_matches_fuzzel_style_unanchored_centering() {
        assert_eq!(
            layer_intent(),
            LayerIntent {
                width: 1120,
                height: 760,
                layer: zwlr_layer_shell_v1::Layer::Overlay,
                keyboard: zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
                exclusive_zone: -1,
            }
        );
    }

    #[test]
    fn configure_after_event_delivery_latches_health_failure_and_close() {
        let mut state = WaylandState::default();
        state.apply_layer_configure(1120, 760);
        assert!(state.configured);
        assert!(!state.failed);

        state.activate_event_delivery();
        state.apply_layer_configure(900, 600);

        assert!(state.failed);
        assert!(state.close);
        assert_eq!(state.configured_size, (1120, 760));
        assert!(state.events.is_empty());
    }

    #[test]
    fn initial_surface_enters_establish_output_overlap_without_failure() {
        let mut state = WaylandState::default();
        state.surface_entered_output(11);
        state.surface_entered_output(12);
        state.surface_left_output(11);

        state.activate_event_delivery();
        state.surface_entered_output(12);

        assert_eq!(state.surface_outputs, HashSet::from([12]));
        assert!(!state.failed);
        assert!(!state.close);
    }

    #[test]
    fn same_scale_output_migration_and_leave_fail_after_event_delivery() {
        let mut migrated = WaylandState {
            scale: 2,
            ..WaylandState::default()
        };
        migrated.surface_entered_output(11);
        migrated.activate_event_delivery();
        migrated.surface_entered_output(12);

        assert_eq!(migrated.scale, 2);
        assert!(migrated.failed);
        assert!(migrated.close);

        let mut left = WaylandState::default();
        left.surface_entered_output(11);
        left.surface_entered_output(12);
        left.activate_event_delivery();
        left.surface_left_output(11);

        assert!(left.failed);
        assert!(left.close);
    }

    #[test]
    fn seat_and_release_versions_follow_protocol_since_rules() {
        assert_eq!(seat_binding_version(2), None);
        assert_eq!(seat_binding_version(3), Some(3));
        assert_eq!(seat_binding_version(12), Some(9));
        assert!(!keyboard_release_supported(2));
        assert!(keyboard_release_supported(3));
        assert!(!pointer_release_supported(2));
        assert!(pointer_release_supported(3));
        assert!(!seat_release_supported(4));
        assert!(seat_release_supported(5));
    }

    #[test]
    fn evdev_and_keysym_translation_matches_backend_neutral_key_contract() {
        assert_eq!(evdev_to_glfw(30), Some(65));
        assert_eq!(physical_key(evdev_to_glfw(30).unwrap()), 0x0007_0004);
        assert_eq!(keysym_to_glfw(u32::from('a')), Some(65));
        assert_eq!(keysym_to_glfw(0xff51), Some(263));
        assert_eq!(evdev_to_glfw(0), None);
    }

    #[test]
    fn pointer_frame_prefers_discrete_scroll_steps() {
        let mut state = WaylandState::default();
        state.activate_event_delivery();
        state.pending_scroll.x = 25.0;
        state.pending_scroll.y = -30.0;
        state.pending_scroll.discrete_y = Some(-1.0);

        state.flush_scroll();

        assert_eq!(
            state.events.pop_front(),
            Some(WindowEvent::Scroll(2.5, -1.0))
        );
        assert!(state.events.is_empty());
    }

    #[test]
    fn repeat_info_reschedules_an_active_key_and_zero_rate_cancels_it() {
        let now = Instant::now();
        let mut state = WaylandState {
            repeating_key: Some(RepeatingKey {
                key: 30,
                next: now,
                interval: Duration::from_millis(25),
            }),
            ..WaylandState::default()
        };

        state.update_repeat_info(20, 400, now);

        let repeat = state.repeating_key.as_ref().unwrap();
        assert_eq!(repeat.key, 30);
        assert_eq!(repeat.interval, Duration::from_millis(50));
        assert_eq!(repeat.next, now + Duration::from_millis(400));

        state.update_repeat_info(0, 400, now);

        assert_eq!(state.repeat_rate, 0);
        assert!(state.repeating_key.is_none());
    }

    #[test]
    fn pointer_capability_removal_preserves_keyboard_repeat() {
        let mut state = WaylandState {
            delivering_events: true,
            repeating_key: Some(RepeatingKey {
                key: 30,
                next: Instant::now(),
                interval: Duration::from_millis(25),
            }),
            ..WaylandState::default()
        };

        state.remove_pointer_capability(true);

        assert!(state.repeating_key.is_some());
        assert_eq!(
            state.events.pop_front(),
            Some(WindowEvent::PointerCancelled)
        );
        assert!(state.events.is_empty());
    }

    #[test]
    fn egl_target_binding_requires_display_context_and_both_surfaces() {
        let mut display = 0_u8;
        let mut context = 0_u8;
        let mut draw = 0_u8;
        let mut other_read = 0_u8;
        let expected = GlContextBinding::new(
            (&mut display as *mut u8).cast(),
            (&mut context as *mut u8).cast(),
            (&mut draw as *mut u8).cast(),
            (&mut draw as *mut u8).cast(),
        );
        assert_eq!(
            egl_target_binding(
                (&mut display as *mut u8).cast(),
                (&mut context as *mut u8).cast(),
                (&mut draw as *mut u8).cast(),
            ),
            expected
        );
        assert_ne!(
            expected,
            GlContextBinding::new(
                (&mut display as *mut u8).cast(),
                (&mut context as *mut u8).cast(),
                (&mut draw as *mut u8).cast(),
                (&mut other_read as *mut u8).cast(),
            )
        );
    }

    #[test]
    fn startup_focus_loss_is_delivered_even_when_focus_was_regained() {
        let mut state = WaylandState::default();
        state.set_keyboard_focus(true);
        state.remove_keyboard_capability();
        state.set_keyboard_focus(true);

        state.activate_event_delivery();

        assert_eq!(state.events.pop_front(), Some(WindowEvent::Focused(false)));
        assert!(state.events.is_empty());
    }

    #[test]
    fn removing_keyboard_capability_delivers_focus_loss() {
        let mut state = WaylandState::default();
        state.set_keyboard_focus(true);
        state.activate_event_delivery();
        state.events.clear();

        state.remove_keyboard_capability();

        assert_eq!(state.events.pop_front(), Some(WindowEvent::Focused(false)));
    }

    #[test]
    fn keyboard_leave_after_activation_delivers_focus_loss() {
        let mut state = WaylandState::default();
        state.set_keyboard_focus(true);
        state.activate_event_delivery();
        state.events.clear();

        state.set_keyboard_focus(false);

        assert!(!state.focused);
        assert_eq!(state.events.pop_front(), Some(WindowEvent::Focused(false)));
        assert!(state.events.is_empty());
    }

    #[test]
    fn seat_removal_loses_focus_and_allows_a_replacement_global() {
        let mut state = WaylandState {
            seat_global_name: Some(42),
            ..WaylandState::default()
        };
        state.set_keyboard_focus(true);
        state.activate_event_delivery();
        state.events.clear();

        state.finish_seat_removal();

        assert_eq!(state.seat_global_name, None);
        assert_eq!(state.events.pop_front(), Some(WindowEvent::Focused(false)));
        assert_eq!(state.events.pop_front(), Some(WindowEvent::InputCancelled));
        assert!(state.events.is_empty());
    }

    #[test]
    fn wake_write_retries_interrupt_and_accepts_a_saturated_counter() {
        let mut results = VecDeque::from([
            Err(io::Error::from(io::ErrorKind::Interrupted)),
            Err(io::Error::from(io::ErrorKind::WouldBlock)),
        ]);
        assert!(write_wake(|| results.pop_front().unwrap()));
        assert!(results.is_empty());
    }

    #[test]
    fn wake_write_rejects_short_and_permanent_failures() {
        assert!(!write_wake(|| Ok(1)));
        assert!(!write_wake(|| Err(io::Error::from(
            io::ErrorKind::BrokenPipe
        ))));
    }

    #[test]
    fn flush_would_block_waits_for_writable_but_permanent_errors_fail() {
        assert_eq!(
            classify_flush_result(Err(WaylandError::Io(io::Error::from(
                io::ErrorKind::WouldBlock
            )))),
            Ok(true)
        );
        assert_eq!(classify_flush_result(Ok(())), Ok(false));
        assert_eq!(
            classify_flush_result(Err(WaylandError::Io(io::Error::from(
                io::ErrorKind::BrokenPipe
            )))),
            Err(())
        );
    }
}
