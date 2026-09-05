#[cfg(not(test))]
mod glfw_x11;
#[cfg(feature = "native-wayland")]
#[cfg_attr(test, allow(dead_code))]
mod wayland;

use std::ffi::c_void;
use std::ffi::CString;
use std::ffi::OsStr;
use std::io;
#[cfg(not(test))]
use std::io::Write;
use std::time::Duration;
#[cfg(not(test))]
use std::time::Instant;

use lynx_launcher_host::runtime::{DesktopApi, EventWake, GlApi};
#[cfg(not(test))]
use lynx_launcher_host::runtime::{RuntimeCore, RuntimeViewOptions};
use lynx_launcher_host::support::{InputState, PointerDispatch, PressedKey, WindowMetrics};
#[cfg(not(test))]
use lynx_launcher_host::{verify_linked_lynx, WindowBackendChoice, WindowRunOptions};
use lynx_sys::{
    LynxKeyEvent, LynxPointerEvent, LYNX_KEY_EVENT_TYPE_DOWN, LYNX_KEY_EVENT_TYPE_REPEAT,
    LYNX_KEY_EVENT_TYPE_UP, LYNX_POINTER_BUTTON_BACK, LYNX_POINTER_BUTTON_FORWARD,
    LYNX_POINTER_BUTTON_MIDDLE, LYNX_POINTER_BUTTON_PRIMARY, LYNX_POINTER_BUTTON_SECONDARY,
    LYNX_POINTER_DEVICE_KIND_MOUSE,
};

