#define GLFW_INCLUDE_NONE
#include <GL/gl.h>
#include <GLFW/glfw3.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <functional>
#include <iostream>
#include <limits>
#include <memory>
#include <mutex>
#include <optional>
#include <queue>
#include <stdexcept>
#include <string>
#include <string_view>
#include <thread>
#include <unordered_map>
#include <utility>
#include <vector>

#include "capi/lynx_generic_resource_fetcher_capi.h"
#include "capi/lynx_load_meta_capi.h"
#include "capi/lynx_log_capi.h"
#include "capi/lynx_native_module_capi.h"
#include "capi/lynx_resource_request_capi.h"
#include "capi/lynx_resource_response_capi.h"
#include "capi/lynx_view_builder_capi.h"
#include "capi/lynx_view_capi.h"
#include "capi/lynx_view_client_capi.h"
#include "capi/lynx_windowless_renderer_capi.h"
#include "lynx_launcher.h"
#include "support.h"

#ifdef USE_WEAK_SUFFIX_NAPI
#include "third_party/weak-node-api/headers/weak_napi_defines.h"
#endif

namespace {

using Clock = std::chrono::steady_clock;

constexpr int kInitialWidth = 1120;
constexpr int kInitialHeight = 760;
constexpr double kMaximumEventWaitSeconds = 0.25;

struct Options {
  std::string bundle;
  std::string lynx_core;
  std::string icu;
  std::optional<double> run_for_seconds;
  bool exit_after_first_frame = false;
  bool check_resources = false;
};

struct RuntimePaths {
  std::filesystem::path bundle;
  std::filesystem::path lynx_core;
  std::filesystem::path icu;
};

void PrintUsage(const char* program) {
  std::cout << "Usage: " << program << " [options]\n"
            << "  --bundle PATH              ReactLynx bundle\n"
            << "  --lynx-core PATH           lynx_core.js\n"
            << "  --icu PATH                 icudtl.dat\n"
            << "  --run-for SECONDS          Exit after a bounded run\n"
            << "  --exit-after-first-frame   Exit after layout and GL present\n"
            << "  --check-resources          Validate resources and Rust ABI "
               "without a window\n"
            << "  --help                     Show this help\n";
}

Options ParseOptions(int argc, char** argv) {
  Options options;
  auto value_after = [&](int& index, const char* option) -> std::string {
    if (index + 1 >= argc) {
      throw std::runtime_error(std::string(option) + " requires a value");
    }
    return argv[++index];
  };
  for (int index = 1; index < argc; ++index) {
    const std::string argument = argv[index];
    if (argument == "--bundle") {
      options.bundle = value_after(index, "--bundle");
    } else if (argument == "--lynx-core") {
      options.lynx_core = value_after(index, "--lynx-core");
    } else if (argument == "--icu") {
      options.icu = value_after(index, "--icu");
    } else if (argument == "--run-for") {
      const std::string value = value_after(index, "--run-for");
      size_t consumed = 0;
      const double seconds = std::stod(value, &consumed);
      if (consumed != value.size() || !std::isfinite(seconds) || seconds <= 0) {
        throw std::runtime_error("--run-for must be a positive number");
      }
      options.run_for_seconds = seconds;
    } else if (argument == "--exit-after-first-frame") {
      options.exit_after_first_frame = true;
    } else if (argument == "--check-resources") {
      options.check_resources = true;
    } else if (argument == "--help") {
      PrintUsage(argv[0]);
      std::exit(0);
    } else {
      throw std::runtime_error("unknown option: " + argument);
    }
  }
  return options;
}

RuntimePaths ResolveRuntimePaths(const Options& options, const char* argv0) {
  const auto executable_directory = launcher_host::ExecutableDirectory(argv0);
  RuntimePaths paths;
  paths.bundle = launcher_host::RequireFile(
      "ReactLynx bundle", options.bundle, "LYNX_LAUNCHER_BUNDLE",
      {executable_directory / "resources/main.lynx.bundle"});
  paths.lynx_core = launcher_host::RequireFile(
      "lynx_core.js", options.lynx_core, "LYNX_LAUNCHER_LYNX_CORE",
      {executable_directory / "resources/lynx_core.js"});
  paths.icu = launcher_host::RequireFile(
      "ICU data", options.icu, "LYNX_LAUNCHER_ICU",
      {executable_directory / "resources/icudtl.dat"});
  return paths;
}

std::string SliceString(LynxSlice slice) {
  if (!slice.data || slice.len == 0) {
    return {};
  }
  return std::string(reinterpret_cast<const char*>(slice.data), slice.len);
}

std::string TakeRustError(LynxStatus status, LynxError* error) {
  std::string message = error ? SliceString(lynx_error_message(error)) : "";
  lynx_error_destroy(error);
  if (message.empty()) {
    message = "Rust platform call failed with status " +
              std::to_string(static_cast<unsigned int>(status));
  }
  return message;
}

class ScheduledTaskQueue {
 public:
  void PushAbsolute(lynx_task_t task, uint64_t target_time_nanos) {
    const auto maximum =
        static_cast<uint64_t>(std::numeric_limits<int64_t>::max());
    const auto target = std::min(target_time_nanos, maximum);
    auto due = Clock::time_point(std::chrono::nanoseconds(target));
    Push(task, std::max(due, Clock::now()));
  }

  void PushAfter(lynx_task_t task, uint64_t interval_nanos) {
    const auto maximum =
        static_cast<uint64_t>(std::numeric_limits<int64_t>::max());
    const auto interval = std::chrono::nanoseconds(
        static_cast<int64_t>(std::min(interval_nanos, maximum)));
    const auto now = Clock::now();
    const auto due = interval > Clock::time_point::max() - now
                         ? Clock::time_point::max()
                         : now + interval;
    Push(task, due);
  }

  void RunDue(const std::function<void(lynx_task_t)>& run) {
    while (true) {
      lynx_task_t task{};
      {
        std::lock_guard lock(mutex_);
        if (tasks_.empty() || tasks_.top().due > Clock::now()) {
          return;
        }
        task = tasks_.top().task;
        tasks_.pop();
      }
      run(task);
    }
  }

  void RunAll(const std::function<void(lynx_task_t)>& run) {
    while (true) {
      lynx_task_t task{};
      {
        std::lock_guard lock(mutex_);
        if (tasks_.empty()) {
          return;
        }
        task = tasks_.top().task;
        tasks_.pop();
      }
      run(task);
    }
  }

  double SecondsUntilNext() const {
    std::lock_guard lock(mutex_);
    if (tasks_.empty()) {
      return kMaximumEventWaitSeconds;
    }
    const auto remaining = tasks_.top().due - Clock::now();
    return std::max(0.0, std::chrono::duration<double>(remaining).count());
  }

  void Clear() {
    std::lock_guard lock(mutex_);
    tasks_ = {};
  }

 private:
  struct Task {
    Clock::time_point due;
    uint64_t sequence;
    lynx_task_t task;
  };

