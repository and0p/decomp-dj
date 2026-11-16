/*
 * dj_connector.h: the connector API of decomp-dj, as docs/connectors.md describes it ("The
 * API" and "Traces"): what a core exports so that dj can see which code ran, stop it at
 * breakpoints, read its registers and trace it.
 *
 * Who needs it:
 *
 * - A comparison core (a decomp or port) that reports its own coverage or traces. It
 *   exports these beside the functions in dj_compare.h.
 * - A connector written for an emulator. dj's own (connectors/mesen and the others) are
 *   patches with their own copies of these declarations.
 *
 * dj finds the functions by name, through the core's proc address callback
 * (RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK), then among the library's exports. A core
 * defines the ones it has. dj_api_version, dj_region_count, dj_get_region, dj_clear and
 * dj_set_breakpoints make a connector; the rest are optional.
 *
 * MIT license, like dj_compare.h. Copy this header into your core, or include it from
 * dj's connectors/include.
 */
#ifndef DJ_CONNECTOR_H
#define DJ_CONNECTOR_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#ifndef DJ_EXPORT
#if defined(_WIN32)
#define DJ_EXPORT __declspec(dllexport)
#else
#define DJ_EXPORT __attribute__((visibility("default")))
#endif
#endif

/* What dj_api_version returns. A connector of another version is ignored, with a warning. */
#define DJ_API_VERSION 3

/* ---- Coverage --------------------------------------------------------------------- */

#define DJ_FLAG_OPCODE 0x01  /* executed as the first byte of an instruction */
#define DJ_FLAG_OPERAND 0x02 /* executed as another byte of an instruction */

struct dj_region {
    const char *name;    /* "PRG", "RAM", "RDRAM": a contract per system (docs/connectors.md) */
    uint8_t *flags;      /* one byte of DJ_FLAG_* per byte of the region, owned by the core */
    size_t size;
    const char *cpu;     /* the instruction set that runs here ("6502", "mips3", "armv4t"), or NULL */
    const uint8_t *data; /* the region's bytes in the CPU's byte order, or NULL */
    int64_t file_offset; /* where offset 0 lies in the ROM file, or -1 */
};

DJ_EXPORT uint32_t dj_api_version(void);
DJ_EXPORT size_t dj_region_count(void);
DJ_EXPORT bool dj_get_region(size_t index, struct dj_region *out);
DJ_EXPORT void dj_clear(void); /* zero every flag */

/* ---- Breakpoints ------------------------------------------------------------------ */

struct dj_break {
    int32_t region; /* index as in dj_get_region */
    int64_t offset; /* into that region */
};

/* Called when the instruction at a breakpoint is about to run, inside retro_run. The core
 * waits for it to return. Non-zero switches that breakpoint off until the list is set
 * again. */
typedef int (*dj_break_fn)(void *user, int region, int64_t offset);

/* Replaces the list. n = 0, or no function, turns breakpoints off. */
DJ_EXPORT void dj_set_breakpoints(const struct dj_break *list, size_t n, dj_break_fn fn, void *user);

/* ---- Optional --------------------------------------------------------------------- */

/* "key=value\n" lines: core options the connector needs while dj drives the core. */
DJ_EXPORT const char *dj_forced_options(void);
/* Which build this is, for a project's [core] build. */
DJ_EXPORT const char *dj_build_id(void);
/* The registers of the CPU that runs the code regions, in the layout docs/connectors.md
 * gives per instruction set. Returns the bytes written, 0 if it can't say. */
DJ_EXPORT size_t dj_read_registers(void *out, size_t cap);

/* ---- Traces (optional) ------------------------------------------------------------ */

#define DJ_TRACE_EXEC 1
#define DJ_TRACE_READ 2
#define DJ_TRACE_WRITE 3

/* One executed instruction, or one byte read or written. 40 bytes. The layout was made for
 * the 6502; docs/connectors.md says how a 32-bit CPU fills it. */
struct dj_trace_event {
    uint64_t cycle;
    int64_t offset;         /* offset in the region, -1 if none */
    uint16_t addr;          /* the CPU address (its low 16 bits) */
    uint8_t kind;           /* DJ_TRACE_* */
    int8_t region;          /* index as in dj_get_region, -1 if none */
    uint8_t value;          /* the byte read or written, or the opcode */
    uint8_t a, x, y, sp, p; /* 6502: the registers before an executed instruction */
    uint8_t operand_count;
    uint8_t operands[3];
    uint16_t addr_high;     /* the CPU address's high 16 bits, with 32-bit addresses; else 0 */
    uint8_t reserved[2];    /* 0 */
};

/* Starts recording, keeping at most `capacity` events; those past it are counted. dj does
 * not read the result. */
DJ_EXPORT bool dj_trace_start(size_t capacity);
DJ_EXPORT void dj_trace_stop(void);
/* The events recorded since dj_trace_start, valid until the next one, and how many were
 * dropped. Either pointer may be NULL. Returns how many events there are. */
DJ_EXPORT size_t dj_trace_read(const struct dj_trace_event **events, uint64_t *dropped);

#ifdef __cplusplus
}
#endif

#endif
