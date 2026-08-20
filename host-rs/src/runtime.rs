use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::{c_char, c_int, c_long, c_void, CStr};
use std::io;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, ThreadId};
use std::time::Duration;

use lynx_sys::{
    LynxTask, LynxUiTaskRunnerConfig, LynxWindowlessRenderer, LYNX_LOG_INFO,
    LYNX_RENDERER_TYPE_GL_DIRECT,
};

const NANOS_PER_SECOND: u64 = 1_000_000_000;
const CLOCK_MONOTONIC: c_int = 1;

#[repr(C)]
struct Timespec {
    seconds: c_long,
    nanoseconds: c_long,
}

unsafe extern "C" {
    fn clock_gettime(clock_id: c_int, time: *mut Timespec) -> c_int;
}

#[derive(Clone, Copy)]
pub struct EventWake {
    user_data: CallbackUserData,
    callback: unsafe fn(*mut c_void),
}

#[derive(Clone, Copy)]
struct CallbackUserData(*mut c_void);

// Runtime initialization requires this userdata to remain valid for callbacks
// from any Lynx posting thread until shutdown completes.
unsafe impl Send for CallbackUserData {}
unsafe impl Sync for CallbackUserData {}

impl EventWake {
    pub fn new(user_data: *mut c_void, callback: unsafe fn(*mut c_void)) -> Self {
        Self {
            user_data: CallbackUserData(user_data),
            callback,
        }
    }

    fn wake(self) -> bool {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: The caller keeps the callback and its optional userdata
            // valid for the complete runtime registration period.
            unsafe { (self.callback)(self.user_data.0) };
        }))
        .is_ok()
    }
}

#[derive(Clone, Copy)]
pub struct GlApi {
    make_context_current: unsafe fn(*mut c_void),
    get_current_context: unsafe fn() -> *mut c_void,
    swap_buffers: unsafe fn(*mut c_void),
    get_proc_address: unsafe fn(*const c_char) -> *mut c_void,
}

impl GlApi {
    pub const fn new(
        make_context_current: unsafe fn(*mut c_void),
        get_current_context: unsafe fn() -> *mut c_void,
        swap_buffers: unsafe fn(*mut c_void),
        get_proc_address: unsafe fn(*const c_char) -> *mut c_void,
    ) -> Self {
        Self {
            make_context_current,
            get_current_context,
            swap_buffers,
            get_proc_address,
        }
    }
}

struct ScheduledQueue<T> {
    state: Mutex<ScheduledQueueState<T>>,
}

struct ScheduledQueueState<T> {
    accepting: bool,
    tasks: BTreeMap<u64, VecDeque<T>>,
}

impl<T> ScheduledQueue<T> {
    fn new() -> Self {
        Self {
            state: Mutex::new(ScheduledQueueState {
                accepting: false,
                tasks: BTreeMap::new(),
            }),
        }
    }

    fn start_accepting(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.accepting || !state.tasks.is_empty() {
            return false;
        }
        state.accepting = true;
        true
    }

    fn stop_accepting(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .accepting = false;
    }

    fn push_absolute(&self, task: T, target_nanos: u64, now_nanos: u64) -> bool {
        self.push_at(task, target_nanos.max(now_nanos))
    }

    fn push_after(&self, task: T, interval_nanos: u64, now_nanos: u64) -> bool {
        self.push_at(task, now_nanos.saturating_add(interval_nanos))
    }

    fn push_at(&self, task: T, deadline_nanos: u64) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.accepting {
            return false;
        }
        state
            .tasks
            .entry(deadline_nanos)
            .or_default()
            .push_back(task);
        true
    }

    fn pop_due(&self, now_nanos: u64) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut entry = state.tasks.first_entry()?;
        if *entry.key() > now_nanos {
            return None;
        }
        let task = entry
            .get_mut()
            .pop_front()
            .expect("scheduled deadline must contain a task");
        if entry.get().is_empty() {
            entry.remove_entry();
        }
        Some(task)
    }

    fn pop_any(&self) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut entry = state.tasks.first_entry()?;
        let task = entry
            .get_mut()
            .pop_front()
            .expect("scheduled deadline must contain a task");
        if entry.get().is_empty() {
            entry.remove_entry();
        }
        Some(task)
    }

    fn next_deadline(&self) -> Option<u64> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .tasks
            .first_key_value()
            .map(|(deadline, _)| *deadline)
    }
}

struct TaskToken(LynxTask);

// Once the queue accepts this opaque token, the embedder returns it to the
// matching runner exactly once. The public SDK has no cancellation function;
// tokens rejected after acceptance closes are deliberately never run.
unsafe impl Send for TaskToken {}

impl TaskToken {
    fn into_raw(self) -> LynxTask {
        self.0
    }
}

struct AcceptedUiTask {
    generation: u64,
    token: TaskToken,
}

#[derive(Clone, Copy, Debug)]
struct UiActivation {
    token: u64,
}

#[derive(Clone, Copy, Debug)]
struct UiDrainingGeneration {
    token: u64,
}

#[derive(Default)]
struct UiLifecycle {
    active_token: Option<u64>,
    next_token: u64,
    platform_thread: Option<ThreadId>,
    wake: Option<EventWake>,
    draining_token: Option<u64>,
    in_flight_posts: usize,
}

struct GlobalUiRunnerState {
    lifecycle: Mutex<UiLifecycle>,
    posts_idle: Condvar,
    tasks: ScheduledQueue<AcceptedUiTask>,
    callback_failed: AtomicBool,
}

impl GlobalUiRunnerState {
    fn new() -> Self {
        Self {
            lifecycle: Mutex::new(UiLifecycle::default()),
            posts_idle: Condvar::new(),
            tasks: ScheduledQueue::new(),
            callback_failed: AtomicBool::new(false),
        }
    }

    fn activate(&self, wake: EventWake) -> io::Result<UiActivation> {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if lifecycle.active_token.is_some() || lifecycle.draining_token.is_some() {
            return Err(io::Error::other(
                "Lynx global UI task runner already has an active host",
            ));
        }
        let next_token = lifecycle
            .next_token
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Lynx global UI host generation overflowed"))?;
        if !self.tasks.start_accepting() {
            return Err(io::Error::other(
                "Lynx global UI task queue was not empty at activation",
            ));
        }
        lifecycle.next_token = next_token;
        let activation = UiActivation { token: next_token };
        lifecycle.platform_thread = Some(thread::current().id());
        lifecycle.wake = Some(wake);
        LYNX_LOG_CALLBACK_FAILED.store(false, Ordering::Release);
        RENDERER_CALLBACK_ENTRY_FAILED.store(false, Ordering::Release);
        self.callback_failed.store(false, Ordering::Release);
        lifecycle.active_token = Some(activation.token);
        Ok(activation)
    }

    fn runs_on_current_thread(&self) -> bool {
        let lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (lifecycle.active_token.is_some() || lifecycle.draining_token.is_some())
            && lifecycle.platform_thread == Some(thread::current().id())
    }

