/*
 * A stub comparison core for WWF No Mercy (USA, Rev 1) on the Mupen64Plus connector.
 *
 * It is not a port of anything. It is what a port's dj side looks like on the N64, with the
 * game's logic left out, and it is here to test breakpoints inside the frame on a real game
 * whose logic ticks cross frames:
 *
 * - a breakpoint where the game's input routine (pads_read) returns: the stub copies the
 *   pad buffer the game just filled, as a port would take its inputs;
 * - a breakpoint at the match's logic tick (match_tick): the stub counts the tick, and its
 *   verdict is whether the pads it copied are the ones the game holds now.
 *
 * Other code sits at match_tick's address when another overlay is loaded. The stub tells
 * the real function by its caller: the return address, and the call instruction there.
 *
 * NM_STUB_LOG=<file>: one line per event the stub acts on, to compare two runs.
 * NM_STUB_FAULT=<n>: the stub spoils its copy of the pads at its nth tick, to see a failure.
 * NM_STUB_DUMP=<file>: RDRAM at the first real tick, in the console's byte order.
 *
 * Build: see the README beside this file. MIT license.
 */
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "dj_compare.h"

/* ---- the game ---------------------------------------------------------------------- */

#define RDRAM_SIZE 0x800000u
#define MATCH_TICK 0x114514u      /* match_tick, in the match overlay */
#define TICK_CALL 0x0D9DFCu       /* its one call, in match_frame */
#define TICK_CALL_WORD 0x0C045145u /* jal 0x80114514 */
#define PADS_RETURN 0x005BD0u     /* pads_read's one "jr ra" (the function is at 0x5818, always loaded) */
#define PADS 0x09EDA0u            /* g_pads: struct Pad[4], 12 bytes each */
#define PADS_LEN 48u

enum { BP_TICK = 1, BP_PADS = 2 };

static const struct dj_breakpoint breakpoints[] = {
    {BP_TICK, "RDRAM", MATCH_TICK, "match_tick"},
    {BP_PADS, "RDRAM", PADS_RETURN, "pads_read returns"},
};

/* The baseline's RDRAM is 32-bit words in the host's byte order (connectors/mupen64plus). */
static uint32_t word(const uint8_t *ram, uint32_t at)
{
    uint32_t w;
    memcpy(&w, ram + (at & ~3u), 4);
    return w;
}

static uint8_t byte(const uint8_t *ram, uint32_t at)
{
    return (uint8_t)(word(ram, at) >> (8 * (3 - (at & 3))));
}

/* ---- the stub's state -------------------------------------------------------------- */

static struct {
    uint8_t pads[PADS_LEN]; /* as copied at the input routine */
    uint32_t pad_reads;     /* since the last tick */
    uint64_t ticks;         /* since the stub last copied the baseline */
} st;

static struct dj_helper helper;
static FILE *logf;
static int dumped;

static void logline(const struct dj_event *ev, const char *fmt, ...)
{
    if (!logf)
        return;
    va_list ap;
    fprintf(logf, "%.8s %5llu%s ", ev->segment ? ev->segment : "-", (unsigned long long)ev->frame, ev->flags & DJ_PREROLL ? " pre" : "");
    va_start(ap, fmt);
    vfprintf(logf, fmt, ap);
    va_end(ap);
    fputc('\n', logf);
    fflush(logf);
}

static void read_pads(const uint8_t *ram, uint8_t *out)
{
    for (uint32_t i = 0; i < PADS_LEN; i++)
        out[i] = byte(ram, PADS + i);
}

static const char *hex(const uint8_t *p, size_t n)
{
    static char s[2 * PADS_LEN + 1];
    for (size_t i = 0; i < n; i++)
        snprintf(s + 2 * i, 3, "%02x", p[i]);
    return s;
}

static int copy(void *user, const struct dj_event *ev, char *reason, size_t len)
{
    (void)user;
    const uint8_t *ram = dj_baseline_memory(ev->baseline, "RAM", RDRAM_SIZE);
    if (!ram) {
        snprintf(reason, len, "the baseline has no 8 MB of RAM");
        return 1;
    }
    if (ev->kind == DJ_EVENT_JUMP) {
        /* A power-on jump: nothing to take yet. Wait for the first tick. */
        snprintf(reason, len, "waiting for a match tick");
        return 1;
    }
    read_pads(ram, st.pads);
    st.pad_reads = 0;
    st.ticks = 0;
    logline(ev, "copy pads %s", hex(st.pads, PADS_LEN));
    return 0;
}

