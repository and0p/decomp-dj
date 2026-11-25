/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/mesen-s/src/LICENSE.txt). It is built into Mesen-S, which is GPL-3.0, as is the built core. */
// decomp-dj coverage connector for Mesen-S; see DjConnector.h and connectors/mesen-s/README.md.
//
// Regions, in index order (all "65816"):
//   ROM   the cartridge's ROM as the file has it, after a 512-byte copier header if there is
//         one. Mesen-S's own mapping (LoROM, HiROM, ExHiROM, and the coprocessor boards')
//         turns each CPU address into an offset here.
//   WRAM  the console's 128 KB of work RAM ($7E0000-$7FFFFF, and its mirror at $0000-$1FFF).
//   SRAM  the cartridge's save RAM, when it has some.
#include "stdafx.h"
#include <cstring>
#include <vector>
#include "DjConnector.h"
#include "Console.h"
#include "CpuTypes.h"
#include "MemoryManager.h"
#include "MemoryMappings.h"
#include "BaseCartridge.h"
#include "CartTypes.h"
#include "DebugTypes.h"
#include "SnesMemoryType.h"

// The build id (connector API 2, optional): the upstream commit and this connector, written
// into DjBuildId.h by build.sh, so the same source gives the same id on every platform.
#if defined(__has_include)
#if __has_include("DjBuildId.h")
#include "DjBuildId.h"
#endif
#endif
#ifndef DJ_BUILD_ID
#define DJ_BUILD_ID "mesen-s@unknown"
#endif

namespace
{
	enum RegionIndex { RegRom = 0, RegWram, RegSram, RegCount };
	const char* const RegionNames[RegCount] = { "ROM", "WRAM", "SRAM" };

	MemoryManager* _mm = nullptr;      // the memory the regions were sized for
	bool _hasRegions = false;          // false for a Game Boy game (Super Game Boy aside)
	size_t _fileSize = 0;
	std::vector<uint8_t> _flags[RegCount];
	const uint8_t* _data[RegCount] = {};
	int64_t _fileOffset[RegCount] = { 0, -1, -1 };

	// Breakpoints: one bit per byte of each region, apart from the coverage flags (the
	// player stores those and tests them for "did anything run").
	std::vector<uint8_t> _brk[RegCount];
	int _brkLeft = 0;
	dj_break_fn _brkFn = nullptr;
	void* _brkUser = nullptr;

	std::vector<dj_trace_event> _trace;
	size_t _traceCap = 0;
	uint64_t _traceDropped = 0;
	int64_t _lastExec = -1;

	void ClearBreaks()
	{
		for(int i = 0; i < RegCount; i++) {
			_brk[i].assign((_flags[i].size() + 7) / 8, 0);
		}
		_brkLeft = 0;
	}

	void Detach()
	{
		_mm = nullptr;
		_hasRegions = false;
		for(int i = 0; i < RegCount; i++) {
			_flags[i].clear();
			_data[i] = nullptr;
		}
		ClearBreaks();
	}

	void Attach(Console* console)
	{
		Detach();
		shared_ptr<BaseCartridge> cart = console ? console->GetCartridge() : nullptr;
		shared_ptr<MemoryManager> mm = console ? console->GetMemoryManager() : nullptr;
		if(!cart || !mm) {
			return;
		}
		_mm = mm.get();
		// A Game Boy game runs on Mesen-S's Game Boy core, which isn't hooked. The Super Game
		// Boy is a SNES cartridge (its BIOS runs on the 65816) and keeps its regions.
		if(cart->GetGameboy() && !cart->GetCoprocessor()) {
			return;
		}
		_hasRegions = true;
		bool header = (cart->GetCartFlags() & CartFlags::CopierHeader) != 0;
		size_t romSize = cart->DebugGetPrgRomSize();
		// Mesen-S pads the ROM to a multiple of 4 KB: the region is as long as the file.
		size_t inFile = _fileSize > (header ? 512u : 0u) ? _fileSize - (header ? 512 : 0) : 0;
		if(inFile && inFile < romSize) {
			romSize = inFile;
		}
		_flags[RegRom].assign(romSize, 0);
		_data[RegRom] = cart->DebugGetPrgRom();
		_fileOffset[RegRom] = header ? 512 : 0;
		_flags[RegWram].assign(MemoryManager::WorkRamSize, 0);
		_data[RegWram] = mm->DebugGetWorkRam();
		_flags[RegSram].assign(cart->DebugGetSaveRamSize(), 0);
		_data[RegSram] = _flags[RegSram].empty() ? nullptr : cart->DebugGetSaveRam();
		// The regions were sized afresh: the frontend sets its breakpoints again.
		ClearBreaks();
	}

