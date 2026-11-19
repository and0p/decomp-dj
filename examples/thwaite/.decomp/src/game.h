/* All of the decomp's state. A save state is this struct, byte for byte. */
#ifndef THW_GAME_H
#define THW_GAME_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "gfx.h"

/* Where the decomp is parked: just before one of the original's wait loops exits. One per
 * wait loop in the breakpoint table (data/map.c). A step runs from one to the next. */
enum thw_wait {
    W_NONE = 0,       /* somewhere the decomp hasn't written yet */
    W_TODO,           /* display_todo's loop: the legal screen */
    W_TITLE,          /* titleScreen's loop */
    W_SCREEN_OFF,     /* screen_off_title_palette: once, before the legal or title screen */
    W_MAIN,           /* main's game loop */
    W_PAUSE,          /* pauseScreen */
    W_PRACTICE,       /* practice_menu */
    W_CUT1, W_CUT2,   /* load_cutscene */
    W_VBLANK,         /* ppu_wait_vblank (nothing calls it) */
    W_COUNT
};

/* What screen_off_title_palette returns to. */
enum { THEN_TODO = 1, THEN_TITLE = 2 };

/* The buttons, in the original's bit order (nes.inc). */
enum {
    KEY_A = 0x80, KEY_B = 0x40, KEY_SELECT = 0x20, KEY_START = 0x10,
    KEY_UP = 0x08, KEY_DOWN = 0x04, KEY_LEFT = 0x02, KEY_RIGHT = 0x01,
    MOUSE_L = KEY_B, MOUSE_R = KEY_A,
};

/* gameState (global.inc) */
enum {
    STATE_INACTIVE, STATE_NEW_LEVEL, STATE_ACTIVE, STATE_LEVEL_REWARD,
    STATE_REBUILDING_SILO, STATE_CUTSCENE, STATE_REBUILDING_HOUSE, STATE_GAMEOVER,
};

#define NUM_BUILDINGS 12
#define BUILDING_SILO0 2
#define BUILDING_SILO1 9
#define BUILDING_DESTROYED 0
#define BUILDING_OK 1
#define BUILDING_THREATENED 2
#define NUM_MISSILES 20
#define NUM_EXPLOSIONS 8
#define NUM_SMOKE 48
#define NUM_VILLAGERS 12
#define NUM_EXCLS 4
#define BG_DIRTY_HOUSES 0x01
#define BG_DIRTY_STATUS 0x02
#define BG_DIRTY_TIP 0x04
#define BG_DIRTY_PRACTICE_METER 0x08

/* Where a step gets what the original got from outside its game state: the controllers' raw
 * bits, the NMI counter where the logic reads it, and the one value the sound engine leaks
 * into the game (splitBalloon reads the channel the split sound went to). Under dj these
 * come from the original (events.c); played on its own, from libretro (libretro.c). False:
 * there is none, which under dj means the decomp read something the original didn't. */
struct step_io {
    void *ctx;
    bool (*pads)(void *ctx, uint8_t raw[2]);
    bool (*mouse)(void *ctx, int port, uint8_t raw[3]);
    bool (*nmis)(void *ctx, uint8_t *value);
    bool (*sound)(void *ctx, uint8_t *channel);
};

/* A missile. 0-3 are the players' (0 and 2 player 1's), 4-19 the enemy's. A free slot has
 * y_hi 0. For a player missile, `target` is the x it explodes at (missileDstX). */
struct missile {
    uint8_t x_hi, x_lo, y_hi, y_lo;
    uint8_t dx_hi, dx_lo, dy_hi, dy_lo;
    uint8_t angle, type, target, time;
};
enum { MISSILE_NORMAL = 0, MISSILE_MIRV = 1, MISSILE_BALLOON = 2 };

struct crosshair {
    uint8_t x_lo, x_hi, y_lo, y_hi;
    uint8_t dx_lo, dx_hi, dy_lo, dy_hi;
};

struct explosion { uint8_t x, y, time; };
struct smoke { uint8_t x, y, speed, time; };
struct villager { uint8_t x, target; };
struct excl { uint8_t time, x; };