static int run(void *user, const struct dj_event *ev, char *reason, size_t len)
{
    (void)user;
    st.ticks++;
    /* A fault made on purpose, to see a failure: NM_STUB_FAULT=<tick>. */
    if (getenv("NM_STUB_FAULT") && st.ticks == strtoull(getenv("NM_STUB_FAULT"), NULL, 10))
        st.pads[5] ^= 0x10;
    if (st.pad_reads == 0) {
        /* The game did not read its pads since the last tick: the stub has no inputs. */
        snprintf(reason, len, "no pad read before tick %llu", (unsigned long long)st.ticks);
        logline(ev, "tick %llu no pads", (unsigned long long)st.ticks);
        return DJ_UNJUDGED;
    }
    return DJ_PENDING;
}

static uint8_t expected[PADS_LEN];
static int differs;

static int compare(void *user, const struct dj_event *ev, char *reason, size_t len)
{
    (void)user;
    const uint8_t *ram = dj_baseline_memory(ev->baseline, "RAM", RDRAM_SIZE);
    read_pads(ram, expected);
    differs = memcmp(expected, st.pads, PADS_LEN) != 0;
    logline(ev, "tick %llu reads %u pads %s %s", (unsigned long long)st.ticks, st.pad_reads, hex(st.pads, PADS_LEN), differs ? "MISMATCH" : "match");
    st.pad_reads = 0;
    if (differs) {
        snprintf(reason, len, "the pads at tick %llu are not the ones copied at the input routine", (unsigned long long)st.ticks);
        return DJ_MISMATCH;
    }
    return DJ_MATCH;
}

DJ_EXPORT size_t dj_breakpoints(const struct dj_breakpoint **out)
{
    *out = breakpoints;
    return sizeof breakpoints / sizeof breakpoints[0];
}

DJ_EXPORT size_t dj_parity_diffs(struct dj_parity_diff *out, size_t cap)
{
    if (!differs)
        return 0;
    if (cap) {
        memset(out, 0, sizeof *out);
        out->label = "The four controllers as the game read them";
        out->symbol = "g_pads";
        out->region = "RDRAM"; /* the connector's name for it, so dj can trace who wrote it */
        out->address = PADS;
        out->expected = expected;
        out->actual = st.pads;
        out->len = PADS_LEN;
    }
    return 1;
}

DJ_EXPORT int dj_handle_event(const struct dj_event *ev, char *reason, size_t len)
{
    if (!logf && getenv("NM_STUB_LOG"))
        logf = fopen(getenv("NM_STUB_LOG"), "a");
    if (ev->kind == DJ_EVENT_JUMP)
        logline(ev, "jump %u", ev->jump_reason);
    if (ev->kind == DJ_EVENT_BREAK) {
        const uint8_t *ram = dj_baseline_memory(ev->baseline, "RAM", RDRAM_SIZE);
        uint64_t regs[33] = {0};
        if (!ram || ev->baseline->registers(ev->baseline->ctx, regs, sizeof regs) != sizeof regs)
            return DJ_PENDING;
        if (ev->breakpoint == BP_PADS) {
            /* The input routine is done: take the pads, as a port takes its inputs. */
            read_pads(ram, st.pads);
            st.pad_reads++;
            logline(ev, "pads %s ra %08x", hex(st.pads, PADS_LEN), (uint32_t)regs[1 + 31]);
            return DJ_PENDING;
        }
        /* match_tick's address. Only the match's own call of it is the tick. */
        uint32_t ra = (uint32_t)regs[1 + 31];
        if (ra != (0x80000000u | (TICK_CALL + 8)) || word(ram, TICK_CALL) != TICK_CALL_WORD) {
            logline(ev, "other code at match_tick, ra %08x sp %08x", ra, (uint32_t)regs[1 + 29]);
            return DJ_PENDING;
        }
        if (!dumped && getenv("NM_STUB_DUMP")) {
            FILE *f = fopen(getenv("NM_STUB_DUMP"), "wb");
            if (f) {
                for (uint32_t a = 0; a < RDRAM_SIZE; a++)
                    fputc(byte(ram, a), f);
                fclose(f);
            }
            dumped = 1;
        }
    }
    return dj_helper_event(&helper, ev, reason, len);
}

/* ---- libretro: the least a core needs. The names and values are libretro's. --------- */

#define RETRO_API DJ_EXPORT
#define W 64
#define H 48

