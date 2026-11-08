# The port as a static library: draws the decomp's game in 3D.
#
#   make            builds build/libtetris_port.a (and the decomp's library)
#
# Link a program with build/libtetris_port.a, ../decomp/build/libtetris.a and
# the maths library (-lm). Needs a C11 compiler.

CC ?= cc
AR ?= ar
CFLAGS ?= -O2
DECOMP ?= ../decomp
PORT_CFLAGS := $(CFLAGS) -std=c11 -Wall -Wextra -Iinclude -I$(DECOMP)/include -Ithird_party
BUILD ?= build

SRCS := $(wildcard src/*.c)
OBJS := $(patsubst src/%.c,$(BUILD)/%.o,$(SRCS))
HDRS := $(wildcard include/tetris/*.h) $(wildcard $(DECOMP)/include/tetris/*.h) $(wildcard third_party/*.h)

all: $(BUILD)/libtetris_port.a $(DECOMP)/build/libtetris.a

$(BUILD)/libtetris_port.a: $(OBJS)
	$(AR) rcs $@ $^

$(DECOMP)/build/libtetris.a: FORCE
	$(MAKE) -C $(DECOMP)

# small3dlib.h is vendored as is: quiet its warnings in the one file that includes it
$(BUILD)/raster_s3l.o: PORT_CFLAGS += -Wno-unused-parameter -Wno-unused-function -Wno-sign-compare -Wno-unused-variable

$(BUILD)/%.o: src/%.c $(HDRS) | $(BUILD)
	$(CC) $(PORT_CFLAGS) -c $< -o $@

$(BUILD):
	mkdir -p $@

clean:
	rm -rf $(BUILD)

.PHONY: all clean FORCE
