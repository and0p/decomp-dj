/*
 * dj_compare.h: for a comparison core (a decomp or port built as a libretro core) that
 * decomp-dj runs beside the original game.
 *
 * Two parts:
 *
 * 1. The API, as docs/connectors.md describes it ("Comparison cores"): the structs dj
 *    passes and the functions a core exports. dj finds the functions by name.
 * 2. A helper, which is not part of the API. A core is not synced by dj: after a jump it
 *    copies what it needs from the baseline, answers "unjudged" for that event, and from
 *    then on runs itself and compares. That is the same few lines in every core that
 *    judges at one place, so dj_helper_event does them. A core that needs more writes its
 *    own dj_handle_event and ignores the helper.
 *
 * A core that also reports its own coverage or traces exports the connector API as well:
 * that is in dj_connector.h, beside this file.
 *
 * MIT license, like dj_decoder.h. Copy this header into your core, or include it from
 * dj's include.
 */
#ifndef DJ_COMPARE_H
#define DJ_COMPARE_H

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- 1. The API ------------------------------------------------------------------- */

/* The baseline, readable for the length of a call. Never write through it. */
struct dj_baseline {
    uint32_t version; /* 2 */
    void *ctx;
    /* A memory region by the name [WRITE] and [hash] use: "RAM", "SRAM", "VRAM", "RTC".
     * NULL if the baseline has none. */
    const uint8_t *(*memory)(void *ctx, const char *name, size_t *size);
    /* The CPU's registers, as the connector's dj_read_registers lays them out (6502: A, X,
     * Y, SP, P, PC low, PC high). Returns the bytes written, 0 if the connector can't say. */
    size_t (*registers)(void *ctx, void *out, size_t cap);
};

enum dj_event_kind { DJ_EVENT_JUMP = 1, DJ_EVENT_CONSOLE_RESET, DJ_EVENT_BREAK, DJ_EVENT_FRAME_END };
enum dj_jump_reason { DJ_JUMP_POWER_ON = 1, DJ_JUMP_STATE, DJ_JUMP_COPY_AGAIN };
enum dj_verdict { DJ_PENDING = 0, DJ_MATCH, DJ_MISMATCH, DJ_UNJUDGED, DJ_HALT };

#define DJ_PREROLL 1u /* in `flags`: this frame is pre-roll, and only a halt counts */

struct dj_event {
    uint32_t size;        /* sizeof(struct dj_event), so fields can be added later */
    uint32_t kind;        /* enum dj_event_kind */
    uint32_t flags;
    uint32_t breakpoint;  /* DJ_EVENT_BREAK: the core's own id */
    uint32_t jump_reason; /* DJ_EVENT_JUMP: enum dj_jump_reason */
    const char *segment;  /* the segment's id */
    uint64_t frame;       /* the frame about to run, running, or just run */
    const struct dj_baseline *baseline;
};

struct dj_breakpoint {
    uint32_t id;        /* the core's own number, handed back in the event */
    const char *region; /* the baseline connector's region: "PRG", "RDRAM" */
    uint64_t offset;    /* in the region, not a CPU address */
    const char *name;   /* for dj's messages */
};

struct dj_write {
    uint32_t size, flags;
    const char *segment;
    uint64_t frame;
    const struct dj_baseline *baseline;
    const char *region; /* what was written: "RAM", "SRAM" */
    uint64_t offset;
    const uint8_t *bytes;
    size_t len;
    const char *at_region; /* the instruction it was made at, or NULL if the line had none */
    uint64_t at_offset;
};

struct dj_parity_diff {
    const char *label;       /* optional plain-text name: "Player 2 x coordinate" */
    const char *region;      /* the baseline connector's region: "RAM", "RDRAM" (NULL: no one place) */
    uint64_t address;        /* offset in that region. With a region, dj can trace who wrote it */
    const char *symbol;      /* optional: a data symbol of the original's, shown beside the address */
    const uint8_t *expected; /* the original's bytes (may be NULL) */
    const uint8_t *actual;   /* the core's bytes (may be NULL) */
    size_t len;
};

#ifndef DJ_EXPORT
#if defined(_WIN32)
#define DJ_EXPORT __declspec(dllexport)
#else
#define DJ_EXPORT __attribute__((visibility("default")))
#endif
#endif

/* What a core exports. Only dj_handle_event is needed.
 *
 * dj_handle_event: an event in the baseline. Run yourself up to it, compare, and return an
 *   enum dj_verdict. `reason` is text for unjudged, mismatch and halt.
 * dj_breakpoints: called once, after the game is loaded. Where in the original to be
 *   called (DJ_EVENT_BREAK), before the instruction there runs. Returns how many.
 * dj_handle_write: a [WRITE] line was applied to the baseline. Returns an enum dj_verdict.
 *   Without it, dj sends a jump (DJ_JUMP_COPY_AGAIN) in its place.
 * dj_parity_diffs: after a mismatch or a halt, what differed: fill up to `cap` and return
 *   how many there are in all. */
