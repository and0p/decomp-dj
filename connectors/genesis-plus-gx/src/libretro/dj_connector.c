/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/genesis-plus-gx/src/LICENSE.txt). It is built into Genesis Plus GX, whose own
 * licence covers the built core. */
/* decomp-dj coverage connector for Genesis Plus GX's libretro core (docs/connectors.md in
 * decomp-dj, API version 3). See connectors/genesis-plus-gx/README.md.
 *
 * Regions, in index order (both "m68k"), for Mega Drive / Genesis cartridges only:
 *   ROM  the cartridge, from byte 0 of a plain (.bin, .md, .gen) file, as long as the ROM;
 *        found through the 68000's memory map, so bank-switching boards are followed.
 *   RAM  the 68000's 64 KB of work RAM at 0xFF0000 (mirrored from 0xE00000).
 *
 * Flags come from the main 68000 (m68k_run and the instruction-stream fetches in
 * core/m68k/m68kcpu.h): 0x01 on an instruction's first byte, 0x02 on the second byte of its
 * opcode word and on every extension word the CPU fetches. The 68000 has no mode bits. A
 * Bcc.W or DBcc that doesn't branch skips its displacement word without fetching it; that
 * word is flagged here from the opcode, so every instruction that ran reads as long as it
 * is. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include "shared.h"
#include "dj_connector.h"

/* The build id (optional): the upstream commit and this connector, written into
 * dj_build_id.h by build.sh, so the same source gives the same id on every platform. */
#if defined(__has_include)
#if __has_include("dj_build_id.h")
#include "dj_build_id.h"
#endif
#endif
#ifndef DJ_BUILD_ID
#define DJ_BUILD_ID "genesis-plus-gx@unknown"
#endif

enum { DJ_ROM, DJ_RAM, DJ_REGIONS };
enum { DJ_TRACE_EXEC = 1, DJ_TRACE_READ = 2, DJ_TRACE_WRITE = 3 };

static const char *const dj_names[DJ_REGIONS] = { "ROM", "RAM" };

static int dj_loaded;
static uint8_t *dj_flags[DJ_REGIONS];
static size_t dj_size[DJ_REGIONS];
/* The ROM in the CPU's (big-endian) order: the core keeps it byte-swapped on little-endian
 * hosts. */
static uint8_t *dj_rom;
static int64_t dj_rom_file_offset = -1;

/* Breakpoints: one bit per byte of each region, apart from the coverage flags. */
static uint8_t *dj_brk[DJ_REGIONS];
static int dj_brk_left;
static dj_break_fn dj_brk_fn;
static void *dj_brk_user;

/* The instruction being run: its region and offset, for telling its opcode word from its
 * extension words. */
static int dj_cur_region = -1;
static size_t dj_cur_off;

/* The trace. */
static int dj_tracing;
static struct dj_trace_event *dj_trace;
static size_t dj_trace_len, dj_trace_alloc, dj_trace_cap;
static uint64_t dj_trace_dropped, dj_trace_insn;
static long dj_trace_last_exec = -1;

/* The region and offset of a CPU address, through the 68000's memory map; -1 if it's in
 * neither region (I/O, the Z80's area, a mapper's registers). */
static int dj_map(unsigned int addr, size_t *off)
{
   const unsigned char *base;
   size_t o;
   addr &= 0xFFFFFF;
   base = m68k.memory_map[addr >> 16].base;
   if (!base)
      return -1;
   if (base == work_ram)
   {
      *off = addr & 0xFFFF;
      return dj_flags[DJ_RAM] ? DJ_RAM : -1;
   }
   if (!dj_flags[DJ_ROM] || base < cart.rom || base >= cart.rom + sizeof(cart.rom))
      return -1;
   o = (size_t)(base - cart.rom) + (addr & 0xFFFF);
   /* A ROM smaller than its area is mirrored through it: fold the copies together. */
   if (o >= dj_size[DJ_ROM])
      o %= dj_size[DJ_ROM];
   *off = o;
   return DJ_ROM;
}

/* The 16-bit word at a CPU address, as the CPU reads it. */
static unsigned int dj_peek16(unsigned int addr)
{
   const unsigned char *base = m68k.memory_map[(addr >> 16) & 0xFF].base;
   if (!base)
      return 0;
   return *(const uint16 *)(base + (addr & 0xFFFE));
}

