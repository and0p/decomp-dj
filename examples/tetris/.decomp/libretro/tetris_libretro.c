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
 * For decomp-dj it is a comparison core that takes events (docs/connectors.md in
 * decomp-dj, "Events"): dj tells it where the original has got to, and it runs itself to
 * the same place and compares. See the last section of this file.
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
    {"tetris_judge", "Compare with the original (decomp-dj); at its wait for the next frame|at frame ends"},
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

/* Reads the core options and the controllers. */
static void poll(uint8_t pad[2])
{
    bool updated = false;
    if (environ_cb(RETRO_ENVIRONMENT_GET_VARIABLE_UPDATE, &updated) && updated && readOptions()) {
        struct retro_game_geometry g = {width, height, MAX_WIDTH, MAX_HEIGHT, (float)width / (float)height};
        environ_cb(RETRO_ENVIRONMENT_SET_GEOMETRY, &g);
    }
    input_poll_cb();
    pad[0] = readPad(0);
    pad[1] = readPad(1);
}

/* Draws the game as it is and sends a frame's worth of silence. */
static void present(void)
{
    if (framebuffer && tetris_port_draw(&game, framebuffer, (int)width, (int)height, (int)width) == 0)
        video_cb(framebuffer, width, height, width * sizeof *framebuffer);
    else
        video_cb(NULL, width, height, 0);

    static int16_t silence[2 * 800];
    static double owed;
    owed += SAMPLE_RATE / FPS;
    size_t n = (size_t)owed;
    owed -= (double)n;
    if (audio_batch_cb)
        audio_batch_cb(silence, n);
}

