/* The game loop, pause, and the game's states (orig: main.s). */
#include <stddef.h>

#include "bg.h"
#include "game.h"
#include "m6502.h"

#define NUM_MADE_DAYS 7

static const char *unwritten_here;

static void not_written(const char *what) {
    if (!unwritten_here) unwritten_here = what;
}

/* ---- The states (doStateAction) --------------------------------------------------------- */

/* orig: doStateNewLevel */
void do_state_new_level(struct game *g) {
    if (g->day == NUM_MADE_DAYS) {
        g->game_state = STATE_INACTIVE;
        return;
    }
    load_level(g, (uint8_t)(g->day * 5 + g->hour));
    g->bg_dirty |= BG_DIRTY_STATUS;
    g->silo_missiles_left[0] = g->silo_missiles_left[1] = 15;
    /* a destroyed silo isn't refilled: the other gets 20 */
    if (!g->houses_standing[BUILDING_SILO0]) {
        g->silo_missiles_left[0] = 0;
        g->silo_missiles_left[1] = 20;
    }
    if (!g->houses_standing[BUILDING_SILO1]) {
        g->silo_missiles_left[1] = 0;
        g->silo_missiles_left[0] = 20;
    }
    g->buildings_destroyed_this_level = 0;
    g->game_state = STATE_ACTIVE;
    /* the hour's music (pently_start_music): no sound */
    init_random_target(g);
}

/* orig: doStateActive. The only state in which the players fire. */
static void do_state_active(struct game *g) {
    if (!(g->houses_standing[BUILDING_SILO0] | g->houses_standing[BUILDING_SILO1])) {
        g->game_state = STATE_GAMEOVER;
        return;
    }
    /* player 1 fires from the left silo with B (either button in a 2-player game)... */
    uint8_t keys = g->num_players == 2 ? (KEY_B | KEY_A) : KEY_B;
    if (keys & g->new_keys[0]) {
        g->zp[2] = g->crosshairs[0].x_hi;
        g->zp[3] = g->crosshairs[0].y_hi;
        fire_player_missile(g, 0);
    }
    /* ...and the last player from the right silo with A */
    keys = g->num_players == 2 ? (KEY_B | KEY_A) : KEY_A;
    int x = (uint8_t)(g->num_players - 1) & 1;
    if (keys & g->new_keys[x]) {
        g->zp[2] = g->crosshairs[x].x_hi;
        g->zp[3] = g->crosshairs[x].y_hi;
        fire_player_missile(g, 1);
    }
    if (g->level.missiles_left) return;
    for (int m = 4; m < NUM_MISSILES; m++)
        if (g->missiles[m].y_hi) return;
    g->game_state = STATE_LEVEL_REWARD;
}

/* orig: doStateLevelReward. Ten points a house left, one a missile left. */
static void do_state_level_reward(struct game *g) {
    if (g->cur_tip != 2) {
        uint8_t n = count_houses_left(g);
        g->zp[0] = n;
        uint8_t a = (uint8_t)(n * 5);
        if (!a) {
            g->game_state = STATE_GAMEOVER;
            return;
        }
        uint8_t c;
        a = asl(a, &c);
        a = adc(a, g->silo_missiles_left[0], &c);
        a = adc(a, g->silo_missiles_left[1], &c);
        g->zp[1] = a;
        add_score(g, a);
        villagers_go_home(g);
        build_level_reward_bar(g);
        return;
    }
    if (g->tip_time_left == 0) {
        warp_villagers_to_targets(g);
        g->game_state = g->is_practice ? STATE_INACTIVE : STATE_REBUILDING_SILO;
    }
    /* MUSIC_CLEARED_LEVEL at 47 tenths left: no sound */
}

/* orig: doStateRebuildSilo */
static void do_state_rebuild_silo(struct game *g) {
    if (g->hour != 4 && g->buildings_destroyed_this_level) {
        g->game_state = STATE_CUTSCENE;
        return;
    }
    if (g->cur_tip != 3) {
        if (!g->houses_standing[BUILDING_SILO0]) {
            g->houses_standing[BUILDING_SILO0]++;
        } else if (!g->houses_standing[BUILDING_SILO1]) {
            g->houses_standing[BUILDING_SILO1]++;
        } else {
            g->game_state = STATE_CUTSCENE;
            return;
        }
        g->cur_tip = 3;
        g->tip_time_left = 40;
        g->bg_dirty |= BG_DIRTY_TIP | BG_DIRTY_HOUSES;
    }
    if (g->tip_time_left == 0) g->game_state = STATE_CUTSCENE;
}

