#define _POSIX_C_SOURCE 200809L
/* Test-only virtual pointer. Not linked into chda. */
#include <linux/input-event-codes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <wayland-client.h>
#include "wlr-virtual-pointer-unstable-v1-client-protocol.h"

static struct zwlr_virtual_pointer_manager_v1 *manager;
static uint32_t timestamp(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (uint32_t)(now.tv_sec * 1000 + now.tv_nsec / 1000000);
}
static void pause_ms(long milliseconds) {
    struct timespec interval = {milliseconds / 1000, milliseconds % 1000 * 1000000};
    nanosleep(&interval, NULL);
}
static void global(void *data, struct wl_registry *registry, uint32_t name,
                   const char *interface, uint32_t version) {
    (void)data;
    if (strcmp(interface, zwlr_virtual_pointer_manager_v1_interface.name) == 0)
        manager = wl_registry_bind(registry, name,
                    &zwlr_virtual_pointer_manager_v1_interface, version < 2 ? version : 2);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data; (void)registry; (void)name;
}
static const struct wl_registry_listener listener = {global, removed};
int main(int argc, char **argv) {
    if (argc != 7) {
        fprintf(stderr, "usage: wayland-drag x1 y1 x2 y2 output_width output_height\n");
        return 2;
    }
    if (!getenv("CHDA_NATIVE_ISOLATED") || strcmp(getenv("CHDA_NATIVE_ISOLATED"), "1"))
        return 2;
    long values[6];
    for (int i = 0; i < 6; i++) {
        char *end;
        values[i] = strtol(argv[i + 1], &end, 10);
        if (*end || values[i] < 0 || values[i] > 100000) return 2;
    }
    if (!values[4] || !values[5] || values[0] >= values[4] || values[2] >= values[4]
        || values[1] >= values[5] || values[3] >= values[5]) return 2;
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) return 1;
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !manager) return 1;
    struct zwlr_virtual_pointer_v1 *pointer =
        zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager, NULL);
    zwlr_virtual_pointer_v1_motion_absolute(pointer, timestamp(), values[0], values[1], values[4], values[5]);
    zwlr_virtual_pointer_v1_frame(pointer);
    wl_display_roundtrip(display);
    pause_ms(150);
    zwlr_virtual_pointer_v1_button(pointer, timestamp(), BTN_LEFT, WL_POINTER_BUTTON_STATE_PRESSED);
    zwlr_virtual_pointer_v1_frame(pointer);
    wl_display_roundtrip(display);
    pause_ms(100);
    for (int i = 1; i <= 20 && (values[0] != values[2] || values[1] != values[3]); i++) {
        long x = values[0] + (values[2] - values[0]) * i / 20;
        long y = values[1] + (values[3] - values[1]) * i / 20;
        zwlr_virtual_pointer_v1_motion_absolute(pointer, timestamp(), x, y, values[4], values[5]);
        zwlr_virtual_pointer_v1_frame(pointer);
        wl_display_roundtrip(display);
        pause_ms(20);
    }
    zwlr_virtual_pointer_v1_button(pointer, timestamp(), BTN_LEFT, WL_POINTER_BUTTON_STATE_RELEASED);
    zwlr_virtual_pointer_v1_frame(pointer);
    wl_display_roundtrip(display);
    zwlr_virtual_pointer_v1_destroy(pointer);
    zwlr_virtual_pointer_manager_v1_destroy(manager);
    wl_registry_destroy(registry);
    wl_display_disconnect(display);
    return 0;
}
