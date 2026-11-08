# The libretro core: the port (../port) plus this wrapper.
#
#   make                 builds tetris_libretro.so (.dylib on macOS, .dll on
#                        Windows) in ../../build/decomp, i.e. examples/tetris/build/decomp
#   make BUILD=dir       builds in dir instead
#
# Needs a C99 compiler and pthreads.

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

CORE := $(BUILD)/tetris_libretro.$(EXT)
PORT_LIB := $(BUILD)/port/libtetris.a
WRAP_CFLAGS := $(CFLAGS) -std=c99 -Wall -Wextra -fPIC -fvisibility=hidden -I../port/include -D_POSIX_C_SOURCE=200809L

$(CORE): $(BUILD)/tetris_libretro.o $(BUILD)/nes.o $(PORT_LIB)
	$(CC) $(SHARED) -o $@ $^ -lpthread -lm

$(BUILD)/%.o: %.c nes.h libretro_api.h ../port/include/tetris/port.h ../port/include/tetris/hw.h | $(BUILD)
	$(CC) $(WRAP_CFLAGS) -c $< -o $@

# The port is built by its own Makefile, with position-independent code for the shared library.
$(PORT_LIB): FORCE | $(BUILD)
	$(MAKE) -C ../port BUILD=$(abspath $(BUILD))/port CFLAGS="$(CFLAGS) -fPIC"

$(BUILD):
	mkdir -p $@

clean:
	rm -rf $(BUILD)/port $(BUILD)/*.o $(CORE)

FORCE:
.PHONY: clean FORCE