	int ToRegion(SnesMemoryType type)
	{
		switch(type) {
			case SnesMemoryType::PrgRom: return RegRom;
			case SnesMemoryType::WorkRam: return RegWram;
			case SnesMemoryType::SaveRam: return RegSram;
			default: return -1;
		}
	}

	// The region and offset a CPU address maps to now, or -1.
	int Locate(MemoryManager* mm, uint32_t addr, int32_t& offset)
	{
		AddressInfo info = mm->GetMemoryMappings()->GetAbsoluteAddress(addr);
		int region = ToRegion(info.Type);
		if(region < 0 || info.Address < 0 || (size_t)info.Address >= _flags[region].size()) {
			return -1;
		}
		offset = info.Address;
		return region;
	}
}

bool DjConnector::Active = false;
bool DjConnector::TraceOn = false;
const CpuState* DjConnector::Regs = nullptr;

void DjConnector::OnLoad(Console* console, size_t fileSize)
{
	_fileSize = fileSize;
	Attach(console);
}

void DjConnector::OnUnload()
{
	Detach();
	Regs = nullptr;
}

void DjConnector::OnExec(Console* console, MemoryManager* mm, uint32_t addr, bool opcode, uint8_t ps, bool emulation)
{
	if(mm != _mm) {
		// The console was loaded again (a power cycle builds new memory): size the regions
		// for it.
		Attach(console);
	}
	if(!_hasRegions) {
		return;
	}
	int32_t off;
	int region = Locate(mm, addr, off);
	if(region < 0) {
		return;
	}
	if(!opcode) {
		_flags[region][off] |= DJ_FLAG_OPERAND;
		return;
	}
	// The register widths the opcode runs with: emulation mode is 8-bit throughout.
	bool m8 = emulation || (ps & ProcFlags::MemoryMode8);
	bool x8 = emulation || (ps & ProcFlags::IndexMode8);
	_flags[region][off] |= DJ_FLAG_OPCODE | (m8 ? DJ_FLAG_M8 : DJ_FLAG_M16) | (x8 ? DJ_FLAG_X8 : DJ_FLAG_X16);
	if(_brkLeft > 0) {
		uint8_t bit = (uint8_t)(1 << (off & 7));
		if(_brk[region][off >> 3] & bit) {
			// The frontend is called here, before the opcode is read and the instruction runs.
			if(_brkFn(_brkUser, region, off)) {
				_brk[region][off >> 3] &= ~bit;
				_brkLeft--;
			}
		}
	}
}

void DjConnector::OnTrace(MemoryManager* mm, int operationType, uint32_t addr, uint8_t value, const CpuState* s)
{
	uint8_t kind;
	switch((MemoryOperationType)operationType) {
		case MemoryOperationType::ExecOpCode: kind = DJ_TRACE_EXEC; break;
		case MemoryOperationType::ExecOperand:
			if(_lastExec >= 0 && (size_t)_lastExec < _trace.size()) {
				dj_trace_event& e = _trace[_lastExec];
				if(e.operand_count < 3) {
					e.operands[e.operand_count++] = value;
				}
			}
			return;
		case MemoryOperationType::Read: kind = DJ_TRACE_READ; break;
		case MemoryOperationType::Write: kind = DJ_TRACE_WRITE; break;
		default: return;
	}
	if(_trace.size() >= _traceCap) {
		_traceDropped++;
		if(kind == DJ_TRACE_EXEC) {
			_lastExec = -1;
		}
		return;
	}
	dj_trace_event e;
	memset(&e, 0, sizeof(e));
	e.cycle = s->CycleCount;
	e.addr = (uint16_t)addr;
	e.addr_high = (uint16_t)((addr >> 16) & 0xFF);
	e.kind = kind;
	e.value = value;
	e.a = (uint8_t)s->A; e.x = (uint8_t)s->X; e.y = (uint8_t)s->Y; e.sp = (uint8_t)s->SP; e.p = s->PS;
	e.region = -1;
	e.offset = -1;
	int32_t off;
	int region = _hasRegions && mm ? Locate(mm, addr, off) : -1;
	if(region >= 0) {
		e.region = (int8_t)region;
		e.offset = off;
	}
	if(kind == DJ_TRACE_EXEC) {
		_lastExec = (int64_t)_trace.size();
	}
	_trace.push_back(e);
}

