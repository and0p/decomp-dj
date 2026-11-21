/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/mupen64plus/src/LICENSE.txt). It is built into Mupen64Plus-Next, which is GPL-2.0, as is the built core. */
/* decomp-dj coverage connector for mupen64plus-libretro-nx (docs/connectors.md, API 3).
 *
 * Regions:
 *   RDRAM  coverage of code as it runs from RAM, by physical address (cpu "mips3"). Its
 *          bytes aren't handed out: the core keeps RDRAM in host-endian 32-bit words, not
 *          the CPU's byte order.
 *   ROM    the cartridge, in .z64 (big-endian) order from byte 0 of the file. No flags are
 *          set yet: telling which ROM bytes ran needs the provenance of RAM code (later).
 *
 * Flags are set by the pure interpreter only (dj_exec, from pure_interp.c); libretro.c
 * forces that interpreter while dj drives the core. */
#include <stdlib.h>
#include <string.h>

#include "dj_connector.h"

// The build id (connector API 2, optional): the upstream commit and this patch, written into
// dj_build_id.h by build.sh, so the same source gives the same id on every platform.
#if defined(__has_include)
#if __has_include("dj_build_id.h")
#include "dj_build_id.h"
#endif
#endif
#ifndef DJ_BUILD_ID
#define DJ_BUILD_ID "mupen64plus@unknown"
#endif
#include "mupen64plus-next_common.h"
#include "main/main.h"
#include "main/rom.h"
#include "device/device.h"
#include "device/memory/memory.h"
#include "osal/preproc.h"

#ifndef RDRAM_MAX_SIZE
#define RDRAM_MAX_SIZE 0x800000 /* 8 MB, with the Expansion Pak */
#endif

bool dj_active = false;

/* The trace: grows up to dj_trace_cap events; those past it are counted, not kept. */
bool dj_trace_on = false;
static struct dj_trace_event *dj_trace_buf = NULL;
static size_t dj_trace_len = 0, dj_trace_room = 0, dj_trace_cap = 0;
static uint64_t dj_trace_dropped = 0, dj_trace_cycle = 0;

static struct dj_trace_event *dj_trace_next(void)
{
   struct dj_trace_event *e;
   if (dj_trace_len >= dj_trace_cap)
   {
      dj_trace_dropped++;
      return NULL;
   }
   if (dj_trace_len >= dj_trace_room)
   {
      size_t room = dj_trace_room ? dj_trace_room * 2 : 65536;
      void *p;
      if (room > dj_trace_cap)
         room = dj_trace_cap;
      p = realloc(dj_trace_buf, room * sizeof(*dj_trace_buf));
      if (!p)
      {
         dj_trace_dropped++;
         return NULL;
      }
      dj_trace_buf = (struct dj_trace_event *)p;
      dj_trace_room = room;
   }
   e = &dj_trace_buf[dj_trace_len++];
   memset(e, 0, sizeof(*e));
   e->cycle = dj_trace_cycle;
   e->region = -1;
   e->offset = -1;
   return e;
}

void dj_trace_mem(int kind, uint32_t vaddr, uint32_t phys, uint32_t value, uint32_t mask)
{
   int i;
   for (i = 0; i < 4; i++)
   {
      int shift = 8 * (3 - i); /* byte i of a big-endian word */
      struct dj_trace_event *e;
      if (!((mask >> shift) & 0xff))
         continue;
      e = dj_trace_next();
      if (!e)
         return;
      e->kind = (uint8_t)kind;
      e->addr = (uint16_t)(vaddr + i);
      e->addr_high = (uint16_t)((vaddr + i) >> 16);
      e->value = (uint8_t)(value >> shift);
      if ((size_t)phys + 4 <= g_dev.rdram.dram_size && (size_t)phys + 4 <= RDRAM_MAX_SIZE)
      {
         e->region = 0;
         e->offset = (int64_t)phys + i;
      }
   }
}

static uint8_t dj_rdram_flags[RDRAM_MAX_SIZE];

/* Breakpoints: one bit per byte of RDRAM, apart from the coverage flags (the player stores
 * those and tests them for "did anything run"). */
