/*
 * High scores: inserting a new score into the table, and the name entry
 * screen.
 *
 * The table holds three entries per game type, plus a fourth slot each that
 * is never shown: entries 0-3 are A-type, 4-7 B-type. Each has a name of six
 * letters (highScoreNames: 0 = space, 1-26 = A-Z, then digits and
 * punctuation), a three-byte BCD score, high byte first (highScoreScoresA,
 * which runs on into highScoreScoresB), and a level (highScoreLevels).
 */
#include "game.h"

/* How long the name entry screen takes to draw with NMIs off, in CPU cycles
 * from the NMI (see busyUntil). */
#define HIGH_SCORE_SCREEN_DRAW 32994

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
        R(A_highScoreScoresA + (uint8_t)(x + 3)) = R(A_highScoreScoresA + x);
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

/* Changes the current letter of the name by one, wrapping through the 44
 * characters. */
static void stepLetter(int up)
{
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
 * handleHighScoreIfNecessary: finds where player 1's score ranks among the
 * three for the game type (a tie ranks above the old score). If it ranks,
 * moves the lower entries down, writes the new score and level with a blank
 * name (adjustHighScores), and jumps to highScoreEntryScreen.
 *
 * highScoreEntryScreen: "Congratulations": the player types a name. Left and
 * right (or B and A) move the blinking cursor along the six letters, holding
 * up or down changes the letter every eighth frame, Start finishes.
 */
int handleHighScoreIfNecessary(void)
{
    PT_BEGIN(PT_handleHighScoreIfNecessary);
    highScoreEntryRawPos = gameType ? 0x04 : 0x00;
    for (;;) { /* @compareWithPos */
        generalCounter2 = highScoreEntryRawPos;
        uint8_t y = (uint8_t)((uint8_t)(highScoreEntryRawPos << 1) + generalCounter2);
        uint8_t old = R(A_highScoreScoresA + y);
        if (old < player1_score[2])
            break;
        if (old == player1_score[2]) { /* @checkHundredsByte */
            y++;
            old = R(A_highScoreScoresA + y);
            if (old < player1_score[1])
                break;
            if (old == player1_score[1]) { /* @checkOnesByte */
                y++;
                if (R(A_highScoreScoresA + y) <= player1_score[0])
                    break;
            }
        }
        /* @tooSmall */
        highScoreEntryRawPos++;
        if (highScoreEntryRawPos == 0x03 || highScoreEntryRawPos == 0x07)
            PT_RETURN(highScoreEntryRawPos);
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
    {
        uint8_t x = highScoreIndexToHighScoreNamesOffset[highScoreEntryRawPos];
        for (int i = 0; i < 6; i++) /* @clearNameLetter */
            highScoreNames[x++] = 0;
        x = highScoreIndexToHighScoreScoresOffset[highScoreEntryRawPos];
        R(A_highScoreScoresA + x++) = player1_score[2];
        R(A_highScoreScoresA + x++) = player1_score[1];
        R(A_highScoreScoresA + x) = player1_score[0];
        highScoreLevels[highScoreEntryRawPos] = player1_levelNumber;
    }

    /* highScoreEntryScreen */
    setMusicTrack(0x09);
    renderMode = 0x02;
    WAIT_DISABLE_RENDERING(0xA214);
    disableNmi();
    /* menu_palette, enter_high_score_nametable, the game type, then showHighScores */
    drawLeftovers(0xC95C, 0x9FFD); /* showHighScores */
    busyUntil(HIGH_SCORE_SCREEN_DRAW); /* more than a frame: the game loses one here */
    renderMode = 0x02;
    waitForVBlankAndEnableNmi();
    WAIT_RESET_OAM(0xA24A);
    WAIT_ENABLE_RENDERING(0xA24D);
    WAIT_RESET_OAM(0xA250);
    generalCounter = (uint8_t)(highScoreEntryRawPos << 1);
    highScoreEntryNameOffsetForRow = (uint8_t)((uint8_t)(generalCounter << 1) + generalCounter);
    highScoreEntryNameOffsetForLetter = 0;
    oamStaging[0] = 0;
    for (;;) { /* @renderFrame (the blinking cursor is a sprite, left out) */
        oamStaging[0] = 0;
        if (newlyPressedButtons_player1 & BUTTON_START) /* @flickerStateSelected_checkForStartPressed */
            break;
        if (newlyPressedButtons_player1 & (BUTTON_A + BUTTON_RIGHT)) { /* @checkForAOrRightPressed */
            highScoreEntryNameOffsetForLetter++;
            if (!sign_set(highScoreEntryNameOffsetForLetter, 0x06))
                highScoreEntryNameOffsetForLetter = 0;
        }
        if (newlyPressedButtons_player1 & (BUTTON_B + BUTTON_LEFT)) { /* @checkForBOrLeftPressed */
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
        WAIT_RESET_OAM(0xA333);
    }
    WAIT_RESET_OAM(0xA339); /* @ret */
    PT_END;
}
