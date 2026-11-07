/*
 * A libretro core that runs the Tetris port: one way to give the port a
 * machine. It loads the player's Tetris (USA) ROM, models the NES hardware
 * the game uses (nes.c) and implements the port's hw.h on it.
 *
 * Threads. The port is written like the original: its main loop runs
 * forever and waits for frames inside hw_idle(). Here it runs on a thread of
 * its own, which takes turns with the frontend's thread; only one of them
 * runs at a time. Each retro_run():
 *
 *   1. starts the vertical blank: sets the PPUSTATUS flag and, if the game
 *      has NMIs enabled, runs tetris_nmi() (on the frontend's thread);
 *   2. hands over to the game thread until it waits for the next frame, or
 *      until the work it reports through hw_cycles() fills a frame (as when it
 *      clears nametables with NMIs off);
 *   3. draws the picture.
 *
 * All libretro callbacks are made from the frontend's thread.
 *
 * Save states. A thread's stack cannot be saved, so a state is the input of
 * every frame since power-on: loading one powers on and replays them. The
 * port is deterministic, so this reproduces the RAM, PPU and mapper exactly.
 * States grow by two bytes a frame. Reset powers the console off and on.
 *
 * Sound is not emulated: the core outputs silence.
 */
#include <pthread.h>
#include <setjmp.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "libretro_api.h"
#include "nes.h"
#include "tetris/hw.h"
#include "tetris/port.h"

#define CYCLES_PER_FRAME 29781 /* NTSC: 341 * 262 / 3 */
#define FPS 60.0988
#define SAMPLE_RATE 48000
#define SAMPLES_PER_FRAME 799

static retro_environment_t environ_cb;
static retro_video_refresh_t video_cb;
static retro_audio_sample_batch_t audio_batch_cb;
static retro_input_poll_t input_poll_cb;
static retro_input_state_t input_state_cb;
static retro_log_printf_t log_cb;

static void logf_(enum retro_log_level level, const char *fmt, ...)
{
    char buf[256];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    if (log_cb)
        log_cb(level, "%s", buf);
    else
        fprintf(stderr, "[tetris] %s", buf);
}

/* ---- The cartridge and the machine ---- */

static uint8_t prgRom[0x8000];
static uint8_t chrRom[0x4000];
static struct nes nes;
static uint32_t framebuffer[NES_WIDTH * NES_HEIGHT];

/* Inputs for each frame since power-on (player 1, player 2; NES bit order). */
static uint8_t *inputLog;
static size_t inputFrames, inputCapacity;

/* ---- The game thread ---- */

static pthread_t gameThread;
static int gameThreadStarted;
static pthread_mutex_t turnLock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t turnChanged = PTHREAD_COND_INITIALIZER;
static int gameTurn;        /* 1 while the game thread runs */
static int restartRequested; /* power on: the game thread starts over at reset */
static int quitRequested;
static jmp_buf powerOn;
static uint32_t frameCycles; /* hw_cycles reported this frame */

static int onGameThread(void) { return gameThreadStarted && pthread_equal(pthread_self(), gameThread); }

/* Game thread: hand the turn back and wait for the next one. */
static void gameYield(void)
{
    pthread_mutex_lock(&turnLock);
    gameTurn = 0;
    pthread_cond_broadcast(&turnChanged);
    while (!gameTurn)
        pthread_cond_wait(&turnChanged, &turnLock);
    int restart = restartRequested, quit = quitRequested;
    restartRequested = 0;
    pthread_mutex_unlock(&turnLock);
    if (quit)
        pthread_exit(NULL);
    if (restart)
        longjmp(powerOn, 1);
}

/* Frontend thread: let the game thread run until it yields. */
static void runGame(void)
{
    pthread_mutex_lock(&turnLock);
    gameTurn = 1;
    pthread_cond_broadcast(&turnChanged);
    while (gameTurn)
        pthread_cond_wait(&turnChanged, &turnLock);
    pthread_mutex_unlock(&turnLock);
}

static void *gameMain(void *arg)
{
    (void)arg;
    pthread_mutex_lock(&turnLock);
    while (!gameTurn)
        pthread_cond_wait(&turnChanged, &turnLock);
    restartRequested = 0;
    int quit = quitRequested;
    pthread_mutex_unlock(&turnLock);
    if (quit)
        return NULL;
    setjmp(powerOn);
    tetris_reset(); /* never returns */
    return NULL;
}