#[cfg(not(test))]
const MAXIMUM_EVENT_WAIT: Duration = Duration::from_millis(250);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyAction {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerButton {
    Primary,
    Secondary,
    Middle,
    Back,
    Forward,
}

#[derive(Clone, Debug, PartialEq)]
enum WindowEvent {
    CursorMoved(f64, f64),
    CursorEntered(bool),
    PointerButton(PointerButton, bool),
    Scroll(f64, f64),
    Key {
        id: i32,
        physical: u64,
        logical: u64,
        action: KeyAction,
    },
    Text {
        codepoint: u32,
        text: String,
    },
    #[cfg(feature = "native-wayland")]
    InputCancelled,
    #[cfg(feature = "native-wayland")]
    PointerCancelled,
    Focused(bool),
    Metrics(WindowMetrics),
}

#[derive(Clone, Copy)]
#[cfg_attr(test, allow(dead_code))]
struct BackendRuntimeAdapters {
    render_target: *mut c_void,
    gl: GlApi,
    desktop: DesktopApi,
    wake: EventWake,
    initial_metrics: WindowMetrics,
}

#[cfg_attr(test, allow(dead_code))]
trait WindowBackend {
    fn show(&mut self) -> io::Result<()>;
    fn runtime_adapters(&self) -> BackendRuntimeAdapters;
    fn activate_event_delivery(&mut self) -> io::Result<()>;
    fn drain_events(&self) -> io::Result<Vec<WindowEvent>>;
    fn wait_for_events(&self, timeout: Duration);
    fn should_close(&self) -> bool;
    fn request_close(&self);
    fn set_text_input_active(&self, active: bool) -> io::Result<()>;
    fn stop_event_delivery(&mut self);
    fn ensure_healthy(&self) -> io::Result<()>;
    fn fatal_cleanup_log(&self, native_callbacks_not_quiesced: bool) -> &'static [u8];
}

trait RuntimeDispatch {
    fn send_pointer_event(&self, event: &mut LynxPointerEvent) -> io::Result<()>;
    fn send_key_event(&self, event: &mut LynxKeyEvent) -> io::Result<()>;
    fn text_input_active(&self) -> bool;
    fn cancel_text_input(&self);
    fn enter_foreground(&self) -> io::Result<()>;
    fn enter_background(&self) -> io::Result<()>;
    fn update_view_metrics(&self, width: f32, height: f32, ratio: f32) -> io::Result<()>;
}

#[cfg(not(test))]
impl RuntimeDispatch for RuntimeCore {
    fn send_pointer_event(&self, event: &mut LynxPointerEvent) -> io::Result<()> {
        self.send_pointer_event(event)
    }

    fn send_key_event(&self, event: &mut LynxKeyEvent) -> io::Result<()> {
        self.send_key_event(event)
    }

    fn text_input_active(&self) -> bool {
        self.text_input_active()
    }

    fn cancel_text_input(&self) {
        self.cancel_text_input();
    }

    fn enter_foreground(&self) -> io::Result<()> {
        self.enter_foreground()
    }

    fn enter_background(&self) -> io::Result<()> {
        self.enter_background()
    }

    fn update_view_metrics(&self, width: f32, height: f32, ratio: f32) -> io::Result<()> {
        self.update_view_metrics(width, height, ratio)
    }
}

struct WindowCoordinator<R> {
    runtime: R,
    input: InputState,
    accepting_input: bool,
    metrics: WindowMetrics,
    input_trace: bool,
    ignore_focus_loss: bool,
}

impl<R: RuntimeDispatch> WindowCoordinator<R> {
    fn new(runtime: R, metrics: WindowMetrics, ignore_focus_loss: bool) -> Self {
        Self {
            runtime,
            input: InputState::default(),
            accepting_input: true,
            metrics,
            input_trace: std::env::var_os("LYNX_LAUNCHER_E2E_SNAPSHOT_TRACE").as_deref()
                == Some(std::ffi::OsStr::new("1")),
            ignore_focus_loss,
        }
    }

    fn dispatch(&mut self, event: WindowEvent) -> io::Result<bool> {
        match event {
            WindowEvent::Focused(false) if !self.ignore_focus_loss => {
                if !self.accepting_input {
                    return Ok(false);
                }
                self.accepting_input = false;
                let cancel_result = self.cancel_input();
                let background_result = self.runtime.enter_background();
                eprintln!("[host-rs] window lost focus; exiting");
                cancel_result.and(background_result)?;
                return Ok(true);
            }
            WindowEvent::Focused(true) => {
                if self.accepting_input {
                    self.runtime.enter_foreground()?;
                }
            }
            WindowEvent::Metrics(metrics) => {
                self.runtime.update_view_metrics(
                    metrics.logical_width,
                    metrics.logical_height,
                    metrics.pixel_ratio,
                )?;
                self.metrics = metrics;
                log_metrics(metrics);
            }
            _ if !self.accepting_input => return Ok(false),
            #[cfg(feature = "native-wayland")]
            WindowEvent::InputCancelled => {
                self.cancel_input()?;
            }
            #[cfg(feature = "native-wayland")]
            WindowEvent::PointerCancelled => {
                let events = self.input.cancel_pointer();
                self.send_pointer_events(events)?;
            }
            WindowEvent::CursorMoved(x, y) => {
                let events = self.input.move_cursor(x, y);
                self.send_pointer_events(events)?;
            }
            WindowEvent::CursorEntered(entered) => {
                let events = self.input.set_pointer_inside(entered);
                self.send_pointer_events(events)?;
            }
            WindowEvent::PointerButton(button, pressed) => {
                let mask = match button {
                    PointerButton::Primary => LYNX_POINTER_BUTTON_PRIMARY,
                    PointerButton::Secondary => LYNX_POINTER_BUTTON_SECONDARY,
                    PointerButton::Middle => LYNX_POINTER_BUTTON_MIDDLE,
                    PointerButton::Back => LYNX_POINTER_BUTTON_BACK,
                    PointerButton::Forward => LYNX_POINTER_BUTTON_FORWARD,
                };
                let events = if pressed {
                    self.input.press_button(mask)
                } else {
                    self.input.release_button(mask)
                };
                self.send_pointer_events(events)?;
            }
            WindowEvent::Scroll(x, y) => {
                let events = self.input.scroll(x, y);
                self.send_pointer_events(events)?;
            }
            WindowEvent::Key {
                id,
                physical,
                logical,
                action,
            } => {
                if self.input_trace {
                    eprintln!(
                        "[host-rs] key key={id} action={} physical={physical} logical={logical}",
                        match action {
                            KeyAction::Press => 1,
                            KeyAction::Repeat => 2,
                            KeyAction::Release => 0,
                        }
                    );
                }
                let pressed = match action {
                    KeyAction::Press => Some((
                        LYNX_KEY_EVENT_TYPE_DOWN,
                        self.input.press_key(id, physical, logical),
                    )),
                    KeyAction::Repeat => self
                        .input
                        .repeated_key(id)
                        .map(|key| (LYNX_KEY_EVENT_TYPE_REPEAT, key)),
                    KeyAction::Release => self
                        .input
                        .release_key(id)
                        .map(|key| (LYNX_KEY_EVENT_TYPE_UP, key)),
                };
                if let Some((event_type, key)) = pressed {
                    self.send_key(
                        event_type,
                        key,
                        if event_type == LYNX_KEY_EVENT_TYPE_UP {
                            std::ptr::null()
                        } else {
                            c"".as_ptr()
                        },
                        false,
                    )?;
                }
            }
            WindowEvent::Text { codepoint, text } => {
                let active = self.runtime.text_input_active();
                if self.input_trace {
                    eprintln!("[host-rs] character codepoint={codepoint} active={active}");
                }
                if active && !text.is_empty() && !text.as_bytes().contains(&0) {
                    let character = CString::new(text).map_err(|_| {
                        io::Error::other("character input unexpectedly contained NUL")
                    })?;
                    let logical = character
                        .to_str()
                        .ok()
                        .and_then(|text| text.chars().next())
                        .map(u64::from)
                        .unwrap_or(0);
                    let key = PressedKey {
                        physical: 0,
                        logical,
                    };
                    self.send_key(LYNX_KEY_EVENT_TYPE_DOWN, key, character.as_ptr(), true)?;
                    self.send_key(LYNX_KEY_EVENT_TYPE_UP, key, std::ptr::null(), true)?;
                }
            }
            WindowEvent::Focused(false) => {}
        }
        Ok(false)
    }

    #[cfg(not(test))]
    fn stop_accepting_input(&mut self) {
        self.accepting_input = false;
    }

    fn cancel_input(&mut self) -> io::Result<()> {
        let (keys, pointer_events) = self.input.cancel();
        self.runtime.cancel_text_input();
        for key in keys {
            self.send_key(LYNX_KEY_EVENT_TYPE_UP, key, std::ptr::null(), true)?;
        }
        self.send_pointer_events(pointer_events)
    }

    fn send_pointer_events(&self, events: Vec<PointerDispatch>) -> io::Result<()> {
        let (cursor_x, cursor_y) = self.input.cursor_position();
        for dispatch in events {
            let mut event = LynxPointerEvent {
                struct_size: std::mem::size_of::<LynxPointerEvent>(),
                phase: dispatch.phase,
                timestamp: 0,
                x: cursor_x * f64::from(self.metrics.framebuffer_scale_x),
                y: cursor_y * f64::from(self.metrics.framebuffer_scale_y),
                device: 0,
                signal_kind: dispatch.signal_kind,
                scroll_delta_x: dispatch.scroll_delta_x * f64::from(self.metrics.pixel_ratio),
                scroll_delta_y: dispatch.scroll_delta_y * f64::from(self.metrics.pixel_ratio),
                device_kind: LYNX_POINTER_DEVICE_KIND_MOUSE,
                buttons: dispatch.buttons,
                pan_x: 0.0,
                pan_y: 0.0,
                scale: 1.0,
                rotation: 0.0,
                is_precise_scroll: 0,
            };
            self.runtime.send_pointer_event(&mut event)?;
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
        self.runtime.send_key_event(&mut event)?;
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
}

#[cfg(not(test))]
pub fn run(options: WindowRunOptions) -> io::Result<()> {
    verify_linked_lynx(&options.expected_lynx_library)?;
    let placement_output = parse_placement_probe(
        std::env::var_os("LYNX_LAUNCHER_E2E_WAYLAND_PLACEMENT_PROBE").as_deref(),
        std::env::var_os("LYNX_LAUNCHER_E2E_WAYLAND_PLACEMENT_OUTPUT").as_deref(),
    )?;
    if placement_output.is_some() {
        if options.window_backend != WindowBackendChoice::Wayland
            || options.run_for_seconds.is_some()
            || options.exit_after_first_frame
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the Wayland placement probe requires only --window-backend wayland",
            ));
        }
        #[cfg(feature = "native-wayland")]
        return wayland::run_placement_probe(placement_output.as_deref().unwrap());
        #[cfg(not(feature = "native-wayland"))]
        return Err(io::Error::other(
            "native Wayland support is not compiled in; build the explicit Wayland backend target",
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
    match options.window_backend {
        WindowBackendChoice::X11 => {
            run_with_backend(glfw_x11::default_backend()?, options, ignore_focus_loss)
        }
        WindowBackendChoice::Wayland => {
            #[cfg(feature = "native-wayland")]
            {
                run_with_backend(
                    wayland::WaylandBackend::initialize(None)?,
                    options,
                    ignore_focus_loss,
                )
            }
            #[cfg(not(feature = "native-wayland"))]
            {
                Err(io::Error::other(
                    "native Wayland support is not compiled in; build the explicit Wayland backend target",
                ))
            }
        }
    }
}

fn parse_placement_probe(
    enabled: Option<&OsStr>,
    output: Option<&OsStr>,
) -> io::Result<Option<String>> {
    match (enabled, output) {
        (None, None) => Ok(None),
        (Some(enabled), Some(output)) if enabled == OsStr::new("1") && !output.is_empty() => output
            .to_str()
            .map(|output| Some(output.to_owned()))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "LYNX_LAUNCHER_E2E_WAYLAND_PLACEMENT_OUTPUT must be UTF-8",
                )
            }),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Wayland placement probe requires PROBE=1 and one nonempty PLACEMENT_OUTPUT",
        )),
    }
}

