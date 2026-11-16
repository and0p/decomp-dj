/* The decomp's state in the original's terms. Every RAM symbol of the original (data/map.c)
 * has a policy. A symbol the decomp models maps to a field of struct game, contiguous or
 * strided (the original's parallel arrays are the decomp's arrays of structs); any other is
 * carried as copied, and still compared, so the original changing it shows up. */
#include "compare.h"

#include <stdio.h>
#include <string.h>

#include "map.h"

enum policy {
    P_NONE = 0,    /* no symbol here: never compared */
    P_EXACT,       /* compared byte for byte */
    P_BASELINE,    /* the original's: taken from it at each wait exit, never compared */
    P_IGNORE,      /* sound, buffers: not copied, not compared */
    P_SCRATCH,     /* copied, never compared: zero-page temporaries, popslide's markers */
    P_PLAY,        /* compared on the playfield's screens (main, pause, practice), else scratch */
    P_CUT,         /* copied; compared in a cutscene's frame loop (W_CUT2) */
};

/* Symbols that aren't game state: sound (Pently), the VRAM transfer buffer and popslide's
 * own bookkeeping. (Part of xferBuf is the playfield's update, and in a cutscene's frame the
 * packets waiting for the blit are compared: see RANGES and compared().) */
static const char *const IGNORED[] = {
    "psg_sfx_state", "soundBSS", "xferBuf", "vector", "sp_save",
};
static const char *const BASELINE[] = {"nmis"};
/* The text decoder's line and the transfer buffer's end: only a cutscene's are the
 * decomp's (the title's and the tips' are drawn from decoded text). */
static const char *const CUT[] = {"dte_output_buf", "popslide_used"};

/* Address ranges with their own policy, over the symbols' (applied after them). */
static const struct { uint16_t addr, len; uint8_t policy; } RANGES[] = {
    {0x000, 16, P_SCRATCH},   /* zero-page temporaries */
    {0x108, 2, P_PLAY},       /* houseXferDstHi, houseXferDstLo */
    {0x10A, 1, P_SCRATCH},    /* popslide's length byte, written by the blit */
    {0x10B, 64, P_PLAY},      /* houseXferBuf */
    {0x14B, 1, P_SCRATCH},    /* popslide's terminator, written by the blit */
};