/* ---- hw.h ---- */

uint8_t hw_read(uint16_t addr) { return nes_read(&nes, addr); }
void hw_write(uint16_t addr, uint8_t value) { nes_write(&nes, addr, value, tetris_ram); }

void hw_idle(void)
{
    if (onGameThread())
        gameYield();
}

void hw_cycles(uint32_t cycles)
{
    if (!onGameThread())
        return; /* work in the NMI is not timed */
    frameCycles += cycles;
    while (frameCycles >= CYCLES_PER_FRAME) {
        uint32_t carry = frameCycles - CYCLES_PER_FRAME;
        gameYield();
        frameCycles += carry;
    }
}

/* ---- Frames ---- */

static void powerOnMachine(void)
{
    memset(tetris_ram, 0, sizeof tetris_ram); /* like Mesen's "All 0s" */
    nes_power(&nes, chrRom);
    tetris_set_prg(prgRom);
    frameCycles = 0;
    pthread_mutex_lock(&turnLock);
    restartRequested = 1; /* the game thread restarts at its next turn */
    pthread_mutex_unlock(&turnLock);
}

/* One frame of the machine, with the given controller buttons. */
static void stepFrame(uint8_t pad1, uint8_t pad2)
{
    nes.buttons[0] = pad1;
    nes.buttons[1] = pad2;
    frameCycles = 0;
    nes_vblank(&nes);
    if (nes.ctrl & 0x80)
        tetris_nmi();
    runGame();
}

static void logInput(uint8_t pad1, uint8_t pad2)
{
    if (inputFrames == inputCapacity) {
        size_t cap = inputCapacity ? inputCapacity * 2 : 4096;
        uint8_t *p = realloc(inputLog, cap * 2);
        if (!p)
            return;
        inputLog = p;
        inputCapacity = cap;
    }
    inputLog[inputFrames * 2] = pad1;
    inputLog[inputFrames * 2 + 1] = pad2;
    inputFrames++;
}

static uint8_t readPad(unsigned port)
{
    static const struct {
        unsigned id;
        uint8_t bit;
    } map[8] = {
        {RETRO_DEVICE_ID_JOYPAD_A, 0x80},    {RETRO_DEVICE_ID_JOYPAD_B, 0x40},
        {RETRO_DEVICE_ID_JOYPAD_SELECT, 0x20}, {RETRO_DEVICE_ID_JOYPAD_START, 0x10},
        {RETRO_DEVICE_ID_JOYPAD_UP, 0x08},   {RETRO_DEVICE_ID_JOYPAD_DOWN, 0x04},
        {RETRO_DEVICE_ID_JOYPAD_LEFT, 0x02}, {RETRO_DEVICE_ID_JOYPAD_RIGHT, 0x01},
    };
    uint8_t b = 0;
    for (int i = 0; i < 8; i++)
        if (input_state_cb(port, RETRO_DEVICE_JOYPAD, 0, map[i].id))
            b |= map[i].bit;
    return b;
}

/* ---- libretro ---- */

RETRO_API unsigned retro_api_version(void) { return RETRO_API_VERSION; }

RETRO_API void retro_set_environment(retro_environment_t cb)
{
    environ_cb = cb;
    struct retro_log_callback log;
    if (cb(RETRO_ENVIRONMENT_GET_LOG_INTERFACE, &log))
        log_cb = log.log;
}
RETRO_API void retro_set_video_refresh(retro_video_refresh_t cb) { video_cb = cb; }
RETRO_API void retro_set_audio_sample(retro_audio_sample_t cb) { (void)cb; }
RETRO_API void retro_set_audio_sample_batch(retro_audio_sample_batch_t cb) { audio_batch_cb = cb; }
RETRO_API void retro_set_input_poll(retro_input_poll_t cb) { input_poll_cb = cb; }
RETRO_API void retro_set_input_state(retro_input_state_t cb) { input_state_cb = cb; }

RETRO_API void retro_init(void) {}

