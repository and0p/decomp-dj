/* The decomp's state in the original's terms. Every RAM symbol of the original (data/map.c)
 * has a policy. A symbol the decomp models maps to a field of struct game; any other is
 * carried as copied, and still compared, so the original changing it shows up. */
#include "compare.h"

#include <stdio.h>
#include <string.h>

#include "map.h"

enum policy {
    P_NONE = 0,    /* no symbol here: never compared */
    P_EXACT,       /* compared byte for byte */
    P_BASELINE,    /* the original's: taken from it at each wait exit, never compared */
    P_IGNORE,      /* scratch, sound, buffers: not copied, not compared */
};

/* Symbols that aren't game state: sound (Pently), the VRAM transfer buffer and its
 * bookkeeping, the text decoder's buffer. */
static const char *const IGNORED[] = {
    "psg_sfx_state", "soundBSS", "xferBuf", "dte_output_buf", "vector", "sp_save", "popslide_used",
};
static const char *const BASELINE[] = {"nmis"};

#define F(sym, field) {sym, offsetof(struct game, field), sizeof(((struct game *)0)->field), #field}
/* The fields the decomp models: the original's symbol, where it is in struct game. */
static const struct field {
    const char *symbol;
    size_t offset, size;
    const char *cname;
} FIELDS[] = {
    F("nmis", nmis),
    F("oam_used", oam_used),
    F("debugHex1", debug_hex[0]),
    F("debugHex2", debug_hex[1]),
    F("cur_keys", cur_keys),
    F("new_keys", new_keys),
    F("numPlayers", num_players),
    F("mouseEnabled", mouse_enabled),
    F("rand0", rand[0]),
    F("rand1", rand[1]),
    F("rand2", rand[2]),
    F("rand3", rand[3]),
    F("isPractice", is_practice),
    F("practiceSide", practice_side),
    F("cur_mbuttons", cur_mbuttons),
    F("new_mbuttons", new_mbuttons),
    F("score100s", score100s),
    F("score1s", score1s),
    F("hiscore100s", hiscore100s),
    F("hiscore1s", hiscore1s),
    F("crosshairXHi", crosshair_x_hi),
    F("crosshairYHi", crosshair_y_hi),
    F("OAM", gfx.oam),
};
#undef F

/* Per byte of the original's RAM: its policy, the symbol it belongs to, and where it is in
 * struct game (or -1: carried). */
static struct {
    int ready;
    uint8_t policy[THW_RAM_SIZE];
    int16_t sym[THW_RAM_SIZE];
    int32_t at[THW_RAM_SIZE];
    int16_t field[THW_RAM_SIZE];
} M;

static int listed(const char *name, const char *const *list, size_t n) {
    for (size_t i = 0; i < n; i++)
        if (strcmp(name, list[i]) == 0) return 1;
    return 0;
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
                                                                                    : P_EXACT;
        for (unsigned k = 0; k < s->size && s->addr + k < THW_RAM_SIZE; k++) {
            unsigned a = s->addr + k;
            /* An alias (missilesOverlay over missileXHi) keeps the first, larger symbol. */
            if (M.sym[a] >= 0 && thw_ramsyms[M.sym[a]].size >= s->size) continue;
            M.sym[a] = (int16_t)i;
            M.policy[a] = p;
        }
    }
    for (size_t f = 0; f < sizeof FIELDS / sizeof *FIELDS; f++) {
        for (unsigned i = 0; i < thw_ramsym_count; i++) {
            if (strcmp(thw_ramsyms[i].name, FIELDS[f].symbol) != 0) continue;
            for (size_t k = 0; k < FIELDS[f].size; k++) {
                M.at[thw_ramsyms[i].addr + k] = (int32_t)(FIELDS[f].offset + k);
                M.field[thw_ramsyms[i].addr + k] = (int16_t)f;
            }
            break;
        }
    }
    M.ready = 1;
}

static uint8_t *byte_of(struct game *g, unsigned a) {
    return M.at[a] >= 0 ? (uint8_t *)g + M.at[a] : &g->carry[a];
}

void compare_import(struct game *g, const uint8_t *ram) {
    build();
    for (unsigned a = 0; a < THW_RAM_SIZE; a++)
        if (M.policy[a] == P_EXACT || M.policy[a] == P_BASELINE) *byte_of(g, a) = ram[a];
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
static char labels[MAX_DIFFS][64];

size_t compare_check(const struct game *g, const uint8_t *ram, char *reason, size_t reason_len) {
    build();
    compare_export(g, have);
    memcpy(want, ram, THW_RAM_SIZE);
    ndiffs = total_diffs = 0;
    unsigned a = 0;
    while (a < THW_RAM_SIZE) {
        if (M.policy[a] != P_EXACT || want[a] == have[a]) {
            a++;
            continue;
        }
        /* A run of differing bytes within one symbol. */
        unsigned start = a;
        int sym = M.sym[a];
        while (a < THW_RAM_SIZE && M.sym[a] == sym && M.policy[a] == P_EXACT && want[a] != have[a]) a++;
        total_diffs++;
        if (ndiffs < MAX_DIFFS) {
            const struct thw_ramsym *s = &thw_ramsyms[sym];
            unsigned idx = start - s->addr;
            int f = M.field[start];
            if (f >= 0) snprintf(labels[ndiffs], sizeof labels[0], "game.%s", FIELDS[f].cname);
            else snprintf(labels[ndiffs], sizeof labels[0], "%s, not modelled yet", s->name);
            if (s->size > 1) {
                size_t l = strlen(labels[ndiffs]);
                snprintf(labels[ndiffs] + l, sizeof labels[0] - l, " (byte %u)", idx);
            }
            diffs[ndiffs] = (struct dj_parity_diff){labels[ndiffs], "RAM", start, s->name, want + start, have + start, a - start};
            ndiffs++;
        }
    }
    if (total_diffs && reason) {
        const struct dj_parity_diff *d = &diffs[0];
        snprintf(reason, reason_len, "%s at RAM 0x%03llX (%s) is 0x%02X, the original's is 0x%02X%s", d->symbol,
                 (unsigned long long)d->address, d->label, d->actual[0], d->expected[0],
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