  struct Later {
    bool operator()(const Task& left, const Task& right) const {
      if (left.due == right.due) {
        return left.sequence > right.sequence;
      }
      return left.due > right.due;
    }
  };

  void Push(lynx_task_t task, Clock::time_point due) {
    {
      std::lock_guard lock(mutex_);
      tasks_.push(Task{due, ++sequence_, task});
    }
    glfwPostEmptyEvent();
  }

  mutable std::mutex mutex_;
  std::priority_queue<Task, std::vector<Task>, Later> tasks_;
  uint64_t sequence_ = 0;
};

class GlobalUiRunnerState {
 public:
  bool Configure() {
    std::call_once(configure_once_, [this]() {
      lynx_windowless_ui_task_runner_config_t config{};
      config.struct_size = sizeof(config);
      config.user_data = this;
      config.runs_on_current_thread_callback = [](void* opaque) {
        return static_cast<GlobalUiRunnerState*>(opaque)->RunsOnCurrentThread();
      };
      config.post_task_callback = [](lynx_task_t task, uint64_t target,
                                     void* opaque) {
        static_cast<GlobalUiRunnerState*>(opaque)->Post(task, target);
      };
      configured_ = lynx_windowless_set_global_ui_task_runner(&config);
    });
    return configured_;
  }

  bool Activate() {
    std::lock_guard lock(state_mutex_);
    if (!configured_ || active_ || draining_) {
      return false;
    }
    platform_thread_ = std::this_thread::get_id();
    active_ = true;
    return true;
  }

  void DeactivateAndDrain() {
    {
      std::lock_guard lock(state_mutex_);
      if (!active_ || platform_thread_ != std::this_thread::get_id()) {
        return;
      }
      active_ = false;
      draining_ = true;
    }
    tasks_.RunAll([](lynx_task_t task) {
      if (!lynx_windowless_run_ui_task(task)) {
        std::cerr << "[host] warning: shutdown UI task was not found\n";
      }
    });
    {
      std::lock_guard lock(state_mutex_);
      draining_ = false;
      platform_thread_ = {};
    }
    tasks_.Clear();
  }

  void RunDue() {
    tasks_.RunDue([](lynx_task_t task) {
      if (!lynx_windowless_run_ui_task(task)) {
        std::cerr << "[host] warning: Lynx UI task was not found\n";
      }
    });
  }

  double SecondsUntilNext() const { return tasks_.SecondsUntilNext(); }

 private:
  bool RunsOnCurrentThread() const {
    std::lock_guard lock(state_mutex_);
    return (active_ || draining_) &&
           platform_thread_ == std::this_thread::get_id();
  }

  void Post(lynx_task_t task, uint64_t target) {
    std::lock_guard lock(state_mutex_);
    if (active_) {
      tasks_.PushAbsolute(task, target);
    }
  }

  mutable std::mutex state_mutex_;
  std::once_flag configure_once_;
  ScheduledTaskQueue tasks_;
  std::thread::id platform_thread_;
  bool configured_ = false;
  bool active_ = false;
  bool draining_ = false;
};

GlobalUiRunnerState& GlobalUiRunner() {
  static auto* state = new GlobalUiRunnerState;
  return *state;
}

class Host;

napi_value LauncherModuleCreator(napi_env env, napi_value exports,
                                 const char* module_name, void* opaque);

uint64_t PhysicalKey(int key) {
  if (key >= GLFW_KEY_A && key <= GLFW_KEY_Z) {
    return 0x00070004 + static_cast<uint64_t>(key - GLFW_KEY_A);
  }
  if (key >= GLFW_KEY_1 && key <= GLFW_KEY_9) {
    return 0x0007001e + static_cast<uint64_t>(key - GLFW_KEY_1);
  }
  if (key >= GLFW_KEY_F1 && key <= GLFW_KEY_F12) {
    return 0x0007003a + static_cast<uint64_t>(key - GLFW_KEY_F1);
  }
  switch (key) {
    case GLFW_KEY_0:
      return 0x00070027;
    case GLFW_KEY_ENTER:
      return 0x00070028;
    case GLFW_KEY_ESCAPE:
      return 0x00070029;
    case GLFW_KEY_BACKSPACE:
      return 0x0007002a;
    case GLFW_KEY_TAB:
      return 0x0007002b;
    case GLFW_KEY_SPACE:
      return 0x0007002c;
    case GLFW_KEY_MINUS:
      return 0x0007002d;
    case GLFW_KEY_EQUAL:
      return 0x0007002e;
    case GLFW_KEY_LEFT_BRACKET:
      return 0x0007002f;
    case GLFW_KEY_RIGHT_BRACKET:
      return 0x00070030;
    case GLFW_KEY_BACKSLASH:
      return 0x00070031;
    case GLFW_KEY_SEMICOLON:
      return 0x00070033;
    case GLFW_KEY_APOSTROPHE:
      return 0x00070034;
    case GLFW_KEY_GRAVE_ACCENT:
      return 0x00070035;
    case GLFW_KEY_COMMA:
      return 0x00070036;
    case GLFW_KEY_PERIOD:
      return 0x00070037;
    case GLFW_KEY_SLASH:
      return 0x00070038;
    case GLFW_KEY_CAPS_LOCK:
      return 0x00070039;
    case GLFW_KEY_PRINT_SCREEN:
      return 0x00070046;
    case GLFW_KEY_SCROLL_LOCK:
      return 0x00070047;
    case GLFW_KEY_PAUSE:
      return 0x00070048;
    case GLFW_KEY_INSERT:
      return 0x00070049;
    case GLFW_KEY_HOME:
      return 0x0007004a;
    case GLFW_KEY_PAGE_UP:
      return 0x0007004b;
    case GLFW_KEY_DELETE:
      return 0x0007004c;
    case GLFW_KEY_END:
      return 0x0007004d;
    case GLFW_KEY_PAGE_DOWN:
      return 0x0007004e;
    case GLFW_KEY_RIGHT:
      return 0x0007004f;
    case GLFW_KEY_LEFT:
      return 0x00070050;
    case GLFW_KEY_DOWN:
      return 0x00070051;
    case GLFW_KEY_UP:
      return 0x00070052;
    case GLFW_KEY_LEFT_CONTROL:
      return 0x000700e0;
    case GLFW_KEY_LEFT_SHIFT:
      return 0x000700e1;
    case GLFW_KEY_LEFT_ALT:
      return 0x000700e2;
    case GLFW_KEY_LEFT_SUPER:
      return 0x000700e3;
    case GLFW_KEY_RIGHT_CONTROL:
      return 0x000700e4;
    case GLFW_KEY_RIGHT_SHIFT:
      return 0x000700e5;
    case GLFW_KEY_RIGHT_ALT:
      return 0x000700e6;
    case GLFW_KEY_RIGHT_SUPER:
      return 0x000700e7;
    default:
      return 0;
  }
}

uint64_t LogicalKey(int key) {
  if (key >= GLFW_KEY_A && key <= GLFW_KEY_Z) {
    return static_cast<uint64_t>('a' + key - GLFW_KEY_A);
  }
  if (key >= GLFW_KEY_0 && key <= GLFW_KEY_9) {
    return static_cast<uint64_t>('0' + key - GLFW_KEY_0);
  }
  if (key >= GLFW_KEY_F1 && key <= GLFW_KEY_F12) {
    return 0x00100000801 + static_cast<uint64_t>(key - GLFW_KEY_F1);
  }
  switch (key) {
    case GLFW_KEY_ENTER:
      return 0x0010000000d;
    case GLFW_KEY_ESCAPE:
      return 0x0010000001b;
    case GLFW_KEY_BACKSPACE:
      return 0x00100000008;
    case GLFW_KEY_TAB:
      return 0x00100000009;
    case GLFW_KEY_SPACE:
      return ' ';
    case GLFW_KEY_MINUS:
      return '-';
    case GLFW_KEY_EQUAL:
      return '=';
    case GLFW_KEY_LEFT_BRACKET:
      return '[';
    case GLFW_KEY_RIGHT_BRACKET:
      return ']';
    case GLFW_KEY_BACKSLASH:
      return '\\';
    case GLFW_KEY_SEMICOLON:
      return ';';
    case GLFW_KEY_APOSTROPHE:
      return '\'';
    case GLFW_KEY_GRAVE_ACCENT:
      return '`';
    case GLFW_KEY_COMMA:
      return ',';
    case GLFW_KEY_PERIOD:
      return '.';
    case GLFW_KEY_SLASH:
      return '/';
    case GLFW_KEY_CAPS_LOCK:
      return 0x00100000104;
    case GLFW_KEY_PRINT_SCREEN:
      return 0x00100000608;
    case GLFW_KEY_SCROLL_LOCK:
      return 0x0010000010c;
    case GLFW_KEY_PAUSE:
      return 0x00100000509;
    case GLFW_KEY_INSERT:
      return 0x00100000407;
    case GLFW_KEY_HOME:
      return 0x00100000306;
    case GLFW_KEY_PAGE_UP:
      return 0x00100000308;
    case GLFW_KEY_DELETE:
      return 0x0010000007f;
    case GLFW_KEY_END:
      return 0x00100000305;
    case GLFW_KEY_PAGE_DOWN:
      return 0x00100000307;
    case GLFW_KEY_RIGHT:
      return 0x00100000303;
    case GLFW_KEY_LEFT:
      return 0x00100000302;
    case GLFW_KEY_DOWN:
      return 0x00100000301;
    case GLFW_KEY_UP:
      return 0x00100000304;
    case GLFW_KEY_LEFT_CONTROL:
      return 0x00200000100;
    case GLFW_KEY_RIGHT_CONTROL:
      return 0x00200000101;
    case GLFW_KEY_LEFT_SHIFT:
      return 0x00200000102;
    case GLFW_KEY_RIGHT_SHIFT:
      return 0x00200000103;
    case GLFW_KEY_LEFT_ALT:
      return 0x00200000104;
    case GLFW_KEY_RIGHT_ALT:
      return 0x00200000105;
    case GLFW_KEY_LEFT_SUPER:
      return 0x00200000106;
    case GLFW_KEY_RIGHT_SUPER:
      return 0x00200000107;
    default:
      return 0x00100000001;
  }
}

class Host {
 public:
  Host(GLFWwindow* window, RuntimePaths paths)
      : window_(window),
        paths_(std::move(paths)),
        platform_thread_(std::this_thread::get_id()) {
    glfwSetWindowUserPointer(window_, this);
    glfwSetCursorPosCallback(window_, CursorPositionCallback);
    glfwSetCursorEnterCallback(window_, CursorEnterCallback);
    glfwSetMouseButtonCallback(window_, MouseButtonCallback);
    glfwSetScrollCallback(window_, ScrollCallback);
    glfwSetKeyCallback(window_, KeyCallback);
    glfwSetCharCallback(window_, CharacterCallback);
    glfwSetFramebufferSizeCallback(window_, FramebufferSizeCallback);
    glfwSetWindowSizeCallback(window_, WindowSizeCallback);
    glfwSetWindowContentScaleCallback(window_, ContentScaleCallback);
    glfwSetWindowFocusCallback(window_, FocusCallback);
  }

