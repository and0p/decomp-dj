/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/mesen-s/src/LICENSE.txt). It is built into Mesen-S, which is GPL-3.0, as is the built core. */
// decomp-dj coverage connector for Mesen-S (SNES, libretro core), connector API version 3
// (docs/connectors.md in decomp-dj).
//
// Records which bytes the SNES's main CPU (the 65816) ran, keyed by offset into a region
// (ROM, WRAM, SRAM), never by CPU address; calls the frontend at breakpoints it sets; and
// records traces. Exposed to frontends through RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK
// as dj_* functions.
#pragma once
#include <stdint.h>
#include <stddef.h>

class Console;
class MemoryManager;
struct CpuState;

#define DJ_API_VERSION 3

// Flag bits stored per byte.
#define DJ_FLAG_OPCODE  0x01  // ran as the first byte of an instruction
#define DJ_FLAG_OPERAND 0x02  // ran as an operand byte
// 65816 mode bits, on opcode bytes: the register widths the opcode ran with.
#define DJ_FLAG_M8      0x04  // 8-bit accumulator (M set, or emulation mode)
#define DJ_FLAG_M16     0x08  // 16-bit accumulator
#define DJ_FLAG_X8      0x10  // 8-bit index registers (X set, or emulation mode)
#define DJ_FLAG_X16     0x20  // 16-bit index registers

struct dj_region
{
	const char* name;      // "ROM", "WRAM", "SRAM"
	uint8_t* flags;        // one byte of DJ_FLAG_* per byte of the region
	size_t size;
	const char* cpu;       // instruction set that runs here: "65816"
	const uint8_t* data;   // the region's bytes, or NULL
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

#define DJ_TRACE_EXEC  1
#define DJ_TRACE_READ  2
#define DJ_TRACE_WRITE 3

// One traced event (40 bytes, as include/dj_connector.h in decomp-dj).
struct dj_trace_event
{
	uint64_t cycle;        // CPU cycles since power-on
	int64_t offset;        // offset into `region`, or -1 when the address maps to no region
	uint16_t addr;         // the CPU address's low 16 bits
	uint8_t kind;          // DJ_TRACE_*
	int8_t region;         // region index as in dj_get_region, or -1
	uint8_t value;         // byte read or written; the opcode for DJ_TRACE_EXEC
	uint8_t a, x, y, sp, p; // the low bytes of A, X, Y and S, and P, before an executed instruction
	uint8_t operand_count; // operand bytes in `operands` (DJ_TRACE_EXEC)
	uint8_t operands[3];
	uint16_t addr_high;    // the CPU address's bank (bits 16-23)
	uint8_t reserved[2];   // 0
};

namespace DjConnector
{
	extern bool Active;   // dj asked for the API version: from here on every fetch is reported
	extern bool TraceOn;
	// The main CPU's registers, for dj_read_registers.
	extern const CpuState* Regs;

	// A game was loaded: size the regions for it, so dj_get_region answers right after
	// retro_load_game. `fileSize` is the ROM file's size, copier header included.
	void OnLoad(Console* console, size_t fileSize);
	void OnUnload();
	// Cpu::Read, for an opcode or operand fetch of the main CPU. `mm` tells a reloaded
	// console (new memory) from the one the regions were sized for.
	void OnExec(Console* console, MemoryManager* mm, uint32_t addr, bool opcode, uint8_t ps, bool emulation);
	// Cpu::Read and Cpu::Write while tracing.
	void OnTrace(MemoryManager* mm, int operationType, uint32_t addr, uint8_t value, const CpuState* s);
	void* GetProcAddress(const char* sym);
}
