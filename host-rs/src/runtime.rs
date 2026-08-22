use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::{c_char, c_int, c_long, c_void, CStr, CString, OsStr};
use std::io;
use std::marker::PhantomData;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, ThreadId};
use std::time::Duration;

use lynx_sys::{
    LynxGenericResourceFetcher, LynxKeyEvent, LynxLoadMeta, LynxPointerEvent, LynxResourceRequest,
    LynxResourceResponse, LynxTask, LynxUiTaskRunnerConfig, LynxView, LynxViewClient,
    LynxWindowlessRenderer, NapiAsyncWork, NapiCallbackInfo, NapiDeferred, NapiEnv, NapiValue,
    LYNX_LOG_INFO, LYNX_RENDERER_TYPE_GL_DIRECT, LYNX_RESOURCE_TYPE_LYNX_CORE_JS, NAPI_AUTO_LENGTH,
    NAPI_OK,
};

use crate::support::{file_uri, read_file};
use lynx_launcher_platform::Launcher;

const NANOS_PER_SECOND: u64 = 1_000_000_000;
const CLOCK_MONOTONIC: c_int = 1;
const FETCHER_FINALIZER_TIMEOUT: Duration = Duration::from_secs(5);
const LAUNCH_WORK_QUIESCENCE_TIMEOUT: Duration = Duration::from_secs(5);
const MAXIMUM_E2E_LAUNCH_WORK_DELAY_MS: u64 = 5_000;
const INJECTED_APPLICATION_ERROR: &str = "injected native application snapshot failure";
const E2E_UNKNOWN_APPLICATION_ID: &str = "lynx-launcher-e2e-unknown.desktop";

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

pub struct RuntimeViewOptions {
    pub bundle: PathBuf,
    pub lynx_core: PathBuf,
    pub icu: PathBuf,
    pub logical_width: f32,
    pub logical_height: f32,
    pub pixel_ratio: f32,
}

struct ApplicationSnapshot {
    id: String,
    name: String,
    icon_uri: Option<String>,
}

#[derive(Default)]
struct FetcherFinalization {
    finalized: Mutex<bool>,
    changed: Condvar,
}

struct LaunchWorkState {
    accepting: bool,
    pending: usize,
}

struct LaunchWorkLifecycle {
    state: Mutex<LaunchWorkState>,
    idle: Condvar,
}

impl Default for LaunchWorkLifecycle {
    fn default() -> Self {
        Self {
            state: Mutex::new(LaunchWorkState {
                accepting: true,
                pending: 0,
            }),
            idle: Condvar::new(),
        }
    }
}

impl LaunchWorkLifecycle {
    fn register(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.accepting {
            return false;
        }
        let Some(pending) = state.pending.checked_add(1) else {
            state.accepting = false;
            return false;
        };
        state.pending = pending;
        true
    }

    fn complete(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        debug_assert!(state.pending > 0);
        if state.pending == 0 {
            return;
        }
        state.pending -= 1;
        if state.pending == 0 {
            self.idle.notify_all();
        }
    }

    fn stop_accepting(&self) -> usize {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.accepting = false;
        state.pending
    }

    fn wait_for_idle(&self, timeout: Duration) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Ok((state, _)) = self
            .idle
            .wait_timeout_while(state, timeout, |state| state.pending != 0)
        else {
            return false;
        };
        state.pending == 0
    }
}

fn parse_e2e_launch_work_delay(value: Option<&OsStr>) -> io::Result<Duration> {
    let Some(value) = value else {
        return Ok(Duration::ZERO);
    };
    let value = value
        .to_str()
        .ok_or_else(|| io::Error::other("LYNX_LAUNCHER_E2E_LAUNCH_WORK_DELAY_MS must be UTF-8"))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(io::Error::other(
            "LYNX_LAUNCHER_E2E_LAUNCH_WORK_DELAY_MS must be an integer from 0 to 5000",
        ));
    }
    let milliseconds = value.parse::<u64>().map_err(|_| {
        io::Error::other("LYNX_LAUNCHER_E2E_LAUNCH_WORK_DELAY_MS must be an integer from 0 to 5000")
    })?;
    if milliseconds > MAXIMUM_E2E_LAUNCH_WORK_DELAY_MS {
        return Err(io::Error::other(
            "LYNX_LAUNCHER_E2E_LAUNCH_WORK_DELAY_MS must be an integer from 0 to 5000",
        ));
    }
    Ok(Duration::from_millis(milliseconds))
}

impl FetcherFinalization {
    fn signal(&self) {
        let mut finalized = self
            .finalized
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *finalized = true;
        self.changed.notify_all();
    }

    fn wait(&self, timeout: Duration) -> bool {
        let Ok(finalized) = self.finalized.lock() else {
            return false;
        };
        if *finalized {
            return true;
        }
        let Ok((finalized, _)) = self
            .changed
            .wait_timeout_while(finalized, timeout, |finalized| !*finalized)
        else {
            return false;
        };
        *finalized
    }
}

struct ViewState {
    core_source: Vec<u8>,
    bundle_source: Vec<u8>,
    bundle_url: CString,
    icu_path: CString,
    launcher: Result<Launcher, String>,
    applications: Result<Vec<ApplicationSnapshot>, String>,
    wake: EventWake,
    first_screen: AtomicBool,
    received_error: AtomicBool,
    callback_failed: AtomicBool,
    fetcher_finalization: FetcherFinalization,
    launch_work: LaunchWorkLifecycle,
    launch_work_delay: Duration,
    native_callbacks_not_quiesced: AtomicBool,
    snapshot_trace: bool,
    injected_application_error: bool,
}

impl ViewState {
    fn load(options: &RuntimeViewOptions, wake: EventWake) -> io::Result<Self> {
        for (label, value) in [
            ("logical width", options.logical_width),
            ("logical height", options.logical_height),
            ("pixel ratio", options.pixel_ratio),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(io::Error::other(format!(
                    "Lynx view {label} must be positive and finite"
                )));
            }
        }

        let core_source = read_file(&options.lynx_core)?;
        let bundle_source = read_file(&options.bundle)?;
        let bundle_url = CString::new(file_uri(&options.bundle)?).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "bundle file URI unexpectedly contained NUL",
            )
        })?;
        let icu_path = CString::new(options.icu.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "ICU data path contains a NUL byte",
            )
        })?;

        let injected_application_error =
            std::env::var_os("LYNX_LAUNCHER_E2E_FORCE_APPLICATIONS_ERROR").as_deref()
                == Some(OsStr::new("1"));
        let snapshot_trace = std::env::var_os("LYNX_LAUNCHER_E2E_SNAPSHOT_TRACE").as_deref()
            == Some(OsStr::new("1"));
        let launch_work_delay = parse_e2e_launch_work_delay(
            std::env::var_os("LYNX_LAUNCHER_E2E_LAUNCH_WORK_DELAY_MS").as_deref(),
        )?;
        let launcher = if injected_application_error {
            Err(INJECTED_APPLICATION_ERROR.to_owned())
        } else {
            Launcher::discover()
                .map_err(|error| format!("could not discover applications: {error}"))
        };
        let mut applications = match &launcher {
            Err(message) => Err(message.clone()),
            Ok(launcher) => (|| -> Result<Vec<ApplicationSnapshot>, String> {
                let mut applications = Vec::with_capacity(launcher.applications().len());
                for application in launcher.applications() {
                    let icon_uri = launcher
                        .resolve_icon(application.id(), 64)
                        .map_err(|error| {
                            format!("could not resolve icon for {}: {error}", application.id())
                        })?
                        .map(|path| file_uri(&path).map_err(|error| error.to_string()))
                        .transpose()?;
                    applications.push(ApplicationSnapshot {
                        id: application.id().to_owned(),
                        name: application.name().to_owned(),
                        icon_uri,
                    });
                }
                Ok(applications)
            })(),
        };
        if let Some(source_id) = std::env::var_os("LYNX_LAUNCHER_E2E_UNKNOWN_APPLICATION_ID") {
            let source_id = source_id.to_string_lossy();
            let snapshots = applications.as_mut().map_err(|message| {
                io::Error::other(format!(
                    "cannot inject an unknown application ID: {message}"
                ))
            })?;
            let matching = snapshots
                .iter()
                .enumerate()
                .filter_map(|(index, application)| (application.id == source_id).then_some(index))
                .collect::<Vec<_>>();
            let [target_index] = matching.as_slice() else {
                let detail = if matching.is_empty() {
                    "did not match an application"
                } else {
                    "matched more than one application"
                };
                return Err(io::Error::other(format!(
                    "LYNX_LAUNCHER_E2E_UNKNOWN_APPLICATION_ID {detail}: {source_id}"
                )));
            };
            snapshots[*target_index].id = E2E_UNKNOWN_APPLICATION_ID.to_owned();
        }

        Ok(Self {
            core_source,
            bundle_source,
            bundle_url,
            icu_path,
            launcher,
            applications,
            wake,
            first_screen: AtomicBool::new(false),
            received_error: AtomicBool::new(false),
            callback_failed: AtomicBool::new(false),
            fetcher_finalization: FetcherFinalization::default(),
            launch_work: LaunchWorkLifecycle::default(),
            launch_work_delay,
            native_callbacks_not_quiesced: AtomicBool::new(false),
            snapshot_trace,
            injected_application_error,
        })
    }

    fn record_callback_failure(&self) {
        self.callback_failed.store(true, Ordering::Release);
        let _ = self.wake.wake();
    }

    fn wait_for_fetcher_finalizer(&self, timeout: Duration) -> bool {
        let finalized = self.fetcher_finalization.wait(timeout);
        if !finalized {
            self.native_callbacks_not_quiesced
                .store(true, Ordering::Release);
        }
        finalized
    }

    fn stop_launch_work_and_wait(&self, timeout: Duration) -> bool {
        let pending = self.launch_work.stop_accepting();
        if pending != 0 {
            eprintln!("[host-rs] shutdown waiting for {pending} Launcher async work");
        }
        let quiesced = self.launch_work.wait_for_idle(timeout);
        if !quiesced {
            self.native_callbacks_not_quiesced
                .store(true, Ordering::Release);
        } else if pending != 0 {
            eprintln!("[host-rs] Launcher async work quiesced during shutdown");
        }
        quiesced
    }
}

