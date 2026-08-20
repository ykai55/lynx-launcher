#include "lynx_sys_shim.h"

#include "capi/lynx_view_builder_capi.h"
#include "capi/lynx_view_capi.h"

extern "C" void lynx_sys_view_builder_set_screen_size(
    lynx_view_builder_t* builder, float width, float height,
    float pixel_ratio) {
  lynx_view_builder_set_screen_size(builder, width, height, pixel_ratio);
}

extern "C" void lynx_sys_view_builder_set_frame(lynx_view_builder_t* builder,
                                                  float x, float y,
                                                  float width, float height) {
  lynx_view_builder_set_frame(builder, x, y, width, height);
}

extern "C" void lynx_sys_view_builder_set_font_scale(
    lynx_view_builder_t* builder, float scale) {
  lynx_view_builder_set_font_scale(builder, scale);
}

extern "C" void lynx_sys_view_update_screen_metrics(
    lynx_view_t* view, float width, float height, float pixel_ratio) {
  lynx_view_update_screen_metrics(view, width, height, pixel_ratio);
}

extern "C" void lynx_sys_view_set_frame(lynx_view_t* view, float x, float y,
                                          float width, float height) {
  lynx_view_set_frame(view, x, y, width, height);
}