    fn begin_post(&self) -> Option<UiPostGuard<'_>> {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let generation = lifecycle.active_token?;
        let wake = lifecycle.wake?;
        let Some(in_flight_posts) = lifecycle.in_flight_posts.checked_add(1) else {
            self.callback_failed.store(true, Ordering::Release);
            return None;
        };
        lifecycle.in_flight_posts = in_flight_posts;
        Some(UiPostGuard {
            state: self,
            generation,
            wake,
        })
    }

    fn post(&self, guard: &UiPostGuard<'_>, task: TaskToken, target_nanos: u64) {
        let now_nanos = match monotonic_nanos() {
            Ok(now_nanos) => now_nanos,
            Err(_) => {
                self.record_callback_failure();
                return;
            }
        };
        let task = AcceptedUiTask {
            generation: guard.generation,
            token: task,
        };
        if self.tasks.push_absolute(task, target_nanos, now_nanos) && !guard.wake.wake() {
            self.callback_failed.store(true, Ordering::Release);
        }
    }

    fn run_due(&self) -> io::Result<()> {
        if !self.runs_on_current_thread() {
            return Err(io::Error::other(
                "Lynx UI tasks may only run on the active platform thread",
            ));
        }
        let generation = self.active_generation_on_current_thread()?;
        while let Some(task) = self.tasks.pop_due(monotonic_nanos()?) {
            if task.generation != generation {
                self.callback_failed.store(true, Ordering::Release);
                return Err(io::Error::other("a UI task crossed Lynx host generations"));
            }
            // SAFETY: Each accepted token is removed from the queue before it
            // is returned once to its Lynx runner.
            if !unsafe { lynx_sys::lynx_windowless_run_ui_task(task.token.into_raw()) } {
                eprintln!("[host-rs] warning: Lynx UI task was not found");
            }
        }
        Ok(())
    }

    fn stop_and_drain(&self, activation: UiActivation) -> io::Result<UiDrainingGeneration> {
        let generation = self.stop_and_wait_for_posts(activation)?;
        while let Some(task) = self.tasks.pop_any() {
            if task.generation != generation {
                self.callback_failed.store(true, Ordering::Release);
                continue;
            }
            // SAFETY: Stopping acceptance freezes the final set. Removing each
            // accepted token before the call prevents shutdown re-entry from
            // duplicating it. Rejected tokens are never executed here.
            if !unsafe { lynx_sys::lynx_windowless_run_ui_task(task.token.into_raw()) } {
                eprintln!("[host-rs] warning: shutdown UI task was not found");
            }
        }
        Ok(UiDrainingGeneration { token: generation })
    }

    fn stop_and_wait_for_posts(&self, activation: UiActivation) -> io::Result<u64> {
        {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if lifecycle.active_token != Some(activation.token)
                || lifecycle.platform_thread != Some(thread::current().id())
            {
                return Err(io::Error::other(
                    "only the active platform host may drain the Lynx UI runner",
                ));
            }
            lifecycle.active_token = None;
            lifecycle.draining_token = Some(activation.token);
            self.posts_idle.notify_all();
        }
        self.tasks.stop_accepting();
        let lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let lifecycle = self
            .posts_idle
            .wait_while(lifecycle, |lifecycle| lifecycle.in_flight_posts != 0)
            .unwrap_or_else(|error| error.into_inner());
        debug_assert_eq!(lifecycle.draining_token, Some(activation.token));
        Ok(activation.token)
    }

    fn finish_deactivation(&self, generation: UiDrainingGeneration) -> io::Result<()> {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if lifecycle.draining_token != Some(generation.token)
            || lifecycle.platform_thread != Some(thread::current().id())
            || lifecycle.in_flight_posts != 0
        {
            return Err(io::Error::other(
                "only the draining platform host may finish its UI generation",
            ));
        }
        lifecycle.draining_token = None;
        lifecycle.platform_thread = None;
        lifecycle.wake = None;
        Ok(())
    }

    fn active_generation_on_current_thread(&self) -> io::Result<u64> {
        let lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if lifecycle.platform_thread != Some(thread::current().id()) {
            return Err(io::Error::other(
                "Lynx UI tasks may only run on the active platform thread",
            ));
        }
        lifecycle
            .active_token
            .ok_or_else(|| io::Error::other("Lynx UI tasks require an active host generation"))
    }

    fn wait_duration(&self, maximum: Duration) -> io::Result<Duration> {
        Ok(wait_duration(&self.tasks, monotonic_nanos()?, maximum))
    }

    fn record_callback_failure(&self) {
        self.callback_failed.store(true, Ordering::Release);
        let wake = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .wake;
        if let Some(wake) = wake {
            let _ = wake.wake();
        }
    }
}

struct UiPostGuard<'a> {
    state: &'a GlobalUiRunnerState,
    generation: u64,
    wake: EventWake,
}

impl Drop for UiPostGuard<'_> {
    fn drop(&mut self) {
        let mut lifecycle = self
            .state
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if lifecycle.in_flight_posts == 0 {
            self.state.callback_failed.store(true, Ordering::Release);
            self.state.posts_idle.notify_all();
            return;
        }
        lifecycle.in_flight_posts -= 1;
        if lifecycle.in_flight_posts == 0 {
            self.state.posts_idle.notify_all();
        }
    }
}

static GLOBAL_UI_RUNNER: OnceLock<GlobalUiRunnerState> = OnceLock::new();
static GLOBAL_UI_RUNNER_CONFIGURED: OnceLock<bool> = OnceLock::new();
static LYNX_LOG_INITIALIZED: OnceLock<()> = OnceLock::new();
static LYNX_LOG_CALLBACK_FAILED: AtomicBool = AtomicBool::new(false);
static RENDERER_CALLBACK_ENTRY_FAILED: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
static LYNX_LOG_INITIALIZATION_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn global_ui_runner() -> &'static GlobalUiRunnerState {
    GLOBAL_UI_RUNNER.get_or_init(GlobalUiRunnerState::new)
}

fn configure_global_ui_runner() -> io::Result<&'static GlobalUiRunnerState> {
    let state = global_ui_runner();
    let configured = *GLOBAL_UI_RUNNER_CONFIGURED.get_or_init(|| {
        let config = LynxUiTaskRunnerConfig {
            struct_size: std::mem::size_of::<LynxUiTaskRunnerConfig>(),
            user_data: (state as *const GlobalUiRunnerState).cast_mut().cast(),
            runs_on_current_thread_callback: Some(ui_runs_on_current_thread_callback),
            post_task_callback: Some(ui_post_task_callback),
        };
        // SAFETY: `state` has a process-stable address and all callbacks remain
        // valid for the life of the process-global Lynx runner.
        unsafe { lynx_sys::lynx_windowless_set_global_ui_task_runner(&config) }
    });
    if !configured {
        return Err(io::Error::other(
            "Lynx rejected the process-global windowless UI task runner",
        ));
    }
    Ok(state)
}

struct RendererState {
    window: WindowHandle,
    gl: GlApi,
    wake: EventWake,
    tasks: ScheduledQueue<TaskToken>,
    callbacks: Mutex<RendererCallbackLifecycle>,
    callbacks_idle: Condvar,
    gl_owner: Mutex<Option<ThreadId>>,
    callback_failed: AtomicBool,
    gl_context_stranded: AtomicBool,
    finalized: AtomicBool,
}

struct RendererCallbackLifecycle {
    closing: bool,
    in_flight: usize,
    accepting_posts: bool,
    in_flight_posts: usize,
}