static void runFrame(void)
{
    uint8_t pad[2];
    poll(pad);
    tetris_tick(&game, pad[0], pad[1]);
    present();
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

/*
 * dj does not run this core's frames or sync it. It sends events (a jump, a console
 * reset, a breakpoint in the original, a frame end) with the original stopped and
 * readable, and the core answers each with a verdict. The helper in dj_compare.h does the
 * part every such core shares: after a jump copy the original (copyNes), then at each
 * judging event run up to it (runToHere) and compare (compareNes).
 *
 * The core option tetris_judge picks the judging event:
 *
 * - "at its wait for the next frame" (the default). The original's main loop waits for the
 *   NMI in two places, and the decomp is parked at the same wait between ticks. A
 *   breakpoint sits in each, on the NOP between clearing verticalBlankingInterval and the
 *   loop that polls it: reached once per wait, with the tick's work done and nothing of
 *   the next begun. There the decomp runs one tick and every byte it keeps is compared.
 *   A third breakpoint, at the NMI, takes the controllers as they are when the original
 *   reads them: a tick that draws a screen with NMIs off ends a frame or two after its
 *   NMI, when dj has moved on to later frames' input.
 * - "at frame ends". One tick per frame. A frame that ends with the original part-way
 *   through drawing a screen can only be judged in part (parity.c, inFlight).
 */
#include "dj_compare.h"

enum { BP_WAIT = 1, BP_NMI = 2 };

static const struct dj_breakpoint breakpoints[] = {
    {BP_WAIT, "PRG", 0x2A36, "updateAudioWaitForNmiAndResetOamStaging, before its wait"}, /* $AA36 */
    {BP_WAIT, "PRG", 0x2A4C, "updateAudioAndWaitForNmi, before its wait"},                /* $AA4C */
    {BP_NMI, "PRG", 0x0005, "nmi"},                                                       /* $8005 */
};

static struct dj_helper helper;
static uint8_t nmiPad[2]; /* the controllers at the original's last NMI */
static int nmis;          /* NMIs the original has run since its last wait */
static int waited;        /* the original reached a wait in the frame now running */
static int atWait;        /* the last frame ended with the original at a wait, as the decomp is */

static int judgeAtWait(void)
{
    static int cached = -1;
    if (cached < 0) {
        struct retro_variable var = {"tetris_judge", NULL};
        cached = !(environ_cb && environ_cb(RETRO_ENVIRONMENT_GET_VARIABLE, &var) && var.value &&
                   strcmp(var.value, "at frame ends") == 0);
    }
    return cached;
}

/* The original's 2 KB of system RAM, or NULL with the reason. */
static const uint8_t *nesRam(const struct dj_event *ev, char *reason, size_t len)
{
    const uint8_t *ram = dj_baseline_memory(ev->baseline, "RAM", 0x800);
    if (!ram)
        snprintf(reason, len, "the baseline doesn't give its 2 KB of system RAM");
    return ram;
}

/* Takes over from the original. At a fresh power-on the decomp powers on too. Otherwise it
 * reads the original's RAM, which works whenever the original is waiting for the next
 * frame (not while it draws a new screen with NMIs off). */
static int copyNes(void *user, const struct dj_event *ev, char *reason, size_t len)
{
    (void)user;
    nmis = 0;
    if (ev->kind == DJ_EVENT_JUMP && ev->jump_reason == DJ_JUMP_POWER_ON) {
        tetris_init(&game);
        snprintf(reason, len, "powered on");
        return 0;
    }
    const uint8_t *ram = nesRam(ev, reason, len);
    return ram ? tetris_load_nes_snapshot(&game, ram, reason, len) : 1;
}

/* Where the decomp can't follow the original: a gameModeState with no handler (9, the
 * two-player game over, which jumps through whatever follows the table), or NMIs left off
 * with the game waiting for one, where the original hangs. */
static int stuck(char *reason, size_t len)
{
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

static int runToHere(void *user, const struct dj_event *ev, char *reason, size_t len)
{
    (void)user;
    if (!judgeAtWait()) {
        runFrame();
        return stuck(reason, len) ? DJ_HALT : DJ_PENDING;
    }
    /* A lag frame: the original's NMI ran in the middle of its tick, at an instruction the
     * decomp can't place. Not judged; the decomp takes the original's state from here. */
    if (nmis > 1) {
        char why[160] = "";
        const uint8_t *ram = nesRam(ev, why, sizeof why);
        int lost = !ram || tetris_load_nes_snapshot(&game, ram, why, sizeof why) != 0;
        snprintf(reason, len, "a lag frame: %d NMIs ran in one tick%s%s", nmis, lost ? "; " : "", lost ? why : "");
        if (lost)
            helper.live = 0;
        nmis = 0;
        present();
        return DJ_UNJUDGED;
    }
    nmis = 0;
    /* One tick: the NMI, then the game up to its next wait. At power-on that is three
     * calls, the first two ending at reset's waits for a vertical blank. */
    uint8_t pad[2];
    poll(pad); /* for the options; the controllers are the ones the NMI saw */
    for (int i = 0; i < 8; i++) {
        tetris_tick(&game, nmiPad[0], nmiPad[1]);
        if (game.ctl.waiting == 1)
            break;
    }
    /* The frames a long draw took have passed: the original is at its wait now too. */
    game.ctl.framesBehind = 0;
    present();
    return stuck(reason, len) ? DJ_HALT : DJ_PENDING;
}

/* Where the code is and the RAM the decomp keeps (tetris_check_nes_parity, in
 * decomp/src/parity.c). */
static int compareNes(void *user, const struct dj_event *ev, char *reason, size_t len)
{
    (void)user;
    const uint8_t *ram = nesRam(ev, reason, len);
    if (!ram)
        return DJ_HALT;
    return tetris_check_nes_parity(&game, ram, reason, len) ? DJ_MISMATCH : DJ_MATCH;
}

static void setUpHelper(void)
{
    helper.judge_at = judgeAtWait() ? BP_WAIT : 0;
    helper.copy = copyNes;
    helper.run = runToHere;
    helper.compare = compareNes;
    /* No `reset`: the console's reset is not the decomp's power-on, so it copies again. */
}

DJ_EXPORT size_t dj_breakpoints(const struct dj_breakpoint **out)
{
    if (!judgeAtWait())
        return 0;
    *out = breakpoints;
    return sizeof breakpoints / sizeof breakpoints[0];
}

DJ_EXPORT int dj_handle_event(const struct dj_event *ev, char *reason, size_t len)
{
    if (!helper.copy)
        setUpHelper();
    if (judgeAtWait()) {
        if (ev->kind == DJ_EVENT_BREAK && ev->breakpoint == BP_NMI) {
            input_poll_cb();
            nmiPad[0] = readPad(0);
            nmiPad[1] = readPad(1);
            nmis++;
            return DJ_PENDING;
        }
        if (ev->kind == DJ_EVENT_BREAK && ev->breakpoint == BP_WAIT)
            waited = 1;
        if (ev->kind == DJ_EVENT_FRAME_END) {
            atWait = waited;
            waited = 0;
        }
        if (ev->kind == DJ_EVENT_JUMP)
            waited = atWait = 0;
    }
    return dj_helper_event(&helper, ev, reason, len);
}

/* Follows a write to the original's RAM ("RAM", $0000-$07FF); other regions mean nothing
 * to the decomp. A write made between frames lands where the decomp is parked, as long as
 * the original is at its wait too. Any other can't be followed: the decomp copies the
 * original again. */
DJ_EXPORT int dj_handle_write(const struct dj_write *w, char *reason, size_t len)
{
    (void)reason;
    (void)len;
    if (!w->region || strcmp(w->region, "RAM") != 0)
        return DJ_PENDING;
    int parked = !w->at_region && (!judgeAtWait() || atWait);
    if (helper.live && parked && w->offset < 0x800 && w->len <= 0x800 - w->offset)
        memcpy(&game.ram.b[w->offset], w->bytes, w->len);
    else
        helper.live = 0;
    return DJ_PENDING;
}
