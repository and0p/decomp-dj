/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/mgba/src/LICENSE.txt). It is built into mGBA, which is MPL-2.0, as is the built core. */
/* decomp-dj coverage connector for mGBA's libretro core (docs/connectors.md in decomp-dj,
 * API version 2). MIT-licensed new file; see connectors/mgba/README.md.
 *
 * Regions, in index order (all "armv4t"):
 *   ROM    the cartridge, from byte 0 of the file, as long as the file; addresses
 *          0x08000000-0x0DFFFFFF, the three wait-state mirrors folded together.
 *   IWRAM  32 KB at 0x03000000 (fast RAM; games copy hot code here).
 *   EWRAM  256 KB at 0x02000000.
 *   BIOS   16 KB at 0x00000000 (mGBA's own HLE BIOS unless a real one is loaded; dj gives
 *          the core an empty system directory, so it's always mGBA's).
 *
 * Flags come from the CPU loop (ARMStep and ThumbStep in src/arm/arm.c), for every
 * instruction it reaches, before its condition is checked: 0x01 on the opcode's first byte,
 * 0x02 on the rest, and a mode bit on the first byte, 0x04 Thumb or 0x08 ARM. Thumb's BL is
 * two halfwords that the CPU runs one after the other; the second is flagged as operand
 * bytes, so the pair reads as one 4-byte instruction, as dj's decoder reads it. */
#include <stdlib.h>
#include <string.h>

#include <mgba/internal/arm/arm.h>
#include <mgba/internal/gba/gba.h>
#include <mgba/internal/gba/memory.h>

#include "dj_connector.h"

// The build id (connector API 2, optional): the upstream commit and this patch, written into
// dj_build_id.h by build.sh, so the same source gives the same id on every platform.
#if defined(__has_include)
#if __has_include("dj_build_id.h")
#include "dj_build_id.h"
#endif
#endif
#ifndef DJ_BUILD_ID
#define DJ_BUILD_ID "mgba@unknown"
#endif

enum { DJ_ROM, DJ_IWRAM, DJ_EWRAM, DJ_BIOS, DJ_REGIONS };

static const char* const djNames[DJ_REGIONS] = { "ROM", "IWRAM", "EWRAM", "BIOS" };
static const size_t djFixedSize[DJ_REGIONS] = { 0, GBA_SIZE_IWRAM, GBA_SIZE_EWRAM, GBA_SIZE_BIOS };

static struct GBA* djGBA;
static uint8_t* djFlags[DJ_REGIONS];
static size_t djSize[DJ_REGIONS];
/* Breakpoints: one bit per byte of each region, apart from the coverage flags (the player
 * stores those and tests them for "did anything run"). */
static uint8_t* djBrk[DJ_REGIONS];
static int djBrkLeft;
static dj_break_fn djBrkFn;
static void* djBrkUser;

static void djExec(uint32_t address, uint32_t opcode, bool thumb) {
	int region;
	uint32_t off;
	switch (address >> 24) {
	case 0x00:
		region = DJ_BIOS;
		off = address;
		break;
	case 0x02:
		region = DJ_EWRAM;
		off = address & (GBA_SIZE_EWRAM - 1);
		break;
	case 0x03:
		region = DJ_IWRAM;
		off = address & (GBA_SIZE_IWRAM - 1);
		break;
	case 0x08: case 0x09: case 0x0A: case 0x0B: case 0x0C: case 0x0D:
		region = DJ_ROM;
		off = address & 0x01FFFFFF;
		/* A 1 MB ROM shows up four times (mGBA mirrors it): fold the mirrors together. */
		if (djSize[DJ_ROM] && off >= djSize[DJ_ROM]) {
			off %= djSize[DJ_ROM];
		}
		break;
	default:
		return; /* VRAM, OAM, I/O: code there isn't counted */
	}
	uint8_t* flags = djFlags[region];
	size_t len = thumb ? 2 : 4;
	if (!flags || off + len > djSize[region]) {
		return;
	}
	if (thumb && (opcode >> 11) == 0x1F) {
		/* The second half of a BL: operand bytes of the pair. */
		flags[off] |= DJ_FLAG_OPERAND;
		flags[off + 1] |= DJ_FLAG_OPERAND;
		return;
	}
	flags[off] |= DJ_FLAG_OPCODE | (thumb ? DJ_FLAG_THUMB : DJ_FLAG_ARM);
	size_t i;
	for (i = 1; i < len; ++i) {
		flags[off + i] |= DJ_FLAG_OPERAND;
	}
	if (djBrkLeft > 0 && djBrk[region] && (djBrk[region][off >> 3] & (1 << (off & 7)))) {
		/* The frontend is called here, before the instruction runs. */
		if (djBrkFn(djBrkUser, region, off)) {
			djBrk[region][off >> 3] &= (uint8_t) ~(1 << (off & 7));
			--djBrkLeft;
		}
	}
}