fn is_packaged_core_request(resource_type: c_int) -> bool {
    resource_type == LYNX_RESOURCE_TYPE_LYNX_CORE_JS
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
    snapshot_trace: AtomicBool,
}

impl GlobalUiRunnerState {
    fn new() -> Self {
        Self {
            lifecycle: Mutex::new(UiLifecycle::default()),
            posts_idle: Condvar::new(),
            tasks: ScheduledQueue::new(),
            callback_failed: AtomicBool::new(false),
            snapshot_trace: AtomicBool::new(false),
        }
    }

    fn activate(&self, wake: EventWake, snapshot_trace: bool) -> io::Result<UiActivation> {
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
        self.snapshot_trace.store(snapshot_trace, Ordering::Release);
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
        let runner = task.token.0.runner;
        let task_id = task.token.0.task;
        let deadline_nanos = target_nanos.max(now_nanos);
        let accepted = self.tasks.push_absolute(task, target_nanos, now_nanos);
        if self.snapshot_trace.load(Ordering::Acquire) {
            eprintln!(
                "[host-rs] queue ui post runner={:p} task={} target={} now={} deadline={} accepted={}",
                runner,
                task_id,
                target_nanos,
                now_nanos,
                deadline_nanos,
                accepted
            );
        }
        if accepted && !guard.wake.wake() {
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
        loop {
            let now_nanos = monotonic_nanos()?;
            let Some(task) = self.tasks.pop_due(now_nanos) else {
                break;
            };
            if task.generation != generation {
                self.callback_failed.store(true, Ordering::Release);
                return Err(io::Error::other("a UI task crossed Lynx host generations"));
            }
            if self.snapshot_trace.load(Ordering::Acquire) {
                eprintln!(
                    "[host-rs] queue ui run runner={:p} task={} now={}",
                    task.token.0.runner, task.token.0.task, now_nanos
                );
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
        self.snapshot_trace.store(false, Ordering::Release);
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
    first_present: AtomicBool,
    text_input_active: AtomicBool,
    snapshot_trace: bool,
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
    fn new(window: *mut c_void, gl: GlApi, wake: EventWake, snapshot_trace: bool) -> Self {
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
            first_present: AtomicBool::new(false),
            text_input_active: AtomicBool::new(false),
            snapshot_trace,
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
            if self.snapshot_trace {
                eprintln!("[host-rs] queue GL make-current reused");
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
        if self.snapshot_trace {
            eprintln!("[host-rs] queue GL make-current acquired");
        }
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
            if self.snapshot_trace {
                eprintln!("[host-rs] queue GL clear-current");
            }
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
        if self.snapshot_trace {
            eprintln!("[host-rs] queue GL present");
        }
        if !self.first_present.swap(true, Ordering::AcqRel) {
            eprintln!("[host-rs] first GL frame presented");
            let _ = self.wake.wake();
        }
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
        let deadline_nanos = now_nanos.saturating_add(interval_nanos);
        let runner = task.0.runner;
        let task_id = task.0.task;
        let accepted = self.tasks.push_after(task, interval_nanos, now_nanos);
        if self.snapshot_trace {
            eprintln!(
                "[host-rs] queue renderer post runner={:p} task={} interval={} now={} deadline={} accepted={}",
                runner,
                task_id,
                interval_nanos,
                now_nanos,
                deadline_nanos,
                accepted
            );
        }
        if accepted && !self.wake.wake() {
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
    snapshot_trace: bool,
) -> io::Result<UiActivation> {
    initialize_lynx_log_once();
    ui_runner.activate(wake, snapshot_trace)
}

fn finish_failed_initialization(
    ui_runner: &GlobalUiRunnerState,
    activation: UiActivation,
) -> io::Result<()> {
    let draining = ui_runner.stop_and_drain(activation)?;
    ui_runner.finish_deactivation(draining)
}

pub struct RuntimeCore {
    view: Option<NonNull<LynxView>>,
    view_client: Option<NonNull<LynxViewClient>>,
    fetcher: Option<NonNull<LynxGenericResourceFetcher>>,
    renderer: Option<NonNull<LynxWindowlessRenderer>>,
    renderer_state: Arc<RendererState>,
    view_state: Arc<ViewState>,
    ui_runner: &'static GlobalUiRunnerState,
    ui_activation: Option<UiActivation>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl RuntimeCore {
    /// # Safety
    ///
    /// `window`, every function in `gl`, and the optional `wake` userdata must
    /// remain valid for callbacks from any Lynx thread until `shutdown` returns.
    pub unsafe fn initialize(
        window: *mut c_void,
        gl: GlApi,
        wake: EventWake,
        view_options: &RuntimeViewOptions,
    ) -> io::Result<Self> {
        if window.is_null() {
            return Err(io::Error::other(
                "cannot initialize the Lynx renderer without a GLFW window",
            ));
        }
        let view_state = Arc::new(ViewState::load(view_options, wake)?);
        let ui_runner = configure_global_ui_runner()?;
        let ui_activation =
            activate_runtime_generation(ui_runner, wake, view_state.snapshot_trace)?;

        let renderer_state = Arc::new(RendererState::new(
            window,
            gl,
            wake,
            view_state.snapshot_trace,
        ));
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
            lynx_sys::lynx_windowless_renderer_bind_show_text_input(
                renderer.as_ptr(),
                Some(show_text_input_callback),
            );
        }

        Ok(Self {
            view: None,
            view_client: None,
            fetcher: None,
            renderer: Some(renderer),
            renderer_state,
            view_state,
            ui_runner,
            ui_activation: Some(ui_activation),
            _thread_bound: PhantomData,
        })
    }

    pub fn initialize_view(&mut self, options: &RuntimeViewOptions) -> io::Result<()> {
        let fetcher_state = Arc::into_raw(Arc::clone(&self.view_state));
        // SAFETY: The fetcher owns this Arc reference until its finalizer runs.
        let fetcher = unsafe {
            lynx_sys::lynx_generic_resource_fetcher_create_with_finalizer(
                fetcher_state.cast_mut().cast(),
                Some(resource_fetcher_finalizer_callback),
            )
        };
        let Some(fetcher) = NonNull::new(fetcher) else {
            // SAFETY: Fetcher creation failed, so ownership was not transferred.
            unsafe { drop(Arc::from_raw(fetcher_state)) };
            return Err(io::Error::other("could not create Lynx resource fetcher"));
        };
        self.fetcher = Some(fetcher);
        // SAFETY: The live fetcher retains its stable ViewState userdata.
        unsafe {
            lynx_sys::lynx_generic_resource_fetcher_bind_fetch_resource(
                fetcher.as_ptr(),
                Some(fetch_resource_callback),
            );
            lynx_sys::lynx_generic_resource_fetcher_bind_fetch_resource_path(
                fetcher.as_ptr(),
                Some(fetch_resource_callback),
            );
        }

        let state_pointer = Arc::as_ptr(&self.view_state).cast_mut().cast();
        let renderer = self
            .renderer
            .ok_or_else(|| io::Error::other("Lynx renderer is not available"))?;
        // SAFETY: The builder is released after the synchronous view creation call.
        let builder = unsafe { lynx_sys::lynx_view_builder_create() };
        let Some(builder) = NonNull::new(builder) else {
            return Err(io::Error::other("could not create Lynx view builder"));
        };
        // SAFETY: All pointers remain valid through view creation. The SDK copies
        // builder configuration before the builder is released.
        let view = unsafe {
            lynx_sys::lynx_sys_view_builder_set_screen_size(
                builder.as_ptr(),
                options.logical_width,
                options.logical_height,
                options.pixel_ratio,
            );
            lynx_sys::lynx_sys_view_builder_set_frame(
                builder.as_ptr(),
                0.0,
                0.0,
                options.logical_width,
                options.logical_height,
            );
            lynx_sys::lynx_sys_view_builder_set_font_scale(builder.as_ptr(), 1.0);
            lynx_sys::lynx_view_builder_set_enable_js_runtime(builder.as_ptr(), true);
            lynx_sys::lynx_view_builder_set_icu_data_path(
                builder.as_ptr(),
                self.view_state.icu_path.as_ptr(),
            );
            lynx_sys::lynx_view_builder_set_windowless_renderer(
                builder.as_ptr(),
                renderer.as_ptr(),
            );
            lynx_sys::lynx_view_builder_set_generic_resource_fetcher(
                builder.as_ptr(),
                fetcher.as_ptr(),
            );
            lynx_sys::lynx_view_builder_register_native_module(
                builder.as_ptr(),
                c"Launcher".as_ptr(),
                Some(launcher_module_creator),
                state_pointer,
            );
            let view = lynx_sys::lynx_view_create(builder.as_ptr(), state_pointer);
            lynx_sys::lynx_view_builder_release(builder.as_ptr());
            view
        };
        let Some(view) = NonNull::new(view) else {
            return Err(io::Error::other("could not create Lynx view"));
        };
        self.view = Some(view);

        // SAFETY: ViewState outlives the client and all client callbacks.
        let client = unsafe { lynx_sys::lynx_view_client_create(state_pointer) };
        let Some(client) = NonNull::new(client) else {
            return Err(io::Error::other("could not create Lynx view client"));
        };
        self.view_client = Some(client);
        // SAFETY: The view and client are live and owned by RuntimeCore.
        unsafe {
            lynx_sys::lynx_view_client_bind_on_first_screen(
                client.as_ptr(),
                Some(first_screen_callback),
            );
            lynx_sys::lynx_view_client_bind_on_received_error(
                client.as_ptr(),
                Some(received_error_callback),
            );
            lynx_sys::lynx_view_add_client(view.as_ptr(), client.as_ptr());
        }
        self.enter_foreground()?;

        // SAFETY: The metadata setters synchronously retain or copy their input;
        // the bundle bytes themselves remain owned by ViewState for the view's
        // complete lifetime.
        let load_meta = unsafe { lynx_sys::lynx_load_meta_create() };
        let Some(load_meta) = NonNull::<LynxLoadMeta>::new(load_meta) else {
            return Err(io::Error::other("could not create Lynx load metadata"));
        };
        unsafe {
            lynx_sys::lynx_load_meta_set_url(
                load_meta.as_ptr(),
                self.view_state.bundle_url.as_ptr(),
            );
            lynx_sys::lynx_load_meta_set_binary_data(
                load_meta.as_ptr(),
                self.view_state.bundle_source.as_ptr().cast_mut(),
                self.view_state.bundle_source.len(),
                None,
                std::ptr::null_mut(),
            );
            lynx_sys::lynx_view_load_template(view.as_ptr(), load_meta.as_ptr());
            lynx_sys::lynx_load_meta_release(load_meta.as_ptr());
        }

        eprintln!("[host-rs] ICU: {}", options.icu.display());
        eprintln!("[host-rs] lynx_core.js: {}", options.lynx_core.display());
        eprintln!("[host-rs] bundle: {}", options.bundle.display());
        Ok(())
    }

    pub fn run_due_tasks(&self) -> io::Result<()> {
        self.ensure_healthy()?;
        self.ui_runner.run_due()?;
        loop {
            let now_nanos = monotonic_nanos()?;
            let Some(task) = self.renderer_state.tasks.pop_due(now_nanos) else {
                break;
            };
            if self.view_state.snapshot_trace {
                eprintln!(
                    "[host-rs] queue renderer run runner={:p} task={} now={}",
                    task.0.runner, task.0.task, now_nanos
                );
            }
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
        let now_nanos = monotonic_nanos()?;
        let renderer_wait = wait_duration(&self.renderer_state.tasks, now_nanos, maximum);
        let wait = ui_wait.min(renderer_wait);
        if self.view_state.snapshot_trace {
            eprintln!(
                "[host-rs] queue wait now={} ui-deadline={:?} renderer-deadline={:?} timeout={}",
                now_nanos,
                self.ui_runner.tasks.next_deadline(),
                self.renderer_state.tasks.next_deadline(),
                wait.as_nanos()
            );
        }
        Ok(wait)
    }

    pub fn ensure_healthy(&self) -> io::Result<()> {
        if self
            .view_state
            .native_callbacks_not_quiesced
            .load(Ordering::Acquire)
        {
            Err(io::Error::other("native callbacks did not quiesce"))
        } else if LYNX_LOG_CALLBACK_FAILED.load(Ordering::Acquire)
            || RENDERER_CALLBACK_ENTRY_FAILED.load(Ordering::Acquire)
            || self.ui_runner.callback_failed.load(Ordering::Acquire)
            || self.renderer_state.callback_failed.load(Ordering::Acquire)
            || self.view_state.callback_failed.load(Ordering::Acquire)
            || self
                .renderer_state
                .gl_context_stranded
                .load(Ordering::Acquire)
        {
            Err(io::Error::other("a Lynx runtime callback failed"))
        } else if self.view_state.received_error.load(Ordering::Acquire) {
            Err(io::Error::other("the Lynx view reported a load error"))
        } else {
            Ok(())
        }
    }

    pub fn first_frame_ready(&self) -> bool {
        self.view_state.first_screen.load(Ordering::Acquire)
            && self.renderer_state.first_present.load(Ordering::Acquire)
    }

    pub fn send_pointer_event(&self, event: &mut LynxPointerEvent) -> io::Result<()> {
        let renderer = self
            .renderer
            .ok_or_else(|| io::Error::other("Lynx renderer is not available for pointer input"))?;
        event.timestamp = usize::try_from(monotonic_nanos()? / 1_000).unwrap_or(usize::MAX);
        // SAFETY: The event remains live for this synchronous renderer call.
        unsafe { lynx_sys::lynx_windowless_renderer_send_pointer_event(renderer.as_ptr(), event) };
        Ok(())
    }

    pub fn send_key_event(&self, event: &mut LynxKeyEvent) -> io::Result<()> {
        let renderer = self
            .renderer
            .ok_or_else(|| io::Error::other("Lynx renderer is not available for key input"))?;
        event.timestamp = (monotonic_nanos()? / 1_000) as f64;
        // SAFETY: The event and optional character remain live for this
        // synchronous renderer call.
        unsafe { lynx_sys::lynx_windowless_renderer_send_key_event(renderer.as_ptr(), event) };
        Ok(())
    }

    pub fn text_input_active(&self) -> bool {
        self.renderer_state
            .text_input_active
            .load(Ordering::Acquire)
    }

    pub fn cancel_text_input(&self) {
        self.renderer_state
            .text_input_active
            .store(false, Ordering::Release);
    }

    pub fn enter_foreground(&self) -> io::Result<()> {
        let view = self
            .view
            .ok_or_else(|| io::Error::other("Lynx view is not available for foregrounding"))?;
        // SAFETY: RuntimeCore owns the live view.
        unsafe { lynx_sys::lynx_view_enter_foreground(view.as_ptr()) };
        if self.view_state.snapshot_trace {
            eprintln!("[host-rs] view entered foreground");
        }
        Ok(())
    }

    pub fn enter_background(&self) -> io::Result<()> {
        let view = self
            .view
            .ok_or_else(|| io::Error::other("Lynx view is not available for backgrounding"))?;
        // SAFETY: RuntimeCore owns the live view.
        unsafe { lynx_sys::lynx_view_enter_background(view.as_ptr()) };
        if self.view_state.snapshot_trace {
            eprintln!("[host-rs] view entered background");
        }
        Ok(())
    }

    pub fn update_view_metrics(
        &self,
        logical_width: f32,
        logical_height: f32,
        pixel_ratio: f32,
    ) -> io::Result<()> {
        for (label, value) in [
            ("logical width", logical_width),
            ("logical height", logical_height),
            ("pixel ratio", pixel_ratio),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(io::Error::other(format!(
                    "Lynx view {label} must be positive and finite"
                )));
            }
        }
        let view = self
            .view
            .ok_or_else(|| io::Error::other("Lynx view is not available for metric updates"))?;
        // SAFETY: Both by-value shim calls synchronously update the live view.
        unsafe {
            lynx_sys::lynx_sys_view_update_screen_metrics(
                view.as_ptr(),
                logical_width,
                logical_height,
                pixel_ratio,
            );
            lynx_sys::lynx_sys_view_set_frame(
                view.as_ptr(),
                0.0,
                0.0,
                logical_width,
                logical_height,
            );
        }
        Ok(())
    }

    pub fn requires_process_exit_without_glfw_cleanup(&self) -> bool {
        self.renderer_state
            .gl_context_stranded
            .load(Ordering::Acquire)
            || self
                .view_state
                .native_callbacks_not_quiesced
                .load(Ordering::Acquire)
    }

    pub fn native_callbacks_not_quiesced(&self) -> bool {
        self.view_state
            .native_callbacks_not_quiesced
            .load(Ordering::Acquire)
    }

    pub fn shutdown(&mut self) -> io::Result<()> {
        if self.view.is_none()
            && self.view_client.is_none()
            && self.fetcher.is_none()
            && self.renderer.is_none()
            && self.ui_activation.is_none()
        {
            return Ok(());
        }

        self.cancel_text_input();

        if !self
            .view_state
            .stop_launch_work_and_wait(LAUNCH_WORK_QUIESCENCE_TIMEOUT)
        {
            return Err(io::Error::other(
                "Launcher async work did not quiesce before timeout",
            ));
        }

        if let Some(view) = self.view.take() {
            // SAFETY: The view is live and task queues still accept release work.
            unsafe {
                lynx_sys::lynx_view_enter_background(view.as_ptr());
                lynx_sys::lynx_view_release(view.as_ptr());
            }
            if self.view_state.snapshot_trace {
                eprintln!("[host-rs] view entered background for shutdown");
            }
        }
        if let Some(client) = self.view_client.take() {
            // SAFETY: View release has quiesced its client callbacks.
            unsafe { lynx_sys::lynx_view_client_release(client.as_ptr()) };
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

        let mut errors = Vec::new();
        let ui_draining = if let Some(activation) = self.ui_activation.take() {
            match self.ui_runner.stop_and_drain(activation) {
                Ok(generation) => Some(generation),
                Err(error) => {
                    self.view_state
                        .native_callbacks_not_quiesced
                        .store(true, Ordering::Release);
                    errors.push(error.to_string());
                    None
                }
            }
        } else {
            None
        };

        self.renderer_state.close_callbacks_and_wait();

        let unregister_result = if let Some(renderer) = self.renderer.take() {
            // SAFETY: Renderer work and UI work have been stopped and drained;
            // userdata remains alive through this release and its finalizer.
            // The SDK release contract must quiesce callbacks before returning;
            // the registry remains live through release and prevents callbacks
            // already entering Rust from dereferencing freed userdata.
            unsafe { lynx_sys::lynx_windowless_renderer_release(renderer.as_ptr()) };
            Some(unregister_renderer_state(renderer, &self.renderer_state))
        } else {
            None
        };
        let released_fetcher = if let Some(fetcher) = self.fetcher.take() {
            // SAFETY: View, renderer, and queued work no longer use the fetcher.
            unsafe { lynx_sys::lynx_generic_resource_fetcher_release(fetcher.as_ptr()) };
            true
        } else {
            false
        };

        if released_fetcher
            && !self
                .view_state
                .wait_for_fetcher_finalizer(FETCHER_FINALIZER_TIMEOUT)
        {
            errors
                .push("Lynx resource fetcher callbacks did not quiesce before timeout".to_owned());
        }

        if unregister_result.is_some() && !self.renderer_state.finalized.load(Ordering::Acquire) {
            self.view_state
                .native_callbacks_not_quiesced
                .store(true, Ordering::Release);
            errors.push("Lynx renderer release did not run its finalizer".to_owned());
        }
        if let Some(Err(error)) = unregister_result {
            self.view_state
                .native_callbacks_not_quiesced
                .store(true, Ordering::Release);
            errors.push(error.to_string());
        }

        let health_result = self.ensure_healthy();
        if let Err(error) = health_result {
            errors.push(error.to_string());
        }

        if !self.native_callbacks_not_quiesced() {
            if let Some(generation) = ui_draining {
                if let Err(error) = self.ui_runner.finish_deactivation(generation) {
                    self.view_state
                        .native_callbacks_not_quiesced
                        .store(true, Ordering::Release);
                    errors.push(error.to_string());
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(errors.join("; ")))
        }
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

unsafe extern "C" fn resource_fetcher_finalizer_callback(
    _fetcher: *mut LynxGenericResourceFetcher,
    user_data: *mut c_void,
) {
    if user_data.is_null() {
        return;
    }
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: Creation transferred exactly one Arc reference to the
        // fetcher, and its finalizer is the only code that consumes it.
        let state = unsafe { Arc::from_raw(user_data.cast::<ViewState>()) };
        state.fetcher_finalization.signal();
    }));
}

unsafe extern "C" fn fetch_resource_callback(
    fetcher: *mut LynxGenericResourceFetcher,
    request: *mut LynxResourceRequest,
    response: *mut LynxResourceResponse,
) {
    let state = if fetcher.is_null() {
        None
    } else {
        // SAFETY: The fetcher retains its Arc-owned userdata until release.
        let user_data = unsafe { lynx_sys::lynx_generic_resource_fetcher_get_user_data(fetcher) };
        NonNull::new(user_data.cast::<ViewState>()).map(|pointer| unsafe { pointer.as_ref() })
    };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = state.ok_or_else(|| "resource fetcher state is unavailable".to_owned())?;
        if request.is_null() {
            return Err("resource request is null".to_owned());
        }
        let resource_type = unsafe { lynx_sys::lynx_resource_request_get_type(request) };
        let request_url = unsafe { lynx_sys::lynx_resource_request_get_url(request) };
        let url = if request_url.is_null() {
            String::new()
        } else {
            // SAFETY: The request owns a NUL-terminated URL through release.
            unsafe { CStr::from_ptr(request_url) }
                .to_string_lossy()
                .into_owned()
        };
        if !is_packaged_core_request(resource_type) {
            return Err(format!(
                "unsupported resource type {resource_type} for URL: {url}"
            ));
        }
        if response.is_null() {
            return Err("resource response is null".to_owned());
        }
        // SAFETY: ViewState owns the immutable bytes until after fetcher release.
        unsafe {
            lynx_sys::lynx_resource_response_set_code(response, 0);
            lynx_sys::lynx_resource_response_set_data(
                response,
                state.core_source.as_ptr().cast_mut(),
                state.core_source.len(),
                None,
                std::ptr::null_mut(),
            );
        }
        Ok(())
    }));

    let completion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(message)) => Some(message),
            Err(_) => {
                if let Some(state) = state {
                    state.record_callback_failure();
                }
                Some("resource fetch callback panicked".to_owned())
            }
        };
        if let (Some(message), Some(response)) = (error, NonNull::new(response)) {
            let message =
                CString::new(message).unwrap_or_else(|_| c"resource fetch failed".to_owned());
            // SAFETY: The response is live until completion below; the SDK
            // copies the error text synchronously.
            unsafe {
                lynx_sys::lynx_resource_response_set_code(response.as_ptr(), -1);
                lynx_sys::lynx_resource_response_set_error_message(
                    response.as_ptr(),
                    message.as_ptr(),
                );
            }
            eprintln!("[host-rs] {}", message.to_string_lossy());
        }
    }));
    if completion.is_err() {
        if let Some(state) = state {
            state.record_callback_failure();
        }
        if !response.is_null() {
            // SAFETY: Static fallback text avoids further Rust work after a
            // contained panic; the response remains live until completion.
            unsafe {
                lynx_sys::lynx_resource_response_set_code(response, -1);
                lynx_sys::lynx_resource_response_set_error_message(
                    response,
                    c"resource fetch callback panicked".as_ptr(),
                );
            }
        }
    }
    if !request.is_null() {
        // SAFETY: The fetch callback consumes each request exactly once.
        unsafe { lynx_sys::lynx_resource_request_release(request) };
    }
    if !response.is_null() {
        // SAFETY: Completion precedes the matching response release.
        unsafe {
            lynx_sys::lynx_resource_response_callback(response);
            lynx_sys::lynx_resource_response_release(response);
        }
    }
}

