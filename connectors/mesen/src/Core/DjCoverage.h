/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/mesen/src/LICENSE.txt). It is built into Mesen, which is GPL-3.0, as is the built core. */
// emu_coverage connector for Mesen (libretro core).
// Records which bytes of each code region the CPU executed, keyed by offset
// into the region (PRG ROM offset, not CPU address), and calls the frontend at
// breakpoints it sets. Exposed to frontends through
// RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK as dj_* functions.
#pragma once
#include <stdint.h>
#include <stddef.h>

class BaseMapper;
struct State;

// Flag bits stored per byte.
#define DJ_FLAG_OPCODE  0x01  // executed as the first byte of an instruction
#define DJ_FLAG_OPERAND 0x02  // executed as an operand byte
// 0x04-0x20: mode bits, defined per `cpu` (none for the 6502). 0x40-0x80: reserved.

#define DJ_API_VERSION 3

struct dj_region
{
	const char* name;      // "RAM", "PRG", "WRAM", "SRAM"
	uint8_t* flags;        // one byte of DJ_FLAG_* per byte of the region
	size_t size;
	const char* cpu;       // instruction set that runs here: "6502"
	const uint8_t* data;   // the region's bytes in the CPU's byte order, or NULL
	int64_t file_offset;   // where offset 0 lies in the ROM file, or -1
};

// A breakpoint: an offset into a region (index as in dj_get_region).
struct dj_break
{
	int32_t region;
	int64_t offset;
};

// Called when the instruction at a breakpoint is about to run, inside retro_run. The core
// waits for it to return. Non-zero switches that breakpoint off until the list is set again.
typedef int (*dj_break_fn)(void* user, int region, int64_t offset);

// Trace events (optional part of the API, dj_trace_*): one per executed instruction and
// per data read or write while tracing is on.
#define DJ_TRACE_EXEC  1
#define DJ_TRACE_READ  2
#define DJ_TRACE_WRITE 3

struct dj_trace_event
{
	uint64_t cycle;        // CPU cycle count
	int64_t offset;        // offset into `region`, or -1 when the address maps to no region
	uint16_t addr;         // CPU address
	uint8_t kind;          // DJ_TRACE_*
	int8_t region;         // region index as in dj_get_region, or -1
	uint8_t value;         // byte read or written; the opcode for DJ_TRACE_EXEC
	uint8_t a, x, y, sp, p; // registers before the instruction (DJ_TRACE_EXEC), or at the access
	uint8_t operand_count; // operand bytes in `operands` (DJ_TRACE_EXEC)
	uint8_t operands[3];
	uint8_t reserved[4];
};

namespace DjCoverage
{
	extern bool TraceOn;
	// The CPU's registers, for dj_read_registers: set by CPU::MemoryRead on every fetch.
	extern const State* Regs;
	inline bool Tracing() { return TraceOn; }
	void OnExec(BaseMapper* mapper, uint16_t addr, bool opcode);
	// Sizes the regions for a newly loaded game, so dj_get_region answers right after
	// retro_load_game.
	void OnLoad(BaseMapper* mapper);
	void OnTrace(BaseMapper* mapper, int operationType, uint16_t addr, uint8_t value, uint8_t a, uint8_t x, uint8_t y, uint8_t sp, uint8_t p, uint64_t cycle);
	void* GetProcAddress(const char* sym);
}