RETRO_API void retro_deinit(void)
{
    if (gameThreadStarted) {
        pthread_mutex_lock(&turnLock);
        quitRequested = 1;
        gameTurn = 1;
        pthread_cond_broadcast(&turnChanged);
        pthread_mutex_unlock(&turnLock);
        pthread_join(gameThread, NULL);
        gameThreadStarted = 0;
        quitRequested = 0;
        gameTurn = 0;
    }
    free(inputLog);
    inputLog = NULL;
    inputFrames = inputCapacity = 0;
}

RETRO_API void retro_get_system_info(struct retro_system_info *info)
{
    memset(info, 0, sizeof *info);
    info->library_name = "Tetris C port";
    info->library_version = "0.1";
    info->valid_extensions = "nes";
    info->need_fullpath = false;
}

RETRO_API void retro_get_system_av_info(struct retro_system_av_info *info)
{
    memset(info, 0, sizeof *info);
    info->geometry.base_width = info->geometry.max_width = NES_WIDTH;
    info->geometry.base_height = info->geometry.max_height = NES_HEIGHT;
    info->geometry.aspect_ratio = 4.0f / 3.0f;
    info->timing.fps = FPS;
    info->timing.sample_rate = SAMPLE_RATE;
}

RETRO_API void retro_set_controller_port_device(unsigned port, unsigned device)
{
    (void)port;
    (void)device;
}

/* The PRG ROM of Tetris (USA), which the port's addresses refer to. */
#define TETRIS_USA_PRG_CRC32 0x943DFBBEu

static uint32_t crc32(const uint8_t *p, size_t n)
{
    uint32_t c = 0xFFFFFFFFu;
    while (n--) {
        c ^= *p++;
        for (int k = 0; k < 8; k++)
            c = (c >> 1) ^ (0xEDB88320u & (0u - (c & 1)));
    }
    return ~c;
}

RETRO_API bool retro_load_game(const struct retro_game_info *game)
{
    if (!game || !game->data || game->size < 16 + sizeof prgRom + sizeof chrRom) {
        logf_(RETRO_LOG_ERROR, "no ROM, or too small\n");
        return false;
    }
    const uint8_t *rom = game->data;
    int mapper = (rom[6] >> 4) | (rom[7] & 0xF0);
    if (memcmp(rom, "NES\x1A", 4) || rom[4] != 2 || rom[5] != 2 || mapper != 1 || (rom[6] & 0x04)) {
        logf_(RETRO_LOG_ERROR, "not Tetris: expected an MMC1 ROM with 32 KB PRG and 16 KB CHR\n");
        return false;
    }
    memcpy(prgRom, rom + 16, sizeof prgRom);
    memcpy(chrRom, rom + 16 + sizeof prgRom, sizeof chrRom);
    if (crc32(prgRom, sizeof prgRom) != TETRIS_USA_PRG_CRC32) {
        logf_(RETRO_LOG_ERROR, "this is not Tetris (USA): the port needs that ROM's PRG (CRC32 %08X)\n",
              TETRIS_USA_PRG_CRC32);
        return false;
    }

    enum retro_pixel_format fmt = RETRO_PIXEL_FORMAT_XRGB8888;
    if (!environ_cb(RETRO_ENVIRONMENT_SET_PIXEL_FORMAT, &fmt)) {
        logf_(RETRO_LOG_ERROR, "the frontend does not support XRGB8888\n");
        return false;
    }
    static const struct retro_input_descriptor desc[] = {
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_LEFT, "Left"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_UP, "Up"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_DOWN, "Down"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_RIGHT, "Right"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_B, "B (rotate left)"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_A, "A (rotate right)"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_SELECT, "Select"},
        {0, RETRO_DEVICE_JOYPAD, 0, RETRO_DEVICE_ID_JOYPAD_START, "Start"},
        {0, 0, 0, 0, NULL},
    };
    environ_cb(RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS, (void *)desc);
    unsigned quirks = RETRO_SERIALIZATION_QUIRK_CORE_VARIABLE_SIZE;
    environ_cb(RETRO_ENVIRONMENT_SET_SERIALIZATION_QUIRKS, &quirks);
    static struct retro_memory_descriptor mem[2];
    static struct retro_memory_map map = {mem, 2};
    memset(mem, 0, sizeof mem);
    mem[0].flags = RETRO_MEMDESC_SYSTEM_RAM;
    mem[0].ptr = tetris_ram;
    mem[0].start = 0x0000;
    mem[0].select = 0xE000; /* $0000-$1FFF: the 2 KB and its mirrors */
    mem[0].len = sizeof tetris_ram;
    mem[1].flags = RETRO_MEMDESC_CONST;
    mem[1].ptr = prgRom;
    mem[1].start = 0x8000;
    mem[1].select = 0x8000;
    mem[1].len = sizeof prgRom;
    environ_cb(RETRO_ENVIRONMENT_SET_MEMORY_MAPS, &map);

    powerOnMachine();
    inputFrames = 0;
    if (!gameThreadStarted) {
        if (pthread_create(&gameThread, NULL, gameMain, NULL)) {
            logf_(RETRO_LOG_ERROR, "could not start the game thread\n");
            return false;
        }
        gameThreadStarted = 1;
    }
    return true;
}