/* Set by do_state_cutscene when it starts a cutscene: the game loop parks at its wait. */
static bool cutscene_started;

/* orig: doStateCutscene. The next hour, or after 5 AM the house to rebuild and a cutscene:
 * the day's (on a perfect run), or one on the houses left, or the investigation's next. */
static void do_state_cutscene(struct game *g) {
    g->hour++;
    if (g->hour < 5) {
        g->game_state = STATE_NEW_LEVEL;
        return;
    }
    uint8_t y = g->house_to_rebuild;
    if (y & 0x80) {
        y = find_random_destroyed_house(g);
        if (y >= NUM_BUILDINGS) y = (g->day & 1) ? BUILDING_SILO1 : BUILDING_SILO0;   /* nothing: a silo */
        g->house_to_rebuild = y;
    }
    uint8_t a = g->day;
    y = g->first_destroyed_house;
    if (!(y & 0x80)) {
        if (y != g->actor_ids[0]) {   /* whose house fell first is the paranoid one */
            g->actor_ids[0] = y;
            cut_choose_villagers(g, 0);
        }
        if (g->day == NUM_MADE_DAYS - 1) {
            a = 14;
        } else {
            uint8_t n = count_houses_left(g);
            g->zp[0] = n;
            uint8_t x = n < 4 ? 2 : n < 10 ? 1 : 0;   /* 0: all, 1: 4-9 left, 2: 1-3 left */
            if (x > g->houses_left_class) {
                g->houses_left_class = x;
                a = (uint8_t)(x + 7);
            } else {
                a = g->investigation_step++;
                if (a >= 2) a = 2;
                a = (uint8_t)(a + 10);
            }
        }
    }
    load_cutscene(g, a);
    cutscene_started = true;
}

/* orig: doStateRebuildHouse */
static void do_state_rebuild_house(struct game *g) {
    if (g->cur_tip != 4) {
        g->hour = 0;
        g->day++;
        g->bg_dirty |= BG_DIRTY_TIP | BG_DIRTY_HOUSES;
        uint8_t y = g->house_to_rebuild;
        if (!(y & 0x80)) {
            g->house_to_rebuild = 0xFF;
            if (!g->houses_standing[y]) {
                g->houses_standing[y] = 1;
                g->cur_tip = 4;
                g->tip_time_left = 40;
                build_house_rebuilt_bar(g, y);
                return;
            }
        }
        g->game_state = STATE_NEW_LEVEL;
        return;
    }
    if (g->tip_time_left == 0) g->game_state = STATE_NEW_LEVEL;
}

/* orig: doStateGameOver */
static void do_state_game_over(struct game *g) {
    if (g->cur_tip != 1) {
        g->cur_tip = 1;
        g->tip_time_left = 50;
        g->bg_dirty |= BG_DIRTY_TIP;
        return;
    }
    if (g->tip_time_left == 0) g->game_state = STATE_INACTIVE;
}

/* orig: doStateAction */
static void do_state_action(struct game *g) {
    switch (g->game_state) {
    case STATE_INACTIVE: break;
    case STATE_NEW_LEVEL: do_state_new_level(g); break;
    case STATE_ACTIVE: do_state_active(g); break;
    case STATE_LEVEL_REWARD: do_state_level_reward(g); break;
    case STATE_REBUILDING_SILO: do_state_rebuild_silo(g); break;
    case STATE_CUTSCENE: do_state_cutscene(g); break;
    case STATE_REBUILDING_HOUSE: do_state_rebuild_house(g); break;
    case STATE_GAMEOVER: do_state_game_over(g); break;
    default: not_written("a gameState past the table"); break;
    }
}

/* orig: tenthUpdates (every frame, despite the name) */
static bool tenth_updates(struct game *g, const struct step_io *io) {
    if (!update_all_explosions(g, io)) return false;
    if (!update_missiles(g, io)) return false;
    update_villagers(g);
    if (g->sub_tenth == 1) test_missile_threats(g);
    return update_smoke(g, io);
}

/* ---- The loop --------------------------------------------------------------------------- */

static enum thw_wait fail_or_unwritten(void) {
    if (unwritten_here && !game_io_error()) game_set_unwritten(unwritten_here);
    return W_NONE;
}

