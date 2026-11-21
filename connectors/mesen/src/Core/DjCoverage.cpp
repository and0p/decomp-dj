/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 the decomp-dj developers. decomp-dj's own file, under the MIT license
 * (connectors/mesen/src/LICENSE.txt). It is built into Mesen, which is GPL-3.0, as is the built core. */
#include "stdafx.h"
#include <cstring>
#include <vector>
#include "DjCoverage.h"
#include "BaseMapper.h"
#include "DebuggerTypes.h"
#include "Types.h"

// The build id (connector API 2, optional): the upstream commit and this patch, written into
// DjBuildId.h by build.sh, so the same source gives the same id on every platform.
#if defined(__has_include)
#if __has_include("DjBuildId.h")
#include "DjBuildId.h"
#endif
#endif
#ifndef DJ_BUILD_ID
#define DJ_BUILD_ID "mesen@unknown"
#endif

namespace
{
	enum RegionIndex { RegRam = 0, RegPrg, RegWram, RegSram, RegCount };
	const char* const RegionNames[RegCount] = { "RAM", "PRG", "WRAM", "SRAM" };

	BaseMapper* _mapper = nullptr;
	std::vector<uint8_t> _flags[RegCount];
	const uint8_t* _data[RegCount] = {};
	int64_t _fileOffset[RegCount] = { -1, -1, -1, -1 };

	std::vector<dj_trace_event> _trace;
	size_t _traceCap = 0;
	uint64_t _traceDropped = 0;
	int64_t _lastExec = -1;

	// Breakpoints: one bit per byte of each region, apart from the coverage flags (the
	// player stores those and tests them for "did anything run").
	std::vector<uint8_t> _brk[RegCount];
	int _brkLeft = 0;
	dj_break_fn _brkFn = nullptr;
	void* _brkUser = nullptr;

	void ClearBreaks()
	{
		for(int i = 0; i < RegCount; i++) {
			_brk[i].assign((_flags[i].size() + 7) / 8, 0);
		}
		_brkLeft = 0;
	}

	void Attach(BaseMapper* mapper)
	{
		_mapper = mapper;
		_flags[RegRam].assign(0x800, 0);
		_flags[RegPrg].assign(mapper->GetMemorySize(DebugMemoryType::PrgRom), 0);
		_flags[RegWram].assign(mapper->GetMemorySize(DebugMemoryType::WorkRam), 0);
		_flags[RegSram].assign(mapper->GetMemorySize(DebugMemoryType::SaveRam), 0);
		// Internal RAM isn't handed out (the player doesn't decode it); PRG, WRAM and SRAM
		// are the mapper's own buffers.
		_data[RegRam] = nullptr;
		_data[RegPrg] = mapper->GetPrgRom();
		_data[RegWram] = _flags[RegWram].empty() ? nullptr : mapper->GetWorkRam();
		_data[RegSram] = _flags[RegSram].empty() ? nullptr : mapper->GetSaveRam();
		// PRG follows the 16-byte iNES header (and a 512-byte trainer, if any); other
		// formats (UNIF, FDS) don't lay PRG out as one run of the file.
		RomInfo info = mapper->GetRomInfo();
		_fileOffset[RegPrg] = info.Format == RomFormat::iNes ? 16 + (info.HasTrainer ? 512 : 0) : -1;
		// The regions were sized afresh: the frontend sets its breakpoints again.
		ClearBreaks();
	}

	int ToRegion(AddressType type)
	{
		switch(type) {
			case AddressType::InternalRam: return RegRam;
			case AddressType::PrgRom: return RegPrg;
			case AddressType::WorkRam: return RegWram;
			case AddressType::SaveRam: return RegSram;
			default: return -1;
		}
	}
}

bool DjCoverage::TraceOn = false;
const State* DjCoverage::Regs = nullptr;

