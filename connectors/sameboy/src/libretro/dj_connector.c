/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/sameboy/src/LICENSE.txt). It is built into SameBoy, which is MIT (Expat) too. */
/* decomp-dj coverage connector for SameBoy's libretro core (docs/connectors.md in decomp-dj,
 * API version 3). See connectors/sameboy/README.md.
 *
 * Regions, in index order (all "sm83"):
 *   ROM   the cartridge, from byte 0 of the file, as long as the file. 0x0000-0x3FFF and
 *         0x4000-0x7FFF are mapped through the bank registers the core keeps for every
 *         mapper (mbc_rom0_bank, mbc_rom_bank), so MBC1, MBC2, MBC3, MBC5, MBC7, MMM01,
 *         HuC1, HuC3, TPP1 and the rest all land on the right file offset.
 *   WRAM  8 KB (32 KB on a Game Boy Color), bank by bank: 0xC000-0xCFFF is offset 0, and
 *         0xD000-0xDFFF is the selected bank (1 to 7) times 0x1000. Echo RAM folds in.
 *   HRAM  127 bytes at 0xFF80 (where games run their OAM DMA routine).
 *   BOOT  the boot ROM while it is mapped: 256 bytes, 2,304 on a Game Boy Color. SameBoy's
 *         own, built from its sources, unless one is in the system directory; dj gives the
 *         core an empty one.
 *   SRAM  the cartridge's RAM, bank by bank, when the cartridge has some.
 *
 * Flags come from SameBoy's own execution callback, which runs after the CPU fetched an
 * opcode and before it runs it: 0x01 on the opcode, 0x02 on its operand bytes (each mapped
 * on its own, so an instruction across a bank boundary is flagged where its bytes are). No
 * mode bits. Memory accesses for traces come from SameBoy's read and write callbacks, which
 * are set only while a trace is recording. */
#include <stdlib.h>
#include <string.h>

#include <Core/gb.h>
#include <Core/random.h>

#include "dj_connector.h"

/* The build id (optional): the upstream commit and this connector, written into
 * dj_build_id.h by build.sh, so the same source gives the same id on every platform. */
#if defined(__has_include)
#if __has_include("dj_build_id.h")
#include "dj_build_id.h"
#endif
#endif
#ifndef DJ_BUILD_ID
#define DJ_BUILD_ID "sameboy@unknown"
#endif

enum { DJ_ROM, DJ_WRAM, DJ_HRAM, DJ_BOOT, DJ_SRAM, DJ_MAX };

static const char* const djNames[DJ_MAX] = { "ROM", "WRAM", "HRAM", "BOOT", "SRAM" };

static GB_gameboy_t* djGB;
static bool djActive;          /* dj asked for the API version: it is driving the core */
static size_t djRegions;       /* 4, or 5 with SRAM */
static uint8_t* djFlags[DJ_MAX];
static size_t djSize[DJ_MAX];
/* Breakpoints: one bit per byte of each region, apart from the coverage flags. */
static uint8_t* djBrk[DJ_MAX];
static int djBrkLeft;
static dj_break_fn djBrkFn;
static void* djBrkUser;
/* The instruction a breakpoint stopped at, for dj_read_registers (the CPU's PC is already
 * past its opcode). */
static bool djInBreak;
static uint16_t djBreakPc;

/* The trace. */
static struct dj_trace_event* djTrace;
static size_t djTraceLen, djTraceAlloc, djTraceCap;
static uint64_t djTraceDropped, djTraceCount;
static bool djTraceOn;
/* The operand bytes the current instruction has still to fetch, which aren't data reads. */
static uint16_t djFetchNext;
static unsigned djFetchLeft;
/* Set while the connector reads memory itself, so the read isn't traced. */
static bool djSelfRead;

/* The SM83's instruction lengths: 1 byte, but for these. 0xCB prefixes a second opcode
 * byte. STOP (0x10) is two bytes, as assemblers write it. */