unsafe extern "C" fn first_screen_callback(client: *mut LynxViewClient) {
    if client.is_null() {
        return;
    }
    // SAFETY: RuntimeCore retains ViewState until after client release.
    let user_data = unsafe { lynx_sys::lynx_view_client_get_user_data(client) };
    let Some(state) = NonNull::new(user_data.cast::<ViewState>()) else {
        return;
    };
    let state = unsafe { state.as_ref() };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !state.first_screen.swap(true, Ordering::AcqRel) {
            eprintln!("[host-rs] first screen layout completed");
            let _ = state.wake.wake();
        }
    }))
    .is_err()
    {
        state.record_callback_failure();
    }
}

unsafe extern "C" fn received_error_callback(
    client: *mut LynxViewClient,
    code: c_int,
    message: *const c_char,
) {
    if client.is_null() {
        return;
    }
    // SAFETY: RuntimeCore retains ViewState until after client release.
    let user_data = unsafe { lynx_sys::lynx_view_client_get_user_data(client) };
    let Some(state) = NonNull::new(user_data.cast::<ViewState>()) else {
        return;
    };
    let state = unsafe { state.as_ref() };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let message = if message.is_null() {
            "".into()
        } else {
            // SAFETY: Lynx supplies a NUL-terminated message for this callback.
            unsafe { CStr::from_ptr(message) }.to_string_lossy()
        };
        state.received_error.store(true, Ordering::Release);
        eprintln!("[lynx-error {code}] {message}");
        let _ = state.wake.wake();
    }))
    .is_err()
    {
        state.record_callback_failure();
    }
}