  ~Host() {
    CancelInput();
    if (view_) {
      lynx_view_enter_background(view_);
      lynx_view_release(view_);
      view_ = nullptr;
    }
    if (view_client_) {
      lynx_view_client_release(view_client_);
      view_client_ = nullptr;
    }
    accept_renderer_tasks_.store(false, std::memory_order_release);
    renderer_tasks_.RunAll([this](lynx_task_t task) {
      if (renderer_) {
        lynx_windowless_renderer_run_task(renderer_, task);
      }
    });
    GlobalUiRunner().DeactivateAndDrain();
    if (renderer_) {
      lynx_windowless_renderer_release(renderer_);
      renderer_ = nullptr;
    }
    if (fetcher_) {
      lynx_generic_resource_fetcher_release(fetcher_);
      fetcher_ = nullptr;
    }
    if (launcher_) {
      lynx_launcher_destroy(launcher_);
      launcher_ = nullptr;
    }
    renderer_tasks_.Clear();
    for (GLFWcursor* cursor : cursors_) {
      if (cursor) {
        glfwDestroyCursor(cursor);
      }
    }
    glfwSetWindowUserPointer(window_, nullptr);
  }

  Host(const Host&) = delete;
  Host& operator=(const Host&) = delete;

  void Initialize() {
    if (lynx_launcher_abi_version() != LYNX_LAUNCHER_ABI_VERSION) {
      throw std::runtime_error("Rust platform ABI mismatch: host expects " +
                               std::to_string(LYNX_LAUNCHER_ABI_VERSION) +
                               ", library provides " +
                               std::to_string(lynx_launcher_abi_version()));
    }
    LynxError* rust_error = nullptr;
    const LynxStatus create_status =
        lynx_launcher_create(&launcher_, &rust_error);
    if (create_status != LYNX_STATUS_OK) {
      throw std::runtime_error("could not initialize application discovery: " +
                               TakeRustError(create_status, rust_error));
    }

    core_source_ = launcher_host::ReadFile(paths_.lynx_core);
    bundle_source_ = launcher_host::ReadFile(paths_.bundle);
    UpdateMetrics();

    if (!GlobalUiRunner().Configure()) {
      throw std::runtime_error(
          "Lynx rejected the process-global windowless UI task runner");
    }
    if (!GlobalUiRunner().Activate()) {
      throw std::runtime_error("Lynx global UI task runner is already active");
    }

    renderer_ = lynx_windowless_renderer_create_with_finalizer(
        kRendererTypeGLDirect, this, nullptr);
    if (!renderer_) {
      throw std::runtime_error("could not create Lynx GLDirect renderer");
    }
    lynx_windowless_renderer_bind_on_gl_make_current(renderer_, GLMakeCurrent);
    lynx_windowless_renderer_bind_on_gl_clear_current(renderer_,
                                                      GLClearCurrent);
    lynx_windowless_renderer_bind_on_gl_present(renderer_, GLPresent);
    lynx_windowless_renderer_bind_on_gl_create_fbo(renderer_, GLCreateFbo);
    lynx_windowless_renderer_bind_on_gl_proc_resolver(renderer_,
                                                      GLProcResolver);
    lynx_windowless_renderer_bind_on_post_task(renderer_, RendererPostTask);
    lynx_windowless_renderer_bind_get_clipboard_data(renderer_, GetClipboard);
    lynx_windowless_renderer_bind_set_clipboard_data(renderer_, SetClipboard);
    lynx_windowless_renderer_bind_activate_system_cursor(renderer_,
                                                         ActivateCursor);
    lynx_windowless_renderer_bind_show_text_input(renderer_, ShowTextInput);

    fetcher_ = lynx_generic_resource_fetcher_create(this);
    if (!fetcher_) {
      throw std::runtime_error("could not create Lynx resource fetcher");
    }
    lynx_generic_resource_fetcher_bind_fetch_resource(fetcher_, FetchResource);
    lynx_generic_resource_fetcher_bind_fetch_resource_path(fetcher_,
                                                           FetchResource);

    lynx_view_builder_t* builder = lynx_view_builder_create();
    if (!builder) {
      throw std::runtime_error("could not create Lynx view builder");
    }
    lynx_view_builder_set_screen_size(builder, logical_width_, logical_height_,
                                      dpr_);
    lynx_view_builder_set_frame(builder, 0.0f, 0.0f, logical_width_,
                                logical_height_);
    lynx_view_builder_set_font_scale(builder, 1.0f);
    lynx_view_builder_set_enable_js_runtime(builder, true);
    const std::string icu_path = paths_.icu.string();
    lynx_view_builder_set_icu_data_path(builder, icu_path.c_str());
    lynx_view_builder_set_windowless_renderer(builder, renderer_);
    lynx_view_builder_set_generic_resource_fetcher(builder, fetcher_);
    lynx_view_builder_register_native_module(builder, "Launcher",
                                             LauncherModuleCreator, this);
    view_ = lynx_view_create(builder, this);
    lynx_view_builder_release(builder);
    if (!view_) {
      throw std::runtime_error("could not create Lynx view");
    }

    view_client_ = lynx_view_client_create(this);
    if (!view_client_) {
      throw std::runtime_error("could not create Lynx view client");
    }
    lynx_view_client_bind_on_page_start(
        view_client_, [](lynx_view_client_t*, const char* url) {
          std::cerr << "[host] loading template: " << (url ? url : "") << '\n';
        });
    lynx_view_client_bind_on_load_success(
        view_client_, [](lynx_view_client_t*) {
          std::cerr << "[host] template load succeeded\n";
        });
    lynx_view_client_bind_on_runtime_ready(
        view_client_, [](lynx_view_client_t*) {
          std::cerr << "[host] JavaScript runtime ready\n";
        });
    lynx_view_client_bind_on_first_screen(
        view_client_, [](lynx_view_client_t* client) {
          auto* host =
              static_cast<Host*>(lynx_view_client_get_user_data(client));
          host->first_screen_.store(true, std::memory_order_release);
          std::cerr << "[host] first screen layout completed\n";
          glfwPostEmptyEvent();
        });
    lynx_view_client_bind_on_received_error(
        view_client_,
        [](lynx_view_client_t* client, int code, const char* message) {
          auto* host =
              static_cast<Host*>(lynx_view_client_get_user_data(client));
          host->received_error_.store(true, std::memory_order_release);
          std::cerr << "[lynx-error " << code << "] "
                    << (message ? message : "") << '\n';
          glfwPostEmptyEvent();
        });
    lynx_view_add_client(view_, view_client_);
    lynx_view_enter_foreground(view_);

    lynx_load_meta_t* load_meta = lynx_load_meta_create();
    if (!load_meta) {
      throw std::runtime_error("could not create Lynx load metadata");
    }
    const std::string bundle_url = launcher_host::FileUri(paths_.bundle);
    lynx_load_meta_set_url(load_meta, bundle_url.c_str());
    lynx_load_meta_set_binary_data(load_meta, bundle_source_.data(),
                                   bundle_source_.size(), nullptr, nullptr);
    lynx_view_load_template(view_, load_meta);
    lynx_load_meta_release(load_meta);

    std::cerr << "[host] ICU: " << paths_.icu << '\n'
              << "[host] lynx_core.js: " << paths_.lynx_core << '\n'
              << "[host] bundle: " << paths_.bundle << '\n';
  }