impl Default for RendererCallbackLifecycle {
    fn default() -> Self {
        Self {
            closing: false,
            in_flight: 0,
            accepting_posts: true,
            in_flight_posts: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct WindowHandle(*mut c_void);

// GLFW permits its context to move to the first renderer thread. Runtime
// initialization guarantees that the window outlives all renderer callbacks.
unsafe impl Send for WindowHandle {}
unsafe impl Sync for WindowHandle {}

impl RendererState {
    fn new(window: *mut c_void, gl: GlApi, wake: EventWake) -> Self {
        let tasks = ScheduledQueue::new();
        assert!(tasks.start_accepting());
        Self {
            window: WindowHandle(window),
            gl,
            wake,
            tasks,
            callbacks: Mutex::new(RendererCallbackLifecycle::default()),
            callbacks_idle: Condvar::new(),
            gl_owner: Mutex::new(None),
            callback_failed: AtomicBool::new(false),
            gl_context_stranded: AtomicBool::new(false),
            finalized: AtomicBool::new(false),
        }
    }

    fn make_current(&self) -> bool {
        let current_thread = thread::current().id();
        let mut owner = self
            .gl_owner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if owner.is_some_and(|owner| owner != current_thread) {
            eprintln!("[host-rs] GL make-current rejected on a second render thread");
            return false;
        }
        let window = self.window.0;
        let previous = unsafe { (self.gl.get_current_context)() };
        if previous == window {
            if owner.is_none() {
                *owner = Some(current_thread);
            }
            return true;
        }
        let mut rollback = GlContextRollback::new(self, previous);
        // SAFETY: The window and GL function table remain valid until after the
        // renderer is released.
        unsafe { (self.gl.make_context_current)(window) };
        if unsafe { (self.gl.get_current_context)() } != window {
            return false;
        }
        if owner.is_none() {
            *owner = Some(current_thread);
        }
        rollback.disarm();
        true
    }

    fn clear_current(&self) -> bool {
        let owner = self
            .gl_owner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let window = self.window.0;
        if *owner != Some(thread::current().id())
            || unsafe { (self.gl.get_current_context)() } != window
        {
            eprintln!("[host-rs] GL clear-current rejected on a non-owner thread");
            return false;
        }
        let mut detach = GlContextDetachVerification::new(self);
        unsafe { (self.gl.make_context_current)(std::ptr::null_mut()) };
        if unsafe { (self.gl.get_current_context)().is_null() } {
            detach.confirm();
            true
        } else {
            false
        }
    }

    fn present(&self) -> bool {
        let owner = self
            .gl_owner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let window = self.window.0;
        if *owner != Some(thread::current().id())
            || unsafe { (self.gl.get_current_context)() } != window
        {
            eprintln!("[host-rs] GL present rejected without the owned context");
            return false;
        }
        unsafe { (self.gl.swap_buffers)(window) };
        true
    }

    fn create_fbo(&self, width: c_int, height: c_int) -> u32 {
        let owner = self
            .gl_owner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *owner != Some(thread::current().id())
            || unsafe { (self.gl.get_current_context)() } != self.window.0
            || width <= 0
            || height <= 0
        {
            return 0;
        }
        // GLDirect renders to GLFW's default framebuffer.
        0
    }

    fn resolve_proc(&self, name: *const c_char) -> *mut c_void {
        if name.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: Lynx supplies a NUL-terminated function name for this callback.
        if unsafe { CStr::from_ptr(name) }
            .to_bytes()
            .starts_with(b"egl")
        {
            return std::ptr::null_mut();
        }
        unsafe { (self.gl.get_proc_address)(name) }
    }

    fn post(&self, task: TaskToken, interval_nanos: u64) {
        let now_nanos = match monotonic_nanos() {
            Ok(now_nanos) => now_nanos,
            Err(_) => {
                self.record_callback_failure();
                return;
            }
        };
        if self.tasks.push_after(task, interval_nanos, now_nanos) && !self.wake.wake() {
            self.callback_failed.store(true, Ordering::Release);
        }
    }

    fn record_callback_failure(&self) {
        self.callback_failed.store(true, Ordering::Release);
        let _ = self.wake.wake();
    }

    fn record_stranded_gl_context(&self) {
        self.gl_context_stranded.store(true, Ordering::Release);
        self.callback_failed.store(true, Ordering::Release);
    }

    fn begin_callback(self: &Arc<Self>, is_post: bool) -> Option<RendererCallbackGuard> {
        let mut callbacks = self
            .callbacks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if callbacks.closing || (is_post && !callbacks.accepting_posts) {
            return None;
        }
        let Some(in_flight) = callbacks.in_flight.checked_add(1) else {
            self.callback_failed.store(true, Ordering::Release);
            return None;
        };
        if is_post {
            let Some(in_flight_posts) = callbacks.in_flight_posts.checked_add(1) else {
                self.callback_failed.store(true, Ordering::Release);
                return None;
            };
            callbacks.in_flight_posts = in_flight_posts;
        }
        callbacks.in_flight = in_flight;
        Some(RendererCallbackGuard {
            state: Arc::clone(self),
            is_post,
        })
    }

    fn stop_accepting_tasks_and_wait_for_posts(&self) {
        {
            let mut callbacks = self
                .callbacks
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            callbacks.accepting_posts = false;
            self.callbacks_idle.notify_all();
        }
        self.tasks.stop_accepting();
        let callbacks = self
            .callbacks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        drop(
            self.callbacks_idle
                .wait_while(callbacks, |callbacks| callbacks.in_flight_posts != 0)
                .unwrap_or_else(|error| error.into_inner()),
        );
    }

    fn close_callbacks_and_wait(&self) {
        let mut callbacks = self
            .callbacks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        callbacks.closing = true;
        self.callbacks_idle.notify_all();
        drop(
            self.callbacks_idle
                .wait_while(callbacks, |callbacks| callbacks.in_flight != 0)
                .unwrap_or_else(|error| error.into_inner()),
        );
    }
}

struct GlContextRollback<'a> {
    state: &'a RendererState,
    previous: *mut c_void,
    armed: bool,
}

impl<'a> GlContextRollback<'a> {
    fn new(state: &'a RendererState, previous: *mut c_void) -> Self {
        Self {
            state,
            previous,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for GlContextRollback<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let restored = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: The callback's GLFW dependencies remain registered, and
            // rollback runs synchronously on the same callback thread.
            unsafe { (self.state.gl.make_context_current)(self.previous) };
            (unsafe { (self.state.gl.get_current_context)() }) == self.previous
        }))
        .unwrap_or(false);
        if !restored {
            self.state.record_stranded_gl_context();
        }
    }
}

struct GlContextDetachVerification<'a> {
    state: &'a RendererState,
    confirmed: bool,
}

impl<'a> GlContextDetachVerification<'a> {
    fn new(state: &'a RendererState) -> Self {
        Self {
            state,
            confirmed: false,
        }
    }

    fn confirm(&mut self) {
        self.confirmed = true;
    }
}

impl Drop for GlContextDetachVerification<'_> {
    fn drop(&mut self) {
        if !self.confirmed {
            self.state.record_stranded_gl_context();
        }
    }
}

struct RendererCallbackGuard {
    state: Arc<RendererState>,
    is_post: bool,
}

impl Drop for RendererCallbackGuard {
    fn drop(&mut self) {
        let mut callbacks = self
            .state
            .callbacks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if callbacks.in_flight == 0 || (self.is_post && callbacks.in_flight_posts == 0) {
            self.state.callback_failed.store(true, Ordering::Release);
            self.state.callbacks_idle.notify_all();
            return;
        }
        callbacks.in_flight -= 1;
        if self.is_post {
            callbacks.in_flight_posts -= 1;
        }
        if callbacks.in_flight == 0 || (self.is_post && callbacks.in_flight_posts == 0) {
            self.state.callbacks_idle.notify_all();
        }
    }
}

