/* The comparison-core API: breakpoints, events, writes, diffs, coverage and traces.
 *
 * How the decomp follows the original (.agents/THWAITE_DECOMP_PLAN.md, "How it runs"):
 * it is parked at a wait loop's exit. When the original reaches the next wait's exit, the
 * decomp runs its own step to there, with the raw input and NMI-counter values the
 * original read on the way (queued at their breakpoints), and compares. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "compare.h"
#include "core.h"
#include "dj_api.h"
#include "map.h"

#ifndef THW_BUILD_ID
#define THW_BUILD_ID "thwaite-decomp@dev"
#endif

uint32_t dj_api_version(void) { return DJ_API_VERSION; }
const char *dj_build_id(void) { return THW_BUILD_ID; }

/* ---- The breakpoint table --------------------------------------------------------------- */

enum bp_role { BP_WAIT = 1, BP_NMIS, BP_PADS, BP_MOUSE, BP_SOUND };

#define MAX_BPS 64
static struct dj_breakpoint bps[MAX_BPS];
static struct { uint8_t role, wait; } bp_info[MAX_BPS];
static size_t nbps;

static const struct { const char *name; uint8_t wait; } WAIT_NAMES[] = {
    {"wait:display_todo", W_TODO},       {"wait:titleScreen", W_TITLE},
    {"wait:screen_off_title_palette", W_SCREEN_OFF}, {"wait:main", W_MAIN},
    {"wait:pauseScreen", W_PAUSE},       {"wait:practice_menu", W_PRACTICE},
    {"wait:load_cutscene.1", W_CUT1},    {"wait:load_cutscene.2", W_CUT2},
    {"wait:ppu_wait_vblank", W_VBLANK},
};

static const char *const WAIT_LABEL[W_COUNT] = {
    "(not written yet)", "display_todo", "titleScreen", "screen_off_title_palette", "main",
    "pauseScreen", "practice_menu", "load_cutscene (1)", "load_cutscene (2)", "ppu_wait_vblank",
};

static void build_table(void) {
    if (nbps) return;
    for (unsigned i = 0; i < thw_bp_count && nbps < MAX_BPS; i++) {
        const struct thw_bp *b = &thw_bps[i];
        bps[nbps] = (struct dj_breakpoint){(uint32_t)nbps, "PRG", b->prg, b->name};
        bp_info[nbps].role = b->kind == THW_BP_WAIT ? BP_WAIT : b->kind == THW_BP_NMIS_READ ? BP_NMIS
                             : strcmp(b->name, "input:read_mouse") == 0 ? BP_MOUSE
                             : strncmp(b->name, "sound:", 6) == 0      ? BP_SOUND
                                                                       : BP_PADS;
        for (size_t k = 0; k < sizeof WAIT_NAMES / sizeof *WAIT_NAMES; k++)
            if (strcmp(b->name, WAIT_NAMES[k].name) == 0) bp_info[nbps].wait = WAIT_NAMES[k].wait;
        nbps++;
    }
}

size_t dj_breakpoints(const struct dj_breakpoint **out) {
    build_table();
    *out = bps;
    return nbps;
}

/* The wait whose spin loop holds PRG offset `pc`, or W_NONE. */
static uint8_t wait_spinning_at(int64_t pc) {
    for (unsigned i = 0; i < thw_bp_count; i++) {
        const struct thw_bp *b = &thw_bps[i];
        if (b->kind == THW_BP_WAIT && pc >= b->spin_lo && pc < b->spin_hi)
            for (size_t k = 0; k < sizeof WAIT_NAMES / sizeof *WAIT_NAMES; k++)
                if (strcmp(b->name, WAIT_NAMES[k].name) == 0) return WAIT_NAMES[k].wait;
    }
    return W_NONE;
}

/* Waits the decomp can copy the original at: loops whose whole state is in RAM. (What is
 * only in VRAM, such as a cutscene's text already drawn, the redraw works out from RAM.) */
static bool copyable(uint8_t w) {
    return w == W_TODO || w == W_TITLE || w == W_MAIN || w == W_PAUSE || w == W_PRACTICE || w == W_CUT1 ||
           w == W_CUT2;
}

/* ---- What the original read during the step ------------------------------------------------ */