static struct dj_trace_event *dj_trace_push(void)
{
   if (dj_trace_len >= dj_trace_cap)
   {
      ++dj_trace_dropped;
      return NULL;
   }
   if (dj_trace_len >= dj_trace_alloc)
   {
      size_t want = dj_trace_alloc ? dj_trace_alloc * 2 : 65536;
      struct dj_trace_event *grown;
      if (want > dj_trace_cap)
         want = dj_trace_cap;
      grown = realloc(dj_trace, want * sizeof *grown);
      if (!grown)
      {
         ++dj_trace_dropped;
         return NULL;
      }
      dj_trace = grown;
      dj_trace_alloc = want;
   }
   memset(&dj_trace[dj_trace_len], 0, sizeof *dj_trace);
   return &dj_trace[dj_trace_len++];
}

static void dj_trace_operand(uint8_t byte)
{
   struct dj_trace_event *e;
   if (dj_trace_last_exec < 0)
      return;
   e = &dj_trace[dj_trace_last_exec];
   if (e->operand_count < 3)
      e->operands[e->operand_count++] = byte;
}

static void dj_flag(int region, size_t off, uint8_t bits)
{
   if (region >= 0 && off < dj_size[region])
      dj_flags[region][off] |= bits;
}

static void dj_exec(unsigned int pc)
{
   size_t off = 0;
   int region;
   unsigned int op;
   int skips; /* a displacement word the instruction may skip without fetching it */

   pc &= 0xFFFFFF;
   region = dj_map(pc, &off);
   dj_cur_region = region;
   dj_cur_off = off;
   op = dj_peek16(pc);
   skips = (op & 0xF0FF) == 0x6000 || (op & 0xF0F8) == 0x50C8; /* Bcc.W, DBcc */

   if (region >= 0)
   {
      dj_flag(region, off, DJ_FLAG_OPCODE);
      dj_flag(region, off + 1, DJ_FLAG_OPERAND);
      if (skips)
      {
         dj_flag(region, off + 2, DJ_FLAG_OPERAND);
         dj_flag(region, off + 3, DJ_FLAG_OPERAND);
      }
   }

   if (dj_tracing)
   {
      struct dj_trace_event *e = dj_trace_push();
      dj_trace_last_exec = -1;
      if (e)
      {
         unsigned int ext = dj_peek16(pc + 2);
         e->cycle = dj_trace_insn;
         e->offset = region >= 0 ? (int64_t) off : -1;
         e->addr = (uint16_t) pc;
         e->addr_high = (uint16_t)(pc >> 16);
         e->kind = DJ_TRACE_EXEC;
         e->region = (int8_t) region;
         e->value = (uint8_t)(op >> 8);
         e->operands[0] = (uint8_t) op;
         e->operand_count = 1;
         if (skips)
         {
            /* The displacement may not be fetched (no branch): take it from memory. */
            e->operands[1] = (uint8_t)(ext >> 8);
            e->operands[2] = (uint8_t) ext;
            e->operand_count = 3;
         }
         dj_trace_last_exec = (long)(e - dj_trace);
      }
      ++dj_trace_insn;
   }

   if (region >= 0 && dj_brk_left > 0 && dj_brk[region] && (dj_brk[region][off >> 3] & (1 << (off & 7))))
   {
      /* The frontend is called here, before the instruction runs. */
      if (dj_brk_fn(dj_brk_user, region, (int64_t) off))
      {
         dj_brk[region][off >> 3] &= (uint8_t) ~(1 << (off & 7));
         --dj_brk_left;
      }
   }
}

/* The CPU fetched `len` bytes of the instruction stream at `pc`: the current instruction's
 * opcode word (already flagged by dj_exec), then its extension words. */
