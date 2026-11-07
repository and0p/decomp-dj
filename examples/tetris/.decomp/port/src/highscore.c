/*
 * High scores: the table on the level menu, inserting a new score, and the
 * name entry screen.
 *
 * The table holds three entries per game type, plus a fourth slot each that
 * is never shown: entries 0-3 are A-type, 4-7 B-type. Each has a name of six
 * letters (highScoreNames, 0 = space, 1-26 = A-Z, then digits and
 * punctuation; see highScoreCharToTile), a three-byte BCD score, high byte
 * first (highScoreScoresA, which runs on into highScoreScoresB), and a level
 * (highScoreLevels).
 */
#include "game.h"

#define wr(addr, v) poke((addr), (uint8_t)(v))

/*
 * showHighScores: draws the high score box (one-player games only) with the
 * current game type's three entries: name, score and level. Levels over 49
 * show as whatever follows byteToBcdTable in ROM.
 */
void showHighScores(void)
{
    if (numberOfPlayers != 0x01)
        return; /* showHighScores_ret */
    /* showHighScores_real */
    bulkCopyToPpu(0x9FFB, ROM_high_scores_nametable);
    generalCounter2 = gameType ? 0x04 : 0x00;
    for (;;) { /* @copyEntry */
        uint8_t x = (uint8_t)((generalCounter2 & 0x03) << 1);
        wr(PPUADDR, romTable(ROM_highScorePpuAddrTable, x)); /* .dbyt: high byte first */
        wr(PPUADDR, romTable(ROM_highScorePpuAddrTable, (uint8_t)(x + 1)));
        generalCounter = (uint8_t)(generalCounter2 << 1);
        uint8_t y = (uint8_t)((uint8_t)(generalCounter << 1) + generalCounter); /* entry * 6 */
        for (x = 0x06; x; x--) { /* @copyChar */
            uint8_t letter = highScoreNames[y];
            generalCounter = y;
            wr(PPUDATA, romTable(ROM_highScoreCharToTile, letter));
            y = (uint8_t)(generalCounter + 1);
        }
        wr(PPUDATA, 0xFF);
        generalCounter = generalCounter2;
        y = (uint8_t)((uint8_t)(generalCounter2 << 1) + generalCounter); /* entry * 3 */
        twoDigsToPPU(highScoreScoresA[y]);
        y++;
        twoDigsToPPU(highScoreScoresA[y]);
        y++;
        twoDigsToPPU(highScoreScoresA[y]);
        wr(PPUDATA, 0xFF);
        twoDigsToPPU(romTable(ROM_byteToBcdTable, highScoreLevels[generalCounter2]));
        generalCounter2++;
        if (generalCounter2 == 0x03 || generalCounter2 == 0x07)
            return;
    }
}

/* copyHighScoreNameToNextIndex: copies the name at offset A (plus 24 for
 * B-type) down one entry. */
static void copyHighScoreNameToNextIndex(uint8_t a)
{
    generalCounter = a;
    if (gameType)
        generalCounter = (uint8_t)(0x18 + generalCounter);
    generalCounter2 = 0x05; /* @offsetAdjustedForGameType */
    do { /* @copyLetter */
        uint8_t x = (uint8_t)(generalCounter + generalCounter2);
        generalCounter3 = highScoreNames[x];
        highScoreNames[(uint8_t)(x + 6)] = generalCounter3;
        generalCounter2--;
    } while (generalCounter2 != 0xFF);
}

/* copyHighScoreScoreToNextIndex: copies the score at offset A (plus 12 for
 * B-type) down one entry. */
static void copyHighScoreScoreToNextIndex(uint8_t a)
{
    uint8_t x = a;
    if (gameType)
        x = (uint8_t)(x + 0x0C);
    for (int i = 0; i < 3; i++, x++) /* @xAdjustedForGameType */
        highScoreScoresA[(uint8_t)(x + 3)] = highScoreScoresA[x];
}

/* copyHighScoreLevelToNextIndex: copies the level at offset A (plus 4 for
 * B-type) down one entry. */