#define QN 64
static struct {
    bool live;       /* the decomp has copied the original since the last jump */
    bool fresh;      /* copied between frames, in the spin of g.wait: its exit is no step */
    uint8_t pads[QN][2];
    uint8_t mouse[QN][4];   /* port, then the report */
    uint8_t nmis[QN];
    uint8_t sound[QN];
    unsigned npads, nmouse, nnmis, nsound, ipads, imouse, inmis, isound;
    /* [WRITE] lines that landed after the step the original was in: applied after the
     * decomp runs that step. */
    struct { uint16_t addr; uint8_t len; uint8_t bytes[16]; } writes[8];
    unsigned nwrites;
} E;

static void clear_queues(void) {
    E.npads = E.nmouse = E.nnmis = E.nsound = E.ipads = E.imouse = E.inmis = E.isound = 0;
    E.nwrites = 0;
}

static bool q_pads(void *ctx, uint8_t raw[2]) {
    (void)ctx;
    if (E.ipads >= E.npads) return false;
    memcpy(raw, E.pads[E.ipads++], 2);
    return true;
}
static bool q_mouse(void *ctx, int port, uint8_t raw[3]) {
    (void)ctx;
    if (E.imouse >= E.nmouse || E.mouse[E.imouse][0] != port) return false;
    memcpy(raw, E.mouse[E.imouse++] + 1, 3);
    return true;
}
static bool q_nmis(void *ctx, uint8_t *v) {
    (void)ctx;
    if (E.inmis >= E.nnmis) return false;
    *v = E.nmis[E.inmis++];
    return true;
}
static bool q_sound(void *ctx, uint8_t *v) {
    (void)ctx;
    if (E.isound >= E.nsound) return false;
    *v = E.sound[E.isound++];
    return true;
}
static const struct step_io QUEUED = {NULL, q_pads, q_mouse, q_nmis, q_sound};

/* ---- The baseline ---------------------------------------------------------------------- */

static const uint8_t *baseline_ram(const struct dj_baseline *b) {
    size_t size = 0;
    const uint8_t *ram = b && b->version >= 2 && b->memory ? b->memory(b->ctx, "RAM", &size) : NULL;
    return ram && size >= THW_RAM_SIZE ? ram : NULL;
}

/* The baseline's 6502 registers: A, X, Y, SP, P, PC (low byte first). */
static bool baseline_regs(const struct dj_baseline *b, uint8_t r[7]) {
    return b && b->version >= 2 && b->registers && b->registers(b->ctx, r, 7) >= 7;
}

static int copy_at(uint8_t w, const uint8_t *ram, bool fresh, char *reason, size_t len) {
    compare_import(&thw_game, ram);
    thw_game.wait = w;
    E.live = true;
    E.fresh = fresh;
    clear_queues();
    game_redraw(&thw_game);
    core_present();
    snprintf(reason, len, "copied the original at %s%s", WAIT_LABEL[w], fresh ? ", between frames" : "");
    return DJ_UNJUDGED;
}

static void trace_before_step(void);
static void trace_after_step(void);

/* ---- Events ------------------------------------------------------------------------------ */

static int on_wait(uint8_t w, const uint8_t *ram, char *reason, size_t len) {
    compare_clear_diffs();
    if (!E.live) {
        if (copyable(w)) return copy_at(w, ram, false, reason, len);
        return DJ_PENDING;
    }
    if (E.fresh && w == thw_game.wait) {
        E.fresh = false;   /* the original only left the spin: nothing ran */
    } else {
        E.fresh = false;
        trace_before_step();
        uint8_t next = (uint8_t)game_step(&thw_game, &QUEUED);
        trace_after_step();
        if (next == W_NONE) {
            const char *why = game_io_error() ? game_io_error() : game_unwritten();
            E.live = false;
            clear_queues();
            if (game_io_error()) {
                /* The decomp went another way than the original earlier in this step. Its
                 * state is partway through the step, but what it got wrong on the way
                 * (often the field that sent it the other way) shows in a compare, and
                 * --explain can trace those. */
                size_t n = compare_check(&thw_game, ram, NULL, 0);
                snprintf(reason, len, "%s (on the way to %s)%s", why, WAIT_LABEL[w],
                         n ? "; the diffs are of the decomp's state partway through its step" : "");
                return DJ_MISMATCH;
            }
            if (copyable(w)) {
                char r2[128];
                copy_at(w, ram, false, r2, sizeof r2);
                snprintf(reason, len, "not written yet: %s; %s", why, r2);
                return DJ_UNJUDGED;
            }
            snprintf(reason, len, "not written yet: %s", why);
            return DJ_UNJUDGED;
        }
        if (next != w) {
            snprintf(reason, len, "the decomp went to %s, the original to %s", WAIT_LABEL[next], WAIT_LABEL[w]);
            clear_queues();
            return DJ_MISMATCH;
        }
        for (unsigned i = 0; i < E.nwrites; i++) compare_write(&thw_game, E.writes[i].addr, E.writes[i].bytes, E.writes[i].len);
    }
    bool unread = E.ipads < E.npads || E.imouse < E.nmouse || E.inmis < E.nnmis || E.isound < E.nsound;
    clear_queues();
    compare_take_baseline(&thw_game, ram);
    core_present();
    if (compare_check(&thw_game, ram, reason, len)) return DJ_MISMATCH;
    if (unread) {
        snprintf(reason, len, "the original read input or the NMI counter that the decomp didn't, on the way to %s", WAIT_LABEL[w]);
        return DJ_MISMATCH;
    }
    return DJ_MATCH;
}

