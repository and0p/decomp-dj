# The libretro core: the decomp (../decomp), drawn by the 3D port (../port),
# plus this wrapper.
#
#   make ROM=tetris.nes  builds tetris_decomp_libretro.so (.dylib on macOS,
#                        .dll on Windows) in ../../build/decomp, that is
#                        examples/tetris/build/decomp. The ROM is needed the first time
#                        only, for the decomp's data tables (../decomp/Makefile)
#   make BUILD=dir       builds in dir instead
#
# Needs a C11 compiler, and Python 3 the first time.

CC ?= cc
CFLAGS ?= -O2
BUILD ?= ../../build/decomp
# decomp-dj's header for comparison cores (dj_compare.h).
DJ_INCLUDE ?= ../../../../connectors/include

UNAME := $(shell uname -s)
ifeq ($(UNAME),Darwin)
  EXT := dylib
  SHARED := -dynamiclib
else ifneq (,$(findstring MINGW,$(UNAME)))
  EXT := dll
  SHARED := -shared
else
  EXT := so
  SHARED := -shared
endif

ABS := $(abspath $(BUILD))
CORE := $(BUILD)/tetris_decomp_libretro.$(EXT)
DECOMP_LIB := $(BUILD)/decomp/libtetris.a
PORT_LIB := $(BUILD)/port/libtetris_port.a
WRAP_CFLAGS := $(CFLAGS) -std=c11 -Wall -Wextra -fPIC -fvisibility=hidden -I../port/include -I../decomp/include -I$(DJ_INCLUDE)

$(CORE): $(BUILD)/tetris_libretro.o $(PORT_LIB) $(DECOMP_LIB)
	$(CC) $(SHARED) -o $@ $^ -lm

$(BUILD)/tetris_libretro.o: tetris_libretro.c libretro_api.h $(DJ_INCLUDE)/dj_compare.h $(wildcard ../port/include/tetris/*.h ../decomp/include/tetris/*.h) | $(BUILD)
	$(CC) $(WRAP_CFLAGS) -c $< -o $@

# The libraries are built by their own Makefiles, position-independent for the shared library.
$(DECOMP_LIB): FORCE | $(BUILD)
	$(MAKE) -C ../decomp BUILD=$(ABS)/decomp CFLAGS="$(CFLAGS) -fPIC -fvisibility=hidden" $(if $(ROM),ROM="$(abspath $(ROM))")

$(PORT_LIB): FORCE | $(BUILD)
	$(MAKE) -C ../port BUILD=$(ABS)/port CFLAGS="$(CFLAGS) -fPIC -fvisibility=hidden" $(ABS)/port/libtetris_port.a

$(BUILD):
	mkdir -p $@

clean:
	rm -rf $(BUILD)/decomp $(BUILD)/port $(BUILD)/*.o $(CORE)

FORCE:
.PHONY: clean FORCE