static RENDERER_STATES: OnceLock<Mutex<HashMap<usize, Arc<RendererState>>>> = OnceLock::new();

fn renderer_states() -> &'static Mutex<HashMap<usize, Arc<RendererState>>> {
    RENDERER_STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_renderer_state(
    renderer: NonNull<LynxWindowlessRenderer>,
    state: &Arc<RendererState>,
) -> io::Result<()> {
    let mut states = renderer_states()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let key = renderer.as_ptr() as usize;
    if states.contains_key(&key) {
        return Err(io::Error::other(
            "Lynx renderer pointer was already registered",
        ));
    }
    states.insert(key, Arc::clone(state));
    Ok(())
}

fn registered_renderer_state(renderer: *mut LynxWindowlessRenderer) -> Option<Arc<RendererState>> {
    renderer_states()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&(renderer as usize))
        .cloned()
}

fn unregister_renderer_state(
    renderer: NonNull<LynxWindowlessRenderer>,
    expected: &Arc<RendererState>,
) -> io::Result<()> {
    let state = renderer_states()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&(renderer.as_ptr() as usize));
    if state.is_some_and(|state| Arc::ptr_eq(&state, expected)) {
        Ok(())
    } else {
        Err(io::Error::other(
            "Lynx renderer state registry lost its active entry",
        ))
    }
}

fn activate_runtime_generation(
    ui_runner: &GlobalUiRunnerState,
    wake: EventWake,
) -> io::Result<UiActivation> {
    initialize_lynx_log_once();
    ui_runner.activate(wake)
}

fn finish_failed_initialization(
    ui_runner: &GlobalUiRunnerState,
    activation: UiActivation,
) -> io::Result<()> {
    let draining = ui_runner.stop_and_drain(activation)?;
    ui_runner.finish_deactivation(draining)
}

pub struct RuntimeCore {
    renderer: Option<NonNull<LynxWindowlessRenderer>>,
    renderer_state: Arc<RendererState>,
    ui_runner: &'static GlobalUiRunnerState,
    ui_activation: Option<UiActivation>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl RuntimeCore {
    /// # Safety
    ///
    /// `window`, every function in `gl`, and the optional `wake` userdata must
    /// remain valid for callbacks from any Lynx thread until `shutdown` returns.
    pub unsafe fn initialize(window: *mut c_void, gl: GlApi, wake: EventWake) -> io::Result<Self> {
        if window.is_null() {
            return Err(io::Error::other(
                "cannot initialize the Lynx renderer without a GLFW window",
            ));
        }
        let ui_runner = configure_global_ui_runner()?;
        let ui_activation = activate_runtime_generation(ui_runner, wake)?;

        let renderer_state = Arc::new(RendererState::new(window, gl, wake));
        let user_data = Arc::as_ptr(&renderer_state).cast_mut().cast();
        // SAFETY: `renderer_state` is heap allocated before registration and
        // remains at the same address until after renderer release/finalization.
        let renderer = unsafe {
            lynx_sys::lynx_windowless_renderer_create_with_finalizer(
                LYNX_RENDERER_TYPE_GL_DIRECT,
                user_data,
                Some(renderer_finalizer_callback),
            )
        };
        let Some(renderer) = NonNull::new(renderer) else {
            renderer_state.tasks.stop_accepting();
            finish_failed_initialization(ui_runner, ui_activation)?;
            return Err(io::Error::other("could not create Lynx GLDirect renderer"));
        };
        if let Err(error) = register_renderer_state(renderer, &renderer_state) {
            renderer_state.tasks.stop_accepting();
            // SAFETY: No renderer callbacks have been bound yet.
            unsafe { lynx_sys::lynx_windowless_renderer_release(renderer.as_ptr()) };
            finish_failed_initialization(ui_runner, ui_activation)?;
            return Err(error);
        }

        // SAFETY: The renderer is live, every callback catches Rust panics, and
        // its userdata remains stable until `shutdown` releases the renderer.
        unsafe {
            lynx_sys::lynx_windowless_renderer_bind_on_gl_make_current(
                renderer.as_ptr(),
                Some(gl_make_current_callback),
            );
            lynx_sys::lynx_windowless_renderer_bind_on_gl_clear_current(
                renderer.as_ptr(),
                Some(gl_clear_current_callback),
            );
            lynx_sys::lynx_windowless_renderer_bind_on_gl_present(
                renderer.as_ptr(),
                Some(gl_present_callback),
            );
            lynx_sys::lynx_windowless_renderer_bind_on_gl_create_fbo(
                renderer.as_ptr(),
                Some(gl_create_fbo_callback),
            );
            lynx_sys::lynx_windowless_renderer_bind_on_gl_proc_resolver(
                renderer.as_ptr(),
                Some(gl_proc_resolver_callback),
            );
            lynx_sys::lynx_windowless_renderer_bind_on_post_task(
                renderer.as_ptr(),
                Some(renderer_post_task_callback),
            );
        }

        Ok(Self {
            renderer: Some(renderer),
            renderer_state,
            ui_runner,
            ui_activation: Some(ui_activation),
            _thread_bound: PhantomData,
        })
    }

    pub fn run_due_tasks(&self) -> io::Result<()> {
        self.ensure_healthy()?;
        self.ui_runner.run_due()?;
        while let Some(task) = self.renderer_state.tasks.pop_due(monotonic_nanos()?) {
            if let Some(renderer) = self.renderer {
                // SAFETY: The live renderer consumes each task after it has been
                // removed from the queue, so the token cannot run twice.
                unsafe {
                    lynx_sys::lynx_windowless_renderer_run_task(renderer.as_ptr(), task.into_raw())
                };
            }
        }
        self.ensure_healthy()
    }

    pub fn wait_duration(&self, maximum: Duration) -> io::Result<Duration> {
        let ui_wait = self.ui_runner.wait_duration(maximum)?;
        let renderer_wait = wait_duration(&self.renderer_state.tasks, monotonic_nanos()?, maximum);
        Ok(ui_wait.min(renderer_wait))
    }

    pub fn ensure_healthy(&self) -> io::Result<()> {
        if LYNX_LOG_CALLBACK_FAILED.load(Ordering::Acquire)
            || RENDERER_CALLBACK_ENTRY_FAILED.load(Ordering::Acquire)
            || self.ui_runner.callback_failed.load(Ordering::Acquire)
            || self.renderer_state.callback_failed.load(Ordering::Acquire)
            || self
                .renderer_state
                .gl_context_stranded
                .load(Ordering::Acquire)
        {
            Err(io::Error::other("a Lynx runtime callback failed"))
        } else {
            Ok(())
        }
    }

    pub fn requires_process_exit_without_glfw_cleanup(&self) -> bool {
        self.renderer_state
            .gl_context_stranded
            .load(Ordering::Acquire)
    }

    pub fn shutdown(&mut self) -> io::Result<()> {
        if self.renderer.is_none() && self.ui_activation.is_none() {
            return Ok(());
        }

        self.renderer_state
            .stop_accepting_tasks_and_wait_for_posts();
        while let Some(task) = self.renderer_state.tasks.pop_any() {
            if let Some(renderer) = self.renderer {
                // SAFETY: Acceptance is stopped and each queued token is removed
                // before this final drain call, preventing shutdown re-entry.
                unsafe {
                    lynx_sys::lynx_windowless_renderer_run_task(renderer.as_ptr(), task.into_raw())
                };
            }
        }

        let ui_draining = if let Some(activation) = self.ui_activation.take() {
            self.ui_runner.stop_and_drain(activation).map(Some)
        } else {
            Ok(None)
        };

        self.renderer_state.close_callbacks_and_wait();

        let unregister_result = if let Some(renderer) = self.renderer.take() {
            // SAFETY: Renderer work and UI work have been stopped and drained;
            // userdata remains alive through this release and its finalizer.
            // The SDK release contract must quiesce callbacks before returning;
            // the registry remains live through release and prevents callbacks
            // already entering Rust from dereferencing freed userdata.
            unsafe { lynx_sys::lynx_windowless_renderer_release(renderer.as_ptr()) };
            unregister_renderer_state(renderer, &self.renderer_state)
        } else {
            Ok(())
        };
        let finalizer_result = if self.renderer_state.finalized.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(io::Error::other(
                "Lynx renderer release did not run its finalizer",
            ))
        };
        let health_result = self.ensure_healthy();
        let ui_finish_result = match ui_draining {
            Ok(Some(generation)) => self.ui_runner.finish_deactivation(generation),
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };

        ui_finish_result?;
        finalizer_result?;
        unregister_result?;
        health_result
    }
}