int dj_handle_event(const struct dj_event *ev, char *reason, size_t reason_len) {
    if (!ev || ev->size < sizeof *ev) {
        snprintf(reason, reason_len, "the event is from another version of the API");
        return DJ_HALT;
    }
    build_table();
    const uint8_t *ram = baseline_ram(ev->baseline);
    switch (ev->kind) {
    case DJ_EVENT_JUMP: {
        E.live = false;
        clear_queues();
        if (ev->jump_reason == DJ_JUMP_POWER_ON) game_power_on(&thw_game);
        uint8_t r[7];
        if (ram && baseline_regs(ev->baseline, r)) {
            uint8_t w = wait_spinning_at((int64_t)(r[5] | r[6] << 8) - 0x8000);
            if (copyable(w)) return copy_at(w, ram, true, reason, reason_len);
        }
        return DJ_PENDING;
    }
    case DJ_EVENT_CONSOLE_RESET:
        E.live = false;   /* parked mid-frame: copy again at the next wait */
        clear_queues();
        return DJ_PENDING;
    case DJ_EVENT_FRAME_END:
        return DJ_PENDING;
    case DJ_EVENT_BREAK:
        break;
    default:
        return DJ_PENDING;
    }
    if (ev->breakpoint >= nbps) return DJ_PENDING;
    if (!ram) {
        snprintf(reason, reason_len, "the baseline gives no RAM through the handle");
        return DJ_HALT;
    }
    switch (bp_info[ev->breakpoint].role) {
    case BP_WAIT:
        return on_wait(bp_info[ev->breakpoint].wait, ram, reason, reason_len);
    case BP_PADS:
        if (E.live && E.npads < QN) {
            E.pads[E.npads][0] = ram[0x00];
            E.pads[E.npads][1] = ram[0x01];
            E.npads++;
        }
        return DJ_PENDING;
    case BP_MOUSE: {
        uint8_t r[7];
        if (E.live && E.nmouse < QN && baseline_regs(ev->baseline, r)) {
            E.mouse[E.nmouse][0] = r[1];   /* X: the port */
            memcpy(E.mouse[E.nmouse] + 1, ram + 1, 3);
            E.nmouse++;
        }
        return DJ_PENDING;
    }
    case BP_NMIS:
        if (E.live && E.nnmis < QN) E.nmis[E.nnmis++] = ram[THW_NMIS_ADDR];
        return DJ_PENDING;
    case BP_SOUND:
        if (E.live && E.nsound < QN) E.sound[E.nsound++] = ram[0x02];
        return DJ_PENDING;
    }
    return DJ_PENDING;
}