  int Run(const Options& options) {
    const auto deadline =
        options.run_for_seconds
            ? std::optional(
                  Clock::now() +
                  std::chrono::duration_cast<Clock::duration>(
                      std::chrono::duration<double>(*options.run_for_seconds)))
            : std::nullopt;
    while (!glfwWindowShouldClose(window_)) {
      GlobalUiRunner().RunDue();
      renderer_tasks_.RunDue([this](lynx_task_t task) {
        if (renderer_) {
          lynx_windowless_renderer_run_task(renderer_, task);
        }
      });

      if (options.exit_after_first_frame &&
          first_screen_.load(std::memory_order_acquire) &&
          present_count_.load(std::memory_order_acquire) > 0) {
        std::cerr << "[host] auto-exit after first rendered frame\n";
        break;
      }
      if (deadline && Clock::now() >= *deadline) {
        std::cerr << "[host] auto-exit after bounded run\n";
        break;
      }

      double wait = std::min(GlobalUiRunner().SecondsUntilNext(),
                             renderer_tasks_.SecondsUntilNext());
      wait = std::min(wait, kMaximumEventWaitSeconds);
      if (deadline) {
        wait = std::min(wait, std::max(0.0, std::chrono::duration<double>(
                                                *deadline - Clock::now())
                                                .count()));
      }
      glfwWaitEventsTimeout(std::max(wait, 0.0005));
    }
    return received_error_.load(std::memory_order_acquire) ? 2 : 0;
  }

  LynxLauncher* launcher() const { return launcher_; }

 private:
  bool IsPlatformThread() const {
    return std::this_thread::get_id() == platform_thread_;
  }

  static Host* FromWindow(GLFWwindow* window) {
    return static_cast<Host*>(glfwGetWindowUserPointer(window));
  }

  static Host* FromRenderer(lynx_windowless_renderer_t* renderer) {
    return static_cast<Host*>(lynx_windowless_renderer_get_user_data(renderer));
  }

  static void RendererPostTask(lynx_windowless_renderer_t* renderer,
                               lynx_task_t task, uint64_t interval_nanos) {
    Host* host = FromRenderer(renderer);
    if (host->accept_renderer_tasks_.load(std::memory_order_acquire)) {
      host->renderer_tasks_.PushAfter(task, interval_nanos);
    }
  }