static void dj_fetch(unsigned int pc, unsigned int len)
{
   unsigned int i;
   pc &= 0xFFFFFF;
   for (i = 0; i < len; i += 2)
   {
      unsigned int a = pc + i;
      size_t off = 0;
      int region = dj_map(a, &off);
      int opcode_word = region == dj_cur_region && off == dj_cur_off;
      if (region >= 0 && !opcode_word)
      {
         dj_flag(region, off, DJ_FLAG_OPERAND);
         dj_flag(region, off + 1, DJ_FLAG_OPERAND);
      }
      if (dj_tracing && !opcode_word)
      {
         unsigned int w = dj_peek16(a);
         dj_trace_operand((uint8_t)(w >> 8));
         dj_trace_operand((uint8_t) w);
      }
   }
}

/* A data read (kind 2) or write (kind 3), while tracing: one event per byte. */
static void dj_access(int kind, int width, unsigned int address, unsigned int value)
{
   int i;
   if (!dj_tracing)
      return;
   for (i = 0; i < width; ++i)
   {
      unsigned int a = (address + i) & 0xFFFFFF;
      size_t off = 0;
      int region = dj_map(a, &off);
      struct dj_trace_event *e = dj_trace_push();
      if (!e)
         return;
      e->cycle = dj_trace_insn ? dj_trace_insn - 1 : 0;
      e->offset = region >= 0 ? (int64_t) off : -1;
      e->addr = (uint16_t) a;
      e->addr_high = (uint16_t)(a >> 16);
      e->kind = (uint8_t) kind;
      e->region = (int8_t) region;
      e->value = (uint8_t)(value >> (8 * (width - 1 - i)));
   }
}

static void dj_free(void)
{
   int i;
   for (i = 0; i < DJ_REGIONS; ++i)
   {
      free(dj_flags[i]);
      dj_flags[i] = NULL;
      free(dj_brk[i]);
      dj_brk[i] = NULL;
      dj_size[i] = 0;
   }
   free(dj_rom);
   dj_rom = NULL;
   dj_rom_file_offset = -1;
   dj_brk_left = 0;
   dj_cur_region = -1;
}

void dj_attach(const char *path)
{
   size_t i;
   int r;
   struct stat st;

   dj_free();
   dj_loaded = 0;
   /* A Mega Drive / Genesis cartridge: not the 8-bit systems (Z80), the Mega CD or the
    * Pico. */
   if (!path || system_hw != SYSTEM_MD || !cart.romsize)
      return;
   dj_loaded = 1;

   dj_size[DJ_ROM] = cart.romsize;
   dj_size[DJ_RAM] = sizeof(work_ram);
   for (r = 0; r < DJ_REGIONS; ++r)
   {
      dj_flags[r] = calloc(dj_size[r], 1);
      dj_brk[r] = calloc((dj_size[r] + 7) / 8, 1);
      if (!dj_flags[r] || !dj_brk[r])
      {
         dj_free();
         dj_loaded = 0;
         return;
      }
   }
   dj_rom = malloc(cart.romsize);
   if (dj_rom)
   {
      for (i = 0; i < cart.romsize; ++i)
         dj_rom[i] = READ_BYTE(cart.rom, i);
   }
   /* A plain dump is the ROM byte for byte. One the core changed while loading it (an .smd
    * file's header and interleaving, a byte-swapped dump) has no file offset. */
   if (stat(path, &st) == 0 && (uint64_t) st.st_size == (uint64_t) cart.romsize)
      dj_rom_file_offset = 0;
}

static unsigned dj_api_version(void)
{
   /* dj is driving the core: from here on every instruction is reported. */
   dj_m68k_exec = dj_exec;
   dj_m68k_fetch = dj_fetch;
   return DJ_API_VERSION;
}

static const char *dj_build_id(void)
{
   return DJ_BUILD_ID;
}

static size_t dj_region_count(void)
{
   return dj_loaded ? DJ_REGIONS : 0;
}

static bool dj_get_region(size_t index, struct dj_region *out)
{
   if (!out || !dj_loaded || index >= DJ_REGIONS)
      return false;
   out->name = dj_names[index];
   out->flags = dj_flags[index];
   out->size = dj_size[index];
   out->cpu = "m68k";
   /* RAM is kept byte-swapped on little-endian hosts and changes as the game runs. */
   out->data = index == DJ_ROM ? dj_rom : NULL;
   out->file_offset = index == DJ_ROM ? dj_rom_file_offset : -1;
   return true;
}