static uint8_t dj_brk[RDRAM_MAX_SIZE / 8];
static int dj_brk_left = 0;
static dj_break_fn dj_brk_fn = NULL;
static void *dj_brk_user = NULL;
static uint8_t *dj_rom_flags;
static uint8_t *dj_rom_data;
static size_t dj_rom_size;

/* The ROM in .z64 order, from the core's ROM buffer, which is there from retro_load_game
 * on (M64CMD_ROM_OPEN stores it in .z64 order). When emulation starts, the core byte-swaps
 * each 32-bit word on little-endian hosts (main.c, g_RomWordsLittleEndian); read it back
 * with S8 then. */
static void dj_capture_rom(void)
{
   const uint8_t *rom = g_mem_base ? (const uint8_t *)mem_base_u32(g_mem_base, MM_CART_ROM) : NULL;
   size_t size = g_rom_size > 0 ? (size_t)g_rom_size : 0;
   size_t i;
   if (dj_rom_data || !rom || size == 0)
      return;
   dj_rom_data = (uint8_t *)malloc(size);
   dj_rom_flags = (uint8_t *)calloc(size, 1);
   if (!dj_rom_data || !dj_rom_flags)
   {
      free(dj_rom_data);
      free(dj_rom_flags);
      dj_rom_data = dj_rom_flags = NULL;
      return;
   }
   for (i = 0; i < size; i++)
      dj_rom_data[i] = rom[g_RomWordsLittleEndian ? (i ^ S8) : i];
   dj_rom_size = size;
}

void dj_exec(const uint32_t *op_address)
{
   const uint8_t *base = (const uint8_t *)g_dev.rdram.dram;
   size_t off;
   if (!base || (const uint8_t *)op_address < base)
      return;
   off = (size_t)((const uint8_t *)op_address - base);
   if (off + 4 > g_dev.rdram.dram_size || off + 4 > RDRAM_MAX_SIZE)
      return; /* not RAM: the boot code in the PIF or SP memory */
   dj_rdram_flags[off] |= DJ_FLAG_OPCODE;
   dj_rdram_flags[off + 1] |= DJ_FLAG_OPERAND;
   dj_rdram_flags[off + 2] |= DJ_FLAG_OPERAND;
   dj_rdram_flags[off + 3] |= DJ_FLAG_OPERAND;
   if (dj_trace_on)
   {
      struct dj_trace_event *e;
      dj_trace_cycle++;
      e = dj_trace_next();
      if (e)
      {
         uint32_t op = *op_address;
         uint32_t pc = *r4300_pc(&g_dev.r4300);
         e->kind = DJ_TRACE_EXEC;
         e->addr = (uint16_t)pc;
         e->addr_high = (uint16_t)(pc >> 16);
         e->region = 0;
         e->offset = (int64_t)off;
         e->value = (uint8_t)(op >> 24);
         e->operand_count = 3;
         e->operands[0] = (uint8_t)(op >> 16);
         e->operands[1] = (uint8_t)(op >> 8);
         e->operands[2] = (uint8_t)op;
      }
   }
   if (dj_brk_left > 0 && (dj_brk[off >> 3] & (1 << (off & 7))) && !dj_state_job_active())
   {
      /* The frontend is called here, before the instruction runs, on the game's coroutine
       * (its stack is 8 MB). */
      if (dj_brk_fn(dj_brk_user, 0, (int64_t)off))
      {
         dj_brk[off >> 3] &= (uint8_t)~(1 << (off & 7));
         dj_brk_left--;
      }
   }
}

static unsigned dj_api_version(void)
{
   dj_active = true;
   return DJ_API_VERSION;
}

static const char *dj_build_id(void) { return DJ_BUILD_ID; }

static size_t dj_region_count(void) { return 2; }

static bool dj_get_region(size_t index, struct dj_region *out)
{
   if (!out)
      return false;
   if (index == 0)
   {
      out->name = "RDRAM";
      out->flags = dj_rdram_flags;
      out->size = RDRAM_MAX_SIZE;
      out->cpu = "mips3";
      out->data = NULL;
      out->file_offset = -1;
      return true;
   }
   if (index == 1)
   {
      dj_capture_rom();
      out->name = "ROM";
      out->flags = dj_rom_flags;
      out->size = dj_rom_size;
      out->cpu = NULL;
      out->data = dj_rom_data;
      out->file_offset = dj_rom_data ? 0 : -1;
      return true;
   }
   return false;
}