  static bool GLMakeCurrent(lynx_windowless_renderer_t* renderer) {
    Host* host = FromRenderer(renderer);
    if (!host->IsGLThread()) {
      std::cerr
          << "[host] GL make-current rejected on a second render thread\n";
      return false;
    }
    Host*& thread_owner = ContextOwnerForThread();
    if (thread_owner == host && glfwGetCurrentContext() == host->window_) {
      return true;
    }
    glfwMakeContextCurrent(host->window_);
    if (glfwGetCurrentContext() != host->window_) {
      return false;
    }
    thread_owner = host;
    return true;
  }

  static bool GLClearCurrent(lynx_windowless_renderer_t* renderer) {
    Host* host = FromRenderer(renderer);
    Host*& thread_owner = ContextOwnerForThread();
    if (thread_owner != host || glfwGetCurrentContext() != host->window_) {
      std::cerr << "[host] GL clear-current rejected on a non-owner thread\n";
      return false;
    }
    glfwMakeContextCurrent(nullptr);
    thread_owner = nullptr;
    return true;
  }

  static bool GLPresent(lynx_windowless_renderer_t* renderer) {
    Host* host = FromRenderer(renderer);
    if (ContextOwnerForThread() != host ||
        glfwGetCurrentContext() != host->window_) {
      std::cerr << "[host] GL present rejected without the owned context\n";
      return false;
    }
    glfwSwapBuffers(host->window_);
    const uint64_t count =
        host->present_count_.fetch_add(1, std::memory_order_acq_rel) + 1;
    if (count == 1) {
      std::cerr << "[host] first GL frame presented\n";
      glfwPostEmptyEvent();
    }
    return true;
  }

  static uint32_t GLCreateFbo(lynx_windowless_renderer_t* renderer, int width,
                              int height) {
    Host* host = FromRenderer(renderer);
    if (ContextOwnerForThread() != host || width <= 0 || height <= 0) {
      return 0;
    }
    // GLFW's default framebuffer is the direct render target.
    return 0;
  }

  static void* GLProcResolver(lynx_windowless_renderer_t*, const char* name) {
    if (!name || std::string_view(name).starts_with("egl")) {
      return nullptr;
    }
    return reinterpret_cast<void*>(glfwGetProcAddress(name));
  }

  static Host*& ContextOwnerForThread() {
    static thread_local Host* owner = nullptr;
    return owner;
  }

  bool IsGLThread() {
    std::lock_guard lock(gl_thread_mutex_);
    const auto current = std::this_thread::get_id();
    if (gl_thread_ == std::thread::id()) {
      gl_thread_ = current;
    }
    return gl_thread_ == current;
  }

  static void FetchResource(lynx_generic_resource_fetcher_t* fetcher,
                            lynx_resource_request_t* request,
                            lynx_resource_response_t* response) {
    auto* host = static_cast<Host*>(
        lynx_generic_resource_fetcher_get_user_data(fetcher));
    const lynx_resource_type_e type = lynx_resource_request_get_type(request);
    const char* request_url = lynx_resource_request_get_url(request);
    const std::string url = request_url ? request_url : "";
    if (type == kLynxResourceTypeLynxCoreJS ||
        url.find("lynx_core.js") != std::string::npos) {
      lynx_resource_response_set_code(response, 0);
      lynx_resource_response_set_data(response, host->core_source_.data(),
                                      host->core_source_.size(), nullptr,
                                      nullptr);
    } else {
      const std::string message = "unsupported resource URL: " + url;
      lynx_resource_response_set_code(response, -1);
      lynx_resource_response_set_error_message(response, message.c_str());
      std::cerr << "[host] " << message << '\n';
    }
    lynx_resource_request_release(request);
    lynx_resource_response_callback(response);
    lynx_resource_response_release(response);
  }

  static const char* GetClipboard(lynx_windowless_renderer_t* renderer) {
    Host* host = FromRenderer(renderer);
    if (!host->IsPlatformThread()) {
      return "";
    }
    const char* value = glfwGetClipboardString(host->window_);
    host->clipboard_cache_ = value ? value : "";
    return host->clipboard_cache_.c_str();
  }

  static void SetClipboard(lynx_windowless_renderer_t* renderer,
                           const char* data) {
    Host* host = FromRenderer(renderer);
    if (host->IsPlatformThread()) {
      glfwSetClipboardString(host->window_, data ? data : "");
    }
  }

  static void ActivateCursor(lynx_windowless_renderer_t* renderer,
                             lynx_cursor_type_e type, const char*) {
    Host* host = FromRenderer(renderer);
    if (!host->IsPlatformThread()) {
      return;
    }
    if (type == kLynxCursorTypeNone) {
      glfwSetInputMode(host->window_, GLFW_CURSOR, GLFW_CURSOR_HIDDEN);
      return;
    }
    glfwSetInputMode(host->window_, GLFW_CURSOR, GLFW_CURSOR_NORMAL);
    int shape = GLFW_ARROW_CURSOR;
    size_t slot = 0;
    switch (type) {
      case kLynxCursorTypeText:
      case kLynxCursorTypeVerticalText:
        shape = GLFW_IBEAM_CURSOR;
        slot = 1;
        break;
      case kLynxCursorTypeClick:
      case kLynxCursorTypeGrab:
      case kLynxCursorTypeGrabbing:
        shape = GLFW_HAND_CURSOR;
        slot = 2;
        break;
      case kLynxCursorTypePrecise:
        shape = GLFW_CROSSHAIR_CURSOR;
        slot = 3;
        break;
      case kLynxCursorTypeResizeLeftRight:
      case kLynxCursorTypeResizeLeft:
      case kLynxCursorTypeResizeRight:
      case kLynxCursorTypeResizeColumn:
        shape = GLFW_HRESIZE_CURSOR;
        slot = 4;
        break;
      case kLynxCursorTypeResizeUpDown:
      case kLynxCursorTypeResizeUp:
      case kLynxCursorTypeResizeDown:
      case kLynxCursorTypeResizeRow:
        shape = GLFW_VRESIZE_CURSOR;
        slot = 5;
        break;
      default:
        break;
    }
    if (!host->cursors_[slot]) {
      host->cursors_[slot] = glfwCreateStandardCursor(shape);
    }
    glfwSetCursor(host->window_, host->cursors_[slot]);
  }

  static void ShowTextInput(lynx_windowless_renderer_t* renderer, bool show) {
    FromRenderer(renderer)->text_input_active_.store(show,
                                                     std::memory_order_release);
  }

  void UpdateMetrics() {
    int window_width = 0;
    int window_height = 0;
    int framebuffer_width = 0;
    int framebuffer_height = 0;
    glfwGetWindowSize(window_, &window_width, &window_height);
    glfwGetFramebufferSize(window_, &framebuffer_width, &framebuffer_height);
    if (window_width <= 0 || window_height <= 0 || framebuffer_width <= 0 ||
        framebuffer_height <= 0) {
      return;
    }
    logical_width_ = static_cast<float>(window_width);
    logical_height_ = static_cast<float>(window_height);
    const float scale_x = static_cast<float>(framebuffer_width) / window_width;
    const float scale_y =
        static_cast<float>(framebuffer_height) / window_height;
    dpr_ = std::max(0.1f, std::max(scale_x, scale_y));
    if (view_) {
      lynx_view_update_screen_metrics(view_, logical_width_, logical_height_,
                                      dpr_);
      lynx_view_set_frame(view_, 0.0f, 0.0f, logical_width_, logical_height_);
    }
  }