impl Drop for RuntimeCore {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn wait_duration<T>(queue: &ScheduledQueue<T>, now_nanos: u64, maximum: Duration) -> Duration {
    queue
        .next_deadline()
        .map(|deadline| Duration::from_nanos(deadline.saturating_sub(now_nanos)))
        .unwrap_or(maximum)
        .min(maximum)
}

fn monotonic_nanos() -> io::Result<u64> {
    let mut time = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    // SAFETY: `time` points to writable storage with Linux's `timespec` layout.
    if unsafe { clock_gettime(CLOCK_MONOTONIC, &mut time) } != 0
        || time.seconds < 0
        || !(0..NANOS_PER_SECOND as c_long).contains(&time.nanoseconds)
    {
        return Err(io::Error::last_os_error());
    }
    let seconds = u64::try_from(time.seconds).unwrap_or(u64::MAX);
    let nanoseconds = u64::try_from(time.nanoseconds).unwrap_or(u64::MAX);
    Ok(seconds
        .saturating_mul(NANOS_PER_SECOND)
        .saturating_add(nanoseconds))
}

fn initialize_lynx_log_once() {
    LYNX_LOG_INITIALIZED.get_or_init(|| {
        #[cfg(test)]
        LYNX_LOG_INITIALIZATION_COUNT.fetch_add(1, Ordering::AcqRel);
        // SAFETY: The process-global log callback has no borrowed userdata and
        // contains all Rust panics before returning to Lynx.
        unsafe {
            lynx_sys::lynx_log_init(Some(lynx_log_callback));
            lynx_sys::lynx_log_set_minimum_level(LYNX_LOG_INFO);
        }
    });
}

unsafe extern "C" fn lynx_log_callback(level: c_int, tag: *const c_char, message: *const c_char) {
    if std::panic::catch_unwind(|| {
        let levels = ["VERBOSE", "DEBUG", "INFO", "WARNING", "ERROR", "FATAL"];
        let level = usize::try_from(level.clamp(0, 5)).unwrap_or(0);
        let tag = if tag.is_null() {
            "".into()
        } else {
            // SAFETY: Lynx log strings are NUL-terminated for this callback.
            unsafe { CStr::from_ptr(tag) }.to_string_lossy()
        };
        let message = if message.is_null() {
            "".into()
        } else {
            // SAFETY: Lynx log strings are NUL-terminated for this callback.
            unsafe { CStr::from_ptr(message) }.to_string_lossy()
        };
        eprintln!("[lynx {} {}] {}", levels[level], tag, message);
    })
    .is_err()
    {
        LYNX_LOG_CALLBACK_FAILED.store(true, Ordering::Release);
    }
}

unsafe extern "C" fn ui_runs_on_current_thread_callback(user_data: *mut c_void) -> bool {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = NonNull::new(user_data.cast::<GlobalUiRunnerState>())?;
        // SAFETY: Lynx retains the process-stable global runner userdata.
        Some(unsafe { state.as_ref() }.runs_on_current_thread())
    })) {
        Ok(Some(result)) => result,
        Ok(None) => false,
        Err(_) => {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Some(state) = NonNull::new(user_data.cast::<GlobalUiRunnerState>()) {
                    // SAFETY: UI runner userdata has process lifetime.
                    unsafe { state.as_ref() }
                        .callback_failed
                        .store(true, Ordering::Release);
                }
            }));
            false
        }
    }
}

unsafe extern "C" fn ui_post_task_callback(
    task: LynxTask,
    target_nanos: u64,
    user_data: *mut c_void,
) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(state) = NonNull::new(user_data.cast::<GlobalUiRunnerState>()) else {
            return;
        };
        // SAFETY: Lynx retains the process-stable global runner userdata.
        let state = unsafe { state.as_ref() };
        let Some(guard) = state.begin_post() else {
            return;
        };
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.post(&guard, TaskToken(task), target_nanos);
        }))
        .is_err()
        {
            state.record_callback_failure();
        }
    }))
    .is_err()
    {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(state) = NonNull::new(user_data.cast::<GlobalUiRunnerState>()) {
                // SAFETY: UI runner userdata has process lifetime.
                unsafe { state.as_ref() }
                    .callback_failed
                    .store(true, Ordering::Release);
            }
        }));
    }
}

enum ContainedCallback<R> {
    Value(R),
    Rejected,
}

fn with_renderer_state<R>(
    renderer: *mut LynxWindowlessRenderer,
    fallback: R,
    is_post: bool,
    callback: impl FnOnce(&RendererState) -> R,
) -> R {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if renderer.is_null() {
            return ContainedCallback::Rejected;
        }
        let Some(state) = registered_renderer_state(renderer) else {
            return ContainedCallback::Rejected;
        };
        let Some(_guard) = state.begin_callback(is_post) else {
            return ContainedCallback::Rejected;
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(&state))) {
            Ok(value) => ContainedCallback::Value(value),
            Err(_) => {
                state.record_callback_failure();
                ContainedCallback::Rejected
            }
        }
    }));
    match result {
        Ok(ContainedCallback::Value(value)) => value,
        Ok(ContainedCallback::Rejected) => fallback,
        Err(_) => {
            RENDERER_CALLBACK_ENTRY_FAILED.store(true, Ordering::Release);
            fallback
        }
    }
}

unsafe extern "C" fn renderer_finalizer_callback(
    renderer: *mut LynxWindowlessRenderer,
    user_data: *mut c_void,
) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(state) = registered_renderer_state(renderer) else {
            return;
        };
        if Arc::as_ptr(&state).cast::<c_void>().cast_mut() != user_data {
            return;
        }
        state.finalized.store(true, Ordering::Release);
    }))
    .is_err()
    {
        RENDERER_CALLBACK_ENTRY_FAILED.store(true, Ordering::Release);
    }
}

unsafe extern "C" fn gl_make_current_callback(renderer: *mut LynxWindowlessRenderer) -> bool {
    with_renderer_state(renderer, false, false, RendererState::make_current)
}

