#include <cstddef>
#include <type_traits>

#include "capi/lynx_log_capi.h"
#include "capi/lynx_windowless_renderer_capi.h"
#include "lynx_sys_shim.h"

extern "C" int lynx_sys_c_header_smoke(void);

static_assert(sizeof(lynx_task_t) == 16);
static_assert(alignof(lynx_task_t) == 8);
static_assert(offsetof(lynx_task_t, runner) == 0);
static_assert(offsetof(lynx_task_t, task) == 8);

static_assert(sizeof(lynx_windowless_ui_task_runner_config_t) == 32);
static_assert(alignof(lynx_windowless_ui_task_runner_config_t) == 8);
static_assert(
    offsetof(lynx_windowless_ui_task_runner_config_t, struct_size) == 0);
static_assert(offsetof(lynx_windowless_ui_task_runner_config_t, user_data) == 8);
static_assert(offsetof(lynx_windowless_ui_task_runner_config_t,
                       runs_on_current_thread_callback) == 16);
static_assert(offsetof(lynx_windowless_ui_task_runner_config_t,
                       post_task_callback) == 24);

static_assert(sizeof(lynx_log_level_e) == sizeof(int));
static_assert(sizeof(lynx_windowless_renderer_type_e) == sizeof(int));
static_assert(LYNX_LOG_INFO == 2);
static_assert(kRendererTypeGLDirect == 2);

static_assert(std::is_same_v<lynx_log_callback_t,
                             void (*)(lynx_log_level_e, const char*,
                                      const char*)>);
static_assert(std::is_same_v<
              decltype(lynx_windowless_ui_task_runner_config_t::
                           runs_on_current_thread_callback),
              bool (*)(void*)>);
static_assert(std::is_same_v<
              decltype(lynx_windowless_ui_task_runner_config_t::
                           post_task_callback),
              void (*)(lynx_task_t, uint64_t, void*)>);
static_assert(std::is_same_v<on_gl_make_current,
                             bool (*)(lynx_windowless_renderer_t*)>);
static_assert(std::is_same_v<on_gl_clear_current,
                             bool (*)(lynx_windowless_renderer_t*)>);
static_assert(std::is_same_v<on_gl_present,
                             bool (*)(lynx_windowless_renderer_t*)>);
static_assert(std::is_same_v<on_gl_create_fbo,
                             uint32_t (*)(lynx_windowless_renderer_t*, int,
                                          int)>);
static_assert(std::is_same_v<on_gl_proc_resolver,
                             void* (*)(lynx_windowless_renderer_t*,
                                       const char*)>);
static_assert(std::is_same_v<on_post_task,
                             void (*)(lynx_windowless_renderer_t*, lynx_task_t,
                                      uint64_t)>);

static_assert(std::is_same_v<decltype(&lynx_log_init),
                             void (*)(lynx_log_callback_t)>);
static_assert(std::is_same_v<decltype(&lynx_log_set_minimum_level),
                             void (*)(lynx_log_level_e)>);
static_assert(std::is_same_v<
              decltype(&lynx_windowless_set_global_ui_task_runner),
              bool (*)(const lynx_windowless_ui_task_runner_config_t*)>);
static_assert(std::is_same_v<decltype(&lynx_windowless_run_ui_task),
                             bool (*)(lynx_task_t)>);
static_assert(std::is_same_v<
              decltype(&lynx_windowless_renderer_create_with_finalizer),
              lynx_windowless_renderer_t* (*)(
                  lynx_windowless_renderer_type_e, void*,
                  void (*)(lynx_windowless_renderer_t*, void*))>);
static_assert(std::is_same_v<
              decltype(&lynx_windowless_renderer_get_user_data),
              void* (*)(lynx_windowless_renderer_t*)>);
static_assert(std::is_same_v<
              decltype(&lynx_windowless_renderer_bind_on_gl_make_current),
              void (*)(lynx_windowless_renderer_t*, on_gl_make_current)>);