static unsigned djLength(uint8_t op) {
	switch (op) {
	case 0x01: case 0x08: case 0x11: case 0x21: case 0x31:
	case 0xC2: case 0xC3: case 0xC4: case 0xCA: case 0xCC: case 0xCD:
	case 0xD2: case 0xD4: case 0xDA: case 0xDC: case 0xEA: case 0xFA:
		return 3;
	case 0x06: case 0x0E: case 0x16: case 0x1E: case 0x26: case 0x2E: case 0x36: case 0x3E:
	case 0x10: case 0x18: case 0x20: case 0x28: case 0x30: case 0x38:
	case 0xC6: case 0xCE: case 0xD6: case 0xDE: case 0xE6: case 0xEE: case 0xF6: case 0xFE:
	case 0xCB: case 0xE0: case 0xE8: case 0xF0: case 0xF8:
		return 2;
	}
	return 1;
}

/* The region and offset that the CPU address `addr` reads right now, as the core's own
 * read handlers (Core/memory.c) work them out; -1 for VRAM, OAM, I/O and unmapped RAM. */
static int djMap(uint16_t addr, size_t* out) {
	GB_gameboy_t* gb = djGB;
	size_t off;
	int region;
	if (addr < 0x8000) {
		if (!gb->boot_rom_finished && (addr < 0x100 || (addr >= 0x200 && addr < 0x900 && GB_is_cgb(gb)))) {
			region = DJ_BOOT;
			off = addr;
		}
		else {
			if (!gb->rom_size) {
				return -1;
			}
			unsigned bank = addr < 0x4000 ? gb->mbc_rom0_bank : gb->mbc_rom_bank;
			region = DJ_ROM;
			off = ((size_t) (addr & 0x3FFF) + (size_t) bank * 0x4000) & (gb->rom_size - 1);
		}
	}
	else if (addr < 0xA000) {
		return -1; /* VRAM */
	}
	else if (addr < 0xC000) {
		/* Cartridge RAM as plain RAM: the common mappers, enabled, with no RTC register or
		 * camera mapped there. */
		const GB_cartridge_t* cart = gb->cartridge_type;
		if (!gb->mbc_ram || !gb->mbc_ram_size || !gb->mbc_ram_enable || gb->camera_registers_mapped) {
			return -1;
		}
		if (cart->mbc_type != GB_NO_MBC && cart->mbc_type != GB_MBC1 && cart->mbc_type != GB_MBC3 && cart->mbc_type != GB_MBC5) {
			return -1;
		}
		unsigned bank = gb->mbc_ram_bank;
		if (cart->mbc_type == GB_MBC3 && !gb->is_mbc30) {
			if (cart->has_rtc && (gb->mbc3.rtc_mapped || bank > 3)) {
				return -1;
			}
			bank &= 3;
		}
		region = DJ_SRAM;
		off = ((size_t) (addr & 0x1FFF) + (size_t) bank * 0x2000) & (gb->mbc_ram_size - 1);
	}
	else if (addr < 0xFE00) {
		unsigned a = addr >= 0xE000 ? addr - 0x2000 : addr; /* echo RAM */
		region = DJ_WRAM;
		off = a < 0xD000 ? a - 0xC000 : (size_t) (a - 0xD000) + (size_t) gb->cgb_ram_bank * 0x1000;
	}
	else if (addr >= 0xFF80 && addr < 0xFFFF) {
		region = DJ_HRAM;
		off = addr - 0xFF80;
	}
	else {
		return -1;
	}
	if ((size_t) region >= djRegions || !djFlags[region] || off >= djSize[region]) {
		return -1;
	}
	*out = off;
	return region;
}