extern "C" {
	static unsigned dj_api_version(void)
	{
		// dj is driving the core: from here on every fetch is reported.
		DjConnector::Active = true;
		return DJ_API_VERSION;
	}

	static const char* dj_build_id(void) { return DJ_BUILD_ID; }

	// Core options the connector needs. Mesen-S fills RAM with random bytes at power-on by
	// default, which would make every run's hashes differ.
	static const char* dj_forced_options(void) { return "mesen-s_ramstate=All 0s\n"; }

	static size_t dj_region_count(void) { return _hasRegions ? RegCount : 0; }

	static bool dj_get_region(size_t index, struct dj_region* out)
	{
		if(!_hasRegions || index >= RegCount || !out) {
			return false;
		}
		out->name = RegionNames[index];
		out->flags = _flags[index].empty() ? nullptr : _flags[index].data();
		out->size = _flags[index].size();
		out->cpu = "65816";
		out->data = _data[index];
		out->file_offset = _fileOffset[index];
		return true;
	}

	static void dj_clear(void)
	{
		for(auto& f : _flags) {
			std::fill(f.begin(), f.end(), 0);
		}
	}

	// Replaces the breakpoints. n = 0, or no function, turns them off. Places outside the
	// regions are left out.
	static void dj_set_breakpoints(const struct dj_break* list, size_t n, dj_break_fn fn, void* user)
	{
		ClearBreaks();
		_brkFn = fn;
		_brkUser = user;
		if(!fn || !list) {
			return;
		}
		for(size_t i = 0; i < n; i++) {
			int region = list[i].region;
			int64_t off = list[i].offset;
			if(region < 0 || region >= RegCount || off < 0 || (size_t)off >= _flags[region].size()) {
				continue;
			}
			uint8_t bit = (uint8_t)(1 << (off & 7));
			if(!(_brk[region][off >> 3] & bit)) {
				_brk[region][off >> 3] |= bit;
				_brkLeft++;
			}
		}
	}

	// The 65816's registers, 16 bytes: A, X, Y, S, D and PC as 16-bit values, low byte
	// first, then K (the program bank), DB (the data bank), P, and E (1 in emulation mode).
	// At a breakpoint PC is the instruction's own address. Returns the bytes written, 0 if
	// `out` is too small or no game is loaded.
	static size_t dj_read_registers(void* out, size_t cap)
	{
		const CpuState* s = DjConnector::Regs;
		if(!s || !out || cap < 16) {
			return 0;
		}
		uint8_t* o = (uint8_t*)out;
		const uint16_t words[6] = { s->A, s->X, s->Y, s->SP, s->D, s->PC };
		for(int i = 0; i < 6; i++) {
			o[i * 2] = (uint8_t)(words[i] & 0xFF);
			o[i * 2 + 1] = (uint8_t)(words[i] >> 8);
		}
		o[12] = s->K;
		o[13] = s->DBR;
		o[14] = s->PS;
		o[15] = s->EmulationMode ? 1 : 0;
		return 16;
	}

	// Starts tracing into a fresh buffer of `capacity` events (0 stops tracing). Events past
	// the capacity are counted as dropped. The buffer is kept until the next call.
	static bool dj_trace_start(size_t capacity)
	{
		_trace.clear();
		_trace.shrink_to_fit();
		_trace.reserve(capacity < (1u << 20) ? capacity : (1u << 20));
		_traceCap = capacity;
		_traceDropped = 0;
		_lastExec = -1;
		DjConnector::TraceOn = capacity > 0;
		return true;
	}

	static void dj_trace_stop(void) { DjConnector::TraceOn = false; }

	// The events recorded since dj_trace_start; valid until the next dj_trace_start.
	static size_t dj_trace_read(const struct dj_trace_event** events, uint64_t* dropped)
	{
		if(events) *events = _trace.empty() ? nullptr : _trace.data();
		if(dropped) *dropped = _traceDropped;
		return _trace.size();
	}
}

void* DjConnector::GetProcAddress(const char* sym)
{
	if(!sym) return nullptr;
	if(!strcmp(sym, "dj_api_version")) return (void*)&dj_api_version;
	if(!strcmp(sym, "dj_build_id")) return (void*)&dj_build_id;
	if(!strcmp(sym, "dj_forced_options")) return (void*)&dj_forced_options;
	if(!strcmp(sym, "dj_region_count")) return (void*)&dj_region_count;
	if(!strcmp(sym, "dj_get_region")) return (void*)&dj_get_region;
	if(!strcmp(sym, "dj_clear")) return (void*)&dj_clear;
	if(!strcmp(sym, "dj_set_breakpoints")) return (void*)&dj_set_breakpoints;
	if(!strcmp(sym, "dj_read_registers")) return (void*)&dj_read_registers;
	if(!strcmp(sym, "dj_trace_start")) return (void*)&dj_trace_start;
	if(!strcmp(sym, "dj_trace_stop")) return (void*)&dj_trace_stop;
	if(!strcmp(sym, "dj_trace_read")) return (void*)&dj_trace_read;
	return nullptr;
}