void dj_attach(void* gba) {
	size_t i;
	djGBA = gba;
	for (i = 0; i < DJ_REGIONS; ++i) {
		free(djFlags[i]);
		djFlags[i] = NULL;
		free(djBrk[i]);
		djBrk[i] = NULL;
		djSize[i] = 0;
	}
	/* The regions are sized afresh: the frontend sets its breakpoints again. */
	djBrkLeft = 0;
	if (!djGBA) {
		return;
	}
	for (i = 0; i < DJ_REGIONS; ++i) {
		/* The ROM as the file has it (memory.romSize is padded for mirrors and the bus). */
		djSize[i] = i == DJ_ROM ? djGBA->pristineRomSize : djFixedSize[i];
		djFlags[i] = djSize[i] ? calloc(djSize[i], 1) : NULL;
		if (!djFlags[i]) {
			djSize[i] = 0;
		}
		djBrk[i] = djSize[i] ? calloc((djSize[i] + 7) / 8, 1) : NULL;
	}
}

static unsigned djApiVersion(void) {
	/* dj is driving the core: from here on every instruction is reported. */
	ARMDjExecHook = djExec;
	return DJ_API_VERSION;
}

static const char* djBuildId(void) {
	return DJ_BUILD_ID;
}

static size_t djRegionCount(void) {
	return djGBA ? DJ_REGIONS : 0;
}

static const uint8_t* djData(size_t index) {
	switch (index) {
	case DJ_ROM: return (const uint8_t*) djGBA->memory.rom;
	case DJ_IWRAM: return (const uint8_t*) djGBA->memory.iwram;
	case DJ_EWRAM: return (const uint8_t*) djGBA->memory.wram;
	case DJ_BIOS: return (const uint8_t*) djGBA->memory.bios;
	}
	return NULL;
}

static bool djGetRegion(size_t index, struct dj_region* out) {
	if (!out || !djGBA || index >= DJ_REGIONS) {
		return false;
	}
	out->name = djNames[index];
	out->flags = djFlags[index];
	out->size = djSize[index];
	out->cpu = "armv4t";
	out->data = djData(index);
	out->file_offset = index == DJ_ROM ? 0 : -1;
	return true;
}

static void djClear(void) {
	size_t i;
	for (i = 0; i < DJ_REGIONS; ++i) {
		if (djFlags[i]) {
			memset(djFlags[i], 0, djSize[i]);
		}
	}
}

/* Replaces the breakpoints. n = 0, or no function, turns them off. Places outside the
 * regions are left out. */
static void djSetBreakpoints(const struct dj_break* list, size_t n, dj_break_fn fn, void* user) {
	size_t i;
	for (i = 0; i < DJ_REGIONS; ++i) {
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
		if (region < 0 || region >= DJ_REGIONS || !djBrk[region] || off < 0 || (size_t) off >= djSize[region]) {
			continue;
		}
		if (!(djBrk[region][off >> 3] & (1 << (off & 7)))) {
			djBrk[region][off >> 3] |= (uint8_t) (1 << (off & 7));
			++djBrkLeft;
		}
	}
}

/* The ARM7TDMI's registers as the CPU holds them: r0 to r15, then the CPSR, each a 32-bit
 * value in the host's byte order (68 bytes). r15 is two instructions past the one about to
 * run. Returns the bytes written, 0 if `out` is too small or no game is loaded. */
static size_t djReadRegisters(void* out, size_t cap) {
	uint32_t* o = out;
	int i;
	if (!out || !djGBA || !djGBA->cpu || cap < 17 * sizeof(uint32_t)) {
		return 0;
	}
	for (i = 0; i < 16; ++i) {
		o[i] = (uint32_t) djGBA->cpu->gprs[i];
	}
	o[16] = (uint32_t) djGBA->cpu->cpsr.packed;
	return 17 * sizeof(uint32_t);
}

void* dj_get_proc_address(const char* sym) {
	if (!sym) return NULL;
	if (!strcmp(sym, "dj_api_version")) return (void*) &djApiVersion;
	if (!strcmp(sym, "dj_build_id")) return (void*) &djBuildId;
	if (!strcmp(sym, "dj_region_count")) return (void*) &djRegionCount;
	if (!strcmp(sym, "dj_get_region")) return (void*) &djGetRegion;
	if (!strcmp(sym, "dj_clear")) return (void*) &djClear;
	if (!strcmp(sym, "dj_set_breakpoints")) return (void*) &djSetBreakpoints;
	if (!strcmp(sym, "dj_read_registers")) return (void*) &djReadRegisters;
	return NULL;
}
