/* The libretro API, and plain play: one step a retro_run.
 *
 * Under dj the decomp is driven by events instead (events.c), and dj doesn't call
 * retro_run on it. */
#include <stdio.h>
#include <string.h>

#include "compare.h"
#include "core.h"
#include "gl.h"
#include "libretro.h"

struct game thw_game;

static retro_environment_t env;
static retro_video_refresh_t video;
static retro_audio_sample_batch_t audio_batch;
static retro_input_poll_t input_poll;
static retro_input_state_t input_state;
static retro_log_printf_t log_cb;
static struct retro_hw_render_callback hw;
static bool gl_ready;
static uint8_t ram_view[2048];
static double audio_owed;

#define FPS 60.0988138974405  /* NTSC NES */
#define RATE 48000.0

static void core_log(enum retro_log_level level, const char *msg) {
    if (log_cb) log_cb(level, "[thwaite-decomp] %s\n", msg);
    else fprintf(stderr, "[thwaite-decomp] %s\n", msg);
}

const uint8_t *core_ram_view(void) { return ram_view; }

static void refresh_ram_view(void) { compare_export(&thw_game, ram_view); }

/* ---- Video ------------------------------------------------------------------------------ */

static void context_reset(void) {
    gl_ready = gl_init((gl_get_proc)hw.get_proc_address);
    if (!gl_ready) core_log(RETRO_LOG_ERROR, gl_error() ? gl_error() : "OpenGL setup failed");
}

static void context_destroy(void) {
    gl_destroy();
    gl_ready = false;
}

static bool video_wanted(void) {
    int flags = 3;
    if (env && env(RETRO_ENVIRONMENT_GET_AUDIO_VIDEO_ENABLE, &flags)) return (flags & 1) != 0;
    return true;
}

/* Draws the current screen and hands it to the frontend. */
void core_present(void) {
    refresh_ram_view();
    if (!video) return;
    if (gl_ready && video_wanted() && gl_draw(&thw_game.gfx, hw.get_current_framebuffer())) {
        video(RETRO_HW_FRAME_BUFFER_VALID, GFX_W, GFX_H, 0);
    } else {
        video(NULL, GFX_W, GFX_H, 0);  /* duplicate the last frame */
    }
}

/* ---- The libretro API ------------------------------------------------------------------- */

void retro_set_environment(retro_environment_t cb) {
    env = cb;
    static const struct retro_variable vars[] = {
        {"thwaite_decomp_flicker", "Explosion and smoke flicker; original|blend"},
        {NULL, NULL},
    };
    env(RETRO_ENVIRONMENT_SET_VARIABLES, (void *)vars);
    bool no_game = true;
    env(RETRO_ENVIRONMENT_SET_SUPPORT_NO_GAME, &no_game);
    struct retro_log_callback log;
    if (env(RETRO_ENVIRONMENT_GET_LOG_INTERFACE, &log)) log_cb = log.log;
}
void retro_set_video_refresh(retro_video_refresh_t cb) { video = cb; }
void retro_set_audio_sample(retro_audio_sample_t cb) { (void)cb; }
void retro_set_audio_sample_batch(retro_audio_sample_batch_t cb) { audio_batch = cb; }
void retro_set_input_poll(retro_input_poll_t cb) { input_poll = cb; }
void retro_set_input_state(retro_input_state_t cb) { input_state = cb; }

void retro_init(void) { game_power_on(&thw_game); }
void retro_deinit(void) {}
unsigned retro_api_version(void) { return RETRO_API_VERSION; }

void retro_get_system_info(struct retro_system_info *info) {
    memset(info, 0, sizeof *info);
    info->library_name = "Thwaite decomp";
    info->library_version = "0.0.0";
    info->valid_extensions = "nes";
    info->need_fullpath = false;
}

void retro_get_system_av_info(struct retro_system_av_info *info) {
    info->geometry = (struct retro_game_geometry){GFX_W, GFX_H, GFX_W, GFX_H, 4.0f / 3.0f};
    info->timing = (struct retro_system_timing){FPS, RATE};
}

void retro_set_controller_port_device(unsigned port, unsigned device) { (void)port; (void)device; }
void retro_reset(void) { game_power_on(&thw_game); }

