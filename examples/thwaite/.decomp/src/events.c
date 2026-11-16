/* The comparison-core API: breakpoints, events, writes, diffs, coverage and traces.
 *
 * Milestone 0: the breakpoint table is real (generated from thwaite.dbg); the events and
 * writes are stubs that say so. Coverage of the C code (SRC) has its region but no
 * instrumented blocks yet. */
#include <stdio.h>
#include <string.h>

#include "core.h"
#include "dj_api.h"
#include "map.h"

#ifndef THW_BUILD_ID
#define THW_BUILD_ID "thwaite-decomp@dev"
#endif

uint32_t dj_api_version(void) { return DJ_API_VERSION; }
const char *dj_build_id(void) { return THW_BUILD_ID; }

/* ---- Breakpoints ---------------------------------------------------------------------- */

#define MAX_BPS 64
static struct dj_breakpoint bps[MAX_BPS];
static size_t nbps;

size_t dj_breakpoints(const struct dj_breakpoint **out) {
    if (!nbps) {
        for (unsigned i = 0; i < thw_bp_count && i < MAX_BPS; i++)
            bps[nbps++] = (struct dj_breakpoint){thw_bps[i].id, "PRG", thw_bps[i].prg, thw_bps[i].name};
    }
    *out = bps;
    return nbps;
}

static const struct thw_bp *bp_by_id(uint32_t id) {
    for (unsigned i = 0; i < thw_bp_count; i++)
        if (thw_bps[i].id == id) return &thw_bps[i];
    return NULL;
}

/* ---- Events and writes ----------------------------------------------------------------- */

int dj_handle_event(const struct dj_event *ev, char *reason, size_t reason_len) {
    if (!ev || ev->size < sizeof *ev) {
        snprintf(reason, reason_len, "the event is from another version of the API");
        return DJ_HALT;
    }
    switch (ev->kind) {
    case DJ_EVENT_JUMP:
    case DJ_EVENT_CONSOLE_RESET:
    case DJ_EVENT_FRAME_END:
        return DJ_PENDING;
    case DJ_EVENT_BREAK: {
        const struct thw_bp *b = bp_by_id(ev->breakpoint);
        if (b && b->kind == THW_BP_NMIS_READ) return DJ_PENDING;
        snprintf(reason, reason_len, "not written yet: the step that ends at %s (milestone 0)", b ? b->name : "an unknown breakpoint");
        return DJ_HALT;
    }
    }
    return DJ_PENDING;
}

int dj_handle_write(const struct dj_write *w, char *reason, size_t reason_len) {
    (void)w;
    snprintf(reason, reason_len, "not written yet: following writes (milestone 2)");
    return DJ_HALT;
}

size_t dj_parity_diffs(struct dj_parity_diff *out, size_t cap) {
    (void)out;
    (void)cap;
    return 0;
}

/* ---- Coverage ------------------------------------------------------------------------- */

/* SRC: one flag a block of the C code, by the id a COV() site registers. None yet. */
#define SRC_BLOCKS 4096
static uint8_t src_flags[SRC_BLOCKS];
static uint8_t ram_flags[2048];

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

/* ---- Traces: not yet (milestone 4) ---------------------------------------------------- */

bool dj_trace_start(size_t capacity) {
    (void)capacity;
    return false;
}
void dj_trace_stop(void) {}
size_t dj_trace_read(const void **events, uint64_t *dropped) {
    *events = NULL;
    *dropped = 0;
    return 0;
}
