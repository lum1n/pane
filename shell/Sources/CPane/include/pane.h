/* Pane C ABI. Generated from crates/pane-ffi/src/lib.rs; keep in sync
 * (`make header` regenerates it with cbindgen when installed).
 * All strings are UTF-8 JSON. Strings returned by Pane must be released
 * with pane_free. */
#ifndef PANE_H
#define PANE_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Called on Pane's core thread; hop to the main thread before touching UI. */
typedef void (*PaneCallback)(void *ctx, const char *json);

/* Start Pane. Call once, on the main thread. support_dir may be NULL. */
bool pane_start(const char *support_dir, PaneCallback on_snapshot, PaneCallback on_request, void *ctx);

/* Error from pane_start, or NULL. Free with pane_free. */
char *pane_last_error(void);

/* Run a command ({"cmd": ...}) and return the reply ({"status": ...}). Blocks up to 5 s. */
char *pane_command(const char *json);

/* Shell events: app_launched, app_terminated, screens_changed (main thread),
 * will_sleep, did_wake, app_open_result. */
bool pane_event(const char *json);

/* Unpark all windows, persist, stop. Bounded to about 3 s. */
bool pane_shutdown(void);

void pane_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* PANE_H */