#[cfg(not(test))]
fn run_with_backend<B: WindowBackend>(
    mut backend: B,
    options: WindowRunOptions,
    ignore_focus_loss: bool,
) -> io::Result<()> {
    backend.show()?;
    let adapters = backend.runtime_adapters();
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
        bundle: options.bundle,
        lynx_core: options.lynx_core,
        icu: options.icu,
        logical_width: adapters.initial_metrics.logical_width,
        logical_height: adapters.initial_metrics.logical_height,
        pixel_ratio: adapters.initial_metrics.pixel_ratio,
    };
    // SAFETY: The backend and all of its adapters remain alive through runtime
    // shutdown, or are deliberately leaked together on the fatal native path.
    let mut runtime = unsafe {
        RuntimeCore::initialize(
            adapters.render_target,
            adapters.gl,
            adapters.desktop,
            adapters.wake,
            &view_options,
        )
    }?;

    let run_result = match runtime.initialize_view(&view_options) {
        Ok(()) => {
            eprintln!("[host-rs] runtime core initialized");
            let mut coordinator =
                WindowCoordinator::new(runtime, adapters.initial_metrics, ignore_focus_loss);
            let loop_result = (|| {
                backend.activate_event_delivery()?;
                dispatch_backend_events(&backend, &mut coordinator)?;
                coordinator.runtime.run_due_tasks()?;
                backend.set_text_input_active(coordinator.runtime.text_input_active())?;
                coordinator.runtime.ensure_healthy()?;
                while !backend.should_close() {
                    backend.ensure_healthy()?;
                    coordinator.runtime.run_due_tasks()?;
                    backend.set_text_input_active(coordinator.runtime.text_input_active())?;
                    if options.exit_after_first_frame && coordinator.runtime.first_frame_ready() {
                        eprintln!("[host-rs] auto-exit after first rendered frame");
                        break;
                    }
                    let mut timeout = coordinator.runtime.wait_duration(MAXIMUM_EVENT_WAIT)?;
                    if let Some(deadline) = deadline {
                        let now = Instant::now();
                        if now >= deadline {
                            eprintln!("[host-rs] auto-exit after bounded run");
                            break;
                        }
                        timeout = timeout.min(deadline.duration_since(now));
                    }
                    backend.wait_for_events(timeout.max(Duration::from_micros(500)));
                    backend.ensure_healthy()?;
                    dispatch_backend_events(&backend, &mut coordinator)?;
                    coordinator.runtime.ensure_healthy()?;
                }
                backend.ensure_healthy()
            })();

            coordinator.stop_accepting_input();
            let cancel_result = coordinator.cancel_input();
            let text_input_result = backend.set_text_input_active(false);
            backend.stop_event_delivery();
            let shutdown_result = coordinator.runtime.shutdown();
            let requires_process_exit = coordinator
                .runtime
                .requires_process_exit_without_native_cleanup();
            let native_callbacks_not_quiesced = coordinator.runtime.native_callbacks_not_quiesced();
            if requires_process_exit {
                let fatal_log = backend.fatal_cleanup_log(native_callbacks_not_quiesced);
                let fatal_error = io::Error::from(io::ErrorKind::Other);
                std::mem::forget(coordinator);
                std::mem::forget(backend);
                let _ = io::stderr().write_all(fatal_log);
                return Err(fatal_error);
            }
            if shutdown_result.is_ok() {
                eprintln!("[host-rs] runtime core shutdown complete");
            }
            cancel_result?;
            text_input_result?;
            loop_result?;
            shutdown_result
        }
        Err(error) => {
            let shutdown_result = runtime.shutdown();
            if runtime.requires_process_exit_without_native_cleanup() {
                let fatal_log = backend.fatal_cleanup_log(runtime.native_callbacks_not_quiesced());
                let fatal_error = io::Error::from(io::ErrorKind::Other);
                std::mem::forget(runtime);
                std::mem::forget(backend);
                let _ = io::stderr().write_all(fatal_log);
                return Err(fatal_error);
            }
            shutdown_result?;
            Err(error)
        }
    };
    run_result
}

