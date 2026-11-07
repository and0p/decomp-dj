# The Tetris port as a static library, independent of any frontend.
#
#   make            builds libtetris.a here
#   make BUILD=dir  puts the objects and library in dir instead
#
# It needs only a C99 compiler. A frontend links libtetris.a and provides
# the functions declared in include/tetris/hw.h.

CC ?= cc
AR ?= ar
CFLAGS ?= -O2
PORT_CFLAGS := $(CFLAGS) -std=c99 -Wall -Wextra -Iinclude
BUILD ?= build

SRCS := $(wildcard src/*.c)
OBJS := $(patsubst src/%.c,$(BUILD)/%.o,$(SRCS))

$(BUILD)/libtetris.a: $(OBJS)
	$(AR) rcs $@ $^

$(BUILD)/%.o: src/%.c src/game.h src/ram_map.h src/rom_labels.h include/tetris/port.h include/tetris/hw.h | $(BUILD)
	$(CC) $(PORT_CFLAGS) -c $< -o $@

$(BUILD):
	mkdir -p $@

clean:
	rm -rf $(BUILD)

.PHONY: clean
