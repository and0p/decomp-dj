# The decomp as a static library: the game logic only, no ROM, no platform.
#
#   make            builds build/libtetris.a
#   make BUILD=dir  puts the objects and library in dir instead
#
# Needs a C11 compiler. A frontend includes include/tetris/tetris.h and links
# libtetris.a.

CC ?= cc
AR ?= ar
CFLAGS ?= -O2
DECOMP_CFLAGS := $(CFLAGS) -std=c11 -Wall -Wextra -Wno-implicit-fallthrough -Iinclude
BUILD ?= build

SRCS := $(wildcard src/*.c)
OBJS := $(patsubst src/%.c,$(BUILD)/%.o,$(SRCS))
HDRS := $(wildcard src/*.h) $(wildcard include/tetris/*.h)

$(BUILD)/libtetris.a: $(OBJS)
	$(AR) rcs $@ $^

$(BUILD)/%.o: src/%.c $(HDRS) | $(BUILD)
	$(CC) $(DECOMP_CFLAGS) -c $< -o $@

$(BUILD):
	mkdir -p $@

clean:
	rm -rf $(BUILD)

.PHONY: clean
