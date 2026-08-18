#ifndef LYNX_LAUNCHER_H_
#define LYNX_LAUNCHER_H_

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LYNX_LAUNCHER_ABI_VERSION 1u

typedef uint32_t LynxStatus;
#define LYNX_STATUS_OK ((LynxStatus)0u)
#define LYNX_STATUS_INVALID_ARGUMENT ((LynxStatus)1u)
#define LYNX_STATUS_NOT_FOUND ((LynxStatus)2u)
#define LYNX_STATUS_PLATFORM_ERROR ((LynxStatus)3u)
#define LYNX_STATUS_PANIC ((LynxStatus)4u)

typedef struct LynxLauncher LynxLauncher;
typedef struct LynxAppList LynxAppList;
typedef struct LynxIcon LynxIcon;
typedef struct LynxError LynxError;

/* UTF-8 bytes. Slices are not NUL-terminated. A zero-length slice may be NULL. */
typedef struct LynxSlice {
  const uint8_t *data;
  size_t len;
} LynxSlice;

/* Every slice in this view is borrowed from the LynxAppList that produced it. */
typedef struct LynxApplicationView {
  LynxSlice id;
  LynxSlice name;
  LynxSlice icon;
} LynxApplicationView;

uint32_t lynx_launcher_abi_version(void);

/* out_error is optional. Status-returning calls set it to NULL before work. */
LynxStatus lynx_launcher_create(LynxLauncher **out_launcher,
                                LynxError **out_error);
void lynx_launcher_destroy(LynxLauncher *launcher);

/* The returned list is a snapshot and remains valid after launcher is destroyed. */
LynxStatus lynx_launcher_get_applications(const LynxLauncher *launcher,
                                          LynxAppList **out_list,
                                          LynxError **out_error);
size_t lynx_app_list_len(const LynxAppList *list);
LynxStatus lynx_app_list_get(const LynxAppList *list, size_t index,
                             LynxApplicationView *out_application,
                             LynxError **out_error);
void lynx_app_list_destroy(LynxAppList *list);

/* A missing icon is success with *out_icon == NULL. size == 0 requests 48 px. */
LynxStatus lynx_launcher_resolve_icon(const LynxLauncher *launcher,
                                      LynxSlice application_id, uint32_t size,
                                      LynxIcon **out_icon,
                                      LynxError **out_error);
LynxSlice lynx_icon_path(const LynxIcon *icon);
void lynx_icon_destroy(LynxIcon *icon);

/* Launch is asynchronous. application_id must be valid UTF-8. */
LynxStatus lynx_launcher_launch(const LynxLauncher *launcher,
                                LynxSlice application_id,
                                LynxError **out_error);

/* The returned message is borrowed from error and is not NUL-terminated. */
LynxSlice lynx_error_message(const LynxError *error);
void lynx_error_destroy(LynxError *error);

/*
 * Handle ownership rules:
 * - Destroy every non-NULL launcher, list, icon, and error exactly once with its
 *   matching destroy function. Destroy functions accept NULL.
 * - Do not reuse an output slot containing a live handle; calls overwrite it.
 * - Do not retain borrowed slices after destroying their owning list/icon/error.
 * - Handles may be read concurrently, but must not be destroyed concurrently
 *   with another call that uses them.
 * - All non-NULL pointers and input slice ranges must point to valid memory for
 *   the duration of the call. Rust panics are contained and reported as
 *   LYNX_STATUS_PANIC where a status is available.
 */

#ifdef __cplusplus
}  /* extern "C" */
#endif

#endif  /* LYNX_LAUNCHER_H_ */
