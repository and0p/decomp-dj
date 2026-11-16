/* The dj comparison-core API, as the decomp uses it.
 *
 * PROVISIONAL. This copies the shapes in .agents/yield-parity/PLAN.md (parity by
 * breakpoints, sections 1-4 and 12), which is not built in dj yet. When dj ships its own
 * header, this file goes and the decomp includes that one. Keep the two in step until then.
 *
 * Copyright 2026 the decomp-dj developers. GPL-3.0-or-later.
 */
#ifndef THW_DJ_API_H
#define THW_DJ_API_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

/* Raised from today's 2 by the breakpoint design (its section 12). Today's dj ignores a
 * connector of another version with a warning, which is what should happen until then. */
#define DJ_API_VERSION 3

/* ---- Coverage (unchanged from docs/connectors.md, minus the tick functions) ---------- */

struct dj_region {
    const char    *name;
    uint8_t       *flags;
    size_t         size;
    const char    *cpu;
    const uint8_t *data;
    int64_t        file_offset;
};

/* ---- Section 1: breakpoints ------------------------------------------------------------ */

struct dj_breakpoint {
    uint32_t    id;       /* the core's own number, handed back in the event */
    const char *region;   /* the baseline connector's region: "PRG" */
    uint64_t    offset;
    const char *name;     /* for dj's messages only */
};

/* ---- Section 2: events and the handle -------------------------------------------------- */

enum dj_event_kind  { DJ_EVENT_JUMP = 1, DJ_EVENT_CONSOLE_RESET, DJ_EVENT_BREAK, DJ_EVENT_FRAME_END };
enum dj_jump_reason { DJ_JUMP_POWER_ON = 1, DJ_JUMP_STATE, DJ_JUMP_COPY_AGAIN };
enum dj_verdict     { DJ_PENDING = 0, DJ_MATCH, DJ_MISMATCH, DJ_UNJUDGED, DJ_HALT };

#define DJ_PREROLL 1u

struct dj_baseline {
    uint32_t version;
    void    *ctx;
    /* A memory region by the name [WRITE] and [hash] use ("RAM"). NULL if none. Read only. */
    const uint8_t *(*memory)(void *ctx, const char *name, size_t *size);
    /* The CPU's registers. Returns the bytes written, 0 if the connector can't say. For
     * the 6502, 7 bytes: A, X, Y, SP, P, then PC low byte first (docs/connectors.md,
     * `dj_read_registers`, connector API 3). */
    size_t (*registers)(void *ctx, void *out, size_t cap);
};

struct dj_event {
    uint32_t size;
    uint32_t kind;          /* enum dj_event_kind */
    uint32_t flags;         /* DJ_PREROLL */
    uint32_t breakpoint;    /* DJ_EVENT_BREAK: the core's own id */
    uint32_t jump_reason;   /* DJ_EVENT_JUMP: enum dj_jump_reason */
    const char *segment;
    uint64_t    frame;
    const struct dj_baseline *baseline;
};

/* ---- Section 3: writes ----------------------------------------------------------------- */

struct dj_write {
    uint32_t size;
    uint32_t flags;
    const char *segment;
    uint64_t    frame;
    const struct dj_baseline *baseline;
    const char    *region;
    uint64_t       offset;
    const uint8_t *bytes;
    size_t         len;
    const char    *at_region;   /* the instruction address, or NULL if the line had none */
    uint64_t       at_offset;
};

/* ---- Section 4: what differed ---------------------------------------------------------- */

struct dj_parity_diff {
    const char    *label;
    const char    *region;
    uint64_t       address;
    const char    *symbol;
    const uint8_t *expected;
    const uint8_t *actual;
    size_t         len;
};

/* ---- What the decomp exports ----------------------------------------------------------- */

#if defined(_WIN32)
#define DJ_EXPORT __declspec(dllexport)
#else
#define DJ_EXPORT __attribute__((visibility("default")))
#endif

DJ_EXPORT uint32_t    dj_api_version(void);
DJ_EXPORT const char *dj_build_id(void);
DJ_EXPORT size_t      dj_region_count(void);
DJ_EXPORT bool        dj_get_region(size_t index, struct dj_region *out);
DJ_EXPORT void        dj_clear(void);
DJ_EXPORT size_t      dj_breakpoints(const struct dj_breakpoint **out);
DJ_EXPORT int         dj_handle_event(const struct dj_event *ev, char *reason, size_t reason_len);
DJ_EXPORT int         dj_handle_write(const struct dj_write *w, char *reason, size_t reason_len);
DJ_EXPORT size_t      dj_parity_diffs(struct dj_parity_diff *out, size_t cap);
DJ_EXPORT bool        dj_trace_start(size_t capacity);
DJ_EXPORT void        dj_trace_stop(void);
DJ_EXPORT size_t      dj_trace_read(const void **events, uint64_t *dropped);

#endif