unsafe extern "C" fn gl_clear_current_callback(renderer: *mut LynxWindowlessRenderer) -> bool {
    with_renderer_state(renderer, false, false, RendererState::clear_current)
}

unsafe extern "C" fn gl_present_callback(renderer: *mut LynxWindowlessRenderer) -> bool {
    with_renderer_state(renderer, false, false, RendererState::present)
}

unsafe extern "C" fn gl_create_fbo_callback(
    renderer: *mut LynxWindowlessRenderer,
    width: c_int,
    height: c_int,
) -> u32 {
    with_renderer_state(renderer, 0, false, |state| state.create_fbo(width, height))
}

unsafe extern "C" fn gl_proc_resolver_callback(
    renderer: *mut LynxWindowlessRenderer,
    name: *const c_char,
) -> *mut c_void {
    with_renderer_state(renderer, std::ptr::null_mut(), false, |state| {
        state.resolve_proc(name)
    })
}

unsafe extern "C" fn renderer_post_task_callback(
    renderer: *mut LynxWindowlessRenderer,
    task: LynxTask,
    interval_nanos: u64,
) {
    with_renderer_state(renderer, (), true, |state| {
        state.post(TaskToken(task), interval_nanos);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    static MAKE_CURRENT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static SWAP_COUNT: AtomicUsize = AtomicUsize::new(0);
    static GLOBAL_HEALTH_TEST_LOCK: Mutex<()> = Mutex::new(());

    thread_local! {
        static CURRENT_CONTEXT: Cell<usize> = const { Cell::new(0) };
        static BIND_THEN_PANIC_GET_COUNT: Cell<usize> = const { Cell::new(0) };
        static TRANSACTION_MAKE_COUNT: Cell<usize> = const { Cell::new(0) };
        static VERIFICATION_GET_COUNT: Cell<usize> = const { Cell::new(0) };
    }

    struct BlockingWake {
        entered: mpsc::SyncSender<()>,
        released: Mutex<bool>,
        released_changed: Condvar,
    }

    impl BlockingWake {
        fn release(&self) {
            *self
                .released
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = true;
            self.released_changed.notify_all();
        }
    }

    unsafe fn count_wake(counter: *mut c_void) {
        // SAFETY: The test retains this AtomicUsize through the activation.
        unsafe { &*counter.cast::<AtomicUsize>() }.fetch_add(1, Ordering::AcqRel);
    }

    unsafe fn ignore_wake(_: *mut c_void) {}

    unsafe fn blocking_wake(state: *mut c_void) {
        // SAFETY: Each test retains this boxed state until its callback exits.
        let state = unsafe { &*state.cast::<BlockingWake>() };
        state.entered.send(()).unwrap();
        let released = state
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        drop(
            state
                .released_changed
                .wait_while(released, |released| !*released)
                .unwrap_or_else(|error| error.into_inner()),
        );
    }

    unsafe fn make_context_current(window: *mut c_void) {
        MAKE_CURRENT_COUNT.fetch_add(1, Ordering::AcqRel);
        CURRENT_CONTEXT.set(window as usize);
    }

    unsafe fn get_current_context() -> *mut c_void {
        CURRENT_CONTEXT.get() as *mut c_void
    }

    unsafe fn swap_buffers(_: *mut c_void) {
        SWAP_COUNT.fetch_add(1, Ordering::AcqRel);
    }

    unsafe fn get_proc_address(_: *const c_char) -> *mut c_void {
        std::ptr::dangling_mut::<c_void>()
    }

    unsafe fn bind_context(window: *mut c_void) {
        CURRENT_CONTEXT.set(window as usize);
    }

    unsafe fn get_context_after_bind_then_panic() -> *mut c_void {
        let call = BIND_THEN_PANIC_GET_COUNT.get() + 1;
        BIND_THEN_PANIC_GET_COUNT.set(call);
        if call == 2 {
            panic!("controlled panic after binding the GL context");
        }
        CURRENT_CONTEXT.get() as *mut c_void
    }

    unsafe fn panic_before_bind() -> *mut c_void {
        panic!("controlled panic before binding the GL context");
    }

    unsafe fn counted_bind_context(window: *mut c_void) {
        TRANSACTION_MAKE_COUNT.set(TRANSACTION_MAKE_COUNT.get() + 1);
        CURRENT_CONTEXT.set(window as usize);
    }

    unsafe fn bind_context_but_fail_restore(window: *mut c_void) {
        let call = TRANSACTION_MAKE_COUNT.get() + 1;
        TRANSACTION_MAKE_COUNT.set(call);
        if call == 1 {
            CURRENT_CONTEXT.set(window as usize);
        }
    }

    unsafe fn leave_context_attached(_: *mut c_void) {}

    unsafe fn fail_bind_verification_then_report_current() -> *mut c_void {
        let call = VERIFICATION_GET_COUNT.get() + 1;
        VERIFICATION_GET_COUNT.set(call);
        if call == 2 {
            return std::ptr::null_mut();
        }
        CURRENT_CONTEXT.get() as *mut c_void
    }

    fn test_gl_api() -> GlApi {
        GlApi::new(
            make_context_current,
            get_current_context,
            swap_buffers,
            get_proc_address,
        )
    }

    fn test_wake() -> EventWake {
        EventWake::new(std::ptr::null_mut(), ignore_wake)
    }

    fn blocking_event_wake() -> (Box<BlockingWake>, mpsc::Receiver<()>) {
        let (entered, entered_rx) = mpsc::sync_channel(0);
        (
            Box::new(BlockingWake {
                entered,
                released: Mutex::new(false),
                released_changed: Condvar::new(),
            }),
            entered_rx,
        )
    }

    fn create_registered_test_renderer(
        state: &Arc<RendererState>,
    ) -> NonNull<LynxWindowlessRenderer> {
        let user_data = Arc::as_ptr(state).cast_mut().cast();
        // SAFETY: The test retains `state` through release and unregister.
        let renderer = unsafe {
            lynx_sys::lynx_windowless_renderer_create_with_finalizer(
                LYNX_RENDERER_TYPE_GL_DIRECT,
                user_data,
                None,
            )
        };
        let renderer = NonNull::new(renderer).unwrap();
        register_renderer_state(renderer, state).unwrap();
        renderer
    }

    fn release_registered_test_renderer(
        renderer: NonNull<LynxWindowlessRenderer>,
        state: &Arc<RendererState>,
    ) {
        // SAFETY: No test callback remains in flight at release.
        unsafe { lynx_sys::lynx_windowless_renderer_release(renderer.as_ptr()) };
        unregister_renderer_state(renderer, state).unwrap();
    }

    #[test]
    fn converts_absolute_and_relative_deadlines_with_saturation() {
        let queue = ScheduledQueue::new();
        assert!(queue.start_accepting());

        assert!(queue.push_absolute(1, 50, 100));
        assert_eq!(queue.next_deadline(), Some(100));
        assert_eq!(queue.pop_due(99), None);
        assert_eq!(queue.pop_due(100), Some(1));

        assert!(queue.push_after(2, 20, 100));
        assert_eq!(queue.next_deadline(), Some(120));
        assert_eq!(queue.pop_due(120), Some(2));

        assert!(queue.push_after(3, 10, u64::MAX - 2));
        assert_eq!(queue.next_deadline(), Some(u64::MAX));
        assert_eq!(queue.pop_due(u64::MAX), Some(3));
    }

    #[test]
    fn equal_deadlines_are_fifo_and_tokens_are_consumed_once() {
        let queue = ScheduledQueue::new();
        assert!(queue.start_accepting());
        for task in 0..4 {
            assert!(queue.push_absolute(task, 10, 0));
        }

        let mut tasks = Vec::new();
        while let Some(task) = queue.pop_due(10) {
            tasks.push(task);
        }
        assert_eq!(tasks, vec![0, 1, 2, 3]);
        assert_eq!(queue.pop_due(10), None);
    }

    #[test]
    fn stopped_queue_rejects_reentry_while_draining_accepted_work() {
        let queue = ScheduledQueue::new();
        assert!(queue.start_accepting());
        assert!(queue.push_absolute(1, 10, 0));
        assert!(queue.push_absolute(2, 20, 0));
        queue.stop_accepting();
        assert!(!queue.push_absolute(3, 0, 0));

        let mut drained = Vec::new();
        while let Some(task) = queue.pop_any() {
            drained.push(task);
            assert!(!queue.push_absolute(task + 10, 0, 0));
        }
        assert_eq!(drained, vec![1, 2]);
    }

    #[test]
    fn concurrent_active_host_is_rejected_and_posts_request_a_wakeup() {
        let wake_count = AtomicUsize::new(0);
        let wake = EventWake::new(
            (&wake_count as *const AtomicUsize).cast_mut().cast(),
            count_wake,
        );
        let state = Arc::new(GlobalUiRunnerState::new());
        let activation = state.activate(wake).unwrap();
        assert!(state.runs_on_current_thread());

        let other = Arc::clone(&state);
        assert!(thread::spawn(move || other.activate(test_wake()))
            .join()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("active host"));

        let guard = state.begin_post().unwrap();
        state.post(
            &guard,
            TaskToken(LynxTask {
                runner: std::ptr::null_mut(),
                task: 1,
            }),
            0,
        );
        drop(guard);
        assert_eq!(wake_count.load(Ordering::Acquire), 1);
        assert!(state.tasks.pop_any().is_some());
        let draining = state.stop_and_drain(activation).unwrap();
        state.finish_deactivation(draining).unwrap();
    }

    #[test]
    fn rejected_active_host_does_not_reset_health_and_log_initializes_once() {
        let _health_test = GLOBAL_HEALTH_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let state = GlobalUiRunnerState::new();
        state.callback_failed.store(true, Ordering::Release);
        LYNX_LOG_CALLBACK_FAILED.store(true, Ordering::Release);
        RENDERER_CALLBACK_ENTRY_FAILED.store(true, Ordering::Release);

        let activation = activate_runtime_generation(&state, test_wake()).unwrap();
        assert!(!state.callback_failed.load(Ordering::Acquire));
        assert!(!LYNX_LOG_CALLBACK_FAILED.load(Ordering::Acquire));
        assert!(!RENDERER_CALLBACK_ENTRY_FAILED.load(Ordering::Acquire));
        assert_eq!(LYNX_LOG_INITIALIZATION_COUNT.load(Ordering::Acquire), 1);

        state.callback_failed.store(true, Ordering::Release);
        LYNX_LOG_CALLBACK_FAILED.store(true, Ordering::Release);
        RENDERER_CALLBACK_ENTRY_FAILED.store(true, Ordering::Release);
        assert!(activate_runtime_generation(&state, test_wake()).is_err());
        assert!(state.callback_failed.load(Ordering::Acquire));
        assert!(LYNX_LOG_CALLBACK_FAILED.load(Ordering::Acquire));
        assert!(RENDERER_CALLBACK_ENTRY_FAILED.load(Ordering::Acquire));
        assert_eq!(LYNX_LOG_INITIALIZATION_COUNT.load(Ordering::Acquire), 1);

        let draining = state.stop_and_drain(activation).unwrap();
        state.finish_deactivation(draining).unwrap();
    }

    #[test]
    fn ui_shutdown_waits_for_blocked_wakeup_and_isolates_generations() {
        let _health_test = GLOBAL_HEALTH_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (mut wake_state, wake_entered) = blocking_event_wake();
        let wake = EventWake::new(
            (&mut *wake_state as *mut BlockingWake).cast(),
            blocking_wake,
        );
        let state = Arc::new(GlobalUiRunnerState::new());
        let (active_tx, active_rx) = mpsc::sync_channel(0);
        let (shutdown_tx, shutdown_rx) = mpsc::sync_channel(0);
        let (draining_tx, draining_rx) = mpsc::sync_channel(0);
        let (finish_tx, finish_rx) = mpsc::sync_channel(0);
        let (done_tx, done_rx) = mpsc::sync_channel(0);

        let owner_state = Arc::clone(&state);
        let owner = thread::spawn(move || {
            let activation = owner_state.activate(wake).unwrap();
            active_tx.send(activation).unwrap();
            shutdown_rx.recv().unwrap();
            let generation = owner_state.stop_and_wait_for_posts(activation).unwrap();
            let task = owner_state.tasks.pop_any().unwrap();
            assert_eq!(task.generation, generation);
            assert_eq!(task.token.into_raw().task, 41);
            assert!(owner_state.tasks.pop_any().is_none());
            let draining = UiDrainingGeneration { token: generation };
            draining_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
            owner_state.finish_deactivation(draining).unwrap();
            done_tx.send(()).unwrap();
        });
        let activation = active_rx.recv().unwrap();

        let callback_state = Arc::clone(&state);
        let callback = thread::spawn(move || unsafe {
            ui_post_task_callback(
                LynxTask {
                    runner: std::ptr::null_mut(),
                    task: 41,
                },
                0,
                Arc::as_ptr(&callback_state).cast_mut().cast(),
            )
        });
        wake_entered.recv().unwrap();
        shutdown_tx.send(()).unwrap();

        let lifecycle = state
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        drop(
            state
                .posts_idle
                .wait_while(lifecycle, |lifecycle| lifecycle.draining_token.is_none())
                .unwrap_or_else(|error| error.into_inner()),
        );
        assert!(matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));

        wake_state.release();
        callback.join().unwrap();
        draining_rx.recv().unwrap();

        state.callback_failed.store(true, Ordering::Release);
        LYNX_LOG_CALLBACK_FAILED.store(true, Ordering::Release);
        RENDERER_CALLBACK_ENTRY_FAILED.store(true, Ordering::Release);
        assert!(activate_runtime_generation(&state, test_wake()).is_err());
        assert!(state.callback_failed.load(Ordering::Acquire));
        assert!(LYNX_LOG_CALLBACK_FAILED.load(Ordering::Acquire));
        assert!(RENDERER_CALLBACK_ENTRY_FAILED.load(Ordering::Acquire));
        finish_tx.send(()).unwrap();
        done_rx.recv().unwrap();
        owner.join().unwrap();

        let next = activate_runtime_generation(&state, test_wake()).unwrap();
        assert_ne!(next.token, activation.token);
        assert!(state.tasks.pop_any().is_none());
        let draining = state.stop_and_drain(next).unwrap();
        state.finish_deactivation(draining).unwrap();
    }

    #[test]
    fn first_successful_render_thread_remains_the_gl_owner() {
        MAKE_CURRENT_COUNT.store(0, Ordering::Release);
        SWAP_COUNT.store(0, Ordering::Release);
        CURRENT_CONTEXT.set(0);
        let state = Arc::new(RendererState::new(
            std::ptr::dangling_mut::<c_void>(),
            test_gl_api(),
            test_wake(),
        ));

        assert!(state.make_current());
        assert_eq!(MAKE_CURRENT_COUNT.load(Ordering::Acquire), 1);
        assert!(state.present());
        assert_eq!(SWAP_COUNT.load(Ordering::Acquire), 1);
        assert!(state.clear_current());

        let other = Arc::clone(&state);
        assert!(!thread::spawn(move || {
            assert!(!other.present());
            assert_eq!(other.create_fbo(10, 10), 0);
            other.make_current()
        })
        .join()
        .unwrap());
        assert_eq!(MAKE_CURRENT_COUNT.load(Ordering::Acquire), 2);

        assert!(state.make_current());
        assert!(state.present());
        assert!(state.clear_current());
        assert!(!state.gl_context_stranded.load(Ordering::Acquire));
    }

    #[test]
    fn make_current_callback_restores_null_after_bind_then_panic() {
        CURRENT_CONTEXT.set(0);
        BIND_THEN_PANIC_GET_COUNT.set(0);
        let gl = GlApi::new(
            bind_context,
            get_context_after_bind_then_panic,
            swap_buffers,
            get_proc_address,
        );
        let state = Arc::new(RendererState::new(
            std::ptr::dangling_mut::<c_void>(),
            gl,
            test_wake(),
        ));
        let renderer = create_registered_test_renderer(&state);

        assert!(!unsafe { gl_make_current_callback(renderer.as_ptr()) });
        assert!(unsafe { get_current_context() }.is_null());
        assert!(state
            .gl_owner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_none());
        assert!(state.callback_failed.load(Ordering::Acquire));
        assert!(!state.gl_context_stranded.load(Ordering::Acquire));
        release_registered_test_renderer(renderer, &state);
    }

    #[test]
    fn make_current_callback_preserves_other_context_when_initial_read_panics() {
        let mut window_storage = 0_u8;
        let mut other_storage = 0_u8;
        let window: *mut c_void = (&mut window_storage as *mut u8).cast();
        let other: *mut c_void = (&mut other_storage as *mut u8).cast();
        CURRENT_CONTEXT.set(other as usize);
        TRANSACTION_MAKE_COUNT.set(0);
        let gl = GlApi::new(
            counted_bind_context,
            panic_before_bind,
            swap_buffers,
            get_proc_address,
        );
        let state = Arc::new(RendererState::new(window, gl, test_wake()));
        let renderer = create_registered_test_renderer(&state);

        assert!(!unsafe { gl_make_current_callback(renderer.as_ptr()) });
        assert_eq!(CURRENT_CONTEXT.get(), other as usize);
        assert_eq!(TRANSACTION_MAKE_COUNT.get(), 0);
        assert!(state.callback_failed.load(Ordering::Acquire));
        assert!(!state.gl_context_stranded.load(Ordering::Acquire));

        release_registered_test_renderer(renderer, &state);
    }

    #[test]
    fn make_current_callback_detects_previous_context_restore_failure() {
        let mut window_storage = 0_u8;
        let mut other_storage = 0_u8;
        let window: *mut c_void = (&mut window_storage as *mut u8).cast();
        let other: *mut c_void = (&mut other_storage as *mut u8).cast();
        CURRENT_CONTEXT.set(other as usize);
        TRANSACTION_MAKE_COUNT.set(0);
        VERIFICATION_GET_COUNT.set(0);
        let gl = GlApi::new(
            bind_context_but_fail_restore,
            fail_bind_verification_then_report_current,
            swap_buffers,
            get_proc_address,
        );
        let state = Arc::new(RendererState::new(window, gl, test_wake()));
        let renderer = create_registered_test_renderer(&state);

        assert!(!unsafe { gl_make_current_callback(renderer.as_ptr()) });
        assert_eq!(TRANSACTION_MAKE_COUNT.get(), 2);
        assert_eq!(CURRENT_CONTEXT.get(), window as usize);
        assert!(state.callback_failed.load(Ordering::Acquire));
        assert!(state.gl_context_stranded.load(Ordering::Acquire));

        release_registered_test_renderer(renderer, &state);

        let runtime = RuntimeCore {
            renderer: None,
            renderer_state: Arc::clone(&state),
            ui_runner: global_ui_runner(),
            ui_activation: None,
            _thread_bound: PhantomData,
        };
        assert!(runtime.requires_process_exit_without_glfw_cleanup());
        assert!(runtime.ensure_healthy().is_err());
    }

    #[test]
    fn clear_current_callback_marks_failed_detach_as_stranded() {
        let mut window_storage = 0_u8;
        let window: *mut c_void = (&mut window_storage as *mut u8).cast();
        CURRENT_CONTEXT.set(window as usize);
        let gl = GlApi::new(
            leave_context_attached,
            get_current_context,
            swap_buffers,
            get_proc_address,
        );
        let state = Arc::new(RendererState::new(window, gl, test_wake()));
        assert!(state.make_current());
        let renderer = create_registered_test_renderer(&state);

        assert!(!unsafe { gl_clear_current_callback(renderer.as_ptr()) });
        assert_eq!(CURRENT_CONTEXT.get(), window as usize);
        assert!(state.callback_failed.load(Ordering::Acquire));
        assert!(state.gl_context_stranded.load(Ordering::Acquire));

        release_registered_test_renderer(renderer, &state);
    }

    #[test]
    fn renderer_shutdown_waits_for_blocked_wakeup_and_rejects_late_tasks() {
        let (mut wake_state, wake_entered) = blocking_event_wake();
        let wake = EventWake::new(
            (&mut *wake_state as *mut BlockingWake).cast(),
            blocking_wake,
        );
        let state = Arc::new(RendererState::new(
            std::ptr::dangling_mut::<c_void>(),
            test_gl_api(),
            wake,
        ));
        let renderer = create_registered_test_renderer(&state);
        let renderer_address = renderer.as_ptr() as usize;

        let callback = thread::spawn(move || unsafe {
            renderer_post_task_callback(
                renderer_address as *mut LynxWindowlessRenderer,
                LynxTask {
                    runner: std::ptr::null_mut(),
                    task: 71,
                },
                0,
            )
        });
        wake_entered.recv().unwrap();

        let shutdown_state = Arc::clone(&state);
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        let shutdown = thread::spawn(move || {
            shutdown_state.stop_accepting_tasks_and_wait_for_posts();
            done_tx.send(()).unwrap();
        });
        let callbacks = state
            .callbacks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        drop(
            state
                .callbacks_idle
                .wait_while(callbacks, |callbacks| callbacks.accepting_posts)
                .unwrap_or_else(|error| error.into_inner()),
        );
        assert!(matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));

        wake_state.release();
        callback.join().unwrap();
        done_rx.recv().unwrap();
        shutdown.join().unwrap();

        let accepted = state.tasks.pop_any().unwrap().into_raw();
        assert_eq!(accepted.task, 71);
        assert!(state.tasks.pop_any().is_none());
        unsafe {
            renderer_post_task_callback(
                renderer.as_ptr(),
                LynxTask {
                    runner: std::ptr::null_mut(),
                    task: 72,
                },
                0,
            )
        };
        assert!(state.tasks.pop_any().is_none());

        state.close_callbacks_and_wait();
        release_registered_test_renderer(renderer, &state);
    }
}