/* The level now being played (levelTable's columns). */
struct level {
    uint8_t missile_speed, missiles_left, release_period, flags;
    uint8_t salvo_sizes[4];
};

/* The background update waiting for the next vblank: two rows of 32 tiles to `dst`
 * (houseXferBuf, inside the original's xferBuf at $0108). `len_marker` and `end_marker` are
 * the popslide header and terminator the blit writes. */
struct bg_update {
    uint8_t dst_hi, dst_lo, len_marker;
    uint8_t tiles[64];
    uint8_t end_marker;
};

#define THW_STATE_VERSION 4

struct game {
    uint32_t version;          /* THW_STATE_VERSION, so an old save state is refused */
    uint8_t  wait;             /* enum thw_wait */
    uint8_t  screen_off_then;  /* THEN_*: where screen_off_title_palette goes on to */
    uint8_t  after_pause;      /* the game loop step goes on after pauseScreen returns */
    uint32_t steps;

    /* The original's zero-page scratch ($00-$0F): copied, never compared. Kept, because the
     * original's routines hand values to each other here. */
    uint8_t zp[16];

    /* The original's RAM the decomp models, by the original's names (compare.c maps them). */
    uint8_t nmis;
    uint8_t oam_used;
    uint8_t debug_hex[2];
    uint8_t cur_keys[2], new_keys[2];
    uint8_t game_state;
    uint8_t num_players;
    uint8_t tv_system;
    uint8_t mouse_enabled[2];
    uint8_t houses_left_class, investigation_step;
    uint8_t rand[4];
    uint8_t lru_targets[NUM_BUILDINGS];
    uint8_t is_practice, practice_side, practice_day, practice_hour;
    uint8_t cur_mbuttons[2], new_mbuttons[2];

    struct level level;
    struct smoke smoke[NUM_SMOKE];
    uint8_t houses_standing[NUM_BUILDINGS];
    uint8_t buildings_destroyed_this_level;
    uint8_t score100s, score1s, hiscore100s, hiscore1s;
    uint8_t day, hour, minute, second, tenth, sub_tenth;
    uint8_t bg_dirty, cur_tip, tip_time_left;
    uint8_t silo_missiles_left[2];
    uint8_t first_destroyed_house, house_to_rebuild;
    struct missile missiles[NUM_MISSILES];
    uint8_t missile_dst_y[4];
    struct crosshair crosshairs[2];
    uint8_t time_till_salvo, salvo_left;
    struct explosion explosions[NUM_EXPLOSIONS];
    struct villager villagers[NUM_VILLAGERS];
    uint8_t doorway_clog_time[NUM_BUILDINGS];
    struct excl excls[NUM_EXCLS];
    uint8_t last_excl_sound_time;

    /* The cutscenes (cutscene.s). gameState holds the cutscene's own state then (CUT_*). */
    uint8_t script_ptr[2];             /* the CPU address of the next script byte, low first */
    uint8_t name_interp_pos;           /* into the names (cutscene.c), or 0xFF: none */
    uint8_t decomp_buf_pos;            /* into dte_buf */
    uint8_t cut_dst_lo, cut_dst_hi;    /* where the next letter goes in VRAM */
    uint8_t cut_scroll, cut_scroll_target;
    uint8_t cut_fade_dir, cut_fade_amount;
    uint8_t actor_ids[3];              /* the villagers playing X, Y, Z: paranoid, detective, laid-back */
    uint8_t cut_actors[4];             /* a script's first four bytes */
    uint8_t dte_buf[32];               /* dte_output_buf: a decompressed line */
    uint8_t popslide_used;             /* the end of the VRAM transfer buffer, from $0100 */

    /* The VRAM transfer buffer (popslide_buf) from $0108: the playfield's update in play;
     * popslide packets, in the cutscenes. Only the first 68 bytes are mapped (compare.c). */
    union {
        struct bg_update bg_update;
        uint8_t xfer[68];
    };
    uint8_t xfer_rest[120 - 68];

    /* The rest of the original's RAM, as last copied: what the decomp doesn't model yet.
     * It is compared too, so a value the original changes and the decomp doesn't shows. */
    uint8_t carry[2048];

    struct gfx gfx;            /* the picture; gfx.oam is the original's OAM buffer */
};