static struct dj_trace_event* djTracePush(void) {
	if (djTraceLen >= djTraceCap) {
		++djTraceDropped;
		return NULL;
	}
	if (djTraceLen == djTraceAlloc) {
		/* Grows as it fills, up to the capacity. */
		size_t n = djTraceAlloc ? djTraceAlloc * 2 : 65536;
		if (n > djTraceCap) {
			n = djTraceCap;
		}
		struct dj_trace_event* t = realloc(djTrace, n * sizeof(*t));
		if (!t) {
			++djTraceDropped;
			return NULL;
		}
		djTrace = t;
		djTraceAlloc = n;
	}
	struct dj_trace_event* e = &djTrace[djTraceLen++];
	memset(e, 0, sizeof(*e));
	return e;
}

static void djTraceAccess(uint8_t kind, uint16_t addr, uint8_t value) {
	struct dj_trace_event* e = djTracePush();
	if (!e) {
		return;
	}
	size_t off;
	int r = djMap(addr, &off);
	e->cycle = djTraceCount;
	e->kind = kind;
	e->addr = addr;
	e->value = value;
	e->region = (int8_t) r;
	e->offset = r < 0 ? -1 : (int64_t) off;
}

static uint8_t djRead(GB_gameboy_t* gb, uint16_t addr, uint8_t data) {
	if (gb != djGB || !djTraceOn || djSelfRead) {
		return data;
	}
	if (djFetchLeft && addr == djFetchNext) {
		/* An operand byte of the instruction running: part of its execute event. */
		++djFetchNext;
		--djFetchLeft;
		return data;
	}
	djTraceAccess(DJ_TRACE_READ, addr, data);
	return data;
}

static bool djWrite(GB_gameboy_t* gb, uint16_t addr, uint8_t data) {
	if (gb == djGB && djTraceOn) {
		djTraceAccess(DJ_TRACE_WRITE, addr, data);
	}
	return true;
}

static void djTraceExec(GB_gameboy_t* gb, uint16_t address, uint8_t opcode, unsigned len, int region, size_t off) {
	/* The opcode's fetch came through djRead before this call: it is this event. */
	if (djTraceLen && djTrace[djTraceLen - 1].kind == DJ_TRACE_READ && djTrace[djTraceLen - 1].addr == address) {
		--djTraceLen;
	}
	djFetchNext = (uint16_t) (address + 1);
	djFetchLeft = len - 1;
	++djTraceCount;
	struct dj_trace_event* e = djTracePush();
	if (!e) {
		return;
	}
	e->cycle = djTraceCount;
	e->kind = DJ_TRACE_EXEC;
	e->addr = address;
	e->value = opcode;
	e->region = (int8_t) region;
	e->offset = region < 0 ? -1 : (int64_t) off;
	e->a = gb->a;
	e->p = gb->f;
	e->operand_count = (uint8_t) (len - 1);
	djSelfRead = true;
	unsigned i;
	for (i = 1; i < len; ++i) {
		e->operands[i - 1] = GB_safe_read_memory(gb, (uint16_t) (address + i));
	}
	djSelfRead = false;
}

static void djExec(GB_gameboy_t* gb, uint16_t address, uint8_t opcode) {
	if (gb != djGB) {
		return;
	}
	size_t off = 0;
	int region = djMap(address, &off);
	unsigned len = djLength(opcode);
	if (djTraceOn) {
		djTraceExec(gb, address, opcode, len, region, off);
	}
	if (region < 0) {
		return;
	}
	djFlags[region][off] |= DJ_FLAG_OPCODE;
	unsigned i;
	for (i = 1; i < len; ++i) {
		size_t o;
		int r = djMap((uint16_t) (address + i), &o);
		if (r >= 0) {
			djFlags[r][o] |= DJ_FLAG_OPERAND;
		}
	}
	if (djBrkLeft > 0 && djBrk[region] && (djBrk[region][off >> 3] & (1 << (off & 7)))) {
		/* The frontend is called here, before the instruction runs. */
		djInBreak = true;
		djBreakPc = address;
		int off_switch = djBrkFn(djBrkUser, region, (int64_t) off);
		djInBreak = false;
		if (off_switch) {
			djBrk[region][off >> 3] &= (uint8_t) ~(1 << (off & 7));
			--djBrkLeft;
		}
	}
}