DJ_EXPORT int dj_handle_event(const struct dj_event *ev, char *reason, size_t reason_len);
DJ_EXPORT size_t dj_breakpoints(const struct dj_breakpoint **out);
DJ_EXPORT int dj_handle_write(const struct dj_write *w, char *reason, size_t reason_len);
DJ_EXPORT size_t dj_parity_diffs(struct dj_parity_diff *out, size_t cap);

/* ---- 2. The helper ---------------------------------------------------------------- */

/* A baseline region of at least `min` bytes, or NULL. */
static inline const uint8_t *dj_baseline_memory(const struct dj_baseline *b, const char *name, size_t min)
{
    size_t n = 0;
    const uint8_t *p = b && b->memory ? b->memory(b->ctx, name, &n) : NULL;
    return p && n >= min ? p : NULL;
}

/*
 * A core that judges at one place: at every frame end, or at one of its breakpoints. Fill
 * in the first fields and call dj_helper_event from dj_handle_event.
 *
 * What it does with each event:
 *
 * | Event            | Judging at frame ends            | Judging at a breakpoint           |
 * | jump             | copy now; unjudged               | stale; pending (*)                |
 * | console reset    | `reset`, or stale; pending       | stale; pending                    |
 * | the judging event| stale: copy; unjudged.  Live: `run`, then `compare`                  |
 * | any other        | pending                          | pending                           |
 *
 * (*) A power-on jump is offered to `copy` there too: a core may prefer to boot itself.
 *
 * "Stale" means the core's state is not the baseline's, and it copies at the next judging
 * event. A halt from `run` or `compare` leaves it stale. After a mismatch it is left as it
 * is and carries on; set `live` to 0 in `compare` to copy again instead. Set `live` to 0
 * yourself wherever else the core loses the baseline (a write it can't follow).
 */
struct dj_helper {
    uint32_t judge_at; /* 0: frame ends. Otherwise the id of the breakpoint to judge at */
    void *user;        /* handed to the functions below */
    /* Take over from the baseline as it is now (or, at a power-on jump, boot). 0: done.
     * Anything else: can't here, with `reason` saying why; it is asked again at the next
     * judging event. */
    int (*copy)(void *user, const struct dj_event *ev, char *reason, size_t reason_len);
    /* Run from the last judging point to this one. DJ_PENDING: done, go on and compare.
     * Any other verdict is given as it is (DJ_HALT where the core can't go on). */
    int (*run)(void *user, const struct dj_event *ev, char *reason, size_t reason_len);
    /* Compare with the baseline and return the verdict. */
    int (*compare)(void *user, const struct dj_event *ev, char *reason, size_t reason_len);
    /* Optional. The console's reset button, for a core judging at frame ends: run your own
     * reset path. 0: followed. Anything else, or no function: stale. */
    int (*reset)(void *user);
    int live; /* the core holds the baseline's state, and what it runs can be judged */
};

static inline int dj_helper_copy_(struct dj_helper *h, const struct dj_event *ev, char *reason, size_t len)
{
    if (len)
        reason[0] = 0;
    h->live = h->copy(h->user, ev, reason, len) == 0;
    if (h->live && len && !reason[0])
        snprintf(reason, len, "copied the baseline");
    return DJ_UNJUDGED; /* nothing was compared here */
}

static inline int dj_helper_event(struct dj_helper *h, const struct dj_event *ev, char *reason, size_t reason_len)
{
    int judging = h->judge_at ? ev->kind == DJ_EVENT_BREAK && ev->breakpoint == h->judge_at : ev->kind == DJ_EVENT_FRAME_END;
    if (ev->kind == DJ_EVENT_JUMP) {
        h->live = 0;
        if (!h->judge_at || ev->jump_reason == DJ_JUMP_POWER_ON)
            return dj_helper_copy_(h, ev, reason, reason_len);
        return DJ_PENDING;
    }
    if (ev->kind == DJ_EVENT_CONSOLE_RESET) {
        if (h->judge_at || !h->live || !h->reset || h->reset(h->user) != 0)
            h->live = 0;
        return DJ_PENDING;
    }
    if (!judging)
        return DJ_PENDING;
    if (!h->live)
        return dj_helper_copy_(h, ev, reason, reason_len);
    int v = h->run(h->user, ev, reason, reason_len);
    if (v == DJ_PENDING)
        v = h->compare(h->user, ev, reason, reason_len);
    if (v == DJ_HALT)
        h->live = 0;
    return v;
}

#ifdef __cplusplus
}
#endif

#endif
