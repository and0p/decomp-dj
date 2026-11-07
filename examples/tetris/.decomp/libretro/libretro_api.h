/*
 * The part of the libretro API (https://www.libretro.com) this core uses,
 * written out here so that the core builds without fetching libretro.h. The
 * names, values and layouts are libretro's; see its libretro.h for the full
 * API and documentation.
 */
#ifndef TETRIS_LIBRETRO_API_H
#define TETRIS_LIBRETRO_API_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32)
#define RETRO_API __declspec(dllexport)
#else
#define RETRO_API __attribute__((visibility("default")))
#endif

#define RETRO_API_VERSION 1

#define RETRO_DEVICE_NONE 0
#define RETRO_DEVICE_JOYPAD 1

#define RETRO_DEVICE_ID_JOYPAD_B 0
#define RETRO_DEVICE_ID_JOYPAD_Y 1
#define RETRO_DEVICE_ID_JOYPAD_SELECT 2
#define RETRO_DEVICE_ID_JOYPAD_START 3
#define RETRO_DEVICE_ID_JOYPAD_UP 4
#define RETRO_DEVICE_ID_JOYPAD_DOWN 5
#define RETRO_DEVICE_ID_JOYPAD_LEFT 6
#define RETRO_DEVICE_ID_JOYPAD_RIGHT 7
#define RETRO_DEVICE_ID_JOYPAD_A 8

#define RETRO_REGION_NTSC 0

#define RETRO_MEMORY_SAVE_RAM 0
#define RETRO_MEMORY_SYSTEM_RAM 2
#define RETRO_MEMORY_VIDEO_RAM 3

#define RETRO_ENVIRONMENT_EXPERIMENTAL 0x10000
#define RETRO_ENVIRONMENT_SET_PIXEL_FORMAT 10
#define RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS 11
#define RETRO_ENVIRONMENT_GET_LOG_INTERFACE 27
#define RETRO_ENVIRONMENT_SET_MEMORY_MAPS (36 | RETRO_ENVIRONMENT_EXPERIMENTAL)
#define RETRO_ENVIRONMENT_SET_SERIALIZATION_QUIRKS 44

#define RETRO_SERIALIZATION_QUIRK_CORE_VARIABLE_SIZE (1 << 2)

#define RETRO_MEMDESC_CONST (1 << 0)
#define RETRO_MEMDESC_SYSTEM_RAM (1 << 2)

enum retro_pixel_format { RETRO_PIXEL_FORMAT_0RGB1555 = 0, RETRO_PIXEL_FORMAT_XRGB8888 = 1, RETRO_PIXEL_FORMAT_RGB565 = 2 };

enum retro_log_level { RETRO_LOG_DEBUG = 0, RETRO_LOG_INFO, RETRO_LOG_WARN, RETRO_LOG_ERROR };
typedef void (*retro_log_printf_t)(enum retro_log_level level, const char *fmt, ...);
struct retro_log_callback {
    retro_log_printf_t log;
};

struct retro_input_descriptor {
    unsigned port;
    unsigned device;
    unsigned index;
    unsigned id;
    const char *description;
};

struct retro_memory_descriptor {
    uint64_t flags;
    void *ptr;
    size_t offset;
    size_t start;
    size_t select;
    size_t disconnect;
    size_t len;
    const char *addrspace;
};

struct retro_memory_map {
    const struct retro_memory_descriptor *descriptors;
    unsigned num_descriptors;
};

struct retro_system_info {
    const char *library_name;
    const char *library_version;
    const char *valid_extensions;
    bool need_fullpath;
    bool block_extract;
};

struct retro_game_geometry {
    unsigned base_width;
    unsigned base_height;
    unsigned max_width;
    unsigned max_height;
    float aspect_ratio;
};

struct retro_system_timing {
    double fps;
    double sample_rate;
};

struct retro_system_av_info {
    struct retro_game_geometry geometry;
    struct retro_system_timing timing;
};

struct retro_game_info {
    const char *path;
    const void *data;
    size_t size;
    const char *meta;
};

typedef bool (*retro_environment_t)(unsigned cmd, void *data);
typedef void (*retro_video_refresh_t)(const void *data, unsigned width, unsigned height, size_t pitch);
typedef void (*retro_audio_sample_t)(int16_t left, int16_t right);
typedef size_t (*retro_audio_sample_batch_t)(const int16_t *data, size_t frames);
typedef void (*retro_input_poll_t)(void);
typedef int16_t (*retro_input_state_t)(unsigned port, unsigned device, unsigned index, unsigned id);

#endif