RETRO_API bool retro_load_game_special(unsigned type, const struct retro_game_info *info, size_t num)
{
    (void)type;
    (void)info;
    (void)num;
    return false;
}

RETRO_API void retro_unload_game(void) {}

RETRO_API unsigned retro_get_region(void) { return RETRO_REGION_NTSC; }

RETRO_API void retro_reset(void)
{
    powerOnMachine();
    inputFrames = 0;
}

RETRO_API void retro_run(void)
{
    input_poll_cb();
    uint8_t pad1 = readPad(0), pad2 = readPad(1);
    logInput(pad1, pad2);
    stepFrame(pad1, pad2);
    nes_render(&nes, framebuffer);
    video_cb(framebuffer, NES_WIDTH, NES_HEIGHT, NES_WIDTH * sizeof framebuffer[0]);
    if (audio_batch_cb) {
        static const int16_t silence[SAMPLES_PER_FRAME * 2];
        audio_batch_cb(silence, SAMPLES_PER_FRAME);
    }
}

/* ---- Save states: "TTRS", version, frame count, then two bytes a frame ---- */

#define STATE_MAGIC 0x53525454u /* "TTRS" */
#define STATE_VERSION 1u
#define STATE_HEADER 12

static void put32(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}
static uint32_t get32(const uint8_t *p) { return (uint32_t)p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24; }

RETRO_API size_t retro_serialize_size(void) { return STATE_HEADER + inputFrames * 2; }

RETRO_API bool retro_serialize(void *data, size_t size)
{
    if (size < retro_serialize_size())
        return false;
    uint8_t *p = data;
    put32(p, STATE_MAGIC);
    put32(p + 4, STATE_VERSION);
    put32(p + 8, (uint32_t)inputFrames);
    if (inputFrames)
        memcpy(p + STATE_HEADER, inputLog, inputFrames * 2);
    return true;
}

RETRO_API bool retro_unserialize(const void *data, size_t size)
{
    const uint8_t *p = data;
    if (size < STATE_HEADER || get32(p) != STATE_MAGIC || get32(p + 4) != STATE_VERSION)
        return false;
    size_t frames = get32(p + 8);
    if (size < STATE_HEADER + frames * 2)
        return false;
    uint8_t *copy = frames ? malloc(frames * 2) : NULL;
    if (frames && !copy)
        return false;
    if (frames)
        memcpy(copy, p + STATE_HEADER, frames * 2);
    powerOnMachine();
    inputFrames = 0;
    for (size_t f = 0; f < frames; f++) {
        logInput(copy[f * 2], copy[f * 2 + 1]);
        stepFrame(copy[f * 2], copy[f * 2 + 1]);
    }
    free(copy);
    nes_render(&nes, framebuffer);
    return true;
}

RETRO_API void retro_cheat_reset(void) {}
RETRO_API void retro_cheat_set(unsigned index, bool enabled, const char *code)
{
    (void)index;
    (void)enabled;
    (void)code;
}

RETRO_API void *retro_get_memory_data(unsigned id)
{
    switch (id) {
    case RETRO_MEMORY_SYSTEM_RAM: return tetris_ram;
    case RETRO_MEMORY_VIDEO_RAM: return nes.ciram;
    default: return NULL;
    }
}

RETRO_API size_t retro_get_memory_size(unsigned id)
{
    switch (id) {
    case RETRO_MEMORY_SYSTEM_RAM: return sizeof tetris_ram;
    case RETRO_MEMORY_VIDEO_RAM: return sizeof nes.ciram;
    default: return 0;
    }
}
