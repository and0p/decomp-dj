/*
 * The NMI's drawing (render). On the NES it writes to the PPU; here it keeps
 * what the game depends on, and what the screen shows of the playfield:
 *
 * - During play the playfield reaches the screen four rows a frame, from
 *   vramRow down; the game waits for vramRow to reach $20 (all copied) before
 *   locking a piece or dealing the next one. Line clears are animated on the
 *   screen only (the rows are already gone from RAM). T->screen.field is
 *   the screen's copy.
 * - outOfDateRenderFlags says which counters to redraw; the redraw clears it.
 */
#include "game.h"

/* The columns blanked in each step of the line clear animation. */
static const uint8_t leftColumns[5] = {0x04, 0x03, 0x02, 0x01, 0x00};
static const uint8_t rightColumns[5] = {0x05, 0x06, 0x07, 0x08, 0x09};

/* Clears the screen's playfields (a fresh nametable shows them empty). */
void screen_clearPlayfields(void)
{
    for (int p = 0; p < 2; p++)
        for (int i = 0; i < 200; i++)
            T->screen.field[p][i] = tileEmpty;
}

/* The screen playfield of the playfield at page playfieldAddr+1 ($04 or $05). */
static uint8_t *screenPlayfield(void) { return T->screen.field[playfieldAddr[1] == 0x05]; }

/*
 * copyPlayfieldRowToVRAM: copies playfield row vramRow (ten tiles) to the
 * screen and moves on to the next row. After the last row (19) vramRow is
 * set to $20, meaning the copy is complete; the game sets vramRow back to a
 * lower row when the playfield changes (updatePlayfield).
 */
static void copyPlayfieldRowToVRAM(void)
{
    uint8_t x = vramRow;
    if (!sign_set(x, 0x15)) /* bpl: rows $15 and up (and $20, "done") */
        return;
    uint8_t y = multBy10Table[x];
    uint8_t *row = screenPlayfield() + y;
    for (int i = 0; i < 0x0A; i++) /* @copyRow */
        row[i] = R(ptr16(A_playfieldAddr) + (uint8_t)(y + i));
    vramRow++;
    if (!sign_set(vramRow, 0x14))
        vramRow = 0x20;
}

/*
 * updateLineClearingAnimation: every fourth frame, blanks one more pair of
 * squares in each completed row, working out from the middle (columns 4 and
 * 5, then 3 and 6, and so on). rowY counts the steps; after the fifth step
 * the whole row is blank and playState moves on. completedRow holds up to
 * four row numbers, 0 meaning none.
 */
static void updateLineClearingAnimation(void)
{
    if (frameCounter[0] & 0x03)
        return;
    generalCounter3 = 0;
    do { /* @whileCounter3LessThan4 */
        uint8_t row = R(A_completedRow + generalCounter3);
        if (row && row < 20) {
            uint8_t *cells = screenPlayfield() + row * 10;
            cells[leftColumns[rowY % 5]] = tileHidden;
            cells[rightColumns[rowY % 5]] = tileHidden;
        }
        generalCounter3++; /* @nextRow */
    } while (generalCounter3 != 0x04);
    rowY++;
    if (!sign_set(rowY, 0x05))
        playState++;
}

/* The playfield row copy and the line clear animation for one player, whose
 * state is in the player1_ or player2_ block at base ($60 or $80) and whose
 * playfield is at page playfieldPage. */
static void renderPlayfield(uint16_t base, uint8_t playfieldPage)
{
    /* offsets within a player block, as for tetriminoX at $40 */
    uint16_t rowYA = base + (A_rowY - A_tetriminoX);
    uint16_t completedRowA = base + (A_completedRow - A_tetriminoX);
    uint16_t playStateA = base + (A_playState - A_tetriminoX);
    uint16_t vramRowA = base + (A_vramRow - A_tetriminoX);

    if (R(playStateA) == 0x04) {
        playfieldAddr[1] = playfieldPage;
        rowY = R(rowYA);
        for (int i = 0; i < 4; i++)
            completedRow[i] = R(completedRowA + i);
        playState = R(playStateA);
        updateLineClearingAnimation();
        R(rowYA) = rowY;
        R(playStateA) = playState;
        R(vramRowA) = 0;
    } else { /* @playStateNotDisplayLineClearingAnimation */
        vramRow = R(vramRowA);
        playfieldAddr[1] = playfieldPage;
        copyPlayfieldRowToVRAM();
        copyPlayfieldRowToVRAM();
        copyPlayfieldRowToVRAM();
        copyPlayfieldRowToVRAM();
        R(vramRowA) = vramRow;
    }
}

/*
 * render_mode_play_and_demo: each frame copies four rows of each playfield
 * to the screen (or, while lines are being cleared, steps that animation
 * instead), then redraws whatever outOfDateRenderFlags marks as changed (the
 * lines, the level and its colors, the score, the statistics). Last it sets
 * the background color, which flashes after a Tetris.
 */
static void render_mode_play_and_demo(void)
{
    renderPlayfield(A_player1_tetriminoX, 0x04);
    if (numberOfPlayers == 0x02) /* @renderPlayer2Playfield */
        renderPlayfield(A_player2_tetriminoX, 0x05);
    outOfDateRenderFlags &= (uint8_t)~RENDER_LINES; /* @renderLines */
    if (numberOfPlayers != 0x02) {
        /* @renderLevel, @renderScore, @renderStats. (renderStats counts the
         * pieces in tmpCurrentPiece.) */
        if (outOfDateRenderFlags & RENDER_STATS)
            tmpCurrentPiece = 0x07;
        outOfDateRenderFlags &= (uint8_t)~(RENDER_LEVEL | RENDER_SCORE | RENDER_STATS);
    }
    ppuScrollX = 0;
    ppuScrollY = 0;
}

/* render: the NMI's drawing for the current renderMode: 0 the legal and
 * title screens, 1 the menus, 2 the name entry, 3 play and the demo, 4 the
 * endings. Only play and the name entry keep any state. */
void render(void)
{
    switch (renderMode) {
    case 2: /* render_mode_congratulations_screen: draws the letter just changed */
        if (outOfDateRenderFlags & RENDER_HIGH_SCORE_LETTER) {
            ppuScrollX = 0;
            ppuScrollY = 0;
            outOfDateRenderFlags = 0;
        }
        break;
    case 3: render_mode_play_and_demo(); break;
    default: break;
    }
}
