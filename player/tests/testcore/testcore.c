/* A tiny libretro core for decomp-dj's tests: 64 bytes of "RAM" that change each frame
 * with the held buttons, save states, and the comparison-core API (dj_sync, dj_write)
 * from docs/connectors.md. Built by player/tests/compare.rs; -DNO_DJ leaves the
 * comparison API out.
 *
 * The core option `testcore_sync` makes dj_sync succeed (ok), decline (fail) or report
 * an error (error). */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define RETRO_API_VERSION 1
#define RETRO_ENVIRONMENT_GET_VARIABLE 15
#define RETRO_ENVIRONMENT_SET_VARIABLES 16
#define RETRO_ENVIRONMENT_SET_SUPPORT_NO_GAME 18
#define RETRO_DEVICE_JOYPAD 1
#define RETRO_MEMORY_SYSTEM_RAM 2

struct retro_variable { const char *key; const char *value; };
struct retro_game_info { const char *path; const void *data; size_t size; const char *meta; };
struct retro_system_info { const char *library_name; const char *library_version; const char *valid_extensions; bool need_fullpath; bool block_extract; };
struct retro_game_geometry { unsigned base_width, base_height, max_width, max_height; float aspect_ratio; };
struct retro_system_timing { double fps, sample_rate; };
struct retro_system_av_info { struct retro_game_geometry geometry; struct retro_system_timing timing; };

typedef bool (*env_t)(unsigned, void *);
typedef void (*video_t)(const void *, unsigned, unsigned, size_t);
typedef void (*audio_t)(int16_t, int16_t);
typedef size_t (*audio_batch_t)(const int16_t *, size_t);
typedef void (*poll_t)(void);
typedef int16_t (*state_t)(unsigned, unsigned, unsigned, unsigned);

static env_t env;
static video_t video;
static poll_t poll_cb;
static state_t input;
static uint8_t ram[64];
static uint16_t pixels[4 * 4];

void retro_set_environment(env_t cb) {
    static const struct retro_variable vars[] = {{"testcore_sync", "Sync result; ok|fail|error"}, {NULL, NULL}};
    bool yes = true;
    env = cb;
    cb(RETRO_ENVIRONMENT_SET_VARIABLES, (void *)vars);
    cb(RETRO_ENVIRONMENT_SET_SUPPORT_NO_GAME, &yes);
}
void retro_set_video_refresh(video_t cb) { video = cb; }
void retro_set_audio_sample(audio_t cb) { (void)cb; }
void retro_set_audio_sample_batch(audio_batch_t cb) { (void)cb; }
void retro_set_input_poll(poll_t cb) { poll_cb = cb; }
void retro_set_input_state(state_t cb) { input = cb; }
void retro_init(void) { memset(ram, 0, sizeof ram); }
void retro_deinit(void) {}
unsigned retro_api_version(void) { return RETRO_API_VERSION; }
void retro_get_system_info(struct retro_system_info *info) {
    memset(info, 0, sizeof *info);
    info->library_name = "testcore";
    info->library_version = "1";
    info->valid_extensions = "bin";
}
void retro_get_system_av_info(struct retro_system_av_info *info) {
    memset(info, 0, sizeof *info);
    info->geometry.base_width = info->geometry.max_width = 4;
    info->geometry.base_height = info->geometry.max_height = 4;
    info->timing.fps = 60.0;
}
void retro_set_controller_port_device(unsigned port, unsigned device) { (void)port; (void)device; }
void retro_reset(void) { memset(ram, 0, sizeof ram); }
void retro_run(void) {
    if (poll_cb) poll_cb();
    uint8_t held = 0;
    for (unsigned b = 0; b < 8; b++)
        if (input && input(0, RETRO_DEVICE_JOYPAD, 0, b)) held |= (uint8_t)(1u << b);
    ram[0]++;
    ram[1] = held;
    ram[2] = (uint8_t)(ram[2] * 31 + held + 1);
    /* Bright, and different from frame to frame, so two pictures out of step are easy to see. */
    for (int i = 0; i < 16; i++) pixels[i] = (uint16_t)((ram[0] * 2654435761u + ram[2] * 40503u + i * 9973u) & 0x7fff);
    if (video) video(pixels, 4, 4, 8);
}
size_t retro_serialize_size(void) { return sizeof ram; }
bool retro_serialize(void *data, size_t size) {
    if (size < sizeof ram) return false;
    memcpy(data, ram, sizeof ram);
    return true;
}
bool retro_unserialize(const void *data, size_t size) {
    if (size < sizeof ram) return false;
    memcpy(ram, data, sizeof ram);
    return true;
}
void retro_cheat_reset(void) {}
void retro_cheat_set(unsigned i, bool e, const char *c) { (void)i; (void)e; (void)c; }
bool retro_load_game(const struct retro_game_info *game) { (void)game; return true; }
bool retro_load_game_special(unsigned t, const struct retro_game_info *g, size_t n) { (void)t; (void)g; (void)n; return false; }
void retro_unload_game(void) {}
unsigned retro_get_region(void) { return 0; }
void *retro_get_memory_data(unsigned id) { return id == RETRO_MEMORY_SYSTEM_RAM ? ram : NULL; }
size_t retro_get_memory_size(unsigned id) { return id == RETRO_MEMORY_SYSTEM_RAM ? sizeof ram : 0; }

