/*
 * A libretro core for the Tetris decomp, drawn by the 3D port. One way to
 * give the two a frontend; they don't depend on it.
 *
 * Each retro_run() reads the controllers, runs one frame of the game
 * (tetris_tick) and draws it (tetris_port_draw) at the resolution the core
 * option asks for. It needs no ROM: started with one, it ignores it. There is
 * no sound: the core sends silence.
 *
 * Save states are the game's whole state (struct tetris), tagged with this
 * build's layout: they load in the same build of the core on the same kind of
 * machine.
 *
 * For decomp-dj it also exports the comparison-core functions (see
 * docs/connectors.md in emu_coverage): dj_sync takes over the game from the
 * baseline's RAM (tetris_load_nes_snapshot), dj_write follows writes to
 * the baseline's RAM, dj_check_parity compares the two every frame
 * (tetris_check_nes_parity) and dj_run stops playback where the decomp can't
 * follow the original.
 */
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "libretro_api.h"
#include "tetris/port.h"
#include "tetris/tetris.h"

#define FPS 60.0988 /* NTSC NES */
#define SAMPLE_RATE 48000

static retro_environment_t environ_cb;
static retro_video_refresh_t video_cb;
static retro_audio_sample_batch_t audio_batch_cb;
static retro_input_poll_t input_poll_cb;
static retro_input_state_t input_state_cb;
static retro_log_printf_t log_cb;

static struct tetris game;
static uint32_t *framebuffer;
static unsigned width = 640, height = 480;

static void logf_(enum retro_log_level level, const char *fmt, ...)
{
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    if (log_cb)
        log_cb(level, "%s", buf);
    else
        fprintf(stderr, "[tetris] %s", buf);
}

/* ---- options ------------------------------------------------------------- */

#define MAX_WIDTH 1280
#define MAX_HEIGHT 960

static const struct retro_variable variables[] = {
    {"tetris_resolution", "Resolution; 640x480|320x240|960x720|1280x960"},
    {NULL, NULL},
};

/* Reads the resolution option. Returns 1 when it changed. */
static int readOptions(void)
{
    struct retro_variable var = {"tetris_resolution", NULL};
    unsigned w = 640, h = 480;
    if (environ_cb && environ_cb(RETRO_ENVIRONMENT_GET_VARIABLE, &var) && var.value &&
        sscanf(var.value, "%ux%u", &w, &h) == 2 && w >= 16 && h >= 16 && w <= MAX_WIDTH && h <= MAX_HEIGHT) {
        /* as asked */
    } else {
        w = 640, h = 480;
    }
    int changed = w != width || h != height;
    width = w, height = h;
    return changed;
}

/* ---- libretro -------------------------------------------------------------- */

RETRO_API unsigned retro_api_version(void) { return RETRO_API_VERSION; }

RETRO_API void retro_set_environment(retro_environment_t cb)
{
    environ_cb = cb;
    struct retro_log_callback log;
    if (cb(RETRO_ENVIRONMENT_GET_LOG_INTERFACE, &log))
        log_cb = log.log;
    bool noGame = true;
    cb(RETRO_ENVIRONMENT_SET_SUPPORT_NO_GAME, &noGame);
    cb(RETRO_ENVIRONMENT_SET_VARIABLES, (void *)variables);
}

RETRO_API void retro_set_video_refresh(retro_video_refresh_t cb) { video_cb = cb; }
RETRO_API void retro_set_audio_sample(retro_audio_sample_t cb) { (void)cb; }
RETRO_API void retro_set_audio_sample_batch(retro_audio_sample_batch_t cb) { audio_batch_cb = cb; }
RETRO_API void retro_set_input_poll(retro_input_poll_t cb) { input_poll_cb = cb; }
RETRO_API void retro_set_input_state(retro_input_state_t cb) { input_state_cb = cb; }

RETRO_API void retro_init(void)
{
    framebuffer = calloc((size_t)MAX_WIDTH * MAX_HEIGHT, sizeof *framebuffer);
    tetris_init(&game);
}

RETRO_API void retro_deinit(void)
{
    free(framebuffer);
    framebuffer = NULL;
}

RETRO_API void retro_get_system_info(struct retro_system_info *info)
{
    memset(info, 0, sizeof *info);
    info->library_name = "Tetris decomp (3D)";
    info->library_version = "0.2";
    info->valid_extensions = "nes";
    info->need_fullpath = true; /* the ROM, if given, is not read */
}

RETRO_API void retro_get_system_av_info(struct retro_system_av_info *info)
{
    memset(info, 0, sizeof *info);
    readOptions();
    info->geometry.base_width = width;
    info->geometry.base_height = height;
    info->geometry.max_width = MAX_WIDTH;
    info->geometry.max_height = MAX_HEIGHT;
    info->geometry.aspect_ratio = (float)width / (float)height;
    info->timing.fps = FPS;
    info->timing.sample_rate = SAMPLE_RATE;
}