enum DeferredSettlementState<D: Copy> {
    Pending(D),
    Settled,
    FatalSettlementFailure,
}

struct DeferredSettlement<D: Copy> {
    state: DeferredSettlementState<D>,
}

impl<D: Copy> DeferredSettlement<D> {
    fn new(deferred: D) -> Self {
        Self {
            state: DeferredSettlementState::Pending(deferred),
        }
    }

    fn is_pending(&self) -> bool {
        matches!(self.state, DeferredSettlementState::Pending(_))
    }

    fn resolve(&mut self, settle: impl FnOnce(D) -> bool) -> bool {
        let DeferredSettlementState::Pending(deferred) = self.state else {
            return false;
        };
        if !settle(deferred) {
            return false;
        }
        self.state = DeferredSettlementState::Settled;
        true
    }

    fn reject(&mut self, settle: impl FnOnce(D) -> bool, on_fatal: impl FnOnce()) -> bool {
        let DeferredSettlementState::Pending(deferred) = self.state else {
            return false;
        };
        if settle(deferred) {
            self.state = DeferredSettlementState::Settled;
            return true;
        }
        self.state = DeferredSettlementState::FatalSettlementFailure;
        on_fatal();
        false
    }

    fn callback_value<T>(&mut self, promise: T, on_fatal: impl FnOnce()) -> Option<T> {
        match self.state {
            DeferredSettlementState::Settled => Some(promise),
            DeferredSettlementState::Pending(_) => {
                self.state = DeferredSettlementState::FatalSettlementFailure;
                on_fatal();
                None
            }
            DeferredSettlementState::FatalSettlementFailure => None,
        }
    }
}