  void SendPointer(
      lynx_pointer_phase_e phase,
      lynx_pointer_signal_kind_e signal = kLynxPointerSignalKindNone,
      double scroll_x = 0.0, double scroll_y = 0.0) {
    if (!renderer_) {
      return;
    }
    if (!pointer_added_ && phase != kLynxPointerPhaseAdd) {
      pointer_added_ = true;
      SendPointer(kLynxPointerPhaseAdd);
    }
    lynx_pointer_event_t event{};
    event.struct_size = sizeof(event);
    event.phase = phase;
    event.timestamp = static_cast<size_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(
            Clock::now().time_since_epoch())
            .count());
    event.x = cursor_x_ * dpr_;
    event.y = cursor_y_ * dpr_;
    event.device = 0;
    event.signal_kind = signal;
    event.scroll_delta_x = scroll_x * dpr_;
    event.scroll_delta_y = scroll_y * dpr_;
    event.device_kind = kLynxPointerDeviceKindMouse;
    event.buttons = pointer_buttons_;
    event.scale = 1.0;
    lynx_windowless_renderer_send_pointer_event(renderer_, &event);
    if (phase == kLynxPointerPhaseRemove) {
      pointer_added_ = false;
    }
  }

  static void CursorPositionCallback(GLFWwindow* window, double x, double y) {
    Host* host = FromWindow(window);
    if (!host) {
      return;
    }
    host->cursor_x_ = x;
    host->cursor_y_ = y;
    host->SendPointer(host->pointer_buttons_ ? kLynxPointerPhaseMove
                                              : kLynxPointerPhaseHover);
  }

  static void CursorEnterCallback(GLFWwindow* window, int entered) {
    Host* host = FromWindow(window);
    if (!host) {
      return;
    }
    if (entered) {
      host->pointer_inside_ = true;
      if (!host->pointer_added_) {
        host->pointer_added_ = true;
        host->SendPointer(kLynxPointerPhaseAdd);
      }
    } else {
      host->pointer_inside_ = false;
      if (host->pointer_added_ && host->pointer_buttons_ == 0) {
        host->SendPointer(kLynxPointerPhaseRemove);
      }
    }
  }

  static int64_t MouseButtonMask(int button) {
    switch (button) {
      case GLFW_MOUSE_BUTTON_LEFT:
        return kLynxPointerMouseButtonsMousePrimary;
      case GLFW_MOUSE_BUTTON_RIGHT:
        return kLynxPointerMouseButtonsMouseSecondary;
      case GLFW_MOUSE_BUTTON_MIDDLE:
        return kLynxPointerMouseButtonsMouseMiddle;
      case GLFW_MOUSE_BUTTON_4:
        return kLynxPointerMouseButtonsMouseBack;
      case GLFW_MOUSE_BUTTON_5:
        return kLynxPointerMouseButtonsMouseForward;
      default:
        return 0;
    }
  }

  static void MouseButtonCallback(GLFWwindow* window, int button, int action,
                                  int) {
    Host* host = FromWindow(window);
    if (!host) {
      return;
    }
    const int64_t mask = MouseButtonMask(button);
    if (!mask) {
      return;
    }
    const int64_t previous = host->pointer_buttons_;
    if (action == GLFW_PRESS) {
      host->pointer_buttons_ |= mask;
      host->SendPointer(previous == 0 ? kLynxPointerPhaseDown
                                      : kLynxPointerPhaseMove);
    } else if (action == GLFW_RELEASE) {
      host->pointer_buttons_ &= ~mask;
      host->SendPointer(host->pointer_buttons_ == 0 ? kLynxPointerPhaseUp
                                                    : kLynxPointerPhaseMove);
      if (host->pointer_buttons_ == 0 && !host->pointer_inside_ &&
          host->pointer_added_) {
        host->SendPointer(kLynxPointerPhaseRemove);
      }
    }
  }

  static void ScrollCallback(GLFWwindow* window, double x_offset,
                             double y_offset) {
    Host* host = FromWindow(window);
    if (!host) {
      return;
    }
    constexpr double kPixelsPerScrollStep = 40.0;
    host->SendPointer(
        host->pointer_buttons_ ? kLynxPointerPhaseMove : kLynxPointerPhaseHover,
        kLynxPointerSignalKindScroll, -x_offset * kPixelsPerScrollStep,
        -y_offset * kPixelsPerScrollStep);
  }

  void SendKeyEvent(lynx_key_event_type_e type, uint64_t physical,
                    uint64_t logical, const char* character, bool synthesized) {
    if (!renderer_) {
      return;
    }
    lynx_key_event_t event{};
    event.struct_size = sizeof(event);
    event.timestamp = static_cast<double>(
        std::chrono::duration_cast<std::chrono::microseconds>(
            Clock::now().time_since_epoch())
            .count());
    event.type = type;
    event.physical = physical;
    event.logical = logical;
    event.character = character;
    event.synthesized = synthesized;
    lynx_windowless_renderer_send_key_event(renderer_, &event);
  }

  static void KeyCallback(GLFWwindow* window, int key, int, int action, int) {
    Host* host = FromWindow(window);
    if (!host || !host->renderer_) {
      return;
    }
    const uint64_t physical = PhysicalKey(key);
    if (physical == 0) {
      return;
    }
    const uint64_t logical = LogicalKey(key);
    if (action == GLFW_PRESS) {
      host->pressed_keys_[key] = {physical, logical};
      host->SendKeyEvent(kLynxKeyEventTypeDown, physical, logical, "", false);
      return;
    }
    const auto pressed = host->pressed_keys_.find(key);
    if (pressed == host->pressed_keys_.end()) {
      return;
    }
    if (action == GLFW_REPEAT) {
      host->SendKeyEvent(kLynxKeyEventTypeRepeat, pressed->second.physical,
                         pressed->second.logical, "", false);
    } else if (action == GLFW_RELEASE) {
      host->SendKeyEvent(kLynxKeyEventTypeUp, pressed->second.physical,
                         pressed->second.logical, nullptr, false);
      host->pressed_keys_.erase(pressed);
    }
  }

  static void CharacterCallback(GLFWwindow* window, unsigned int codepoint) {
    Host* host = FromWindow(window);
    if (!host || !host->renderer_ ||
        !host->text_input_active_.load(std::memory_order_acquire)) {
      return;
    }
    const std::string character = launcher_host::Utf8FromCodepoint(codepoint);
    if (character.empty()) {
      return;
    }
    host->SendKeyEvent(kLynxKeyEventTypeDown, 0, codepoint, character.c_str(),
                       true);
    host->SendKeyEvent(kLynxKeyEventTypeUp, 0, codepoint, nullptr, true);
  }