struct retro_system_info { const char *library_name, *library_version, *valid_extensions; bool need_fullpath, block_extract; };
struct retro_game_geometry { unsigned base_width, base_height, max_width, max_height; float aspect_ratio; };
struct retro_system_timing { double fps, sample_rate; };
struct retro_system_av_info { struct retro_game_geometry geometry; struct retro_system_timing timing; };
struct retro_game_info;
typedef bool (*retro_environment_t)(unsigned cmd, void *data);
typedef void (*retro_video_refresh_t)(const void *data, unsigned width, unsigned height, size_t pitch);

static retro_environment_t environ_cb;
static retro_video_refresh_t video_cb;
static uint32_t picture[W * H];

RETRO_API unsigned retro_api_version(void) { return 1; }
RETRO_API void retro_set_environment(retro_environment_t cb)
{
    bool no_game = true;
    environ_cb = cb;
    cb(18 /* SET_SUPPORT_NO_GAME */, &no_game);
}
RETRO_API void retro_set_video_refresh(retro_video_refresh_t cb) { video_cb = cb; }
RETRO_API void retro_set_audio_sample(void *cb) { (void)cb; }
RETRO_API void retro_set_audio_sample_batch(void *cb) { (void)cb; }
RETRO_API void retro_set_input_poll(void *cb) { (void)cb; }
RETRO_API void retro_set_input_state(void *cb) { (void)cb; }
RETRO_API void retro_init(void)
{
    helper.judge_at = BP_TICK;
    helper.copy = copy;
    helper.run = run;
    helper.compare = compare;
}
RETRO_API void retro_deinit(void) {}
RETRO_API void retro_get_system_info(struct retro_system_info *info)
{
    memset(info, 0, sizeof *info);
    info->library_name = "No Mercy stub";
    info->library_version = "1";
    info->valid_extensions = "z64";
}
RETRO_API void retro_get_system_av_info(struct retro_system_av_info *info)
{
    memset(info, 0, sizeof *info);
    info->geometry.base_width = info->geometry.max_width = W;
    info->geometry.base_height = info->geometry.max_height = H;
    info->geometry.aspect_ratio = (float)W / H;
    info->timing.fps = 60.0;
    info->timing.sample_rate = 44100.0;
}
RETRO_API void retro_set_controller_port_device(unsigned port, unsigned device) { (void)port; (void)device; }
RETRO_API void retro_reset(void) {}
/* The picture: one row of 16 squares per controller, lit for the buttons held. */
RETRO_API void retro_run(void)
{
    for (int i = 0; i < W * H; i++)
        picture[i] = helper.live ? 0x00202020 : 0x00400000;
    for (int p = 0; p < 4; p++) {
        unsigned held = st.pads[p * 12 + 4] << 8 | st.pads[p * 12 + 5];
        for (int b = 0; b < 16; b++)
            if (held & (0x8000u >> b))
                for (int y = 0; y < 3; y++)
                    for (int x = 0; x < 3; x++)
                        picture[(8 + p * 8 + y) * W + b * 4 + x] = 0x00ffffff;
    }
    if (video_cb)
        video_cb(picture, W, H, W * sizeof picture[0]);
}
RETRO_API size_t retro_serialize_size(void) { return sizeof st; }
RETRO_API bool retro_serialize(void *data, size_t size) { if (size < sizeof st) return false; memcpy(data, &st, sizeof st); return true; }
RETRO_API bool retro_unserialize(const void *data, size_t size) { if (size < sizeof st) return false; memcpy(&st, data, sizeof st); return true; }
RETRO_API void retro_cheat_reset(void) {}
RETRO_API void retro_cheat_set(unsigned index, bool enabled, const char *code) { (void)index; (void)enabled; (void)code; }
RETRO_API bool retro_load_game(const struct retro_game_info *game)
{
    int fmt = 1; /* XRGB8888 */
    (void)game;
    return environ_cb(10 /* SET_PIXEL_FORMAT */, &fmt);
}
RETRO_API bool retro_load_game_special(unsigned type, const struct retro_game_info *info, size_t num) { (void)type; (void)info; (void)num; return false; }
RETRO_API void retro_unload_game(void) {}
RETRO_API unsigned retro_get_region(void) { return 0; }
RETRO_API void *retro_get_memory_data(unsigned id) { return id == 2 ? (void *)&st : NULL; }
RETRO_API size_t retro_get_memory_size(unsigned id) { return id == 2 ? sizeof st : 0; }
