#include "lynx_sys_shim.h"

int lynx_sys_c_header_smoke(void) {
  void (*volatile set_screen_size)(lynx_view_builder_t*, float, float, float) =
      lynx_sys_view_builder_set_screen_size;
  void (*volatile set_builder_frame)(lynx_view_builder_t*, float, float, float,
                                     float) = lynx_sys_view_builder_set_frame;
  void (*volatile set_font_scale)(lynx_view_builder_t*, float) =
      lynx_sys_view_builder_set_font_scale;
  void (*volatile update_screen_metrics)(lynx_view_t*, float, float, float) =
      lynx_sys_view_update_screen_metrics;
  void (*volatile set_view_frame)(lynx_view_t*, float, float, float, float) =
      lynx_sys_view_set_frame;

  return set_screen_size != 0 && set_builder_frame != 0 &&
         set_font_scale != 0 && update_screen_metrics != 0 &&
         set_view_frame != 0;
}