static void dj_clear(void)
{
   memset(dj_rdram_flags, 0, sizeof(dj_rdram_flags));
   if (dj_rom_flags)
      memset(dj_rom_flags, 0, dj_rom_size);
}

/* The core options this connector needs, which libretro.c forces while dj drives the core.
 * dj takes them as its own values, so the options it records are the ones that run. */
static const char *dj_forced_options(void)
{
   return CORE_NAME "-cpucore=pure_interpreter\n"
          CORE_NAME "-rdp-plugin=gliden64\n"
          CORE_NAME "-rsp-plugin=hle\n"
          CORE_NAME "-ThreadedRenderer=False\n";
}

/* Replaces the breakpoints. n = 0, or no function, turns them off. Only RDRAM (region 0)
 * runs code; places elsewhere, or not on a word, are left out. */
static void dj_set_breakpoints(const struct dj_break *list, size_t n, dj_break_fn fn, void *user)
{
   size_t i;
   memset(dj_brk, 0, sizeof(dj_brk));
   dj_brk_left = 0;
   dj_brk_fn = fn;
   dj_brk_user = user;
   if (!fn || !list)
      return;
   for (i = 0; i < n; i++)
   {
      int64_t off = list[i].offset;
      if (list[i].region != 0 || off < 0 || off + 4 > RDRAM_MAX_SIZE || (off & 3))
         continue;
      if (!(dj_brk[off >> 3] & (1 << (off & 7))))
      {
         dj_brk[off >> 3] |= (uint8_t)(1 << (off & 7));
         dj_brk_left++;
      }
   }
}

/* The R4300's registers: the PC, then r0 to r31, each a 64-bit value in the host's byte
 * order (264 bytes). Returns the bytes written, 0 if `out` is too small. */
static size_t dj_read_registers(void *out, size_t cap)
{
   const int64_t *r = r4300_regs(&g_dev.r4300);
   uint64_t *o = (uint64_t *)out;
   int i;
   if (!out || !r || cap < 33 * sizeof(uint64_t))
      return 0;
   o[0] = *r4300_pc(&g_dev.r4300);
   for (i = 0; i < 32; i++)
      o[1 + i] = (uint64_t)r[i];
   return 33 * sizeof(uint64_t);
}

/* Starts a trace that keeps at most `capacity` events; 0 frees the last one. */
static void dj_trace_start(size_t capacity)
{
   free(dj_trace_buf);
   dj_trace_buf = NULL;
   dj_trace_len = dj_trace_room = 0;
   dj_trace_cap = capacity;
   dj_trace_dropped = 0;
   dj_trace_cycle = 0;
   dj_trace_on = capacity > 0;
}

static void dj_trace_stop(void) { dj_trace_on = false; }

/* The events recorded since dj_trace_start; valid until the next dj_trace_start. */
static size_t dj_trace_read(const struct dj_trace_event **events, uint64_t *dropped)
{
   if (events) *events = dj_trace_len ? dj_trace_buf : NULL;
   if (dropped) *dropped = dj_trace_dropped;
   return dj_trace_len;
}

void *dj_get_proc_address(const char *sym)
{
   if (!sym) return NULL;
   if (!strcmp(sym, "dj_api_version")) return (void *)&dj_api_version;
   if (!strcmp(sym, "dj_region_count")) return (void *)&dj_region_count;
   if (!strcmp(sym, "dj_get_region")) return (void *)&dj_get_region;
   if (!strcmp(sym, "dj_clear")) return (void *)&dj_clear;
   if (!strcmp(sym, "dj_set_breakpoints")) return (void *)&dj_set_breakpoints;
   if (!strcmp(sym, "dj_read_registers")) return (void *)&dj_read_registers;
   if (!strcmp(sym, "dj_forced_options")) return (void *)&dj_forced_options;
   if (!strcmp(sym, "dj_build_id")) return (void *)&dj_build_id;
   if (!strcmp(sym, "dj_trace_start")) return (void *)&dj_trace_start;
   if (!strcmp(sym, "dj_trace_stop")) return (void *)&dj_trace_stop;
   if (!strcmp(sym, "dj_trace_read")) return (void *)&dj_trace_read;
   return NULL;
}
