/*
 * Rendering during play (and the demo): run from the NMI while renderMode is
 * 3. It copies changed playfield rows to the nametable, animates cleared
 * lines, redraws the lines, level, score and statistics counters when they
 * change, and flashes the background after a Tetris.
 */
#include "game.h"

#define wr(addr, v) poke((addr), (uint8_t)(v))

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
        R(A_playfieldAddr + 1) = playfieldPage;
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
        R(A_playfieldAddr + 1) = playfieldPage;
        copyPlayfieldRowToVRAM();
        copyPlayfieldRowToVRAM();
        copyPlayfieldRowToVRAM();
        copyPlayfieldRowToVRAM();
        R(vramRowA) = vramRow;
    }
}

/*
 * render_mode_play_and_demo: the NMI's work during a game. Each frame copies
 * four rows of each playfield to the screen (or, while lines are being
 * cleared, steps that animation instead), then redraws whatever
 * outOfDateRenderFlags marks as changed. Last, it sets the background color:
 * after a Tetris (completedLines = 4) it flashes white every fourth frame,
 * with a sound every eighth.
 */
void render_mode_play_and_demo(void)
{
    renderPlayfield(A_player1_tetriminoX, 0x04);
    if (numberOfPlayers == 0x02) /* @renderPlayer2Playfield */
        renderPlayfield(A_player2_tetriminoX, 0x05);

    /* @renderLines */
    if (outOfDateRenderFlags & RENDER_LINES) {
        if (numberOfPlayers != 0x02) {
            wr(PPUADDR, 0x20);
            wr(PPUADDR, 0x73);
            wr(PPUDATA, player1_lines[1]);
            twoDigsToPPU(player1_lines[0]);
        } else { /* @renderLinesTwoPlayers */
            wr(PPUADDR, 0x20);
            wr(PPUADDR, 0x68);
            wr(PPUDATA, player1_lines[1]);
            twoDigsToPPU(player1_lines[0]);
            wr(PPUADDR, 0x20);
            wr(PPUADDR, 0x7A);
            wr(PPUDATA, player2_lines[1]);
            twoDigsToPPU(player2_lines[0]);
        }
        outOfDateRenderFlags &= (uint8_t)~RENDER_LINES;
    }

    /* @renderLevel. levelDisplayTable has 30 entries; above level 29 the read
     * runs on into the tables after it, as on the cartridge. */
    if ((outOfDateRenderFlags & RENDER_LEVEL) && numberOfPlayers != 0x02) {
        generalCounter = prg8((uint16_t)(ROM_levelDisplayTable + player1_levelNumber));
        wr(PPUADDR, 0x22);
        wr(PPUADDR, 0xBA);
        twoDigsToPPU(generalCounter);
        updatePaletteForLevel();
        outOfDateRenderFlags &= (uint8_t)~RENDER_LEVEL;
    }

    /* @renderScore: the score is three BCD bytes, low byte first */
    if (numberOfPlayers != 0x02 && (outOfDateRenderFlags & RENDER_SCORE)) {
        wr(PPUADDR, 0x21);
        wr(PPUADDR, 0x18);
        twoDigsToPPU(player1_score[2]);
        twoDigsToPPU(player1_score[1]);
        twoDigsToPPU(player1_score[0]);
        outOfDateRenderFlags &= (uint8_t)~RENDER_SCORE;
    }

    /* @renderStats: how many of each piece have been dealt (BCD, two bytes each) */
    if (numberOfPlayers != 0x02 && (outOfDateRenderFlags & RENDER_STATS)) {
        tmpCurrentPiece = 0;
        do { /* @renderPieceStat */
            uint8_t x = (uint8_t)(tmpCurrentPiece << 1);
            wr(PPUADDR, prg8((uint16_t)(ROM_pieceToPpuStatAddr + x))); /* .dbyt: high byte first */
            wr(PPUADDR, prg8((uint16_t)(ROM_pieceToPpuStatAddr + 1 + x)));
            wr(PPUDATA, R(A_statsByType + 1 + x));
            twoDigsToPPU(R(A_statsByType + x));
            tmpCurrentPiece++;
        } while (tmpCurrentPiece != 0x07);
        outOfDateRenderFlags &= (uint8_t)~RENDER_STATS;
    }

    /* @renderTetrisFlashAndSound: background color, palette entry $3F0E */
    wr(PPUADDR, 0x3F);
    wr(PPUADDR, 0x0E);
    uint8_t color = 0x00;
    if (completedLines == 0x04 && !(frameCounter[0] & 0x03)) {
        color = 0x30;
        if (!(frameCounter[0] & 0x07))
            soundEffectSlot1Init = 0x09;
    }
    wr(PPUDATA, color); /* @setPaletteColor */
    ppuScrollX = 0;
    wr(PPUSCROLL, 0);
    ppuScrollY = 0;
    wr(PPUSCROLL, 0);
}