struct NapiPromiseSettlement {
    deferred: DeferredSettlement<NapiDeferred>,
    promise: NapiValue,
}

impl NapiPromiseSettlement {
    unsafe fn callback_value(&mut self, env: NapiEnv) -> NapiValue {
        self.deferred
            .callback_value(self.promise, || unsafe {
                throw_promise_settlement_failure(env)
            })
            .unwrap_or(std::ptr::null_mut())
    }

    unsafe fn reject(&mut self, env: NapiEnv, message: &str) -> bool {
        unsafe { reject_napi_deferred(env, &mut self.deferred, message) }
    }

    unsafe fn resolve(&mut self, env: NapiEnv, resolution: NapiValue) -> bool {
        unsafe { resolve_napi_deferred(env, &mut self.deferred, resolution) }
    }
}

unsafe fn reject_napi_deferred(
    env: NapiEnv,
    settlement: &mut DeferredSettlement<NapiDeferred>,
    message: &str,
) -> bool {
    if !settlement.is_pending() {
        return false;
    }
    let mut text = std::ptr::null_mut();
    let mut error = std::ptr::null_mut();
    // SAFETY: The UTF-8 bytes remain live through these synchronous calls.
    let created_error = unsafe {
        lynx_sys::napi_create_string_utf8_weak(
            env,
            message.as_ptr().cast(),
            message.len(),
            &mut text,
        ) == NAPI_OK
            && lynx_sys::napi_create_error_weak(env, std::ptr::null_mut(), text, &mut error)
                == NAPI_OK
    };
    if !created_error {
        // SAFETY: `error` is writable and receives the environment singleton.
        let _ = unsafe { lynx_sys::napi_get_undefined_weak(env, &mut error) };
    }
    settlement.reject(
        |deferred| {
            // SAFETY: This owner exposes a pending deferred to exactly one
            // reject call. A failed call permanently poisons ownership.
            (unsafe { lynx_sys::napi_reject_deferred_weak(env, deferred, error) }) == NAPI_OK
        },
        || unsafe { throw_promise_settlement_failure(env) },
    )
}

unsafe fn resolve_napi_deferred(
    env: NapiEnv,
    settlement: &mut DeferredSettlement<NapiDeferred>,
    resolution: NapiValue,
) -> bool {
    settlement.resolve(|deferred| {
        // SAFETY: This owner removes the deferred immediately after a
        // successful resolve, before any later operation can panic.
        (unsafe { lynx_sys::napi_resolve_deferred_weak(env, deferred, resolution) }) == NAPI_OK
    })
}

unsafe fn throw_promise_settlement_failure(env: NapiEnv) {
    // SAFETY: The static strings outlive this best-effort N-API call. Failure is
    // intentionally ignored because the deferred is already poisoned.
    let _ = unsafe {
        lynx_sys::napi_throw_error_weak(
            env,
            std::ptr::null(),
            c"failed to settle native Promise".as_ptr(),
        )
    };
}

unsafe fn create_promise(env: NapiEnv) -> Option<NapiPromiseSettlement> {
    let mut deferred = std::ptr::null_mut();
    let mut promise = std::ptr::null_mut();
    // SAFETY: Outputs point to writable storage for this N-API environment.
    (unsafe { lynx_sys::napi_create_promise_weak(env, &mut deferred, &mut promise) } == NAPI_OK)
        .then(|| NapiPromiseSettlement {
            deferred: DeferredSettlement::new(deferred),
            promise,
        })
}

unsafe fn set_string_property(env: NapiEnv, object: NapiValue, name: &CStr, value: &str) -> bool {
    let mut string = std::ptr::null_mut();
    // SAFETY: Inputs remain live for both synchronous N-API calls.
    unsafe {
        lynx_sys::napi_create_string_utf8_weak(env, value.as_ptr().cast(), value.len(), &mut string)
            == NAPI_OK
            && lynx_sys::napi_set_named_property_weak(env, object, name.as_ptr(), string) == NAPI_OK
    }
}

unsafe fn get_applications_impl(
    env: NapiEnv,
    info: NapiCallbackInfo,
    settlement: &mut NapiPromiseSettlement,
) {
    let mut argument_count = 0;
    let mut opaque = std::ptr::null_mut();
    // SAFETY: No arguments are requested; `opaque` receives callback data.
    if unsafe {
        lynx_sys::napi_get_cb_info_weak(
            env,
            info,
            &mut argument_count,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut opaque,
        )
    } != NAPI_OK
    {
        let _ = unsafe {
            settlement.reject(env, "could not read Launcher.getApplications callback data")
        };
        return;
    }
    let Some(state) = NonNull::new(opaque.cast::<ViewState>()) else {
        let _ = unsafe { settlement.reject(env, "launcher platform is not available") };
        return;
    };
    // SAFETY: The native module is owned by the live view and RuntimeCore.
    let state = unsafe { state.as_ref() };
    let applications_snapshot = match &state.applications {
        Ok(applications) => applications,
        Err(message) => {
            if state.injected_application_error {
                eprintln!("[host-rs] Launcher.getApplications rejected: {message}");
            }
            let _ = unsafe { settlement.reject(env, message) };
            return;
        }
    };
    if applications_snapshot.len() > u32::MAX as usize {
        let _ = unsafe { settlement.reject(env, "application snapshot is too large") };
        return;
    }

    let mut applications = std::ptr::null_mut();
    if unsafe {
        lynx_sys::napi_create_array_with_length_weak(
            env,
            applications_snapshot.len(),
            &mut applications,
        )
    } != NAPI_OK
    {
        let _ = unsafe { settlement.reject(env, "could not create the applications array") };
        return;
    }

    for (index, application) in applications_snapshot.iter().enumerate() {
        let mut object = std::ptr::null_mut();
        let converted = unsafe {
            lynx_sys::napi_create_object_weak(env, &mut object) == NAPI_OK
                && set_string_property(env, object, c"id", &application.id)
                && set_string_property(env, object, c"name", &application.name)
                && application
                    .icon_uri
                    .as_ref()
                    .is_none_or(|icon_uri| set_string_property(env, object, c"iconUri", icon_uri))
                && lynx_sys::napi_set_element_weak(env, applications, index as u32, object)
                    == NAPI_OK
        };
        if !converted {
            let _ =
                unsafe { settlement.reject(env, "could not convert an application to JavaScript") };
            return;
        }
    }

    // Trace before settlement so an I/O panic is handled while the deferred is
    // still owned and pending.
    if state.snapshot_trace {
        for (index, application) in applications_snapshot.iter().enumerate() {
            eprintln!(
                "[host-rs] snapshot index={index} id={} name={} icon={}",
                application.id,
                application.name,
                if application.icon_uri.is_some() {
                    "present"
                } else {
                    "missing"
                }
            );
        }
    }

    // SAFETY: The completed array belongs to this environment. Resolve failure
    // leaves ownership pending for one rejection attempt.
    if !unsafe { settlement.resolve(env, applications) } {
        let _ =
            unsafe { settlement.reject(env, "could not resolve Launcher.getApplications Promise") };
        return;
    }

    // Settlement is already consumed. Diagnostics are isolated so they can
    // never re-enter a panic handler with the old deferred.
    let application_count = applications_snapshot.len();
    let _ = std::panic::catch_unwind(|| {
        eprintln!("[host-rs] Launcher.getApplications resolved {application_count} applications");
    });
}