#define FIELD(sym, field) {sym, 0, offsetof(struct game, field), 0, #field}
/* An array of structs standing for one of the original's parallel arrays. */
#define STRIDED(sym, array, member) \
    {sym, 0, offsetof(struct game, array[0].member), sizeof(((struct game *)0)->array[0]), #array "[%u]." #member}
/* Bytes with no symbol of their own, from an address. */
#define AT(addr, field) {NULL, addr, offsetof(struct game, field), 0, #field}
static const struct field {
    const char *symbol;
    uint16_t addr;           /* for AT(): where, when there's no symbol */
    size_t offset, stride;   /* stride 0: contiguous */
    const char *cname;
} FIELDS[] = {
    AT(0x000, zp),
    FIELD("nmis", nmis),
    FIELD("oam_used", oam_used),
    FIELD("debugHex1", debug_hex[0]),
    FIELD("debugHex2", debug_hex[1]),
    FIELD("cur_keys", cur_keys),
    FIELD("new_keys", new_keys),
    FIELD("gameState", game_state),
    FIELD("numPlayers", num_players),
    FIELD("tvSystem", tv_system),
    FIELD("mouseEnabled", mouse_enabled),
    FIELD("housesLeftClass", houses_left_class),
    FIELD("investigationStep", investigation_step),
    FIELD("rand0", rand[0]),
    FIELD("rand1", rand[1]),
    FIELD("rand2", rand[2]),
    FIELD("rand3", rand[3]),
    FIELD("lruTargets", lru_targets),
    FIELD("isPractice", is_practice),
    FIELD("practiceSide", practice_side),
    FIELD("practiceDay", practice_day),
    FIELD("practiceHour", practice_hour),
    FIELD("cur_mbuttons", cur_mbuttons),
    FIELD("new_mbuttons", new_mbuttons),
    AT(0x108, bg_update),
    FIELD("OAM", gfx.oam),
    FIELD("levelMissileSpeed", level.missile_speed),
    FIELD("enemyMissilesLeft", level.missiles_left),
    FIELD("levelReleasePeriod", level.release_period),
    FIELD("levelFlags", level.flags),
    FIELD("levelSalvoSizes", level.salvo_sizes),
    STRIDED("smokeX", smoke, x),
    STRIDED("smokeY", smoke, y),
    STRIDED("smokeSpeed", smoke, speed),
    STRIDED("smokeTime", smoke, time),
    FIELD("housesStanding", houses_standing),
    FIELD("buildingsDestroyedThisLevel", buildings_destroyed_this_level),
    FIELD("score100s", score100s),
    FIELD("score1s", score1s),
    FIELD("hiscore100s", hiscore100s),
    FIELD("hiscore1s", hiscore1s),
    FIELD("gameDay", day),
    FIELD("gameHour", hour),
    FIELD("gameMinute", minute),
    FIELD("gameSecond", second),
    FIELD("gameTenthSecond", tenth),
    FIELD("gameSubTenth", sub_tenth),
    FIELD("bgDirty", bg_dirty),
    FIELD("curTip", cur_tip),
    FIELD("tipTimeLeft", tip_time_left),
    FIELD("siloMissilesLeft", silo_missiles_left),
    FIELD("firstDestroyedHouse", first_destroyed_house),
    FIELD("houseToRebuild", house_to_rebuild),
    STRIDED("missileXHi", missiles, x_hi),
    STRIDED("missileXLo", missiles, x_lo),
    STRIDED("missileYHi", missiles, y_hi),
    STRIDED("missileYLo", missiles, y_lo),
    STRIDED("missileDXHi", missiles, dx_hi),
    STRIDED("missileDXLo", missiles, dx_lo),
    STRIDED("missileDYHi", missiles, dy_hi),
    STRIDED("missileDYLo", missiles, dy_lo),
    STRIDED("missileAngle", missiles, angle),
    STRIDED("missileType", missiles, type),
    STRIDED("missileTarget", missiles, target),
    STRIDED("missileTime", missiles, time),
    FIELD("missileDstY", missile_dst_y),
    STRIDED("crosshairXLo", crosshairs, x_lo),
    STRIDED("crosshairXHi", crosshairs, x_hi),
    STRIDED("crosshairYLo", crosshairs, y_lo),
    STRIDED("crosshairYHi", crosshairs, y_hi),
    STRIDED("crosshairDXLo", crosshairs, dx_lo),
    STRIDED("crosshairDXHi", crosshairs, dx_hi),
    STRIDED("crosshairDYLo", crosshairs, dy_lo),
    STRIDED("crosshairDYHi", crosshairs, dy_hi),
    FIELD("timeTillSalvo", time_till_salvo),
    FIELD("salvoLeft", salvo_left),
    STRIDED("explodeX", explosions, x),
    STRIDED("explodeY", explosions, y),
    STRIDED("explodeTime", explosions, time),
    STRIDED("villagerX", villagers, x),
    STRIDED("villagerTarget", villagers, target),
    FIELD("doorwayClogTime", doorway_clog_time),
    STRIDED("exclTime", excls, time),
    STRIDED("exclX", excls, x),
    FIELD("lastExclSoundTime", last_excl_sound_time),
    FIELD("script_ptr", script_ptr),
    FIELD("name_interp_pos", name_interp_pos),
    FIELD("decomp_buf_pos", decomp_buf_pos),
    FIELD("cutscene_vram_dst_lo", cut_dst_lo),
    FIELD("cutscene_vram_dst_hi", cut_dst_hi),
    FIELD("cutscene_x_scroll", cut_scroll),
    FIELD("cutscene_x_scroll_target", cut_scroll_target),
    FIELD("cutFadeDir", cut_fade_dir),
    FIELD("cutFadeAmount", cut_fade_amount),
    FIELD("variable_actor_ids", actor_ids),
    FIELD("cutscene_actors", cut_actors),
    FIELD("dte_output_buf", dte_buf),
    FIELD("popslide_used", popslide_used),
};
#undef FIELD
#undef STRIDED
#undef AT

/* Per byte of the original's RAM: its policy, the symbol it belongs to, where it is in
 * struct game (or -1: carried), the field, and the element. */
static struct {
    int ready;
    uint8_t policy[THW_RAM_SIZE];
    int16_t sym[THW_RAM_SIZE];
    int32_t at[THW_RAM_SIZE];
    int16_t field[THW_RAM_SIZE];
    uint16_t elem[THW_RAM_SIZE];
} M;

static int listed(const char *name, const char *const *list, size_t n) {
    for (size_t i = 0; i < n; i++)
        if (strcmp(name, list[i]) == 0) return 1;
    return 0;
}

static int sym_named(const char *name) {
    for (unsigned i = 0; i < thw_ramsym_count; i++)
        if (strcmp(thw_ramsyms[i].name, name) == 0) return (int)i;
    return -1;
}

static void map_field(size_t f, unsigned addr, unsigned count) {
    const struct field *fd = &FIELDS[f];
    for (unsigned k = 0; k < count && addr + k < THW_RAM_SIZE; k++) {
        M.at[addr + k] = (int32_t)(fd->offset + (fd->stride ? k * fd->stride : k));
        M.field[addr + k] = (int16_t)f;
        M.elem[addr + k] = (uint16_t)k;
    }
}

static void build(void) {
    if (M.ready) return;
    for (int a = 0; a < THW_RAM_SIZE; a++) {
        M.sym[a] = -1;
        M.at[a] = -1;
        M.field[a] = -1;
    }
    for (unsigned i = 0; i < thw_ramsym_count; i++) {
        const struct thw_ramsym *s = &thw_ramsyms[i];
        uint8_t p = listed(s->name, IGNORED, sizeof IGNORED / sizeof *IGNORED)    ? P_IGNORE
                    : listed(s->name, BASELINE, sizeof BASELINE / sizeof *BASELINE) ? P_BASELINE
                    : listed(s->name, CUT, sizeof CUT / sizeof *CUT)                ? P_CUT
                                                                                    : P_EXACT;
        for (unsigned k = 0; k < s->size && s->addr + k < THW_RAM_SIZE; k++) {
            unsigned a = s->addr + k;
            /* An alias (missilesOverlay over missileXHi) keeps the first, larger symbol. */
            if (M.sym[a] >= 0 && thw_ramsyms[M.sym[a]].size >= s->size) continue;
            M.sym[a] = (int16_t)i;
            M.policy[a] = p;
        }
    }
    for (size_t r = 0; r < sizeof RANGES / sizeof *RANGES; r++)
        for (unsigned k = 0; k < RANGES[r].len; k++) M.policy[RANGES[r].addr + k] = RANGES[r].policy;
    for (size_t f = 0; f < sizeof FIELDS / sizeof *FIELDS; f++) {
        if (FIELDS[f].symbol) {
            int i = sym_named(FIELDS[f].symbol);
            if (i >= 0) map_field(f, thw_ramsyms[i].addr, thw_ramsyms[i].size);
            continue;
        }
        /* an address: the run of RANGES bytes from it */
        unsigned addr = FIELDS[f].addr, n = 0;
        while (addr + n < THW_RAM_SIZE && (M.policy[addr + n] == P_SCRATCH || M.policy[addr + n] == P_PLAY)) {
            n++;
            if (addr == 0 && n == 16) break;
        }
        map_field(f, addr, n);
    }
    M.ready = 1;
}

static uint8_t *byte_of(struct game *g, unsigned a) {
    return M.at[a] >= 0 ? (uint8_t *)g + M.at[a] : &g->carry[a];
}

static int on_playfield(const struct game *g) {
    return g->wait == W_MAIN || g->wait == W_PAUSE || g->wait == W_PRACTICE;
}

/* In a cutscene's frame: the decoded line up to its control character, and the popslide
 * packets waiting for the blit ($0108 to popslide_used), as far as they are mapped. Set
 * from the original's RAM by compare_check. */
static unsigned cut_dte_end, cut_xfer_end;

static int compared(const struct game *g, unsigned a) {
    if (M.policy[a] == P_EXACT) return 1;
    if (g->wait == W_CUT2) {
        if (a >= 0x108 && a < cut_xfer_end) return 1;
        return M.policy[a] == P_CUT && (a < 0x180 || a >= 0x1A0 || a < cut_dte_end);
    }
    return M.policy[a] == P_PLAY && on_playfield(g);
}

void compare_import(struct game *g, const uint8_t *ram) {
    build();
    for (unsigned a = 0; a < THW_RAM_SIZE; a++)
        if (M.policy[a] != P_NONE && M.policy[a] != P_IGNORE) *byte_of(g, a) = ram[a];
}

void compare_export(const struct game *g, uint8_t *ram) {
    build();
    memset(ram, 0, THW_RAM_SIZE);
    for (unsigned a = 0; a < THW_RAM_SIZE; a++)
        if (M.policy[a] != P_NONE) ram[a] = *byte_of((struct game *)g, a);
}

void compare_write(struct game *g, uint16_t addr, const uint8_t *bytes, size_t len) {
    build();
    for (size_t k = 0; k < len && addr + k < THW_RAM_SIZE; k++) *byte_of(g, (unsigned)(addr + k)) = bytes[k];
}

void compare_take_baseline(struct game *g, const uint8_t *ram) {
    build();
    for (unsigned a = 0; a < THW_RAM_SIZE; a++)
        if (M.policy[a] == P_BASELINE) *byte_of(g, a) = ram[a];
}

/* ---- Differences ------------------------------------------------------------------------- */

#define MAX_DIFFS 64
static struct dj_parity_diff diffs[MAX_DIFFS];
static size_t ndiffs, total_diffs;
static uint8_t want[THW_RAM_SIZE], have[THW_RAM_SIZE];
static char labels[MAX_DIFFS][128];
static const char *first_name;   /* the first difference's symbol, for the reason */

static void label(char *out, size_t n, unsigned a) {
    int f = M.field[a];
    const struct thw_ramsym *s = M.sym[a] >= 0 ? &thw_ramsyms[M.sym[a]] : NULL;
    if (f < 0) {
        snprintf(out, n, "%s, not modelled yet", s ? s->name : "?");
        return;
    }
    char c[64];
    if (FIELDS[f].stride) snprintf(c, sizeof c, FIELDS[f].cname, M.elem[a]);
    else if (!s || s->size > 1) snprintf(c, sizeof c, "%s (byte %u)", FIELDS[f].cname, M.elem[a]);
    else snprintf(c, sizeof c, "%s", FIELDS[f].cname);
    snprintf(out, n, "game.%s", c);
}

size_t compare_check(const struct game *g, const uint8_t *ram, char *reason, size_t reason_len) {
    build();
    compare_export(g, have);
    memcpy(want, ram, THW_RAM_SIZE);
    ndiffs = total_diffs = 0;
    cut_dte_end = 0x180;
    while (cut_dte_end < 0x1A0 && want[cut_dte_end] >= 0x10) cut_dte_end++;
    cut_dte_end++;
    cut_xfer_end = 0x100 + (unsigned)want[0x303];
    if (cut_xfer_end > 0x14C) cut_xfer_end = 0x14C;
    unsigned a = 0;
    while (a < THW_RAM_SIZE) {
        if (!compared(g, a) || want[a] == have[a]) {
            a++;
            continue;
        }
        /* A run of differing bytes within one field (one element, for a strided one). */
        unsigned start = a;
        int sym = M.sym[a], field = M.field[a];
        a++;
        while (a < THW_RAM_SIZE && M.sym[a] == sym && M.field[a] == field && compared(g, a) && want[a] != have[a] &&
               (field < 0 || !FIELDS[field].stride))
            a++;
        total_diffs++;
        if (ndiffs < MAX_DIFFS) {
            char c[96];
            label(c, sizeof c, start);
            const char *name = start >= 0x108 && start < 0x14C ? (g->wait == W_CUT2 ? "popslide_buf" : "houseXferBuf")
                               : sym >= 0                    ? thw_ramsyms[sym].name
                                                             : "?";
            if (!ndiffs) first_name = name;
            /* Region and address, so dj can find the original's writer (--explain); the
             * original's symbol goes in the label, since dj takes `symbol` instead of an
             * address. */
            snprintf(labels[ndiffs], sizeof labels[0], "%s (%s)", c, name);
            diffs[ndiffs] = (struct dj_parity_diff){labels[ndiffs], "RAM", start, NULL, want + start, have + start, a - start};
            ndiffs++;
        }
    }
    if (total_diffs && reason) {
        const struct dj_parity_diff *d = &diffs[0];
        char c[96];
        label(c, sizeof c, (unsigned)d->address);
        snprintf(reason, reason_len, "%s at RAM 0x%03llX (%s) is 0x%02X, the original's is 0x%02X%s", first_name,
                 (unsigned long long)d->address, c, d->actual[0], d->expected[0],
                 total_diffs > 1 ? " (and more)" : "");
    }
    return total_diffs;
}

void compare_clear_diffs(void) { ndiffs = total_diffs = 0; }

size_t compare_diffs(struct dj_parity_diff *out, size_t cap) {
    /* Only the first MAX_DIFFS are kept, so that is all there is to ask for. */
    for (size_t i = 0; i < ndiffs && i < cap; i++) out[i] = diffs[i];
    return ndiffs;
}