RETRO_API void retro_set_controller_port_device(unsigned port, unsigned device)
{
    (void)port;
    (void)device;
}

RETRO_API bool retro_load_game(const struct retro_game_info *game_)
{
    (void)game_; /* no ROM needed: a ROM given is ignored */
    enum retro_pixel_format fmt = RETRO_PIXEL_FORMAT_XRGB8888;
    if (!environ_cb(RETRO_ENVIRONMENT_SET_PIXEL_FORMAT, &fmt)) {
        logf_(RETRO_LOG_ERROR, "the frontend does not support XRGB8888\n");
        return false;
    }
    static const struct retro_input_descriptor desc[] = {
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_LEFT, "Left"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_UP, "Up"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_DOWN, "Down"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_RIGHT, "Right"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_B, "B (rotate left)"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_A, "A (rotate right)"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_SELECT, "Select"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_START, "Start"},
        {0, 0, 0, 0, NULL},
    };
    environ_cb(RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS, (void *)desc);
    tetris_init(&game);
    return true;
}

RETRO_API bool retro_load_game_special(unsigned type, const struct retro_game_info *info, size_t num)
{
    (void)type;
    (void)info;
    (void)num;
    return false;
}

RETRO_API void retro_unload_game(void) {}
RETRO_API unsigned retro_get_region(void) { return RETRO_REGION_NTSC; }
RETRO_API void retro_reset(void) { tetris_init(&game); }

static uint8_t readPad(unsigned port)
{
    static const struct {
        unsigned id;
        uint8_t bit;
    } map[8] = {
        {RETRO_DEVICE_ID_JOYPAD_A, TETRIS_BUTTON_A},       {RETRO_DEVICE_ID_JOYPAD_B, TETRIS_BUTTON_B},
        {RETRO_DEVICE_ID_JOYPAD_SELECT, TETRIS_BUTTON_SELECT}, {RETRO_DEVICE_ID_JOYPAD_START, TETRIS_BUTTON_START},
        {RETRO_DEVICE_ID_JOYPAD_UP, TETRIS_BUTTON_UP},     {RETRO_DEVICE_ID_JOYPAD_DOWN, TETRIS_BUTTON_DOWN},
        {RETRO_DEVICE_ID_JOYPAD_LEFT, TETRIS_BUTTON_LEFT}, {RETRO_DEVICE_ID_JOYPAD_RIGHT, TETRIS_BUTTON_RIGHT},
    };
    uint8_t b = 0;
    for (int i = 0; i < 8; i++)
        if (input_state_cb(port, RETRO_DEVICE_JOYPAD, 0, map[i].id))
            b |= map[i].bit;
    return b;
}

static void runFrame(void)
{
    bool updated = false;
    if (environ_cb(RETRO_ENVIRONMENT_GET_VARIABLE_UPDATE, &updated) && updated && readOptions()) {
        struct retro_game_geometry g = {width, height, MAX_WIDTH, MAX_HEIGHT, (float)width / (float)height};
        environ_cb(RETRO_ENVIRONMENT_SET_GEOMETRY, &g);
    }

    input_poll_cb();
    tetris_tick(&game, readPad(0), readPad(1));

    if (framebuffer && tetris_port_draw(&game, framebuffer, (int)width, (int)height, (int)width) == 0)
        video_cb(framebuffer, width, height, width * sizeof *framebuffer);
    else
        video_cb(NULL, width, height, 0);

    /* silence, a frame's worth */
    static int16_t silence[2 * 800];
    static double owed;
    owed += SAMPLE_RATE / FPS;
    size_t n = (size_t)owed;
    owed -= (double)n;
    if (audio_batch_cb)
        audio_batch_cb(silence, n);
}

RETRO_API void retro_run(void) { runFrame(); }

/* ---- save states --------------------------------------------------------- */

#define STATE_MAGIC 0x53525454u /* "TTRS" */
#define STATE_VERSION 2u
#define STATE_HEADER 12

static void put32(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)v, p[1] = (uint8_t)(v >> 8), p[2] = (uint8_t)(v >> 16), p[3] = (uint8_t)(v >> 24);
}
static uint32_t get32(const uint8_t *p) { return (uint32_t)p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24; }

RETRO_API size_t retro_serialize_size(void) { return STATE_HEADER + sizeof game; }

RETRO_API bool retro_serialize(void *data, size_t size)
{
    if (size < retro_serialize_size())
        return false;
    uint8_t *p = data;
    put32(p, STATE_MAGIC);
    put32(p + 4, STATE_VERSION);
    put32(p + 8, (uint32_t)sizeof game);
    memcpy(p + STATE_HEADER, &game, sizeof game);
    return true;
}

