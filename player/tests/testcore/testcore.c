/* A tiny libretro core for decomp-dj's tests: 64 bytes of "RAM" that change each frame
 * with the held buttons, save states, and the comparison-core API (dj_sync, dj_write)
 * from docs/connectors.md. Built by player/tests/compare.rs; -DNO_DJ leaves the
 * comparison API out.
 *
 * The core option `testcore_sync` makes dj_sync succeed (ok), decline (fail) or report
 * an error (error).
 *
 * -DCONNECTOR gives it a dj connector (API 3), for use as a baseline: a 256-byte "PRG"
 * region whose "instructions" a frame runs in a fixed pattern (see retro_run), with
 * coverage flags, breakpoints and registers.
 *
 * -DEVENTS makes it a comparison core that takes events (dj_handle_event, dj_handle_write,
 * dj_breakpoints) in place of dj_sync and the rest. -DNO_WRITE_HANDLER leaves
 * dj_handle_write out. The core option `testcore_judge` picks how it judges: "frame" (at
 * each frame end), "tick" (at its breakpoint where the baseline's tick ends) or "copy"
 * (copies the baseline at every event and answers match). */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
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

#ifdef CONNECTOR
static void conn_register(bool (*cb)(unsigned, void *));
#endif
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
#ifdef CONNECTOR
    conn_register(cb);
#endif
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

/* One frame's logic, in two halves. With a connector, the baseline runs the "tick end"
 * instruction between them, so a breakpoint there sees RAM part-way through the frame. */
static void phase_a(uint8_t held) { ram[0]++; ram[1] = held; }
static void phase_b(void) { ram[2] = (uint8_t)(ram[2] * 31 + ram[1] + 1); }

#ifdef CONNECTOR
static void exec(unsigned off);
#else
static void exec(unsigned off) { (void)off; }
#endif