static void dj_clear(void)
{
   int i;
   for (i = 0; i < DJ_REGIONS; ++i)
      if (dj_flags[i])
         memset(dj_flags[i], 0, dj_size[i]);
}

/* Replaces the breakpoints. n = 0, or no function, turns them off. Places outside the
 * regions are left out. */
static void dj_set_breakpoints(const struct dj_break *list, size_t n, dj_break_fn fn, void *user)
{
   size_t i;
   for (i = 0; i < DJ_REGIONS; ++i)
      if (dj_brk[i])
         memset(dj_brk[i], 0, (dj_size[i] + 7) / 8);
   dj_brk_left = 0;
   dj_brk_fn = fn;
   dj_brk_user = user;
   if (!fn || !list)
      return;
   for (i = 0; i < n; ++i)
   {
      int region = list[i].region;
      int64_t off = list[i].offset;
      if (region < 0 || region >= DJ_REGIONS || !dj_brk[region] || off < 0 || (size_t) off >= dj_size[region])
         continue;
      if (!(dj_brk[region][off >> 3] & (1 << (off & 7))))
      {
         dj_brk[region][off >> 3] |= (uint8_t)(1 << (off & 7));
         ++dj_brk_left;
      }
   }
}

/* The 68000's registers: D0 to D7, A0 to A7 (A7 the stack pointer in use), the PC, then
 * the SR, each a 32-bit value in the host's byte order (72 bytes). At a breakpoint the PC
 * is the address of the instruction about to run. */
static size_t dj_read_registers(void *out, size_t cap)
{
   uint32_t *o = out;
   int i;
   if (!out || !dj_loaded || cap < 18 * sizeof(uint32_t))
      return 0;
   for (i = 0; i < 16; ++i)
      o[i] = (uint32_t) m68k_get_reg((m68k_register_t)(M68K_REG_D0 + i));
   o[16] = (uint32_t) m68k_get_reg(M68K_REG_PC);
   o[17] = (uint32_t) m68k_get_reg(M68K_REG_SR);
   return 18 * sizeof(uint32_t);
}

static bool dj_trace_start(size_t capacity)
{
   free(dj_trace);
   dj_trace = NULL;
   dj_trace_len = dj_trace_alloc = 0;
   dj_trace_cap = capacity;
   dj_trace_dropped = 0;
   dj_trace_insn = 0;
   dj_trace_last_exec = -1;
   dj_tracing = capacity > 0;
   dj_m68k_access = dj_tracing ? dj_access : NULL;
   /* The hooks are needed whether or not dj asked for the API version first. */
   dj_m68k_exec = dj_exec;
   dj_m68k_fetch = dj_fetch;
   return true;
}

static void dj_trace_stop(void)
{
   dj_tracing = 0;
   dj_m68k_access = NULL;
   dj_trace_last_exec = -1;
}

/* The events recorded since dj_trace_start; valid until the next dj_trace_start. */
static size_t dj_trace_read(const struct dj_trace_event **events, uint64_t *dropped)
{
   if (events)
      *events = dj_trace_len ? dj_trace : NULL;
   if (dropped)
      *dropped = dj_trace_dropped;
   return dj_trace_len;
}

void *dj_get_proc_address(const char *sym)
{
   if (!sym) return NULL;
   if (!strcmp(sym, "dj_api_version")) return (void *) &dj_api_version;
   if (!strcmp(sym, "dj_build_id")) return (void *) &dj_build_id;
   if (!strcmp(sym, "dj_region_count")) return (void *) &dj_region_count;
   if (!strcmp(sym, "dj_get_region")) return (void *) &dj_get_region;
   if (!strcmp(sym, "dj_clear")) return (void *) &dj_clear;
   if (!strcmp(sym, "dj_set_breakpoints")) return (void *) &dj_set_breakpoints;
   if (!strcmp(sym, "dj_read_registers")) return (void *) &dj_read_registers;
   if (!strcmp(sym, "dj_trace_start")) return (void *) &dj_trace_start;
   if (!strcmp(sym, "dj_trace_stop")) return (void *) &dj_trace_stop;
   if (!strcmp(sym, "dj_trace_read")) return (void *) &dj_trace_read;
   return NULL;
}