bool retro_load_game(const struct retro_game_info *game) {
    (void)game;  /* the decomp brings its own data; the ROM, if given, isn't read */
    enum retro_pixel_format fmt = RETRO_PIXEL_FORMAT_XRGB8888;
    env(RETRO_ENVIRONMENT_SET_PIXEL_FORMAT, &fmt);
    memset(&hw, 0, sizeof hw);
    hw.context_type = RETRO_HW_CONTEXT_OPENGL_CORE;
    hw.version_major = 3;
    hw.version_minor = 2;
    hw.context_reset = context_reset;
    hw.context_destroy = context_destroy;
    hw.bottom_left_origin = true;
    if (!env(RETRO_ENVIRONMENT_SET_HW_RENDER, &hw)) {
        core_log(RETRO_LOG_ERROR, "this frontend can't give an OpenGL 3.2 core context, which the decomp draws with");
        return false;
    }
    game_power_on(&thw_game);
    refresh_ram_view();
    return true;
}

bool retro_load_game_special(unsigned type, const struct retro_game_info *info, size_t num) {
    (void)type; (void)info; (void)num;
    return false;
}
void retro_unload_game(void) {}
unsigned retro_get_region(void) { return RETRO_REGION_NTSC; }

static void read_input(void *out);

/* ---- Plain play: the step's input from libretro ------------------------------------------ */

/* The raw bits the original would read from a joypad: buttons in its bit order. */
static bool lr_pads(void *ctx, uint8_t raw[2]) {
    (void)ctx;
    struct pads { uint8_t held[2]; } in;
    read_input((void *)&in);
    raw[0] = in.held[0];
    raw[1] = in.held[1];
    return true;
}

/* A joypad read past its 8 buttons gives 1s, so read_mouse sees $FF: no mouse. */
static bool lr_mouse(void *ctx, int port, uint8_t raw[3]) {
    (void)ctx;
    (void)port;
    raw[0] = raw[1] = raw[2] = 0xFF;
    return true;
}

/* On its own the decomp never lags: the counter is the step count. */
static bool lr_nmis(void *ctx, uint8_t *v) {
    (void)ctx;
    *v = thw_game.nmis;
    return true;
}

static void read_input(void *out) {
    struct pads { uint8_t held[2]; } *in = out;
    static const struct { unsigned id; uint8_t key; } MAP[] = {
        {RETRO_DEVICE_ID_JOYPAD_A, KEY_A}, {RETRO_DEVICE_ID_JOYPAD_B, KEY_B},
        {RETRO_DEVICE_ID_JOYPAD_SELECT, KEY_SELECT}, {RETRO_DEVICE_ID_JOYPAD_START, KEY_START},
        {RETRO_DEVICE_ID_JOYPAD_UP, KEY_UP}, {RETRO_DEVICE_ID_JOYPAD_DOWN, KEY_DOWN},
        {RETRO_DEVICE_ID_JOYPAD_LEFT, KEY_LEFT}, {RETRO_DEVICE_ID_JOYPAD_RIGHT, KEY_RIGHT},
    };
    memset(in, 0, sizeof *in);
    if (!input_state) return;
    for (unsigned port = 0; port < 2; port++)
        for (unsigned i = 0; i < sizeof MAP / sizeof MAP[0]; i++)
            if (input_state(port, RETRO_DEVICE_JOYPAD, 0, MAP[i].id)) in->held[port] |= MAP[i].key;
}

static void silence(void) {
    static int16_t zeros[2 * 1024];
    audio_owed += RATE / FPS;
    size_t n = (size_t)audio_owed;
    audio_owed -= (double)n;
    if (audio_batch && n) audio_batch(zeros, n > 1024 ? 1024 : n);
}

void retro_run(void) {
    static const struct step_io LIBRETRO_IO = {NULL, lr_pads, lr_mouse, lr_nmis};
    if (input_poll) input_poll();
    if (thw_game.wait != W_NONE) {
        thw_game.nmis++;
        game_step(&thw_game, &LIBRETRO_IO);
        if (thw_game.wait == W_NONE) core_log(RETRO_LOG_WARN, game_unwritten());
    }
    core_present();
    silence();
}

size_t retro_serialize_size(void) { return sizeof thw_game; }
bool retro_serialize(void *data, size_t size) {
    if (size < sizeof thw_game) return false;
    memcpy(data, &thw_game, sizeof thw_game);
    return true;
}
bool retro_unserialize(const void *data, size_t size) {
    if (size < sizeof thw_game) return false;
    struct game g;
    memcpy(&g, data, sizeof g);
    if (g.version != THW_STATE_VERSION) return false;
    thw_game = g;
    refresh_ram_view();
    return true;
}

void retro_cheat_reset(void) {}
void retro_cheat_set(unsigned index, bool enabled, const char *code) { (void)index; (void)enabled; (void)code; }

void *retro_get_memory_data(unsigned id) { return id == RETRO_MEMORY_SYSTEM_RAM ? ram_view : NULL; }
size_t retro_get_memory_size(unsigned id) { return id == RETRO_MEMORY_SYSTEM_RAM ? sizeof ram_view : 0; }