/* gameLoop from notPaused: everything after input, up to the wait (or a cutscene's). */
static enum thw_wait game_loop_tail(struct game *g, const struct step_io *io);

static enum thw_wait game_loop_rest(struct game *g, const struct step_io *io) {
    move_crosshair(g, 0);
    move_crosshair(g, 1);
    g->oam_used = 0;
    int x = 0;
    do draw_crosshair(g, x++);
    while (x < g->num_players);
    cutscene_started = false;
    do_state_action(g);
    if (unwritten_here) return fail_or_unwritten();
    if (cutscene_started) return W_CUT1;
    return game_loop_tail(g, io);
}

/* doStateCutscene after its cutscene, then the rest of the game loop. */
enum thw_wait play_after_cutscene(struct game *g, const struct step_io *io) {
    unwritten_here = NULL;
    clear_all_missiles(g);
    g->game_state = STATE_REBUILDING_HOUSE;
    setup_game_bg(g);
    return game_loop_tail(g, io);
}

/* gameLoop after doStateAction. */
static enum thw_wait game_loop_tail(struct game *g, const struct step_io *io) {
    if (!tenth_updates(g, io)) return fail_or_unwritten();
    build_bg_update(g);
    draw_bg_update(g);   /* blitted after the wait, before the frame is drawn */
    if (g->oam_used) ppu_clear_oam(g, g->oam_used);
    g->gfx.sprite_table = 1;
    return W_MAIN;
}

/* orig: pauseScreen, up to its wait: "PAUSE" in sprites. */
static enum thw_wait pause_enter(struct game *g) {
    static const uint8_t TEXT[5] = {'P', 'A', 'U', 'S', 'E'};
    for (int y = 0; y < 5; y++) g->gfx.oam[y] = (struct gfx_sprite){111, TEXT[y], 0x01, (uint8_t)((y << 4) + 92)};
    ppu_clear_oam(g, 20);
    g->after_pause = 1;
    return W_PAUSE;
}

/* gameLoop's top: the clock, the controllers, maybe pause, then the rest. */
static enum thw_wait game_loop(struct game *g, const struct step_io *io) {
    inc_game_clock(g);
    if (!read_pads(g, io) || !mouse_to_vel(g, io)) return W_NONE;
    if (g->new_keys[0] & KEY_START) return pause_enter(g);
    return game_loop_rest(g, io);
}

/* main, after a game ends: the high score, then the title or the practice menu. */
static enum thw_wait game_over_to_main(struct game *g) {
    uint8_t c = g->score1s >= g->hiscore1s;
    sbc(g->score100s, g->hiscore100s, &c);
    if (c) {
        g->hiscore1s = g->score1s;
        g->hiscore100s = g->score100s;
    }
    if (!g->is_practice) {
        title_enter_legal(g);
        return W_SCREEN_OFF;
    }
    g->debug_hex[0] = 0xC0;
    g->debug_hex[1] = 0xDE;
    new_game(g);
    return practice_enter(g);
}

/* main, from practice_nocutscene: a game starts. */
enum thw_wait play_begin(struct game *g, const struct step_io *io) {
    unwritten_here = NULL;
    g->score100s = g->score1s = 0;
    setup_game_bg(g);
    clear_all_missiles(g);
    init_villagers(g);
    g->game_state = STATE_NEW_LEVEL;
    g->houses_left_class = 0;
    g->investigation_step = 0;
    return game_loop(g, io);
}

/* From main's wait or pauseScreen's. */
enum thw_wait play_step(struct game *g, const struct step_io *io) {
    unwritten_here = NULL;
    if (g->wait == W_PAUSE) {
        g->gfx.sprite_table = 0;   /* pause shows its sprites from pattern table 0 */
        if (!read_pads(g, io)) return W_NONE;
        bool quit = (g->new_keys[0] & KEY_SELECT) && g->is_practice;
        if (quit) g->game_state = STATE_INACTIVE;
        if (!quit && !(g->new_keys[0] & KEY_START)) return W_PAUSE;
        g->after_pause = 0;
        return game_loop_rest(g, io);
    }
    blit_bg_update(g);
    g->gfx.sprite_table = 1;
    if (g->game_state == STATE_INACTIVE) return game_over_to_main(g);
    return game_loop(g, io);
}

void play_redraw(struct game *g) {
    draw_playfield(g);
    g->gfx.sprite_table = g->wait == W_PAUSE ? 0 : 1;
    g->gfx.bg_on = g->gfx.sprites_on = true;
}