void game_power_on(struct game *g);
/* Runs from g->wait to the next wait the original would reach, and returns it (also left
 * in g->wait). W_NONE: the step went into code the decomp doesn't have; game_unwritten()
 * says what. */
enum thw_wait game_step(struct game *g, const struct step_io *io);
const char *game_unwritten(void);
/* Rebuilds the picture of the screen the decomp is parked in, from the state alone (after
 * the state was copied from the original). */
void game_redraw(struct game *g);
/* What went wrong getting input in the last step, or NULL. */
const char *game_io_error(void);
void game_set_io_error(const char *what);
void game_set_unwritten(const char *what);

/* title.c */
enum thw_wait title_step(struct game *g, const struct step_io *io);
void title_redraw(struct game *g);
void title_enter_legal(struct game *g);
bool read_mouse(struct game *g, const struct step_io *io, int port, uint8_t raw[3]);
void ppu_clear_oam(struct game *g, unsigned x);
/* pads.c */
bool read_pads(struct game *g, const struct step_io *io);
void random_steps(struct game *g, int n);
uint8_t choose_random_target(struct game *g);
void init_random_target(struct game *g);
uint8_t find_random_destroyed_house(struct game *g);
bool get_nmis(const struct step_io *io, uint8_t *v);
/* play.c: the game loop, pause and the states */
enum thw_wait play_step(struct game *g, const struct step_io *io);
enum thw_wait play_begin(struct game *g, const struct step_io *io);
void play_redraw(struct game *g);
enum thw_wait play_after_cutscene(struct game *g, const struct step_io *io);
void do_state_new_level(struct game *g);
/* bg.c */
void setup_game_bg(struct game *g);
void draw_game_bg(struct game *g);
void build_bg_update(struct game *g);
void blit_bg_update(struct game *g);
void draw_bg_update(struct game *g);
void new_game(struct game *g);
void add_score(struct game *g, uint8_t points);
void inc_game_clock(struct game *g);
uint8_t count_houses_left(struct game *g);
void build_level_reward_bar(struct game *g);
void build_house_rebuilt_bar(struct game *g, uint8_t house);
void display_textfile(struct game *g, const unsigned char *txt);
/* missiles.c */
void clear_all_missiles(struct game *g);
bool update_missiles(struct game *g, const struct step_io *io);
void draw_crosshair(struct game *g, int x);
void move_crosshair(struct game *g, int x);
bool mouse_to_vel(struct game *g, const struct step_io *io);
void fire_player_missile(struct game *g, int silo);
/* explosion.c, smoke.c, scurry.c */
void clear_explosions(struct game *g);
void make_explosion(struct game *g, uint8_t x, uint8_t y);
bool update_all_explosions(struct game *g, const struct step_io *io);
void make_smoke(struct game *g, uint8_t x, uint8_t y, uint8_t speed);
bool update_smoke(struct game *g, const struct step_io *io);
void clear_all_smoke(struct game *g);
void init_villagers(struct game *g);
void update_villagers(struct game *g);
void villagers_go_home(struct game *g);
void warp_villagers_to_targets(struct game *g);
void test_missile_threats(struct game *g);
/* cutscene.c */
enum { CUT_STATE_SCRIPT = 0, CUT_STATE_WAIT_A = 2, CUT_STATE_CLS = 4, CUT_STATE_NEW_GRAPH = 6 };
void cut_choose_villagers(struct game *g, int choose_paranoid);
enum thw_wait load_cutscene(struct game *g, uint8_t n);
enum thw_wait cut_step(struct game *g, const struct step_io *io);
void cut_redraw(struct game *g);
/* A villager's name (0-13), as tile numbers, NUL-terminated. */
const uint8_t *cut_name(uint8_t id);
/* The building the last cutscene's camera went to (from its scroll target), or 0xFF. */
uint8_t cut_house_shown(const struct game *g);
uint8_t undte_line(struct game *g, const uint8_t *src, size_t avail);
/* levels.c */
void load_level(struct game *g, uint8_t n);
/* practice.c */
enum thw_wait practice_enter(struct game *g);
enum thw_wait practice_step(struct game *g, const struct step_io *io);
void practice_redraw(struct game *g);

#endif