static void copyHighScoreLevelToNextIndex(uint8_t a)
{
    uint8_t x = a;
    if (gameType)
        x = (uint8_t)(x + 0x04);
    highScoreLevels[(uint8_t)(x + 1)] = highScoreLevels[x];
}

static void highScoreEntryScreen(void);

/*
 * handleHighScoreIfNecessary: finds where player 1's score ranks among the
 * three for the game type (a tie ranks above the old score). If it ranks,
 * moves the lower entries down, writes the new score and level with a blank
 * name (adjustHighScores), and runs the name entry screen.
 */
void handleHighScoreIfNecessary(void)
{
    highScoreEntryRawPos = gameType ? 0x04 : 0x00;
    for (;;) { /* @compareWithPos */
        generalCounter2 = highScoreEntryRawPos;
        uint8_t y = (uint8_t)((uint8_t)(highScoreEntryRawPos << 1) + generalCounter2);
        uint8_t old = highScoreScoresA[y];
        if (old < player1_score[2])
            break;
        if (old == player1_score[2]) { /* @checkHundredsByte */
            y++;
            old = highScoreScoresA[y];
            if (old < player1_score[1])
                break;
            if (old == player1_score[1]) { /* @checkOnesByte */
                y++;
                if (highScoreScoresA[y] <= player1_score[0])
                    break;
            }
        }
        /* @tooSmall */
        highScoreEntryRawPos++;
        if (highScoreEntryRawPos == 0x03 || highScoreEntryRawPos == 0x07)
            return;
    }

    /* adjustHighScores */
    if (sign_set(highScoreEntryRawPos & 0x03, 0x02)) {
        copyHighScoreNameToNextIndex(0x06);
        copyHighScoreScoreToNextIndex(0x03);
        copyHighScoreLevelToNextIndex(0x01);
        if (!(highScoreEntryRawPos & 0x03)) {
            copyHighScoreNameToNextIndex(0x00);
            copyHighScoreScoreToNextIndex(0x00);
            copyHighScoreLevelToNextIndex(0x00);
        }
    }
    /* @doneMovingOldScores */
    uint8_t x = romTable(ROM_highScoreIndexToHighScoreNamesOffset, highScoreEntryRawPos);
    for (int i = 0; i < 6; i++) /* @clearNameLetter */
        highScoreNames[x++] = 0;
    x = romTable(ROM_highScoreIndexToHighScoreScoresOffset, highScoreEntryRawPos);
    highScoreScoresA[x++] = player1_score[2];
    highScoreScoresA[x++] = player1_score[1];
    highScoreScoresA[x] = player1_score[0];
    highScoreLevels[highScoreEntryRawPos] = player1_levelNumber;
    highScoreEntryScreen();
}

/* Changes the current letter of the name by one, wrapping through the 44
 * characters. */
static void stepLetter(int up)
{
    soundEffectSlot1Init = 0x01;
    generalCounter = highScoreEntryNameOffsetForRow;
    uint8_t x = (uint8_t)(highScoreEntryNameOffsetForRow + highScoreEntryNameOffsetForLetter);
    generalCounter = highScoreNames[x];
    if (up) {
        generalCounter++;
        if (!sign_set(generalCounter, 0x2C)) /* @letterDoesNotOverflow */
            generalCounter = (uint8_t)(generalCounter - 0x2C);
    } else {
        generalCounter--;
        if (generalCounter & 0x80) /* @letterDoesNotUnderflow */
            generalCounter = (uint8_t)(generalCounter + 0x2C);
    }
    highScoreNames[x] = generalCounter;
}

/*
 * highScoreEntryScreen: "Congratulations": the player types a name. Left and
 * right (or B and A) move the blinking cursor along the six letters, holding
 * up or down changes the letter every eighth frame, Start finishes. The NMI
 * draws the changed letter (render_mode_congratulations_screen).
 */
