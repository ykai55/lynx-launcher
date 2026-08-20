#include <cstddef>
#include <type_traits>

#include "capi/lynx_generic_resource_fetcher_capi.h"
#include "capi/lynx_load_meta_capi.h"
#include "capi/lynx_log_capi.h"
#include "capi/lynx_resource_request_capi.h"
#include "capi/lynx_resource_response_capi.h"
#include "capi/lynx_view_builder_capi.h"
#include "capi/lynx_view_capi.h"
#include "capi/lynx_view_client_capi.h"
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
static_assert(kLynxResourceTypeLynxCoreJS == 7);

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

static_assert(std::is_same_v<
              decltype(&lynx_generic_resource_fetcher_create_with_finalizer),
              lynx_generic_resource_fetcher_t* (*)(
                  void*, void (*)(lynx_generic_resource_fetcher_t*, void*))>);
static_assert(std::is_same_v<
              decltype(&lynx_generic_resource_fetcher_get_user_data),
              void* (*)(lynx_generic_resource_fetcher_t*)>);
static_assert(std::is_same_v<
              decltype(&lynx_generic_resource_fetcher_bind_fetch_resource),
              void (*)(lynx_generic_resource_fetcher_t*,
                       fetch_resource_func)>);
static_assert(std::is_same_v<
              decltype(&lynx_generic_resource_fetcher_bind_fetch_resource_path),
              void (*)(lynx_generic_resource_fetcher_t*,
                       fetch_resource_func)>);
static_assert(std::is_same_v<
              decltype(&lynx_generic_resource_fetcher_release),
              void (*)(lynx_generic_resource_fetcher_t*)>);
static_assert(std::is_same_v<decltype(&lynx_resource_request_get_type),
                             lynx_resource_type_e (*)(
                                 lynx_resource_request_t*)>);
static_assert(std::is_same_v<decltype(&lynx_resource_request_get_url),
                             const char* (*)(lynx_resource_request_t*)>);
static_assert(std::is_same_v<decltype(&lynx_resource_request_release),
                             void (*)(lynx_resource_request_t*)>);
static_assert(std::is_same_v<decltype(&lynx_resource_response_set_code),
                             void (*)(lynx_resource_response_t*, int)>);
static_assert(std::is_same_v<
              decltype(&lynx_resource_response_set_error_message),
              void (*)(lynx_resource_response_t*, const char*)>);
static_assert(std::is_same_v<
              decltype(&lynx_resource_response_set_data),
              void (*)(lynx_resource_response_t*, uint8_t*, size_t,
                       void (*)(uint8_t*, size_t, void*), void*)>);
static_assert(std::is_same_v<decltype(&lynx_resource_response_callback),
                             void (*)(lynx_resource_response_t*)>);
static_assert(std::is_same_v<decltype(&lynx_resource_response_release),
                             void (*)(lynx_resource_response_t*)>);

static_assert(std::is_same_v<decltype(&lynx_view_builder_create),
                             lynx_view_builder_t* (*)()>);
static_assert(std::is_same_v<
              decltype(&lynx_view_builder_set_enable_js_runtime),
              void (*)(lynx_view_builder_t*, bool)>);
static_assert(std::is_same_v<decltype(&lynx_view_builder_set_icu_data_path),
                             void (*)(lynx_view_builder_t*, const char*)>);
static_assert(std::is_same_v<
              decltype(&lynx_view_builder_set_windowless_renderer),
              void (*)(lynx_view_builder_t*, lynx_windowless_renderer_t*)>);
static_assert(std::is_same_v<
              decltype(&lynx_view_builder_set_generic_resource_fetcher),
              void (*)(lynx_view_builder_t*,
                       lynx_generic_resource_fetcher_t*)>);
static_assert(std::is_same_v<
              decltype(&lynx_view_builder_register_native_module),
              void (*)(lynx_view_builder_t*, const char*, napi_module_creator,
                       void*)>);
static_assert(std::is_same_v<decltype(&lynx_view_builder_release),
                             void (*)(lynx_view_builder_t*)>);
static_assert(std::is_same_v<decltype(&lynx_view_create),
                             lynx_view_t* (*)(lynx_view_builder_t*, void*)>);
static_assert(std::is_same_v<decltype(&lynx_view_add_client),
                             void (*)(lynx_view_t*, lynx_view_client_t*)>);
static_assert(std::is_same_v<decltype(&lynx_view_enter_foreground),
                             void (*)(lynx_view_t*)>);
static_assert(std::is_same_v<decltype(&lynx_view_enter_background),
                             void (*)(lynx_view_t*)>);
static_assert(std::is_same_v<decltype(&lynx_view_load_template),
                             void (*)(lynx_view_t*, lynx_load_meta_t*)>);
static_assert(std::is_same_v<decltype(&lynx_view_release),
                             void (*)(lynx_view_t*)>);
static_assert(std::is_same_v<decltype(&lynx_view_client_create),
                             lynx_view_client_t* (*)(void*)>);
static_assert(std::is_same_v<decltype(&lynx_view_client_get_user_data),
                             void* (*)(lynx_view_client_t*)>);
static_assert(std::is_same_v<decltype(&lynx_view_client_bind_on_first_screen),
                             void (*)(lynx_view_client_t*, on_first_screen)>);
static_assert(std::is_same_v<
              decltype(&lynx_view_client_bind_on_received_error),
              void (*)(lynx_view_client_t*, on_received_error)>);
static_assert(std::is_same_v<decltype(&lynx_view_client_release),
                             void (*)(lynx_view_client_t*)>);