#[cfg(not(test))]
fn dispatch_backend_events<R: RuntimeDispatch>(
    backend: &impl WindowBackend,
    coordinator: &mut WindowCoordinator<R>,
) -> io::Result<()> {
    for event in backend.drain_events()? {
        if coordinator.dispatch(event)? {
            backend.request_close();
        }
    }
    backend.set_text_input_active(coordinator.runtime.text_input_active())?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct FakeRuntime {
        backgrounds: Cell<usize>,
        foregrounds: Cell<usize>,
        keys: RefCell<Vec<i32>>,
        pointers: RefCell<Vec<(i32, i64)>>,
        text_cancellations: Cell<usize>,
        metrics: RefCell<Vec<(f32, f32, f32)>>,
    }

    impl RuntimeDispatch for FakeRuntime {
        fn send_pointer_event(&self, event: &mut LynxPointerEvent) -> io::Result<()> {
            self.pointers
                .borrow_mut()
                .push((event.phase, event.buttons));
            Ok(())
        }

        fn send_key_event(&self, event: &mut LynxKeyEvent) -> io::Result<()> {
            self.keys.borrow_mut().push(event.event_type);
            Ok(())
        }

        fn text_input_active(&self) -> bool {
            true
        }

        fn cancel_text_input(&self) {
            self.text_cancellations
                .set(self.text_cancellations.get() + 1);
        }

        fn enter_foreground(&self) -> io::Result<()> {
            self.foregrounds.set(self.foregrounds.get() + 1);
            Ok(())
        }

        fn enter_background(&self) -> io::Result<()> {
            self.backgrounds.set(self.backgrounds.get() + 1);
            Ok(())
        }

        fn update_view_metrics(&self, width: f32, height: f32, ratio: f32) -> io::Result<()> {
            self.metrics.borrow_mut().push((width, height, ratio));
            Ok(())
        }
    }

    fn metrics() -> WindowMetrics {
        WindowMetrics {
            logical_width: 1120.0,
            logical_height: 760.0,
            pixel_ratio: 1.0,
            framebuffer_scale_x: 1.0,
            framebuffer_scale_y: 1.0,
        }
    }

    #[test]
    fn early_focus_loss_closes_the_coordinator() {
        let mut coordinator = WindowCoordinator::new(FakeRuntime::default(), metrics(), false);

        assert!(coordinator.dispatch(WindowEvent::Focused(false)).unwrap());
        assert_eq!(coordinator.runtime.backgrounds.get(), 1);
        assert!(!coordinator.accepting_input);
    }

    #[test]
    fn late_input_after_terminal_focus_loss_is_ignored() {
        let mut coordinator = WindowCoordinator::new(FakeRuntime::default(), metrics(), false);
        coordinator
            .dispatch(WindowEvent::Key {
                id: 1,
                physical: 2,
                logical: 3,
                action: KeyAction::Press,
            })
            .unwrap();
        coordinator.dispatch(WindowEvent::Focused(false)).unwrap();
        let dispatched_before_late_events = coordinator.runtime.keys.borrow().len();

        let late_events = [
            WindowEvent::Key {
                id: 1,
                physical: 2,
                logical: 3,
                action: KeyAction::Release,
            },
            WindowEvent::Key {
                id: 1,
                physical: 2,
                logical: 3,
                action: KeyAction::Repeat,
            },
            WindowEvent::Text {
                codepoint: u32::from('x'),
                text: "x".into(),
            },
            WindowEvent::CursorMoved(1.0, 2.0),
            WindowEvent::CursorEntered(true),
            WindowEvent::PointerButton(PointerButton::Primary, true),
            WindowEvent::PointerButton(PointerButton::Secondary, true),
            WindowEvent::PointerButton(PointerButton::Middle, true),
            WindowEvent::PointerButton(PointerButton::Back, true),
            WindowEvent::PointerButton(PointerButton::Forward, true),
            WindowEvent::Scroll(0.0, 1.0),
            WindowEvent::Focused(true),
        ];
        for event in late_events {
            assert!(!coordinator.dispatch(event).unwrap());
        }
        assert_eq!(
            coordinator.runtime.keys.borrow().len(),
            dispatched_before_late_events
        );
        assert_eq!(coordinator.runtime.foregrounds.get(), 0);
    }

    #[cfg(feature = "native-wayland")]
    #[test]
    fn backend_input_cancellation_releases_keys_and_pointer_buttons() {
        let mut coordinator = WindowCoordinator::new(FakeRuntime::default(), metrics(), false);
        coordinator
            .dispatch(WindowEvent::Key {
                id: 30,
                physical: 0x0007_0004,
                logical: u64::from('a'),
                action: KeyAction::Press,
            })
            .unwrap();
        coordinator
            .dispatch(WindowEvent::CursorEntered(true))
            .unwrap();
        coordinator
            .dispatch(WindowEvent::PointerButton(PointerButton::Primary, true))
            .unwrap();

        assert!(!coordinator.dispatch(WindowEvent::InputCancelled).unwrap());

        assert_eq!(
            *coordinator.runtime.keys.borrow(),
            vec![LYNX_KEY_EVENT_TYPE_DOWN, LYNX_KEY_EVENT_TYPE_UP]
        );
        assert!(coordinator.runtime.pointers.borrow().contains(&(
            lynx_sys::LYNX_POINTER_PHASE_CANCEL,
            LYNX_POINTER_BUTTON_PRIMARY
        )));
        assert_eq!(coordinator.runtime.text_cancellations.get(), 1);
        assert!(coordinator.accepting_input);
    }

    #[cfg(feature = "native-wayland")]
    #[test]
    fn pointer_cancellation_preserves_keyboard_and_text_input() {
        let mut coordinator = WindowCoordinator::new(FakeRuntime::default(), metrics(), false);
        coordinator
            .dispatch(WindowEvent::Key {
                id: 30,
                physical: 0x0007_0004,
                logical: u64::from('a'),
                action: KeyAction::Press,
            })
            .unwrap();
        coordinator
            .dispatch(WindowEvent::CursorEntered(true))
            .unwrap();
        coordinator
            .dispatch(WindowEvent::PointerButton(PointerButton::Primary, true))
            .unwrap();

        assert!(!coordinator.dispatch(WindowEvent::PointerCancelled).unwrap());
        coordinator
            .dispatch(WindowEvent::Key {
                id: 30,
                physical: 0x0007_0004,
                logical: u64::from('a'),
                action: KeyAction::Repeat,
            })
            .unwrap();
        coordinator
            .dispatch(WindowEvent::Text {
                codepoint: u32::from('x'),
                text: "x".into(),
            })
            .unwrap();

        assert_eq!(
            *coordinator.runtime.keys.borrow(),
            vec![
                LYNX_KEY_EVENT_TYPE_DOWN,
                LYNX_KEY_EVENT_TYPE_REPEAT,
                LYNX_KEY_EVENT_TYPE_DOWN,
                LYNX_KEY_EVENT_TYPE_UP,
            ]
        );
        assert_eq!(coordinator.runtime.text_cancellations.get(), 0);
        assert!(coordinator.runtime.pointers.borrow().ends_with(&[
            (
                lynx_sys::LYNX_POINTER_PHASE_CANCEL,
                LYNX_POINTER_BUTTON_PRIMARY,
            ),
            (lynx_sys::LYNX_POINTER_PHASE_REMOVE, 0),
        ]));
    }

    #[test]
    fn metrics_event_updates_runtime_and_pointer_conversion_state() {
        let mut coordinator = WindowCoordinator::new(FakeRuntime::default(), metrics(), false);
        let updated = WindowMetrics {
            logical_width: 800.0,
            logical_height: 600.0,
            pixel_ratio: 2.0,
            framebuffer_scale_x: 2.0,
            framebuffer_scale_y: 2.0,
        };

        coordinator.dispatch(WindowEvent::Metrics(updated)).unwrap();

        assert_eq!(coordinator.metrics, updated);
        assert_eq!(
            *coordinator.runtime.metrics.borrow(),
            vec![(800.0, 600.0, 2.0)]
        );
    }

    #[test]
    fn placement_probe_environment_is_strict() {
        assert_eq!(parse_placement_probe(None, None).unwrap(), None);
        assert_eq!(
            parse_placement_probe(Some(OsStr::new("1")), Some(OsStr::new("DP-1"))).unwrap(),
            Some("DP-1".to_owned())
        );
        assert!(parse_placement_probe(Some(OsStr::new("true")), None).is_err());
        assert!(parse_placement_probe(None, Some(OsStr::new("DP-1"))).is_err());
        assert!(parse_placement_probe(Some(OsStr::new("1")), Some(OsStr::new(""))).is_err());
    }
}