static void highScoreEntryScreen(void)
{
    incrementInitRam();
    setMMC1Control(MMC1_4KCHR_32KPRG_H_MIRROR);
    setMusicTrack(0x09);
    renderMode = 0x02;
    updateAudioWaitForNmiAndDisablePpuRendering();
    disableNmi();
    changeCHRBank0(CHR_TITLE_MENU);
    changeCHRBank1(CHR_TITLE_MENU);
    bulkCopyToPpu(0xA222, ROM_menu_palette);
    bulkCopyToPpu(0xA227, ROM_enter_high_score_nametable);
    wr(PPUADDR, 0x20);
    wr(PPUADDR, 0x6D);
    wr(PPUDATA, (uint8_t)(0x0A + gameType)); /* "A" or "B" */
    showHighScores();
    renderMode = 0x02;
    waitForVBlankAndEnableNmi();
    updateAudioWaitForNmiAndResetOamStaging();
    updateAudioWaitForNmiAndEnablePpuRendering();
    updateAudioWaitForNmiAndResetOamStaging();
    generalCounter = (uint8_t)(highScoreEntryRawPos << 1);
    highScoreEntryNameOffsetForRow = (uint8_t)((uint8_t)(generalCounter << 1) + generalCounter);
    highScoreEntryNameOffsetForLetter = 0;
    oamStaging[0] = 0;
    spriteYOffset = romTable(ROM_highScorePosToY, highScoreEntryRawPos & 0x03);
    for (;;) { /* @renderFrame */
        oamStaging[0] = 0;
        spriteXOffset = romTable(ROM_highScoreNamePosToX, highScoreEntryNameOffsetForLetter);
        spriteIndexInOamContentLookup = 0x0E;
        if (!(frameCounter[0] & MENU_CURSOR_MASK))
            spriteIndexInOamContentLookup = 0x02;
        loadSpriteIntoOamStaging(); /* @flickerStateSelected_checkForStartPressed */
        if (newlyPressedButtons_player1 & BUTTON_START) {
            soundEffectSlot1Init = 0x02;
            break;
        }
        if (newlyPressedButtons_player1 & (BUTTON_A + BUTTON_RIGHT)) { /* @checkForAOrRightPressed */
            soundEffectSlot1Init = 0x01;
            highScoreEntryNameOffsetForLetter++;
            if (!sign_set(highScoreEntryNameOffsetForLetter, 0x06))
                highScoreEntryNameOffsetForLetter = 0;
        }
        if (newlyPressedButtons_player1 & (BUTTON_B + BUTTON_LEFT)) { /* @checkForBOrLeftPressed */
            soundEffectSlot1Init = 0x01;
            highScoreEntryNameOffsetForLetter--;
            if (highScoreEntryNameOffsetForLetter & 0x80)
                highScoreEntryNameOffsetForLetter = 0x05;
        }
        if ((heldButtons_player1 & BUTTON_DOWN) && !(frameCounter[0] & 0x07)) /* @checkForDownPressed */
            stepLetter(0);
        if ((heldButtons_player1 & BUTTON_UP) && !(frameCounter[0] & 0x07)) /* @checkForUpPressed */
            stepLetter(1);
        /* @waitForVBlank */
        highScoreEntryCurrentLetter =
            highScoreNames[(uint8_t)(highScoreEntryNameOffsetForRow + highScoreEntryNameOffsetForLetter)];
        outOfDateRenderFlags = RENDER_HIGH_SCORE_LETTER;
        updateAudioWaitForNmiAndResetOamStaging();
    }
    updateAudioWaitForNmiAndResetOamStaging(); /* @ret */
}

/* render_mode_congratulations_screen: the NMI's work on the name entry
 * screen: draws the letter just changed. */
void render_mode_congratulations_screen(void)
{
    if (!(outOfDateRenderFlags & RENDER_HIGH_SCORE_LETTER))
        return;
    uint8_t x = (uint8_t)((highScoreEntryRawPos & 0x03) << 1);
    wr(PPUADDR, romTable(ROM_highScorePpuAddrTable, x));
    generalCounter = romTable(ROM_highScorePpuAddrTable, (uint8_t)(x + 1));
    wr(PPUADDR, (uint8_t)(generalCounter + highScoreEntryNameOffsetForLetter));
    wr(PPUDATA, romTable(ROM_highScoreCharToTile, highScoreEntryCurrentLetter));
    ppuScrollX = 0;
    wr(PPUSCROLL, 0);
    ppuScrollY = 0;
    wr(PPUSCROLL, 0);
    outOfDateRenderFlags = 0;
}