static_assert(std::is_same_v<
              decltype(&lynx_windowless_renderer_bind_on_gl_clear_current),
              void (*)(lynx_windowless_renderer_t*, on_gl_clear_current)>);
static_assert(std::is_same_v<
              decltype(&lynx_windowless_renderer_bind_on_gl_present),
              void (*)(lynx_windowless_renderer_t*, on_gl_present)>);
static_assert(std::is_same_v<
              decltype(&lynx_windowless_renderer_bind_on_gl_create_fbo),
              void (*)(lynx_windowless_renderer_t*, on_gl_create_fbo)>);
static_assert(std::is_same_v<
              decltype(&lynx_windowless_renderer_bind_on_gl_proc_resolver),
              void (*)(lynx_windowless_renderer_t*, on_gl_proc_resolver)>);
static_assert(std::is_same_v<
              decltype(&lynx_windowless_renderer_bind_on_post_task),
              void (*)(lynx_windowless_renderer_t*, on_post_task)>);
static_assert(std::is_same_v<decltype(&lynx_windowless_renderer_run_task),
                             void (*)(lynx_windowless_renderer_t*,
                                      lynx_task_t)>);
static_assert(std::is_same_v<decltype(&lynx_windowless_renderer_release),
                             void (*)(lynx_windowless_renderer_t*)>);

static_assert(std::is_same_v<decltype(&lynx_sys_view_builder_set_screen_size),
                             void (*)(lynx_view_builder_t*, float, float,
                                      float)>);
static_assert(std::is_same_v<decltype(&lynx_sys_view_builder_set_frame),
                             void (*)(lynx_view_builder_t*, float, float, float,
                                      float)>);
static_assert(std::is_same_v<decltype(&lynx_sys_view_builder_set_font_scale),
                             void (*)(lynx_view_builder_t*, float)>);
static_assert(std::is_same_v<decltype(&lynx_sys_view_update_screen_metrics),
                             void (*)(lynx_view_t*, float, float, float)>);
static_assert(std::is_same_v<decltype(&lynx_sys_view_set_frame),
                             void (*)(lynx_view_t*, float, float, float,
                                      float)>);

auto volatile kLogInit = &lynx_log_init;
auto volatile kLogSetMinimumLevel = &lynx_log_set_minimum_level;
auto volatile kSetGlobalUiTaskRunner =
    &lynx_windowless_set_global_ui_task_runner;
auto volatile kRunUiTask = &lynx_windowless_run_ui_task;
auto volatile kCreateRenderer =
    &lynx_windowless_renderer_create_with_finalizer;
auto volatile kGetRendererUserData =
    &lynx_windowless_renderer_get_user_data;
auto volatile kBindMakeCurrent =
    &lynx_windowless_renderer_bind_on_gl_make_current;
auto volatile kBindClearCurrent =
    &lynx_windowless_renderer_bind_on_gl_clear_current;
auto volatile kBindPresent = &lynx_windowless_renderer_bind_on_gl_present;
auto volatile kBindCreateFbo =
    &lynx_windowless_renderer_bind_on_gl_create_fbo;
auto volatile kBindProcResolver =
    &lynx_windowless_renderer_bind_on_gl_proc_resolver;
auto volatile kBindPostTask = &lynx_windowless_renderer_bind_on_post_task;
auto volatile kRunRendererTask = &lynx_windowless_renderer_run_task;
auto volatile kReleaseRenderer = &lynx_windowless_renderer_release;
auto volatile kSetScreenSize = &lynx_sys_view_builder_set_screen_size;
auto volatile kSetBuilderFrame = &lynx_sys_view_builder_set_frame;
auto volatile kSetFontScale = &lynx_sys_view_builder_set_font_scale;
auto volatile kUpdateScreenMetrics = &lynx_sys_view_update_screen_metrics;
auto volatile kSetViewFrame = &lynx_sys_view_set_frame;

int main() { return lynx_sys_c_header_smoke() ? 0 : 1; }