unsafe extern "C" fn get_applications_callback(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let created = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        create_promise(env)
    }));
    let Ok(Some(mut settlement)) = created else {
        return std::ptr::null_mut();
    };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        get_applications_impl(env, info, &mut settlement)
    }))
    .is_err()
    {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            settlement.reject(env, "Launcher.getApplications callback panicked")
        }));
    }
    // SAFETY: Callback completion either returns the original settled Promise
    // or converts any fatal/pending ownership state to a synchronous NULL path.
    unsafe { settlement.callback_value(env) }
}

struct LaunchAsyncWork {
    state: Arc<ViewState>,
    id: String,
    result: Option<Result<(), String>>,
    settlement: DeferredSettlement<NapiDeferred>,
    work: NapiAsyncWork,
    delay: Duration,
}

impl LaunchAsyncWork {
    fn complete_registration(&self) {
        self.state.launch_work.complete();
    }

    fn record_delete_failure(&self) {
        self.state
            .native_callbacks_not_quiesced
            .store(true, Ordering::Release);
        let _ = self.state.wake.wake();
    }
}

// N-API transfers exclusive access to this allocation from the JS callback to
// execute and then completion. Execute touches only Rust-owned fields; N-API
// handles are used only after ownership returns to the JS thread.
unsafe impl Send for LaunchAsyncWork {}

unsafe fn launch_application_inputs(
    env: NapiEnv,
    info: NapiCallbackInfo,
) -> Result<(Arc<ViewState>, String), String> {
    let mut argument_count = 2;
    let mut arguments = [std::ptr::null_mut(); 2];
    let mut opaque = std::ptr::null_mut();
    // SAFETY: The argument array and callback-data output are writable for this
    // synchronous callback-info query.
    if unsafe {
        lynx_sys::napi_get_cb_info_weak(
            env,
            info,
            &mut argument_count,
            arguments.as_mut_ptr(),
            std::ptr::null_mut(),
            &mut opaque,
        )
    } != NAPI_OK
    {
        return Err("could not read launchApplication arguments".to_owned());
    }
    if argument_count != 1 {
        return Err("launchApplication requires one application id".to_owned());
    }

    let mut length = 0;
    // SAFETY: A NULL buffer requests the UTF-8 byte length only.
    if unsafe {
        lynx_sys::napi_get_value_string_utf8_weak(
            env,
            arguments[0],
            std::ptr::null_mut(),
            0,
            &mut length,
        )
    } != NAPI_OK
    {
        return Err("application id must be a string".to_owned());
    }
    let buffer_size = length
        .checked_add(1)
        .ok_or_else(|| "application id is too large".to_owned())?;
    let mut bytes = vec![0_u8; buffer_size];
    let mut copied = 0;
    // SAFETY: `bytes` has `buffer_size` initialized bytes and remains live for
    // the synchronous UTF-8 copy.
    if unsafe {
        lynx_sys::napi_get_value_string_utf8_weak(
            env,
            arguments[0],
            bytes.as_mut_ptr().cast(),
            buffer_size,
            &mut copied,
        )
    } != NAPI_OK
        || copied > length
    {
        return Err("could not read application id".to_owned());
    }
    bytes.truncate(copied);
    let id = String::from_utf8(bytes)
        .map_err(|_| "application id must contain valid UTF-8".to_owned())?;

    let state = NonNull::new(opaque.cast::<ViewState>())
        .ok_or_else(|| "launcher platform is not available".to_owned())?;
    // SAFETY: Native module callbacks run while RuntimeCore owns the Arc that
    // supplied this pointer. The retained reference is released with the work.
    unsafe { Arc::increment_strong_count(state.as_ptr()) };
    let state = unsafe { Arc::from_raw(state.as_ptr()) };
    Ok((state, id))
}

unsafe extern "C" fn execute_launch_application(_env: NapiEnv, data: *mut c_void) {
    let Some(mut work) = NonNull::new(data.cast::<LaunchAsyncWork>()) else {
        return;
    };
    // SAFETY: N-API invokes execute once with the Box exclusively owned by the
    // queued work; completion cannot run until execute returns.
    let work = unsafe { work.as_mut() };
    if !work.delay.is_zero() {
        eprintln!(
            "[host-rs] Launcher.launchApplication worker entered pending delay_ms={}",
            work.delay.as_millis()
        );
        thread::sleep(work.delay);
    }
    work.result = Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            work.state
                .launcher
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|launcher| launcher.launch(&work.id).map_err(|error| error.to_string()))
        }))
        .unwrap_or_else(|_| Err("Launcher.launchApplication worker panicked".to_owned())),
    );
}

unsafe fn settle_launch_application(
    env: NapiEnv,
    status: c_int,
    work: &mut LaunchAsyncWork,
) -> (bool, Option<String>) {
    let result = if status == NAPI_OK {
        work.result
            .take()
            .unwrap_or_else(|| Err("launch worker completed without a result".to_owned()))
    } else {
        Err(format!("launch worker failed with N-API status {status}"))
    };

    match result {
        Ok(()) => {
            let mut undefined = std::ptr::null_mut();
            // SAFETY: `undefined` receives the environment singleton before it
            // is passed to this work's sole deferred owner.
            if unsafe { lynx_sys::napi_get_undefined_weak(env, &mut undefined) } != NAPI_OK
                || !unsafe { resolve_napi_deferred(env, &mut work.settlement, undefined) }
            {
                let _ = unsafe {
                    reject_napi_deferred(
                        env,
                        &mut work.settlement,
                        "could not resolve Launcher.launchApplication Promise",
                    )
                };
                return (
                    false,
                    Some("could not resolve Launcher.launchApplication Promise".to_owned()),
                );
            }
            (true, None)
        }
        Err(message) => {
            let _ = unsafe { reject_napi_deferred(env, &mut work.settlement, &message) };
            (false, Some(message))
        }
    }
}

unsafe extern "C" fn complete_launch_application(env: NapiEnv, status: c_int, data: *mut c_void) {
    let Some(data) = NonNull::new(data.cast::<LaunchAsyncWork>()) else {
        std::process::abort();
    };
    // SAFETY: Successful queueing transferred this Box to the exactly-once
    // completion callback.
    let mut work = unsafe { Box::from_raw(data.as_ptr()) };
    let settled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        settle_launch_application(env, status, &mut work)
    }));
    if settled.is_err() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            reject_napi_deferred(
                env,
                &mut work.settlement,
                "Launcher.launchApplication completion panicked",
            )
        }));
    }
    if work.settlement.is_pending() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            reject_napi_deferred(
                env,
                &mut work.settlement,
                "Launcher.launchApplication completion did not settle",
            )
        }));
    }

    let (resolved, rejection) = settled.unwrap_or((false, None));
    let handle = work.work;
    // SAFETY: Completion owns the valid handle and deletes it exactly once
    // before dropping its data Box.
    if unsafe { lynx_sys::napi_delete_async_work_weak(env, handle) } != NAPI_OK {
        work.record_delete_failure();
        std::mem::forget(work);
        return;
    }
    let _ = std::panic::catch_unwind(|| {
        if resolved {
            eprintln!("[host-rs] Launcher.launchApplication Promise resolved");
        } else if let Some(message) = rejection {
            eprintln!("[host-rs] Launcher.launchApplication Promise rejected: {message}");
        } else {
            eprintln!("[host-rs] Launcher.launchApplication Promise rejected");
        }
    });
    work.complete_registration();
    drop(work);
}