#ifndef NO_DJ
struct dj_baseline { uint32_t version; void *ctx; void *(*get_proc)(void *ctx, const char *name); };
#define DJ_SYNC_BOOT 1u

static const char *sync_setting(void) {
    struct retro_variable v = {"testcore_sync", NULL};
    if (env && env(RETRO_ENVIRONMENT_GET_VARIABLE, &v) && v.value) return v.value;
    return "ok";
}

/* Copies the baseline's RAM (the baseline is another testcore). ram[63] counts boots. */
int dj_sync(const struct dj_baseline *b, uint64_t frame, uint32_t flags, char *reason, size_t len) {
    const char *s = sync_setting();
    if (strcmp(s, "fail") == 0) { snprintf(reason, len, "told to fail at frame %llu", (unsigned long long)frame); return 1; }
    if (strcmp(s, "error") == 0) { snprintf(reason, len, "told to report an error"); return 2; }
    void *(*mem)(unsigned) = (void *(*)(unsigned))b->get_proc(b->ctx, "retro_get_memory_data");
    size_t (*size)(unsigned) = (size_t (*)(unsigned))b->get_proc(b->ctx, "retro_get_memory_size");
    if (!mem || !size || size(RETRO_MEMORY_SYSTEM_RAM) != sizeof ram) { snprintf(reason, len, "the baseline isn't a testcore"); return 1; }
    uint8_t boots = ram[63];
    memcpy(ram, mem(RETRO_MEMORY_SYSTEM_RAM), sizeof ram);
    ram[63] = (uint8_t)(boots + ((flags & DJ_SYNC_BOOT) ? 1 : 0));
    return 0;
}

/* Parity: the first 63 bytes of RAM match the baseline's (ram[63] counts boots). */
int dj_check_parity(const struct dj_baseline *b, uint64_t frame, char *reason, size_t len) {
    void *(*mem)(unsigned) = (void *(*)(unsigned))b->get_proc(b->ctx, "retro_get_memory_data");
    if (!mem) { snprintf(reason, len, "no baseline RAM"); return 2; }
    const uint8_t *base = mem(RETRO_MEMORY_SYSTEM_RAM);
    for (unsigned i = 0; i < 63; i++)
        if (base[i] != ram[i]) {
            snprintf(reason, len, "RAM 0x%02X is 0x%02X here, 0x%02X in the baseline (frame %llu)", i, ram[i], base[i], (unsigned long long)frame);
            return 1;
        }
    return 0;
}

/* The core option `testcore_break` makes the frame where ram[0] becomes 10 go wrong:
 * "mismatch" changes RAM there (a parity failure), "tick" fails the tick with a reason. */
static const char *break_setting(void) {
    struct retro_variable v = {"testcore_break", NULL};
    if (env && env(RETRO_ENVIRONMENT_GET_VARIABLE, &v) && v.value) return v.value;
    return "off";
}
int dj_run(char *reason, size_t len) {
    retro_run();
    if (ram[0] != 10) return 0;
    const char *s = break_setting();
    if (strcmp(s, "mismatch") == 0) ram[2] ^= 0x55;
    if (strcmp(s, "tick") == 0) { snprintf(reason, len, "told to fail the tick"); return 1; }
    return 0;
}

int dj_write(const struct dj_baseline *b, const char *region, uint64_t address, const uint8_t *bytes, size_t n, char *reason, size_t len) {
    (void)b;
    if (strcmp(region, "RAM") != 0) return 1;
    if (address + n > sizeof ram) { snprintf(reason, len, "past the end"); return 2; }
    memcpy(ram + address, bytes, n);
    return 0;
}
#endif