static void djHook(void) {
	if (djGB) {
		GB_set_execution_callback(djGB, djActive ? djExec : NULL);
		GB_set_read_memory_callback(djGB, djActive && djTraceOn ? djRead : NULL);
		GB_set_write_memory_callback(djGB, djActive && djTraceOn ? djWrite : NULL);
	}
}

void dj_attach(void* gb, size_t rom_file_size) {
	size_t i;
	if (djGB && djGB != gb) {
		GB_set_execution_callback(djGB, NULL);
		GB_set_read_memory_callback(djGB, NULL);
		GB_set_write_memory_callback(djGB, NULL);
	}
	djGB = gb;
	for (i = 0; i < DJ_MAX; ++i) {
		free(djFlags[i]);
		djFlags[i] = NULL;
		free(djBrk[i]);
		djBrk[i] = NULL;
		djSize[i] = 0;
	}
	djRegions = 0;
	/* The regions are sized afresh: the frontend sets its breakpoints again. */
	djBrkLeft = 0;
	if (!djGB) {
		return;
	}
	djSize[DJ_ROM] = rom_file_size < djGB->rom_size ? rom_file_size : djGB->rom_size;
	djSize[DJ_WRAM] = djGB->ram_size;
	djSize[DJ_HRAM] = sizeof(djGB->hram);
	djSize[DJ_BOOT] = GB_is_cgb(djGB) ? 0x900 : 0x100;
	djSize[DJ_SRAM] = djGB->mbc_ram ? djGB->mbc_ram_size : 0;
	djRegions = djSize[DJ_SRAM] ? 5 : 4;
	for (i = 0; i < djRegions; ++i) {
		djFlags[i] = djSize[i] ? calloc(djSize[i], 1) : NULL;
		djBrk[i] = djSize[i] ? calloc((djSize[i] + 7) / 8, 1) : NULL;
		if (!djFlags[i] || !djBrk[i]) {
			djSize[i] = 0;
		}
	}
	djHook();
}

static unsigned djApiVersion(void) {
	/* dj is driving the core: from here on every instruction is reported. Power-on RAM is
	 * filled from a generator seeded by the clock; with it off, every power-on is the same. */
	djActive = true;
	GB_random_set_enabled(false);
	djHook();
	return DJ_API_VERSION;
}

static const char* djBuildId(void) {
	return DJ_BUILD_ID;
}

/* The real-time clock (MBC3, HuC3, TPP1) follows emulated time, not the host's clock. */
static const char* djForcedOptions(void) {
	return "sameboy_rtc=accurate\n";
}

static size_t djRegionCount(void) {
	return djGB ? djRegions : 0;
}

static const uint8_t* djData(size_t index) {
	switch (index) {
	case DJ_ROM: return djGB->rom;
	case DJ_WRAM: return djGB->ram;
	case DJ_HRAM: return djGB->hram;
	case DJ_BOOT: return djGB->boot_rom;
	case DJ_SRAM: return djGB->mbc_ram;
	}
	return NULL;
}

static bool djGetRegion(size_t index, struct dj_region* out) {
	if (!out || !djGB || index >= djRegions) {
		return false;
	}
	out->name = djNames[index];
	out->flags = djFlags[index];
	out->size = djSize[index];
	out->cpu = "sm83";
	out->data = djData(index);
	out->file_offset = index == DJ_ROM ? 0 : -1;
	return true;
}

static void djClear(void) {
	size_t i;
	for (i = 0; i < DJ_MAX; ++i) {
		if (djFlags[i]) {
			memset(djFlags[i], 0, djSize[i]);
		}
	}
}

/* Replaces the breakpoints. n = 0, or no function, turns them off. Places outside the
 * regions are left out. */
