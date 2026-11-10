/* decomp-dj decoder plugins: the C interface (docs/decoders.md in decomp-dj).
 * MIT license. Copy this header into your plugin or core.
 *
 * A decoder answers one question for one instruction set: what is the instruction at these
 * bytes? decomp-dj builds instructions, basic blocks, successors and callers from the
 * answers. A plugin is a shared library (.so, .dylib, .dll) that exports dj_decoders(); a
 * libretro core with a coverage connector may export it too, for the CPUs it runs.
 *
 * Rules:
 *   - Decoders are pure: the same bytes, address and mode always give the same answer, they
 *     keep no state between calls, and they may be called from several threads at once.
 *   - Everything handed out (tables, strings) stays valid while the library is loaded.
 *   - No errors cross the interface: bytes that aren't an instruction get length 0.
 *   - Mode numbers (ARM 0 / Thumb 1, 65816 register widths, ...) mean what the decoder
 *     says; they must match the mode bits the CPU's connector sets (docs/connectors.md). */
#ifndef DJ_DECODER_H
#define DJ_DECODER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define DJ_DECODER_API 1

/* How control leaves an instruction. A conditional jump is a branch. */
enum dj_kind {
	DJ_NEXT = 0,   /* on to the next instruction */
	DJ_JUMP = 1,   /* to target (or an unknown address: no target) */
	DJ_CALL = 2,   /* to target, coming back after the instruction and its delay slots */
	DJ_RETURN = 3, /* back to the caller */
	DJ_STOP = 4    /* execution doesn't go on (BRK, BREAK, an undefined instruction) */
};

/* One decoded instruction. decomp-dj zeroes it before each call: set only what applies. */
struct dj_instruction {
	uint8_t length;          /* bytes; 0: not an instruction */
	uint8_t kind;            /* enum dj_kind */
	uint8_t conditional;     /* 1: execution may also go on to the next instruction */
	uint8_t delay_slots;     /* instructions that run before control moves (MIPS: 1) */
	uint8_t has_target;      /* 1: target is set */
	uint8_t has_target_mode; /* 1: target_mode is set */
	uint8_t data_ref_length; /* bytes of data the instruction reads at data_ref; 0: none */
	uint8_t reserved;
	uint32_t mode_after;     /* the mode the next instruction runs in */
	uint32_t target_mode;    /* the mode code runs in at target, when the jump changes it */
	uint64_t target;         /* CPU address control goes to */
	uint64_t data_ref;       /* CPU address of a literal the instruction reads (PC-relative
	                            loads): the sweep won't read those bytes as code */
	const char* site;        /* how a call site through this instruction is listed ("BL",
	                            "JSR"), or NULL */
};

/* One instruction set. */
struct dj_decoder {
	uint32_t api_version;    /* DJ_DECODER_API */
	uint32_t size;           /* sizeof(struct dj_decoder): later versions add fields at the end */
	const char* cpu;         /* the name connectors report (dj_region.cpu): "armv4t", "65816" */
	uint8_t max_length;      /* the most bytes one instruction takes */
	uint8_t fixed_length;    /* every instruction this long; 0: lengths vary */
	uint8_t reserved[6];
	/* Instructions in mode start at multiples of this many bytes. NULL: 1. */
	uint8_t (*alignment)(uint32_t mode);
	/* The mode an opcode ran in, from the connector's mode bits on its first byte: return 1
	 * and set *mode when the bits say, else 0. NULL: never. */
	int (*mode_from_flags)(uint8_t flags, uint32_t* mode);
	/* Decode the instruction at bytes (available bytes there, maybe fewer than max_length at
	 * the end of the code; a length past them means it doesn't fit), which runs at CPU
	 * address pc in mode. Bytes are in the ROM file's order. Required. */
	void (*decode)(const uint8_t* bytes, size_t available, uint64_t pc, uint32_t mode, struct dj_instruction* out);
};

/* The one function a plugin exports: its decoders, one table per instruction set, and how
 * many in *count. The array and the tables stay valid while the library is loaded. */
#if defined(_WIN32)
__declspec(dllexport)
#else
__attribute__((visibility("default")))
#endif
const struct dj_decoder* const* dj_decoders(size_t* count);

#ifdef __cplusplus
}
#endif

#endif
