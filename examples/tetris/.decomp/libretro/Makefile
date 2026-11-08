# The libretro core: the decomp (../decomp), drawn by the 3D port (../port),
# plus this wrapper.
#
#   make                 builds tetris_decomp_libretro.so (.dylib on macOS,
#                        .dll on Windows) in ../../build/decomp, that is
#                        examples/tetris/build/decomp
#   make BUILD=dir       builds in dir instead
#
# Needs a C11 compiler.

CC ?= cc
CFLAGS ?= -O2
BUILD ?= ../../build/decomp

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
WRAP_CFLAGS := $(CFLAGS) -std=c11 -Wall -Wextra -fPIC -fvisibility=hidden -I../port/include -I../decomp/include

$(CORE): $(BUILD)/tetris_libretro.o $(PORT_LIB) $(DECOMP_LIB)
	$(CC) $(SHARED) -o $@ $^ -lm

$(BUILD)/tetris_libretro.o: tetris_libretro.c libretro_api.h $(wildcard ../port/include/tetris/*.h ../decomp/include/tetris/*.h) | $(BUILD)
	$(CC) $(WRAP_CFLAGS) -c $< -o $@

# The libraries are built by their own Makefiles, position-independent for the shared library.
$(DECOMP_LIB): FORCE | $(BUILD)
	$(MAKE) -C ../decomp BUILD=$(ABS)/decomp CFLAGS="$(CFLAGS) -fPIC -fvisibility=hidden"

$(PORT_LIB): FORCE | $(BUILD)
	$(MAKE) -C ../port BUILD=$(ABS)/port CFLAGS="$(CFLAGS) -fPIC -fvisibility=hidden" $(ABS)/port/libtetris_port.a

$(BUILD):
	mkdir -p $@

clean:
	rm -rf $(BUILD)/decomp $(BUILD)/port $(BUILD)/*.o $(CORE)

FORCE:
.PHONY: clean FORCE