static void djSetBreakpoints(const struct dj_break* list, size_t n, dj_break_fn fn, void* user) {
	size_t i;
	for (i = 0; i < DJ_MAX; ++i) {
		if (djBrk[i]) {
			memset(djBrk[i], 0, (djSize[i] + 7) / 8);
		}
	}
	djBrkLeft = 0;
	djBrkFn = fn;
	djBrkUser = user;
	if (!fn || !list) {
		return;
	}
	for (i = 0; i < n; ++i) {
		int region = list[i].region;
		int64_t off = list[i].offset;
		if (region < 0 || (size_t) region >= djRegions || !djBrk[region] || off < 0 || (size_t) off >= djSize[region]) {
			continue;
		}
		if (!(djBrk[region][off >> 3] & (1 << (off & 7)))) {
			djBrk[region][off >> 3] |= (uint8_t) (1 << (off & 7));
			++djBrkLeft;
		}
	}
}

/* The SM83's registers, 12 bytes: A, F, B, C, D, E, H, L, then SP and PC with their low
 * byte first. PC is the address of the instruction about to run at a breakpoint, else as
 * the CPU holds it. Returns the bytes written, 0 if `out` is too small or no game is loaded. */
static size_t djReadRegisters(void* out, size_t cap) {
	uint8_t* o = out;
	if (!out || !djGB || cap < 12) {
		return 0;
	}
	uint16_t pc = djInBreak ? djBreakPc : djGB->pc;
	o[0] = djGB->a;
	o[1] = djGB->f;
	o[2] = djGB->b;
	o[3] = djGB->c;
	o[4] = djGB->d;
	o[5] = djGB->e;
	o[6] = djGB->h;
	o[7] = djGB->l;
	o[8] = (uint8_t) (djGB->sp & 0xFF);
	o[9] = (uint8_t) (djGB->sp >> 8);
	o[10] = (uint8_t) (pc & 0xFF);
	o[11] = (uint8_t) (pc >> 8);
	return 12;
}

/* Starts tracing into a fresh buffer of up to `capacity` events (0 stops tracing). Events
 * past the capacity are counted as dropped. The buffer is kept until the next call. */
static void djTraceStart(size_t capacity) {
	free(djTrace);
	djTrace = NULL;
	djTraceLen = djTraceAlloc = 0;
	djTraceCap = capacity;
	djTraceDropped = djTraceCount = 0;
	djFetchLeft = 0;
	djTraceOn = capacity > 0;
	djHook();
}

static void djTraceStop(void) {
	djTraceOn = false;
	djHook();
}

/* The events recorded since dj_trace_start; valid until the next dj_trace_start. */
static size_t djTraceRead(const struct dj_trace_event** events, uint64_t* dropped) {
	if (events) {
		*events = djTraceLen ? djTrace : NULL;
	}
	if (dropped) {
		*dropped = djTraceDropped;
	}
	return djTraceLen;
}

void* dj_get_proc_address(const char* sym) {
	if (!sym) return NULL;
	if (!strcmp(sym, "dj_api_version")) return (void*) &djApiVersion;
	if (!strcmp(sym, "dj_build_id")) return (void*) &djBuildId;
	if (!strcmp(sym, "dj_forced_options")) return (void*) &djForcedOptions;
	if (!strcmp(sym, "dj_region_count")) return (void*) &djRegionCount;
	if (!strcmp(sym, "dj_get_region")) return (void*) &djGetRegion;
	if (!strcmp(sym, "dj_clear")) return (void*) &djClear;
	if (!strcmp(sym, "dj_set_breakpoints")) return (void*) &djSetBreakpoints;
	if (!strcmp(sym, "dj_read_registers")) return (void*) &djReadRegisters;
	if (!strcmp(sym, "dj_trace_start")) return (void*) &djTraceStart;
	if (!strcmp(sym, "dj_trace_stop")) return (void*) &djTraceStop;
	if (!strcmp(sym, "dj_trace_read")) return (void*) &djTraceRead;
	return NULL;
}
