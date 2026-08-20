#ifndef LYNX_SYS_SHIM_H_
#define LYNX_SYS_SHIM_H_

#ifdef __cplusplus
extern "C" {
#endif

typedef struct lynx_view_builder_t lynx_view_builder_t;
typedef struct lynx_view_t lynx_view_t;

void lynx_sys_view_builder_set_screen_size(lynx_view_builder_t* builder,
                                           float width, float height,
                                           float pixel_ratio);
void lynx_sys_view_builder_set_frame(lynx_view_builder_t* builder, float x,
                                     float y, float width, float height);
void lynx_sys_view_builder_set_font_scale(lynx_view_builder_t* builder,
                                          float scale);
void lynx_sys_view_update_screen_metrics(lynx_view_t* view, float width,
                                         float height, float pixel_ratio);
void lynx_sys_view_set_frame(lynx_view_t* view, float x, float y, float width,
                             float height);

#ifdef __cplusplus
}
#endif

#endif  // LYNX_SYS_SHIM_H_
