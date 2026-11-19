# The decomp as a static library: the game logic only, no platform.
#
#   make ROM=tetris.nes  builds build/libtetris.a. The ROM is needed the first time only:
#                        the game's data tables (src/data.c) are read from it, since they
#                        are not kept in the repository (tools/gen_data.py)
#   make                 builds it once src/data.c is there
#   make BUILD=dir       puts the objects and library in dir instead
#
# Needs a C11 compiler, and Python 3 to make src/data.c. A frontend includes include/tetris/tetris.h and links
# libtetris.a.

CC ?= cc
AR ?= ar
CFLAGS ?= -O2
DECOMP_CFLAGS := $(CFLAGS) -std=c11 -Wall -Wextra -Wno-implicit-fallthrough -Iinclude
BUILD ?= build

ROM ?=

SRCS := $(sort $(wildcard src/*.c) src/data.c)
OBJS := $(patsubst src/%.c,$(BUILD)/%.o,$(SRCS))
HDRS := $(wildcard src/*.h) $(wildcard include/tetris/*.h)

$(BUILD)/libtetris.a: $(OBJS)
	$(AR) rcs $@ $^

$(BUILD)/%.o: src/%.c $(HDRS) | $(BUILD)
	$(CC) $(DECOMP_CFLAGS) -c $< -o $@

# The game's own tables, from the ROM (Tetris, USA; the script checks it is the right one).
src/data.c: src/data.h ../tools/gen_data.py
	@if [ -z "$(ROM)" ]; then echo "src/data.c holds the game's data tables and is made from your Tetris ROM: run make with ROM=path/to/tetris.nes" >&2; exit 1; fi
	python3 ../tools/gen_data.py "$(ROM)" src/data.h $@

$(BUILD):
	mkdir -p $@

clean:
	rm -rf $(BUILD)

.PHONY: clean