void DjCoverage::OnTrace(BaseMapper* mapper, int operationType, uint16_t addr, uint8_t value, uint8_t a, uint8_t x, uint8_t y, uint8_t sp, uint8_t p, uint64_t cycle)
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
	e.cycle = cycle;
	e.addr = addr;
	e.kind = kind;
	e.value = value;
	e.a = a; e.x = x; e.y = y; e.sp = sp; e.p = p;
	e.region = -1;
	e.offset = -1;
	if(mapper) {
		AddressTypeInfo info;
		mapper->GetAbsoluteAddressAndType(addr, &info);
		int region = ToRegion(info.Type);
		if(info.Address >= 0 && region >= 0) {
			e.region = (int8_t)region;
			e.offset = info.Address;
		}
	}
	if(kind == DJ_TRACE_EXEC) {
		_lastExec = (int64_t)_trace.size();
	}
	_trace.push_back(e);
}

void DjCoverage::OnLoad(BaseMapper* mapper)
{
	if(mapper) {
		Attach(mapper);
	}
}

void DjCoverage::OnExec(BaseMapper* mapper, uint16_t addr, bool opcode)
{
	if(mapper != _mapper) {
		Attach(mapper);
	}
	AddressTypeInfo info;
	mapper->GetAbsoluteAddressAndType(addr, &info);
	if(info.Address < 0) {
		return;
	}
	int region = ToRegion(info.Type);
	if(region < 0 || (size_t)info.Address >= _flags[region].size()) {
		return;
	}
	_flags[region][info.Address] |= opcode ? DJ_FLAG_OPCODE : DJ_FLAG_OPERAND;
	if(opcode && _brkLeft > 0) {
		size_t at = (size_t)info.Address;
		uint8_t bit = (uint8_t)(1 << (at & 7));
		if(_brk[region][at >> 3] & bit) {
			// The frontend is called here, in the opcode fetch, before the instruction runs.
			if(_brkFn(_brkUser, region, info.Address)) {
				_brk[region][at >> 3] &= ~bit;
				_brkLeft--;
			}
		}
	}
}

extern "C" {
	static unsigned dj_api_version(void) { return DJ_API_VERSION; }
	static const char* dj_build_id(void) { return DJ_BUILD_ID; }
	static size_t dj_region_count(void) { return RegCount; }

	static bool dj_get_region(size_t index, struct dj_region* out)
	{
		if(index >= RegCount || !out) {
			return false;
		}
		out->name = RegionNames[index];
		out->flags = _flags[index].empty() ? nullptr : _flags[index].data();
		out->size = _flags[index].size();
		out->cpu = "6502";
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

	// The 6502's registers: A, X, Y, SP, P, then PC low byte first (7 bytes). Returns the
	// bytes written, 0 if `out` is too small or the CPU hasn't run yet.
	static size_t dj_read_registers(void* out, size_t cap)
	{
		const State* s = DjCoverage::Regs;
		if(!s || !out || cap < 7) {
			return 0;
		}
		uint8_t* o = (uint8_t*)out;
		o[0] = s->A; o[1] = s->X; o[2] = s->Y; o[3] = s->SP; o[4] = s->PS;
		o[5] = (uint8_t)(s->PC & 0xFF); o[6] = (uint8_t)(s->PC >> 8);
		return 7;
	}

	// Starts tracing into a fresh buffer of `capacity` events (0 stops tracing). Events past
	// the capacity are counted as dropped. The buffer is kept until the next call.
	static void dj_trace_start(size_t capacity)
	{
		_trace.clear();
		_trace.shrink_to_fit();
		_trace.reserve(capacity);
		_traceCap = capacity;
		_traceDropped = 0;
		_lastExec = -1;
		DjCoverage::TraceOn = capacity > 0;
	}

	static void dj_trace_stop(void) { DjCoverage::TraceOn = false; }

	// The events recorded since dj_trace_start; valid until the next dj_trace_start.
	static size_t dj_trace_read(const struct dj_trace_event** events, uint64_t* dropped)
	{
		if(events) *events = _trace.empty() ? nullptr : _trace.data();
		if(dropped) *dropped = _traceDropped;
		return _trace.size();
	}
}

void* DjCoverage::GetProcAddress(const char* sym)
{
	if(!sym) return nullptr;
	if(!strcmp(sym, "dj_api_version")) return (void*)&dj_api_version;
	if(!strcmp(sym, "dj_build_id")) return (void*)&dj_build_id;
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