void retro_run(void) {
    if (poll_cb) poll_cb();
    uint8_t held = 0;
    for (unsigned b = 0; b < 8; b++)
        if (input && input(0, RETRO_DEVICE_JOYPAD, 0, b)) held |= (uint8_t)(1u << b);
    /* For the batch tests: crash while Select is held, or never return while Start is, when
     * the environment asks (the core runs in a child dj that inherits it). */
    if ((held & 4) && getenv("TESTCORE_CRASH_ON_SELECT")) *(volatile int *)0 = 1;
    if ((held & 8) && getenv("TESTCORE_HANG_ON_START")) for (volatile int spin = 1; spin;) {}
    /* What a frame "runs", for the connector:
     *   0x10            the frame's start, every frame
     *   0x80            where a tick ends, between the two halves; not on a lag frame
     *                   (every seventh)
     *   0x20 + buttons  code that depends on the buttons held (the low four)
     *   0x90            a wait that spins 1,500 times, on one frame in sixteen */
    exec(0x10);
    phase_a(held);
    if (ram[0] % 7 != 0) exec(0x80);
    phase_b();
    exec(0x20 + (held & 0x0f));
    if (ram[0] % 16 == 5) for (int i = 0; i < 1500; i++) exec(0x90);
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

#ifdef CONNECTOR
/* ---- a dj connector, API 3 (docs/connectors.md) ---- */
#define RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK 33
typedef void (*retro_proc_address_t)(void);
struct retro_get_proc_address_interface { retro_proc_address_t (*get_proc_address)(const char *sym); };
struct dj_region { const char *name; uint8_t *flags; size_t size; const char *cpu; const uint8_t *data; int64_t file_offset; };
struct dj_break { int32_t region; int64_t offset; };
typedef int (*dj_break_fn)(void *user, int region, int64_t offset);

static uint8_t prg[256], prg_flags[256], brk[256];
static int brk_left;
static dj_break_fn brk_fn;
static void *brk_user;
static unsigned last_exec;

static void exec(unsigned off) {
    off &= 0xff;
    prg_flags[off] |= 1;
    last_exec = off;
    if (brk_left > 0 && brk[off] && brk_fn(brk_user, 0, off)) { brk[off] = 0; brk_left--; }
}
static unsigned conn_api_version(void) { return 3; }
static size_t conn_region_count(void) { return 1; }
static bool conn_get_region(size_t i, struct dj_region *out) {
    if (i != 0 || !out) return false;
    memset(prg, 0xEA, sizeof prg); /* NOPs */
    out->name = "PRG"; out->flags = prg_flags; out->size = sizeof prg; out->cpu = "6502"; out->data = prg; out->file_offset = -1;
    return true;
}
static void conn_clear(void) { memset(prg_flags, 0, sizeof prg_flags); }
static void conn_set_breakpoints(const struct dj_break *list, size_t n, dj_break_fn fn, void *user) {
    memset(brk, 0, sizeof brk);
    brk_left = 0; brk_fn = fn; brk_user = user;
    if (!fn || !list) return;
    for (size_t i = 0; i < n; i++)
        if (list[i].region == 0 && list[i].offset >= 0 && list[i].offset < 256 && !brk[list[i].offset]) { brk[list[i].offset] = 1; brk_left++; }
}
/* As a 6502's: A, X, Y, SP, P, PC low, PC high. */
static size_t conn_read_registers(void *out, size_t cap) {
    uint8_t *o = out;
    if (!out || cap < 7) return 0;
    o[0] = ram[0]; o[1] = ram[1]; o[2] = ram[2]; o[3] = 0xff; o[4] = 0;
    o[5] = (uint8_t)last_exec; o[6] = 0x80;
    return 7;
}
static retro_proc_address_t conn_proc(const char *sym) {
    if (!strcmp(sym, "dj_api_version")) return (retro_proc_address_t)conn_api_version;
    if (!strcmp(sym, "dj_region_count")) return (retro_proc_address_t)conn_region_count;
    if (!strcmp(sym, "dj_get_region")) return (retro_proc_address_t)conn_get_region;
    if (!strcmp(sym, "dj_clear")) return (retro_proc_address_t)conn_clear;
    if (!strcmp(sym, "dj_set_breakpoints")) return (retro_proc_address_t)conn_set_breakpoints;
    if (!strcmp(sym, "dj_read_registers")) return (retro_proc_address_t)conn_read_registers;
    return NULL;
}
static void conn_register(env_t cb) {
    static struct retro_get_proc_address_interface iface = {conn_proc};
    cb(RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK, &iface);
}
#endif

#ifndef NO_DJ
struct dj_baseline { uint32_t version; void *ctx; void *(*get_proc)(void *ctx, const char *name); };
#define DJ_SYNC_BOOT 1u

static const char *sync_setting(void) {
    struct retro_variable v = {"testcore_sync", NULL};
    if (env && env(RETRO_ENVIRONMENT_GET_VARIABLE, &v) && v.value) return v.value;
    return "ok";
}

static const char *break_setting(void) {
    struct retro_variable v = {"testcore_break", NULL};
    if (env && env(RETRO_ENVIRONMENT_GET_VARIABLE, &v) && v.value) return v.value;
    return "off";
}

#ifndef EVENTS
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

#endif /* !EVENTS */

struct dj_parity_diff {
    const char *label; const char *region; uint64_t address; const char *symbol;
    const uint8_t *expected; const uint8_t *actual; size_t len;
};

/* The differences the last dj_check_parity found: one per differing byte, then one with a
 * label alone (a value kept in no one place), to test diffs without an address. */
static struct dj_parity_diff diffs[64];
static size_t ndiffs;
static uint8_t base_copy[64];
static const char *labels[] = {"frame counter", "buttons held", "mixed state"};

#ifndef EVENTS
/* Parity: the first 63 bytes of RAM match the baseline's (ram[63] counts boots). */
int dj_check_parity(const struct dj_baseline *b, uint64_t frame, char *reason, size_t len) {
    void *(*mem)(unsigned) = (void *(*)(unsigned))b->get_proc(b->ctx, "retro_get_memory_data");
    ndiffs = 0;
    if (!mem) { snprintf(reason, len, "no baseline RAM"); return 2; }
    const uint8_t *base = mem(RETRO_MEMORY_SYSTEM_RAM);
    memcpy(base_copy, base, sizeof base_copy);
    if (ram[0] == 10 && strcmp(break_setting(), "crash-check") == 0) *(volatile int *)0 = 1;
    for (unsigned i = 0; i < 63; i++)
        if (base[i] != ram[i]) {
            struct dj_parity_diff d = {i < 3 ? labels[i] : NULL, "RAM", i, NULL, base_copy + i, ram + i, 1};
            diffs[ndiffs++] = d;
        }
    if (ndiffs) {
        struct dj_parity_diff d = {"checksum of the state", NULL, 0, NULL, NULL, NULL, 0};
        diffs[ndiffs++] = d;
        unsigned i = (unsigned)diffs[0].address;
        snprintf(reason, len, "RAM 0x%02X is 0x%02X here, 0x%02X in the baseline (frame %llu)", i, ram[i], base[i], (unsigned long long)frame);
        return 1;
    }
    return 0;
}

#endif /* !EVENTS */

size_t dj_parity_diffs(struct dj_parity_diff *out, size_t cap) {
    for (size_t i = 0; i < ndiffs && i < cap; i++) out[i] = diffs[i];
    return ndiffs;
}

#ifndef EVENTS
/* The core option `testcore_break` makes the frame where ram[0] becomes 10 go wrong:
 * "mismatch" changes RAM there (a parity failure), "tick" fails the tick with a reason,
 * "crash" crashes the core, "hang" never returns, and "crash-check" crashes in
 * dj_check_parity on that frame. */
int dj_run(char *reason, size_t len) {
    retro_run();
    if (ram[0] != 10) return 0;
    const char *s = break_setting();
    if (strcmp(s, "mismatch") == 0) ram[2] ^= 0x55;
    if (strcmp(s, "tick") == 0) { snprintf(reason, len, "told to fail the tick"); return 1; }
    if (strcmp(s, "crash") == 0) *(volatile int *)0 = 1;
    if (strcmp(s, "hang") == 0) for (volatile int spin = 1; spin;) {}
    return 0;
}

int dj_write(const struct dj_baseline *b, const char *region, uint64_t address, const uint8_t *bytes, size_t n, char *reason, size_t len) {
    (void)b;
    if (strcmp(region, "RAM") != 0) return 1;
    if (address + n > sizeof ram) { snprintf(reason, len, "past the end"); return 2; }
    memcpy(ram + address, bytes, n);
    return 0;
}
#endif /* !EVENTS */

#ifdef EVENTS
/* ---- a comparison core that takes events (docs/connectors.md) ---- */
struct dj_baseline2 {
    uint32_t version; void *ctx;
    const uint8_t *(*memory)(void *ctx, const char *name, size_t *size);
    size_t (*registers)(void *ctx, void *out, size_t cap);
};
struct dj_event {
    uint32_t size, kind, flags, breakpoint, jump_reason;
    const char *segment; uint64_t frame; const struct dj_baseline2 *baseline;
};
struct dj_write {
    uint32_t size, flags; const char *segment; uint64_t frame; const struct dj_baseline2 *baseline;
    const char *region; uint64_t offset; const uint8_t *bytes; size_t len; const char *at_region; uint64_t at_offset;
};
struct dj_breakpoint { uint32_t id; const char *region; uint64_t offset; const char *name; };
enum { EV_JUMP = 1, EV_CONSOLE_RESET, EV_BREAK, EV_FRAME_END };
enum { PENDING, MATCH, MISMATCH, UNJUDGED, HALT };

/* What it has been sent, for the tests: "J1 " a jump with reason 1, "R " a console reset,
 * "B1 " a break with id 1, "F " a frame end, "W " a write, "Wa " a write at an instruction. */
static char events[8192];
static void note(const char *what) { if (strlen(events) + strlen(what) + 1 < sizeof events) strcat(events, what); }
const char *testcore_events(void) { return events; }

static const char *judge_setting(void) {
    struct retro_variable v = {"testcore_judge", NULL};
    if (env && env(RETRO_ENVIRONMENT_GET_VARIABLE, &v) && v.value) return v.value;
    return "frame";
}

/* Whether it has copied the baseline since the last jump. */
static bool live;
/* A write to make once its own tick has run: the baseline made it at that instruction. */
static bool write_waiting;
static uint8_t write_bytes[8];
static size_t write_len, write_at;
static bool halted_once;

static const uint8_t *base_ram(const struct dj_baseline2 *b) {
    size_t n = 0;
    const uint8_t *p = b->memory(b->ctx, "RAM", &n);
    return n == sizeof ram ? p : NULL;
}

/* Copies the baseline. The event it copies at is unjudged: nothing was compared. */
static int copy(const struct dj_baseline2 *b, char *reason, size_t len) {
    const uint8_t *base = base_ram(b);
    if (!base) { snprintf(reason, len, "the baseline has no RAM like mine"); return HALT; }
    memcpy(ram, base, sizeof ram);
    live = true;
    write_waiting = false;
    snprintf(reason, len, "copied the baseline");
    return UNJUDGED;
}

static int compare(const struct dj_baseline2 *b, char *reason, size_t len) {
    const uint8_t *base = base_ram(b);
    ndiffs = 0;
    if (!base) { snprintf(reason, len, "the baseline has no RAM like mine"); return HALT; }
    memcpy(base_copy, base, sizeof base_copy);
    for (unsigned i = 0; i < 63; i++)
        if (base[i] != ram[i]) {
            struct dj_parity_diff d = {i < 3 ? labels[i] : NULL, "RAM", i, NULL, base_copy + i, ram + i, 1};
            diffs[ndiffs++] = d;
        }
    if (!ndiffs) return MATCH;
    unsigned i = (unsigned)diffs[0].address;
    snprintf(reason, len, "RAM 0x%02X is 0x%02X here, 0x%02X in the baseline", i, ram[i], base[i]);
    return MISMATCH;
}

static uint8_t held_now(void) {
    uint8_t held = 0;
    for (unsigned b = 0; b < 8; b++)
        if (input && input(0, RETRO_DEVICE_JOYPAD, 0, b)) held |= (uint8_t)(1u << b);
    return held;
}

/* The core option `testcore_break` makes it go wrong once its frame counter reaches 10:
 * "mismatch" changes its RAM there, for good; "heal" does the same and copies the baseline
 * again when the counter reaches 14; "halt" answers halt, once; "unjudged" answers
 * unjudged, and goes stale, while the counter is 7, 8 or 9. Returns a verdict, or -1 to go
 * on and compare. */
static int go_wrong(char *reason, size_t len) {
    const char *s = break_setting();
    if (ram[0] == 10 && (!strcmp(s, "mismatch") || !strcmp(s, "heal"))) ram[2] ^= 0x55;
    if (ram[0] == 14 && !strcmp(s, "heal")) { live = false; snprintf(reason, len, "starting again"); return UNJUDGED; }
    if (ram[0] == 10 && !strcmp(s, "halt") && !halted_once) { halted_once = true; snprintf(reason, len, "told to halt"); return HALT; }
    if (ram[0] >= 7 && ram[0] <= 9 && !strcmp(s, "unjudged")) { live = false; snprintf(reason, len, "an overlay I don't know"); return UNJUDGED; }
    return -1;
}

int dj_handle_event(const struct dj_event *ev, char *reason, size_t len) {
    const char *judge = judge_setting();
    char what[16];
    switch (ev->kind) {
        case EV_JUMP: snprintf(what, sizeof what, "J%u ", ev->jump_reason); break;
        case EV_CONSOLE_RESET: snprintf(what, sizeof what, "R "); break;
        case EV_BREAK: snprintf(what, sizeof what, "B%u ", ev->breakpoint); break;
        default: snprintf(what, sizeof what, "F "); break;
    }
    note(what);
    if (!strcmp(judge, "copy")) {
        int r = copy(ev->baseline, reason, len);
        if (r == HALT) return r;
        reason[0] = 0;
        return ev->kind == EV_JUMP ? UNJUDGED : MATCH;
    }
    if (!strcmp(judge, "tick")) {
        /* Parked where the baseline's tick ends: between the two halves of a frame. */
        if (ev->kind == EV_JUMP || ev->kind == EV_CONSOLE_RESET) { live = false; return PENDING; }
        if (ev->kind != EV_BREAK || ev->breakpoint != 1) return PENDING;
        if (!live) return copy(ev->baseline, reason, len);
        /* The rest of the frame the last tick was in, then this frame up to its tick. */
        phase_b();
        phase_a(held_now());
        if (write_waiting) { memcpy(ram + write_at, write_bytes, write_len); write_waiting = false; }
        const uint8_t *base = base_ram(ev->baseline);
        if (base && base[0] != ram[0]) { live = false; snprintf(reason, len, "a lag frame: the baseline ran a frame with no tick"); return UNJUDGED; }
        int r = go_wrong(reason, len);
        return r >= 0 ? r : compare(ev->baseline, reason, len);
    }
    /* "frame": parked at a frame boundary. */
    if (ev->kind == EV_JUMP) return copy(ev->baseline, reason, len);
    if (ev->kind == EV_CONSOLE_RESET) { if (live) memset(ram, 0, sizeof ram); return PENDING; }
    if (ev->kind != EV_FRAME_END) return PENDING;
    if (!live) return copy(ev->baseline, reason, len);
    phase_a(held_now());
    phase_b();
    int r = go_wrong(reason, len);
    return r >= 0 ? r : compare(ev->baseline, reason, len);
}

#ifndef NO_WRITE_HANDLER
int dj_handle_write(const struct dj_write *w, char *reason, size_t len) {
    const char *judge = judge_setting();
    (void)reason; (void)len;
    note(w->at_region ? "Wa " : "W ");
    if (strcmp(w->region, "RAM") != 0 || w->offset + w->len > sizeof ram || w->len > sizeof write_bytes) { live = false; return PENDING; }
    if (!strcmp(judge, "copy")) return PENDING;
    if (!strcmp(judge, "tick")) {
        /* Only a write made at its own breakpoint lands where it is parked. It has yet to
         * run up to there, so the write waits until it has. */
        if (w->at_region && !strcmp(w->at_region, "PRG") && w->at_offset == 0x80 && live) {
            memcpy(write_bytes, w->bytes, w->len); write_len = w->len; write_at = (size_t)w->offset; write_waiting = true;
        } else live = false;
        return PENDING;
    }
    /* "frame": a write between frames lands where it is parked. */
    if (!w->at_region && live) memcpy(ram + w->offset, w->bytes, w->len);
    else live = false;
    return PENDING;
}
#endif

size_t dj_breakpoints(const struct dj_breakpoint **out) {
    static const struct dj_breakpoint list[] = {{1, "PRG", 0x80, "tick_end"}, {2, "PRG", 0x10, "frame_start"}};
    if (strcmp(judge_setting(), "tick") != 0) return 0;
    *out = list;
    return 2;
}
#endif /* EVENTS */
#endif