RETRO_API bool retro_unserialize(const void *data, size_t size)
{
    const uint8_t *p = data;
    if (size < retro_serialize_size() || get32(p) != STATE_MAGIC || get32(p + 4) != STATE_VERSION ||
        get32(p + 8) != sizeof game)
        return false;
    memcpy(&game, p + STATE_HEADER, sizeof game);
    return true;
}

RETRO_API void retro_cheat_reset(void) {}
RETRO_API void retro_cheat_set(unsigned index, bool enabled, const char *code)
{
    (void)index;
    (void)enabled;
    (void)code;
}

/* The game's RAM, laid out as the NES's, for frontends that look at it. */
RETRO_API void *retro_get_memory_data(unsigned id) { return id == RETRO_MEMORY_SYSTEM_RAM ? game.ram.b : NULL; }
RETRO_API size_t retro_get_memory_size(unsigned id) { return id == RETRO_MEMORY_SYSTEM_RAM ? sizeof game.ram.b : 0; }

/* ---- decomp-dj comparison core --------------------------------------------- */

struct dj_baseline {
    uint32_t version;
    void *ctx;
    void *(*get_proc)(void *ctx, const char *name);
};

#define DJ_SYNC_BOOT 1u

/* The baseline's 2 KB of system RAM, or NULL with the reason. */
static const uint8_t *baselineRam(const struct dj_baseline *b, char *reason, size_t len)
{
    void *(*getData)(unsigned) = b && b->get_proc ? (void *(*)(unsigned))b->get_proc(b->ctx, "retro_get_memory_data") : NULL;
    size_t (*getSize)(unsigned) = b && b->get_proc ? (size_t(*)(unsigned))b->get_proc(b->ctx, "retro_get_memory_size") : NULL;
    const uint8_t *ram = getData ? getData(RETRO_MEMORY_SYSTEM_RAM) : NULL;
    if (!ram || !getSize || getSize(RETRO_MEMORY_SYSTEM_RAM) < 0x800) {
        snprintf(reason, len, "the baseline doesn't give its 2 KB of system RAM");
        return NULL;
    }
    return ram;
}

/* Brings the game to the baseline's state at the start of a frame. At a
 * fresh power-on it powers on too; otherwise it takes over from the
 * baseline's RAM, which works whenever the original is waiting for the next
 * frame (not while it draws a new screen with NMIs off). */
RETRO_API int dj_sync(const struct dj_baseline *b, uint64_t frame, uint32_t flags, char *reason, size_t len)
{
    (void)frame;
    if (flags & DJ_SYNC_BOOT) {
        tetris_init(&game);
        return 0;
    }
    const uint8_t *ram = baselineRam(b, reason, len);
    if (!ram)
        return 2;
    return tetris_load_nes_snapshot(&game, ram, reason, len) ? 1 : 0;
}

/* Follows a write to the baseline's RAM ("RAM", $0000-$07FF); other regions
 * mean nothing to the decomp and are ignored. */
RETRO_API int dj_write(const struct dj_baseline *b, const char *region, uint64_t address, const uint8_t *bytes,
                       size_t n, char *reason, size_t len)
{
    (void)b;
    if (!region || strcmp(region, "RAM") != 0)
        return 1;
    if (address >= 0x800 || n > 0x800 - address) {
        snprintf(reason, len, "$%llX+%zu is past the 2 KB of RAM", (unsigned long long)address, n);
        return 2;
    }
    memcpy(&game.ram.b[address], bytes, n);
    return 0;
}

/* Compares the game with the baseline after a frame: where the code is and
 * the RAM the decomp keeps (tetris_check_nes_parity, in decomp/src/parity.c).
 * Every frame is judged; nothing returns "skip". */
RETRO_API int dj_check_parity(const struct dj_baseline *b, uint64_t frame, char *reason, size_t len)
{
    (void)frame;
    const uint8_t *ram = baselineRam(b, reason, len);
    if (!ram)
        return 3;
    return tetris_check_nes_parity(&game, ram, reason, len) ? 1 : 0;
}

/* One frame, as retro_run, stopping where the decomp can't follow the
 * original: a gameModeState with no handler (9, the two-player game over, which
 * jumps through whatever follows the table), or NMIs left off with the game
 * waiting for one, where the original hangs. */
RETRO_API int dj_run(char *reason, size_t len)
{
    runFrame();
    if (game.ram.v.gameModeState > 0x08) {
        snprintf(reason, len, "gameModeState is %u, which has no handler (the original jumps through the bytes after its table)",
                 game.ram.v.gameModeState);
        return 1;
    }
    if (game.ctl.waiting == 1 && !(game.ram.v.currentPpuCtrl & 0x80)) {
        snprintf(reason, len, "the game waits for an NMI with NMIs off: the original hangs here");
        return 1;
    }
    return 0;
}