static_assert(std::is_same_v<decltype(&lynx_load_meta_create),
                             lynx_load_meta_t* (*)()>);
static_assert(std::is_same_v<decltype(&lynx_load_meta_set_url),
                             void (*)(lynx_load_meta_t*, const char*)>);
static_assert(std::is_same_v<
              decltype(&lynx_load_meta_set_binary_data),
              void (*)(lynx_load_meta_t*, uint8_t*, size_t,
                       void (*)(uint8_t*, size_t, void*), void*)>);
static_assert(std::is_same_v<decltype(&lynx_load_meta_release),
                             void (*)(lynx_load_meta_t*)>);

static_assert(std::is_same_v<decltype(&napi_get_undefined_weak),
                             napi_status_weak (*)(napi_env_weak,
                                                  napi_value_weak*)>);
static_assert(std::is_same_v<decltype(&napi_create_promise_weak),
                             napi_status_weak (*)(napi_env_weak,
                                                  napi_deferred_weak*,
                                                  napi_value_weak*)>);
static_assert(std::is_same_v<decltype(&napi_resolve_deferred_weak),
                             napi_status_weak (*)(napi_env_weak,
                                                  napi_deferred_weak,
                                                  napi_value_weak)>);
static_assert(std::is_same_v<decltype(&napi_reject_deferred_weak),
                             napi_status_weak (*)(napi_env_weak,
                                                  napi_deferred_weak,
                                                  napi_value_weak)>);

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
auto volatile kCreateFetcher =
    &lynx_generic_resource_fetcher_create_with_finalizer;
auto volatile kGetFetcherUserData =
    &lynx_generic_resource_fetcher_get_user_data;
auto volatile kBindFetch = &lynx_generic_resource_fetcher_bind_fetch_resource;
auto volatile kBindFetchPath =
    &lynx_generic_resource_fetcher_bind_fetch_resource_path;
auto volatile kReleaseFetcher = &lynx_generic_resource_fetcher_release;
auto volatile kGetRequestType = &lynx_resource_request_get_type;
auto volatile kGetRequestUrl = &lynx_resource_request_get_url;
auto volatile kReleaseRequest = &lynx_resource_request_release;
auto volatile kSetResponseCode = &lynx_resource_response_set_code;
auto volatile kSetResponseError =
    &lynx_resource_response_set_error_message;
auto volatile kSetResponseData = &lynx_resource_response_set_data;
auto volatile kRespond = &lynx_resource_response_callback;
auto volatile kReleaseResponse = &lynx_resource_response_release;
auto volatile kCreateBuilder = &lynx_view_builder_create;
auto volatile kEnableJsRuntime = &lynx_view_builder_set_enable_js_runtime;
auto volatile kSetIcuPath = &lynx_view_builder_set_icu_data_path;
auto volatile kSetRenderer = &lynx_view_builder_set_windowless_renderer;
auto volatile kSetFetcher = &lynx_view_builder_set_generic_resource_fetcher;
auto volatile kRegisterNativeModule =
    &lynx_view_builder_register_native_module;
auto volatile kReleaseBuilder = &lynx_view_builder_release;
auto volatile kCreateView = &lynx_view_create;
auto volatile kAddClient = &lynx_view_add_client;
auto volatile kEnterForeground = &lynx_view_enter_foreground;
auto volatile kEnterBackground = &lynx_view_enter_background;
auto volatile kLoadTemplate = &lynx_view_load_template;
auto volatile kReleaseView = &lynx_view_release;
auto volatile kCreateClient = &lynx_view_client_create;
auto volatile kGetClientUserData = &lynx_view_client_get_user_data;
auto volatile kBindFirstScreen = &lynx_view_client_bind_on_first_screen;
auto volatile kBindReceivedError = &lynx_view_client_bind_on_received_error;
auto volatile kReleaseClient = &lynx_view_client_release;
auto volatile kCreateLoadMeta = &lynx_load_meta_create;
auto volatile kSetLoadUrl = &lynx_load_meta_set_url;
auto volatile kSetLoadData = &lynx_load_meta_set_binary_data;
auto volatile kReleaseLoadMeta = &lynx_load_meta_release;
auto volatile kNapiGetUndefined = &napi_get_undefined_weak;
auto volatile kNapiCreateObject = &napi_create_object_weak;
auto volatile kNapiCreateArray = &napi_create_array_with_length_weak;
auto volatile kNapiCreateString = &napi_create_string_utf8_weak;
auto volatile kNapiCreateFunction = &napi_create_function_weak;
auto volatile kNapiCreateError = &napi_create_error_weak;
auto volatile kNapiSetNamedProperty = &napi_set_named_property_weak;
auto volatile kNapiSetElement = &napi_set_element_weak;
auto volatile kNapiGetCallbackInfo = &napi_get_cb_info_weak;
auto volatile kNapiThrowError = &napi_throw_error_weak;
auto volatile kNapiCreatePromise = &napi_create_promise_weak;
auto volatile kNapiResolveDeferred = &napi_resolve_deferred_weak;
auto volatile kNapiRejectDeferred = &napi_reject_deferred_weak;
auto volatile kSetScreenSize = &lynx_sys_view_builder_set_screen_size;
auto volatile kSetBuilderFrame = &lynx_sys_view_builder_set_frame;
auto volatile kSetFontScale = &lynx_sys_view_builder_set_font_scale;
auto volatile kUpdateScreenMetrics = &lynx_sys_view_update_screen_metrics;
auto volatile kSetViewFrame = &lynx_sys_view_set_frame;

int main() { return lynx_sys_c_header_smoke() ? 0 : 1; }