  void CancelInput() {
    for (const auto& [key, pressed] : pressed_keys_) {
      static_cast<void>(key);
      SendKeyEvent(kLynxKeyEventTypeUp, pressed.physical, pressed.logical,
                   nullptr, true);
    }
    pressed_keys_.clear();
    if (pointer_buttons_ != 0) {
      SendPointer(kLynxPointerPhaseCancel);
      pointer_buttons_ = 0;
    }
    if (pointer_added_) {
      SendPointer(kLynxPointerPhaseRemove);
    }
    pointer_inside_ = false;
    text_input_active_.store(false, std::memory_order_release);
  }

  static void FramebufferSizeCallback(GLFWwindow* window, int, int) {
    if (Host* host = FromWindow(window)) {
      host->UpdateMetrics();
    }
  }

  static void WindowSizeCallback(GLFWwindow* window, int, int) {
    if (Host* host = FromWindow(window)) {
      host->UpdateMetrics();
    }
  }

  static void ContentScaleCallback(GLFWwindow* window, float, float) {
    if (Host* host = FromWindow(window)) {
      host->UpdateMetrics();
    }
  }

  static void FocusCallback(GLFWwindow* window, int focused) {
    Host* host = FromWindow(window);
    if (!host || !host->view_) {
      return;
    }
    if (focused) {
      lynx_view_enter_foreground(host->view_);
    } else {
      host->CancelInput();
      lynx_view_enter_background(host->view_);
    }
  }

  struct PressedKey {
    uint64_t physical;
    uint64_t logical;
  };

  GLFWwindow* window_ = nullptr;
  RuntimePaths paths_;
  std::thread::id platform_thread_;
  LynxLauncher* launcher_ = nullptr;
  lynx_windowless_renderer_t* renderer_ = nullptr;
  lynx_generic_resource_fetcher_t* fetcher_ = nullptr;
  lynx_view_t* view_ = nullptr;
  lynx_view_client_t* view_client_ = nullptr;
  ScheduledTaskQueue renderer_tasks_;
  std::atomic<bool> accept_renderer_tasks_{true};
  std::mutex gl_thread_mutex_;
  std::thread::id gl_thread_;
  std::vector<uint8_t> core_source_;
  std::vector<uint8_t> bundle_source_;
  std::atomic<bool> first_screen_{false};
  std::atomic<bool> received_error_{false};
  std::atomic<uint64_t> present_count_{0};
  float logical_width_ = kInitialWidth;
  float logical_height_ = kInitialHeight;
  float dpr_ = 1.0f;
  double cursor_x_ = 0;
  double cursor_y_ = 0;
  int64_t pointer_buttons_ = 0;
  bool pointer_added_ = false;
  bool pointer_inside_ = false;
  std::atomic<bool> text_input_active_{false};
  std::unordered_map<int, PressedKey> pressed_keys_;
  std::string clipboard_cache_;
  GLFWcursor* cursors_[6]{};
};

void RejectPromise(napi_env env, napi_deferred deferred,
                   const std::string& message) {
  napi_value text = nullptr;
  napi_value error = nullptr;
  if (napi_create_string_utf8(env, message.c_str(), message.size(), &text) ==
          napi_ok &&
      napi_create_error(env, nullptr, text, &error) == napi_ok) {
    napi_reject_deferred(env, deferred, error);
    return;
  }
  napi_value undefined = nullptr;
  napi_get_undefined(env, &undefined);
  napi_reject_deferred(env, deferred, undefined);
}

bool SetStringProperty(napi_env env, napi_value object, const char* name,
                       const std::string& value) {
  napi_value string = nullptr;
  return napi_create_string_utf8(env, value.data(), value.size(), &string) ==
             napi_ok &&
         napi_set_named_property(env, object, name, string) == napi_ok;
}

napi_value GetApplications(napi_env env, napi_callback_info info) {
  void* opaque = nullptr;
  size_t argc = 0;
  napi_get_cb_info(env, info, &argc, nullptr, nullptr, &opaque);
  auto* host = static_cast<Host*>(opaque);

  napi_deferred deferred = nullptr;
  napi_value promise = nullptr;
  if (napi_create_promise(env, &deferred, &promise) != napi_ok) {
    return nullptr;
  }
  if (!host || !host->launcher()) {
    RejectPromise(env, deferred, "launcher platform is not available");
    return promise;
  }

  LynxAppList* raw_list = nullptr;
  LynxError* rust_error = nullptr;
  const LynxStatus list_status =
      lynx_launcher_get_applications(host->launcher(), &raw_list, &rust_error);
  if (list_status != LYNX_STATUS_OK) {
    RejectPromise(env, deferred, TakeRustError(list_status, rust_error));
    return promise;
  }
  std::unique_ptr<LynxAppList, decltype(&lynx_app_list_destroy)> list(
      raw_list, lynx_app_list_destroy);
  const size_t length = lynx_app_list_len(list.get());
  napi_value applications = nullptr;
  if (napi_create_array_with_length(env, length, &applications) != napi_ok) {
    RejectPromise(env, deferred, "could not create the applications array");
    return promise;
  }

  for (size_t index = 0; index < length; ++index) {
    LynxApplicationView application{};
    rust_error = nullptr;
    const LynxStatus get_status =
        lynx_app_list_get(list.get(), index, &application, &rust_error);
    if (get_status != LYNX_STATUS_OK) {
      RejectPromise(env, deferred, TakeRustError(get_status, rust_error));
      return promise;
    }

    const std::string id = SliceString(application.id);
    const std::string name = SliceString(application.name);
    napi_value object = nullptr;
    if (napi_create_object(env, &object) != napi_ok ||
        !SetStringProperty(env, object, "id", id) ||
        !SetStringProperty(env, object, "name", name)) {
      RejectPromise(env, deferred,
                    "could not convert an application to JavaScript");
      return promise;
    }

    LynxIcon* raw_icon = nullptr;
    rust_error = nullptr;
    const LynxSlice id_slice{application.id.data, application.id.len};
    const LynxStatus icon_status = lynx_launcher_resolve_icon(
        host->launcher(), id_slice, 64, &raw_icon, &rust_error);
    if (icon_status != LYNX_STATUS_OK) {
      RejectPromise(env, deferred, TakeRustError(icon_status, rust_error));
      return promise;
    }
    std::unique_ptr<LynxIcon, decltype(&lynx_icon_destroy)> icon(
        raw_icon, lynx_icon_destroy);
    if (icon) {
      const std::string icon_path = SliceString(lynx_icon_path(icon.get()));
      if (!icon_path.empty() &&
          !SetStringProperty(env, object, "iconUri",
                             launcher_host::FileUri(icon_path))) {
        RejectPromise(env, deferred, "could not convert an icon URI");
        return promise;
      }
    }
    if (napi_set_element(env, applications, index, object) != napi_ok) {
      RejectPromise(env, deferred, "could not append an application");
      return promise;
    }
  }

  if (napi_resolve_deferred(env, deferred, applications) != napi_ok) {
    return nullptr;
  }
  std::cerr << "[host] Launcher.getApplications resolved " << length
            << " applications\n";
  return promise;
}