int dj_handle_write(const struct dj_write *w, char *reason, size_t reason_len) {
    if (!w || w->size < sizeof *w) {
        snprintf(reason, reason_len, "the write is from another version of the API");
        return DJ_HALT;
    }
    if (strcmp(w->region, "RAM") != 0) {
        snprintf(reason, reason_len, "a write to %s: the decomp only follows RAM", w->region);
        return DJ_HALT;
    }
    if (!E.live) return DJ_PENDING;   /* it copies the original, write and all, at its next wait */
    if (w->at_region) {
        /* At an instruction: exact when it is the exit of the wait the decomp is parked at. */
        for (unsigned i = 0; i < nbps; i++)
            if (bp_info[i].role == BP_WAIT && bp_info[i].wait == thw_game.wait && bps[i].offset == w->at_offset) {
                compare_write(&thw_game, (uint16_t)w->offset, w->bytes, w->len);
                return DJ_PENDING;
            }
    } else {
        /* Between frames, with the original in a wait's spin: its step is done and the write
         * lands after it, so the decomp applies it after running that step. */
        uint8_t r[7];
        if (baseline_regs(w->baseline, r) && wait_spinning_at((int64_t)(r[5] | r[6] << 8) - 0x8000) != W_NONE &&
            E.nwrites < 8 && w->len <= 16) {
            E.writes[E.nwrites].addr = (uint16_t)w->offset;
            E.writes[E.nwrites].len = (uint8_t)w->len;
            memcpy(E.writes[E.nwrites].bytes, w->bytes, w->len);
            E.nwrites++;
            return DJ_PENDING;
        }
    }
    E.live = false;
    clear_queues();
    snprintf(reason, reason_len, "a write landed in the middle of a step: copying the original again at its next wait");
    return DJ_UNJUDGED;
}

size_t dj_parity_diffs(struct dj_parity_diff *out, size_t cap) { return compare_diffs(out, cap); }

/* ---- Coverage ------------------------------------------------------------------------- */

/* SRC: one flag a block of the C code, by the id a COV() site registers. None yet. */
#define SRC_BLOCKS 4096
static uint8_t src_flags[SRC_BLOCKS];
static uint8_t ram_flags[THW_RAM_SIZE];

size_t dj_region_count(void) { return 2; }

bool dj_get_region(size_t index, struct dj_region *out) {
    if (index == 0) {
        *out = (struct dj_region){"SRC", src_flags, SRC_BLOCKS, NULL, NULL, -1};
        return true;
    }
    if (index == 1) {
        *out = (struct dj_region){"RAM", ram_flags, sizeof ram_flags, NULL, core_ram_view(), -1};
        return true;
    }
    return false;
}

void dj_clear(void) {
    memset(src_flags, 0, sizeof src_flags);
    memset(ram_flags, 0, sizeof ram_flags);
}

/* ---- Traces ------------------------------------------------------------------------------
 *
 * The decomp has no instructions to trace, so its trace is what its steps did to the
 * original's RAM layout: after each step, a write event for every byte of the 2 KB view
 * that changed, in address order (region "RAM", `cycle` the step's number). dj traces only
 * the baseline today; this is the decomp's side, for when it traces both. */

static struct {
    bool on;
    struct dj_trace_event *buf;
    size_t cap, n;
    uint64_t dropped;
    uint8_t before[THW_RAM_SIZE];
} T;

bool dj_trace_start(size_t capacity) {
    dj_trace_stop();
    T.buf = capacity ? calloc(capacity, sizeof *T.buf) : NULL;
    if (capacity && !T.buf) return false;
    T.cap = capacity;
    T.n = 0;
    T.dropped = 0;
    T.on = true;
    return true;
}

void dj_trace_stop(void) {
    free(T.buf);
    T.buf = NULL;
    T.cap = T.n = 0;
    T.on = false;
}

size_t dj_trace_read(const struct dj_trace_event **events, uint64_t *dropped) {
    static struct dj_trace_event *out;
    static size_t out_cap;
    if (out_cap < T.n) {
        free(out);
        out = malloc(T.n * sizeof *out);
        out_cap = out ? T.n : 0;
    }
    size_t n = out ? T.n : 0;
    if (n) memcpy(out, T.buf, n * sizeof *out);
    if (events) *events = out;
    if (dropped) *dropped = T.dropped;
    T.n = 0;
    T.dropped = 0;
    return n;
}

static void trace_before_step(void) {
    if (T.on) compare_export(&thw_game, T.before);
}

static void trace_after_step(void) {
    if (!T.on) return;
    static uint8_t after[THW_RAM_SIZE];
    compare_export(&thw_game, after);
    for (unsigned a = 0; a < THW_RAM_SIZE; a++) {
        if (after[a] == T.before[a]) continue;
        if (T.n >= T.cap) {
            T.dropped++;
            continue;
        }
        T.buf[T.n++] = (struct dj_trace_event){.cycle = thw_game.steps, .offset = a, .addr = (uint16_t)a,
                                               .kind = DJ_TRACE_WRITE, .region = 1, .value = after[a]};
    }
}
