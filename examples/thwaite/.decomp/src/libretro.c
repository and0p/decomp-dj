/* The libretro API, and plain play: one step a retro_run.
 *
 * Under dj the decomp is driven by events instead (events.c), and dj doesn't call
 * retro_run on it. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "compare.h"
#include "core.h"
#include "gl.h"
#include "libretro.h"
#include "voxel.h"

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

/* The render style (thwaite_decomp_render): flat, the console's picture at 256x240, or
 * voxel (voxel.c) at 4x. */
#define VOXEL_SCALE 4
static bool voxel_on;
static struct voxel_opts voxel_opts = {22.0f, 50.0f, 0.2f};

static const char *variable(const char *key) {
    struct retro_variable v = {key, NULL};
    return env && env(RETRO_ENVIRONMENT_GET_VARIABLE, &v) ? v.value : NULL;
}

/* Reads the options; true when the picture's size changed. */
static bool read_options(void) {
    bool was = voxel_on;
    const char *v = variable("thwaite_decomp_render");
    voxel_on = v && strcmp(v, "voxel") == 0;
    if ((v = variable("thwaite_decomp_voxel_tilt"))) voxel_opts.tilt_deg = (float)atof(v);
    if ((v = variable("thwaite_decomp_voxel_fov"))) voxel_opts.fov_deg = (float)atof(v);
    if ((v = variable("thwaite_decomp_voxel_outline"))) voxel_opts.outline = strcmp(v, "off") == 0 ? 0.0f : (float)atof(v) / 100.0f;
    return was != voxel_on;
}

static unsigned out_w(void) { return voxel_on ? GFX_W * VOXEL_SCALE : GFX_W; }
static unsigned out_h(void) { return voxel_on ? GFX_H * VOXEL_SCALE : GFX_H; }

static void options_changed(void) {
    bool updated = false;
    if (!env || !env(RETRO_ENVIRONMENT_GET_VARIABLE_UPDATE, &updated) || !updated) return;
    if (read_options()) {
        struct retro_game_geometry geo = {out_w(), out_h(), GFX_W * VOXEL_SCALE, GFX_H * VOXEL_SCALE, 4.0f / 3.0f};
        env(RETRO_ENVIRONMENT_SET_GEOMETRY, &geo);
    }
}

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
    options_changed();
    if (!gl_ready || !video_wanted()) {
        video(NULL, out_w(), out_h(), 0);   /* duplicate the last frame */
        return;
    }
    uintptr_t fbo = hw.get_current_framebuffer();
    bool drawn = voxel_on ? gl_draw_voxel(&thw_game.gfx, fbo, (int)out_w(), (int)out_h(), &voxel_opts)
                          : gl_draw(&thw_game.gfx, fbo);
    video(drawn ? RETRO_HW_FRAME_BUFFER_VALID : NULL, out_w(), out_h(), 0);
}

/* ---- The libretro API ------------------------------------------------------------------- */

void retro_set_environment(retro_environment_t cb) {
    env = cb;
    static const struct retro_variable vars[] = {
        {"thwaite_decomp_flicker", "Explosion and smoke flicker; original|blend"},
        {"thwaite_decomp_render", "Render style; flat|voxel"},
        {"thwaite_decomp_voxel_tilt", "Voxel camera tilt in degrees, negative from above; 22|0|10|15|30|40|-10|-15|-22|-30"},
        {"thwaite_decomp_voxel_fov", "Voxel camera field of view (degrees); 50|10|30|40|60|70"},
        {"thwaite_decomp_voxel_outline", "Voxel outlines (darkening, percent); 20|off|10|30|40"},
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
    read_options();
    info->geometry = (struct retro_game_geometry){out_w(), out_h(), GFX_W * VOXEL_SCALE, GFX_H * VOXEL_SCALE, 4.0f / 3.0f};
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
    hw.depth = true;   /* the voxel picture */
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

/* The split sound's channel: the decomp doesn't model the sound engine's pooling, so 0. */
static bool lr_sound(void *ctx, uint8_t *v) {
    (void)ctx;
    *v = 0;
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
    static const struct step_io LIBRETRO_IO = {NULL, lr_pads, lr_mouse, lr_nmis, lr_sound};
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