napi_value LaunchApplication(napi_env env, napi_callback_info info) {
  napi_value arguments[1]{};
  size_t argc = 1;
  void* opaque = nullptr;
  napi_get_cb_info(env, info, &argc, arguments, nullptr, &opaque);
  auto* host = static_cast<Host*>(opaque);

  napi_deferred deferred = nullptr;
  napi_value promise = nullptr;
  if (napi_create_promise(env, &deferred, &promise) != napi_ok) {
    return nullptr;
  }
  if (!host || !host->launcher()) {
    RejectPromise(env, deferred, "launcher platform is not available");
    return promise;
  }
  if (argc != 1) {
    RejectPromise(env, deferred, "launchApplication requires an id string");
    return promise;
  }

  size_t length = 0;
  if (napi_get_value_string_utf8(env, arguments[0], nullptr, 0, &length) !=
      napi_ok) {
    RejectPromise(env, deferred, "application id must be a string");
    return promise;
  }
  std::string id(length, '\0');
  size_t copied = 0;
  if (napi_get_value_string_utf8(env, arguments[0], id.data(), id.size() + 1,
                                 &copied) != napi_ok) {
    RejectPromise(env, deferred, "could not read application id");
    return promise;
  }
  id.resize(copied);
  const LynxSlice id_slice{reinterpret_cast<const uint8_t*>(id.data()),
                           id.size()};
  LynxError* rust_error = nullptr;
  const LynxStatus status =
      lynx_launcher_launch(host->launcher(), id_slice, &rust_error);
  if (status != LYNX_STATUS_OK) {
    RejectPromise(env, deferred, TakeRustError(status, rust_error));
    return promise;
  }

  napi_value undefined = nullptr;
  if (napi_get_undefined(env, &undefined) != napi_ok ||
      napi_resolve_deferred(env, deferred, undefined) != napi_ok) {
    return nullptr;
  }
  return promise;
}

napi_value LauncherModuleCreator(napi_env env, napi_value exports, const char*,
                                 void* opaque) {
  napi_value get_applications = nullptr;
  napi_value launch_application = nullptr;
  if (napi_create_function(env, "getApplications", NAPI_AUTO_LENGTH,
                           GetApplications, opaque,
                           &get_applications) != napi_ok ||
      napi_create_function(env, "launchApplication", NAPI_AUTO_LENGTH,
                           LaunchApplication, opaque,
                           &launch_application) != napi_ok ||
      napi_set_named_property(env, exports, "getApplications",
                              get_applications) != napi_ok ||
      napi_set_named_property(env, exports, "launchApplication",
                              launch_application) != napi_ok) {
    napi_throw_error(env, nullptr, "failed to initialize Launcher module");
  }
  return exports;
}

int CheckResources(const RuntimePaths& paths) {
  const auto icu = launcher_host::ReadFile(paths.icu);
  const auto core = launcher_host::ReadFile(paths.lynx_core);
  const auto bundle = launcher_host::ReadFile(paths.bundle);
  if (lynx_launcher_abi_version() != LYNX_LAUNCHER_ABI_VERSION) {
    throw std::runtime_error("Rust platform ABI version mismatch");
  }
  LynxLauncher* launcher = nullptr;
  LynxError* error = nullptr;
  LynxStatus status = lynx_launcher_create(&launcher, &error);
  if (status != LYNX_STATUS_OK) {
    throw std::runtime_error(TakeRustError(status, error));
  }
  LynxAppList* list = nullptr;
  status = lynx_launcher_get_applications(launcher, &list, &error);
  if (status != LYNX_STATUS_OK) {
    lynx_launcher_destroy(launcher);
    throw std::runtime_error(TakeRustError(status, error));
  }
  const size_t count = lynx_app_list_len(list);
  lynx_app_list_destroy(list);
  lynx_launcher_destroy(launcher);
  std::cout << "resource check passed: " << bundle.size() << " bundle bytes, "
            << core.size() << " lynx_core.js bytes, " << icu.size()
            << " ICU bytes, " << count << " applications\n";
  return 0;
}

void LynxLog(lynx_log_level_e level, const char* tag, const char* message) {
  static const char* names[] = {"VERBOSE", "DEBUG", "INFO",
                                "WARNING", "ERROR", "FATAL"};
  const int index = std::clamp(static_cast<int>(level), 0, 5);
  std::fprintf(stderr, "[lynx %s %s] %s\n", names[index], tag ? tag : "",
               message ? message : "");
}

void GlfwError(int code, const char* description) {
  std::fprintf(stderr, "[glfw %d] %s\n", code,
               description ? description : "unknown error");
}

}  // namespace

int main(int argc, char** argv) {
  try {
    const Options options = ParseOptions(argc, argv);
    const RuntimePaths paths = ResolveRuntimePaths(options, argv[0]);
    if (options.check_resources) {
      return CheckResources(paths);
    }
    if (!std::getenv("DISPLAY")) {
      throw std::runtime_error(
          "DISPLAY is not set; the GLFW host requires XWayland/X11");
    }

    lynx_log_init(LynxLog);
    lynx_log_set_minimum_level(LYNX_LOG_INFO);
    glfwSetErrorCallback(GlfwError);
    if (!glfwInit()) {
      throw std::runtime_error("GLFW initialization failed");
    }
    struct GlfwGuard {
      ~GlfwGuard() { glfwTerminate(); }
    } glfw_guard;

    glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);
    glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 3);
    glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);
    glfwWindowHint(GLFW_OPENGL_FORWARD_COMPAT, GLFW_TRUE);
    GLFWwindow* window = glfwCreateWindow(kInitialWidth, kInitialHeight,
                                          "Lynx Launcher", nullptr, nullptr);
    if (!window) {
      throw std::runtime_error(
          "could not create an X11 OpenGL 3.3 GLFW window");
    }
    struct WindowGuard {
      GLFWwindow* window;
      ~WindowGuard() { glfwDestroyWindow(window); }
    } window_guard{window};

    glfwMakeContextCurrent(window);
    glfwSwapInterval(1);
    int framebuffer_width = 0;
    int framebuffer_height = 0;
    glfwGetFramebufferSize(window, &framebuffer_width, &framebuffer_height);
    glViewport(0, 0, framebuffer_width, framebuffer_height);
    glClearColor(0.035f, 0.039f, 0.043f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);
    glfwSwapBuffers(window);
    std::cerr << "[host] OpenGL: "
              << reinterpret_cast<const char*>(glGetString(GL_VERSION))
              << " via GLFW X11\n";
    glfwMakeContextCurrent(nullptr);

    Host host(window, paths);
    host.Initialize();
    return host.Run(options);
  } catch (const std::exception& error) {
    std::cerr << "lynx-launcher: " << error.what() << '\n';
    return 1;
  }
}