/* The playfield's rows in the one-player nametable: vramPlayfieldRows is a
 * table of words. */
static uint8_t vramPlayfieldRowsByte(uint8_t i) { return prg8((uint16_t)(ROM_vramPlayfieldRows + i)); }

/*
 * copyPlayfieldRowToVRAM: copies playfield row vramRow (ten tiles) to the
 * nametable and moves on to the next row. After the last row (19) vramRow is
 * set to $20, meaning the copy is complete; the game sets vramRow back to a
 * lower row when the playfield changes (updatePlayfield). In two-player games
 * the playfields sit left and right of where the one-player playfield is.
 */
void copyPlayfieldRowToVRAM(void)
{
    uint8_t x = vramRow;
    if (!sign_set(x, 0x15)) /* bpl: rows $15 and up (and $20, "done") */
        return;
    uint8_t y = prg8((uint16_t)(ROM_multBy10Table + x));
    x = (uint8_t)(x << 1);
    wr(PPUADDR, vramPlayfieldRowsByte((uint8_t)(x + 1)));
    uint8_t lo = vramPlayfieldRowsByte(x);
    if (numberOfPlayers == 0x01)
        lo = (uint8_t)(lo + 0x06); /* @onePlayer */
    else if (R(A_playfieldAddr + 1) == 0x05)
        lo = (uint8_t)(lo + 0x0C); /* @playerTwo */
    else
        lo = (uint8_t)(lo - 0x02);
    wr(PPUADDR, lo);
    for (int i = 0; i < 0x0A; i++) { /* @copyRow */
        wr(PPUDATA, peek((uint16_t)(ptr16(A_playfieldAddr) + y)));
        y++;
    }
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
void updateLineClearingAnimation(void)
{
    if (frameCounter[0] & 0x03)
        return;
    generalCounter3 = 0;
    do { /* @whileCounter3LessThan4 */
        uint8_t row = R(A_completedRow + generalCounter3);
        if (row) {
            uint8_t y = (uint8_t)(row << 1);
            generalCounter = vramPlayfieldRowsByte(y);
            if (numberOfPlayers == 0x01)
                generalCounter = (uint8_t)(generalCounter + 0x06);
            else if (R(A_playfieldAddr + 1) == 0x04)
                generalCounter = (uint8_t)(generalCounter - 0x02);
            else
                generalCounter = (uint8_t)(generalCounter + 0x0C); /* @player2 */
            /* @updateVRAM */
            y++;
            generalCounter2 = vramPlayfieldRowsByte(y);
            wr(PPUADDR, generalCounter2);
            wr(PPUADDR, (uint8_t)(prg8((uint16_t)(ROM_leftColumns + rowY)) + generalCounter));
            wr(PPUDATA, 0xFF);
            wr(PPUADDR, generalCounter2);
            wr(PPUADDR, (uint8_t)(prg8((uint16_t)(ROM_rightColumns + rowY)) + generalCounter));
            wr(PPUDATA, 0xFF);
        }
        generalCounter3++; /* @nextRow */
    } while (generalCounter3 != 0x04);
    rowY++;
    if (!sign_set(rowY, 0x05))
        playState++;
}

/*
 * updatePaletteForLevel: the pieces' colors change with the level, in a
 * cycle of ten. Writes the level's four colors to background palette 2
 * ($3F08) and sprite palette 2 ($3F18).
 */
void updatePaletteForLevel(void)
{
    uint8_t a = player1_levelNumber;
    while (!sign_set(a, 0x0A)) /* @mod10 */
        a = (uint8_t)(a - 0x0A);
    uint8_t x = (uint8_t)(a << 2); /* @copyPalettes */
    generalCounter = 0;
    do { /* @copyPalette */
        wr(PPUADDR, 0x3F);
        wr(PPUADDR, (uint8_t)(0x08 + generalCounter));
        for (int i = 0; i < 4; i++)
            wr(PPUDATA, prg8((uint16_t)(ROM_colorTable + i + x)));
        generalCounter = (uint8_t)(generalCounter + 0x10);
    } while (generalCounter != 0x20);
}