unsafe fn queue_launch_application(
    env: NapiEnv,
    state: Arc<ViewState>,
    id: String,
    settlement: NapiPromiseSettlement,
) -> Result<NapiValue, (NapiPromiseSettlement, &'static str)> {
    if !state.launch_work.register() {
        return Err((settlement, "Launcher is shutting down"));
    }
    let mut resource_name = std::ptr::null_mut();
    // SAFETY: The static resource name remains live through this synchronous
    // conversion; N-API retains the resulting value for work creation.
    if unsafe {
        lynx_sys::napi_create_string_utf8_weak(
            env,
            c"Launcher.launchApplication".as_ptr(),
            NAPI_AUTO_LENGTH,
            &mut resource_name,
        )
    } != NAPI_OK
    {
        state.launch_work.complete();
        return Err((settlement, "could not create launch async resource name"));
    }

    let NapiPromiseSettlement { deferred, promise } = settlement;
    let delay = state.launch_work_delay;
    let data = Box::into_raw(Box::new(LaunchAsyncWork {
        state,
        id,
        result: None,
        settlement: deferred,
        work: std::ptr::null_mut(),
        delay,
    }));
    let mut handle = std::ptr::null_mut();
    // SAFETY: `data` stays owned by this function until queue success transfers
    // it to completion. Both callbacks contain Rust panics.
    if unsafe {
        lynx_sys::napi_create_async_work_weak(
            env,
            std::ptr::null_mut(),
            resource_name,
            Some(execute_launch_application),
            Some(complete_launch_application),
            data.cast(),
            &mut handle,
        )
    } != NAPI_OK
    {
        // SAFETY: Work creation failed, so N-API did not take the data pointer.
        let data = unsafe { Box::from_raw(data) };
        data.complete_registration();
        return Err((
            NapiPromiseSettlement {
                deferred: data.settlement,
                promise,
            },
            "could not create launch async work",
        ));
    }
    // SAFETY: Work has not been queued, so this function still has exclusive
    // access to the data allocation.
    unsafe { (*data).work = handle };
    if unsafe { lynx_sys::napi_queue_async_work_weak(env, handle) } != NAPI_OK {
        // SAFETY: Queue failure guarantees neither callback owns `data`. The
        // valid work handle must be deleted before reclaiming its allocation.
        if unsafe { lynx_sys::napi_delete_async_work_weak(env, handle) } != NAPI_OK {
            // SAFETY: The unqueued work and its data must stay allocated because
            // the SDK did not accept deletion. Shutdown observes the retained
            // pending registration and takes the process-fatal forget path.
            let data = unsafe { Box::from_raw(data) };
            data.record_delete_failure();
            std::mem::forget(data);
            return Ok(promise);
        }
        let data = unsafe { Box::from_raw(data) };
        data.complete_registration();
        return Err((
            NapiPromiseSettlement {
                deferred: data.settlement,
                promise,
            },
            "could not queue launch async work",
        ));
    }
    Ok(promise)
}

unsafe extern "C" fn launch_application_callback(
    env: NapiEnv,
    info: NapiCallbackInfo,
) -> NapiValue {
    let created = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        create_promise(env)
    }));
    let Ok(Some(mut settlement)) = created else {
        return std::ptr::null_mut();
    };
    let inputs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        launch_application_inputs(env, info)
    }));
    let (state, id) = match inputs {
        Ok(Ok(inputs)) => inputs,
        Ok(Err(message)) => {
            let _ = unsafe { settlement.reject(env, &message) };
            return unsafe { settlement.callback_value(env) };
        }
        Err(_) => {
            let _ =
                unsafe { settlement.reject(env, "Launcher.launchApplication callback panicked") };
            return unsafe { settlement.callback_value(env) };
        }
    };

    match unsafe { queue_launch_application(env, state, id, settlement) } {
        Ok(promise) => promise,
        Err((mut settlement, message)) => {
            let _ = unsafe { settlement.reject(env, message) };
            unsafe { settlement.callback_value(env) }
        }
    }
}

unsafe extern "C" fn launcher_module_creator(
    env: NapiEnv,
    exports: NapiValue,
    _module_name: *const c_char,
    opaque: *mut c_void,
) -> NapiValue {
    let initialized = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut get_applications = std::ptr::null_mut();
        let mut launch_application = std::ptr::null_mut();
        // SAFETY: The view owns `opaque`; both callbacks and properties are
        // created synchronously in this N-API environment.
        unsafe {
            lynx_sys::napi_create_function_weak(
                env,
                c"getApplications".as_ptr(),
                NAPI_AUTO_LENGTH,
                Some(get_applications_callback),
                opaque,
                &mut get_applications,
            ) == NAPI_OK
                && lynx_sys::napi_create_function_weak(
                    env,
                    c"launchApplication".as_ptr(),
                    NAPI_AUTO_LENGTH,
                    Some(launch_application_callback),
                    opaque,
                    &mut launch_application,
                ) == NAPI_OK
                && lynx_sys::napi_set_named_property_weak(
                    env,
                    exports,
                    c"getApplications".as_ptr(),
                    get_applications,
                ) == NAPI_OK
                && lynx_sys::napi_set_named_property_weak(
                    env,
                    exports,
                    c"launchApplication".as_ptr(),
                    launch_application,
                ) == NAPI_OK
        }
    }))
    .unwrap_or(false);
    if !initialized {
        // SAFETY: Throwing records an exception in this environment; the
        // returned exports value remains owned by Lynx.
        let _ = unsafe {
            lynx_sys::napi_throw_error_weak(
                env,
                std::ptr::null(),
                c"failed to initialize Launcher module".as_ptr(),
            )
        };
    }
    exports
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

unsafe extern "C" fn show_text_input_callback(renderer: *mut LynxWindowlessRenderer, show: bool) {
    with_renderer_state(renderer, (), false, |state| {
        state.text_input_active.store(show, Ordering::Release);
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

    fn test_view_state() -> Arc<ViewState> {
        Arc::new(ViewState {
            core_source: Vec::new(),
            bundle_source: Vec::new(),
            bundle_url: CString::new("file:///bundle").unwrap(),
            icu_path: CString::new("/icu").unwrap(),
            launcher: Err("launcher unavailable in unit tests".to_owned()),
            applications: Ok(Vec::new()),
            wake: test_wake(),
            first_screen: AtomicBool::new(false),
            received_error: AtomicBool::new(false),
            callback_failed: AtomicBool::new(false),
            fetcher_finalization: FetcherFinalization::default(),
            launch_work: LaunchWorkLifecycle::default(),
            launch_work_delay: Duration::ZERO,
            native_callbacks_not_quiesced: AtomicBool::new(false),
            snapshot_trace: false,
            injected_application_error: false,
        })
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
    fn resource_fetch_policy_accepts_only_lynx_core_requests() {
        assert!(is_packaged_core_request(LYNX_RESOURCE_TYPE_LYNX_CORE_JS));
        for resource_type in [0, 1, 6, 8, c_int::MAX] {
            assert!(!is_packaged_core_request(resource_type));
        }
    }

    #[test]
    fn reject_failure_is_fatal_and_returns_null_with_one_throw() {
        let mut settlement = DeferredSettlement::new(7);
        let reject_count = Cell::new(0);
        let throw_count = Cell::new(0);

        assert!(!settlement.reject(
            |deferred| {
                assert_eq!(deferred, 7);
                reject_count.set(reject_count.get() + 1);
                false
            },
            || throw_count.set(throw_count.get() + 1),
        ));
        assert_eq!(
            settlement.callback_value("original promise", || {
                throw_count.set(throw_count.get() + 1)
            }),
            None
        );
        assert_eq!(reject_count.get(), 1);
        assert_eq!(throw_count.get(), 1);
    }

    #[test]
    fn resolve_failure_then_reject_success_returns_original_promise() {
        let mut settlement = DeferredSettlement::new(11);
        let resolve_count = Cell::new(0);
        let reject_count = Cell::new(0);
        let throw_count = Cell::new(0);

        assert!(!settlement.resolve(|deferred| {
            assert_eq!(deferred, 11);
            resolve_count.set(resolve_count.get() + 1);
            false
        }));
        assert!(settlement.reject(
            |deferred| {
                assert_eq!(deferred, 11);
                reject_count.set(reject_count.get() + 1);
                true
            },
            || throw_count.set(throw_count.get() + 1),
        ));
        assert_eq!(
            settlement.callback_value("original promise", || {
                throw_count.set(throw_count.get() + 1)
            }),
            Some("original promise")
        );
        assert_eq!(resolve_count.get(), 1);
        assert_eq!(reject_count.get(), 1);
        assert_eq!(throw_count.get(), 0);
    }

    #[test]
    fn successful_settlement_cannot_be_used_twice() {
        let mut settlement = DeferredSettlement::new(13);
        let resolve_count = Cell::new(0);
        let reject_count = Cell::new(0);
        let throw_count = Cell::new(0);

        assert!(settlement.resolve(|deferred| {
            assert_eq!(deferred, 13);
            resolve_count.set(resolve_count.get() + 1);
            true
        }));
        assert!(!settlement.reject(
            |_| {
                reject_count.set(reject_count.get() + 1);
                true
            },
            || throw_count.set(throw_count.get() + 1),
        ));
        assert_eq!(
            settlement.callback_value("original promise", || {
                throw_count.set(throw_count.get() + 1)
            }),
            Some("original promise")
        );
        assert_eq!(resolve_count.get(), 1);
        assert_eq!(reject_count.get(), 0);
        assert_eq!(throw_count.get(), 0);
    }

    #[test]
    fn pre_settle_panic_rejects_once_and_returns_original_promise() {
        let mut settlement = DeferredSettlement::new(17);
        let reject_count = Cell::new(0);
        let throw_count = Cell::new(0);
        let result = std::panic::catch_unwind(|| panic!("controlled pre-settle panic"));

        if result.is_err() {
            assert!(settlement.reject(
                |deferred| {
                    assert_eq!(deferred, 17);
                    reject_count.set(reject_count.get() + 1);
                    true
                },
                || throw_count.set(throw_count.get() + 1),
            ));
        }
        assert!(!settlement.reject(
            |_| {
                reject_count.set(reject_count.get() + 1);
                true
            },
            || throw_count.set(throw_count.get() + 1),
        ));
        assert_eq!(
            settlement.callback_value("original promise", || {
                throw_count.set(throw_count.get() + 1)
            }),
            Some("original promise")
        );
        assert_eq!(reject_count.get(), 1);
        assert_eq!(throw_count.get(), 0);
    }

    #[test]
    fn async_launch_work_owns_view_state_until_completion() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send::<LaunchAsyncWork>();
        assert_send_sync::<Launcher>();

        let state = test_view_state();
        let weak_state = Arc::downgrade(&state);
        let work = LaunchAsyncWork {
            state: Arc::clone(&state),
            id: "fixture.desktop".to_owned(),
            result: None,
            settlement: DeferredSettlement::new(std::ptr::null_mut()),
            work: std::ptr::null_mut(),
            delay: Duration::ZERO,
        };

        drop(state);
        assert!(weak_state.upgrade().is_some());
        drop(work);
        assert!(weak_state.upgrade().is_none());
    }

    #[test]
    fn launch_work_shutdown_closes_registration_and_waits_for_completion() {
        let lifecycle = Arc::new(LaunchWorkLifecycle::default());
        assert!(lifecycle.register());

        let completing = Arc::clone(&lifecycle);
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            completing.complete();
        });

        assert_eq!(lifecycle.stop_accepting(), 1);
        assert!(lifecycle.wait_for_idle(Duration::from_secs(1)));
        assert!(!lifecycle.register());
        worker.join().unwrap();
    }

    #[test]
    fn launch_work_shutdown_timeout_keeps_registration_pending() {
        let lifecycle = LaunchWorkLifecycle::default();
        assert!(lifecycle.register());
        assert_eq!(lifecycle.stop_accepting(), 1);
        assert!(!lifecycle.wait_for_idle(Duration::from_millis(1)));
        assert!(!lifecycle.register());
        lifecycle.complete();
        assert_eq!(lifecycle.stop_accepting(), 0);
        assert!(lifecycle.wait_for_idle(Duration::ZERO));
    }

    #[test]
    fn e2e_launch_work_delay_is_strict_bounded_and_disabled_by_default() {
        assert_eq!(parse_e2e_launch_work_delay(None).unwrap(), Duration::ZERO);
        assert_eq!(
            parse_e2e_launch_work_delay(Some(OsStr::new("750"))).unwrap(),
            Duration::from_millis(750)
        );
        assert_eq!(
            parse_e2e_launch_work_delay(Some(OsStr::new("5000"))).unwrap(),
            Duration::from_secs(5)
        );
        for invalid in ["", "-1", "+1", " 1", "1 ", "1.0", "5001"] {
            assert!(parse_e2e_launch_work_delay(Some(OsStr::new(invalid))).is_err());
        }
    }

    #[test]
    fn launch_worker_applies_the_pending_delay_before_platform_launch() {
        let state = test_view_state();
        let delay = Duration::from_millis(10);
        let mut work = LaunchAsyncWork {
            state,
            id: "fixture.desktop".to_owned(),
            result: None,
            settlement: DeferredSettlement::new(std::ptr::null_mut()),
            work: std::ptr::null_mut(),
            delay,
        };
        let started = std::time::Instant::now();

        unsafe {
            execute_launch_application(
                std::ptr::null_mut(),
                (&mut work as *mut LaunchAsyncWork).cast(),
            )
        };

        assert!(started.elapsed() >= delay);
        assert!(matches!(work.result, Some(Err(_))));
    }

    #[test]
    fn launch_work_timeout_latches_process_fatal_callback_ownership() {
        let state = test_view_state();
        assert!(state.launch_work.register());

        assert!(!state.stop_launch_work_and_wait(Duration::ZERO));
        assert!(state.native_callbacks_not_quiesced.load(Ordering::Acquire));
        assert!(!state.launch_work.register());

        state.launch_work.complete();
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
        let activation = state.activate(wake, false).unwrap();
        assert!(state.runs_on_current_thread());

        let other = Arc::clone(&state);
        assert!(thread::spawn(move || other.activate(test_wake(), false))
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
    fn ui_draining_lease_outlives_delayed_fetcher_finalizer() {
        let state = Arc::new(GlobalUiRunnerState::new());
        let activation = state.activate(test_wake(), false).unwrap();
        let draining = state.stop_and_drain(activation).unwrap();
        let view_state = test_view_state();
        let (attempted_tx, attempted_rx) = mpsc::sync_channel(0);
        let (finalize_tx, finalize_rx) = mpsc::sync_channel(0);

        let other_state = Arc::clone(&state);
        let other_view_state = Arc::clone(&view_state);
        let finalizer = thread::spawn(move || {
            attempted_tx
                .send(other_state.activate(test_wake(), false).is_err())
                .unwrap();
            finalize_rx.recv().unwrap();
            other_view_state.fetcher_finalization.signal();
        });

        assert!(attempted_rx.recv().unwrap());
        finalize_tx.send(()).unwrap();
        assert!(view_state.wait_for_fetcher_finalizer(Duration::from_secs(1)));
        assert!(!view_state
            .native_callbacks_not_quiesced
            .load(Ordering::Acquire));
        assert!(state.activate(test_wake(), false).is_err());

        state.finish_deactivation(draining).unwrap();
        finalizer.join().unwrap();
        let next = state.activate(test_wake(), false).unwrap();
        let next_draining = state.stop_and_drain(next).unwrap();
        state.finish_deactivation(next_draining).unwrap();
    }

    #[test]
    fn fetcher_finalizer_timeout_latches_fatal_cleanup() {
        let view_state = test_view_state();
        assert!(!view_state.wait_for_fetcher_finalizer(Duration::ZERO));
        assert!(view_state
            .native_callbacks_not_quiesced
            .load(Ordering::Acquire));

        let runtime = RuntimeCore {
            view: None,
            view_client: None,
            fetcher: None,
            renderer: None,
            renderer_state: Arc::new(RendererState::new(
                std::ptr::dangling_mut::<c_void>(),
                test_gl_api(),
                test_wake(),
                false,
            )),
            view_state,
            ui_runner: global_ui_runner(),
            ui_activation: None,
            _thread_bound: PhantomData,
        };
        assert!(runtime.requires_process_exit_without_glfw_cleanup());
        assert!(runtime.native_callbacks_not_quiesced());
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

        let activation = activate_runtime_generation(&state, test_wake(), false).unwrap();
        assert!(!state.callback_failed.load(Ordering::Acquire));
        assert!(!LYNX_LOG_CALLBACK_FAILED.load(Ordering::Acquire));
        assert!(!RENDERER_CALLBACK_ENTRY_FAILED.load(Ordering::Acquire));
        assert_eq!(LYNX_LOG_INITIALIZATION_COUNT.load(Ordering::Acquire), 1);

        state.callback_failed.store(true, Ordering::Release);
        LYNX_LOG_CALLBACK_FAILED.store(true, Ordering::Release);
        RENDERER_CALLBACK_ENTRY_FAILED.store(true, Ordering::Release);
        assert!(activate_runtime_generation(&state, test_wake(), false).is_err());
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
            let activation = owner_state.activate(wake, false).unwrap();
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
        assert!(activate_runtime_generation(&state, test_wake(), false).is_err());
        assert!(state.callback_failed.load(Ordering::Acquire));
        assert!(LYNX_LOG_CALLBACK_FAILED.load(Ordering::Acquire));
        assert!(RENDERER_CALLBACK_ENTRY_FAILED.load(Ordering::Acquire));
        finish_tx.send(()).unwrap();
        done_rx.recv().unwrap();
        owner.join().unwrap();

        let next = activate_runtime_generation(&state, test_wake(), false).unwrap();
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
            false,
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
            false,
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
        let state = Arc::new(RendererState::new(window, gl, test_wake(), false));
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
        let state = Arc::new(RendererState::new(window, gl, test_wake(), false));
        let renderer = create_registered_test_renderer(&state);

        assert!(!unsafe { gl_make_current_callback(renderer.as_ptr()) });
        assert_eq!(TRANSACTION_MAKE_COUNT.get(), 2);
        assert_eq!(CURRENT_CONTEXT.get(), window as usize);
        assert!(state.callback_failed.load(Ordering::Acquire));
        assert!(state.gl_context_stranded.load(Ordering::Acquire));

        release_registered_test_renderer(renderer, &state);

        let runtime = RuntimeCore {
            view: None,
            view_client: None,
            fetcher: None,
            renderer: None,
            renderer_state: Arc::clone(&state),
            view_state: test_view_state(),
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
        let state = Arc::new(RendererState::new(window, gl, test_wake(), false));
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
            false,
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
