/*
 * The screens before a game: the legal screen, the title screen, the game
 * type and music menu, the level (and height) menu, and starting the demo.
 *
 * Each game mode handler draws its screen with NMIs off, then loops inside
 * itself, one frame per pass, until the player moves on. It leaves in A what
 * the original leaves, which the main loop compares with gameModeState (0 on
 * these screens); none of these values is 0, so the main loop never waits
 * after them.
 *
 * Drawing a screen: the handler waits a frame and turns the picture off, then
 * with NMIs off copies the palette and nametable to the PPU (left out here:
 * busy() stands for the time it takes), then waits for the vertical blank to
 * end, turns NMIs back on and over three frames turns the picture back on.
 */
#include "game.h"

/* What copying a nametable (1 KB) and a palette takes, roughly. */
#define NAMETABLE_CYCLES (1024 * 16 + 800)
#define PALETTE_CYCLES (32 * 16 + 200)

/*
 * gameMode_legalScreen (0): the copyright screen. Shown for 255 frames, then
 * up to 255 more, which Start skips. Leaves A = BUTTON_START if Start was
 * pressed, else $FF from the last wait.
 */
int gameMode_legalScreen(void)
{
    PT_BEGIN(PT_gameMode_legalScreen);
    /* updateAudio2 */
    renderMode = 0;
    WAIT_DISABLE_RENDERING(0x8209);
    disableNmi();
    busy(PALETTE_CYCLES + NAMETABLE_CYCLES); /* legal_screen_palette, legal_screen_nametable */
    waitForVBlankAndEnableNmi();
    WAIT_RESET_OAM(0x8226);
    WAIT_ENABLE_RENDERING(0x8229);
    WAIT_RESET_OAM(0x822C);
    sleepCounter = LEGAL_SLEEP_TIME;
    PT_CALL(0x823A, sleep_for_a_vblanks());
    generalCounter = LEGAL_SLEEP_TIME;
    for (;;) { /* @waitForStartButton */
        if (newlyPressedButtons_player1 == BUTTON_START) {
            regA = BUTTON_START;
            break;
        }
        WAIT_RESET_OAM(0x8247);
        if (!--generalCounter)
            break;
    }
    gameMode++; /* @continueToNextScreen */
    PT_RETURN(regA);
    PT_END;
}

/*
 * gameMode_titleScreen (1): waits for Start, which goes to the game type
 * menu. If nobody presses it for about 21 seconds (frameCounter's high byte
 * reaching 5, counted from 0) the demo starts instead. Leaves A = 2 after
 * Start, 6 for the demo.
 */
int gameMode_titleScreen(void)
{
    PT_BEGIN(PT_gameMode_titleScreen);
    /* updateAudio2 */
    renderMode = 0;
    R(0xD0) = 0; /* demo recording off */
    displayNextPiece = 0;
    WAIT_DISABLE_RENDERING(0x825C);
    disableNmi();
    busy(PALETTE_CYCLES + NAMETABLE_CYCLES); /* menu_palette, title_screen_nametable */
    waitForVBlankAndEnableNmi();
    WAIT_RESET_OAM(0x8279);
    WAIT_ENABLE_RENDERING(0x827C);
    WAIT_RESET_OAM(0x827F);
    frameCounter[1] = 0;
    for (;;) { /* @waitForStartButton */
        WAIT_RESET_OAM(0x828F);
        if (newlyPressedButtons_player1 == BUTTON_START) { /* @startButtonPressed */
            gameMode++;
            PT_RETURN(0x02);
        }
        if (frameCounter[1] == 0x05) { /* @timeout */
            gameMode = 0x06;
            PT_RETURN(0x06);
        }
    }
    PT_END;
}

/*
 * gameMode_gameTypeMenu (2): chooses A-type or B-type (left and right) and
 * the music (up and down; the menu plays each choice). Start goes on to the
 * level menu, B back to the title. Two blinking cursors mark the choices.
 * Leaves A = 2.
 */
int gameMode_gameTypeMenu(void)
{
    PT_BEGIN(PT_gameMode_gameTypeMenu);
    renderMode = 0x01;
    WAIT_DISABLE_RENDERING(0x82DF);
    disableNmi();
    busy(PALETTE_CYCLES + NAMETABLE_CYCLES); /* menu_palette, game_type_menu_nametable */
    waitForVBlankAndEnableNmi();
    WAIT_RESET_OAM(0x82FC);
    WAIT_ENABLE_RENDERING(0x82FF);
    WAIT_RESET_OAM(0x8302);
    setMusicTrack(musicType);
    for (;;) { /* L830B */
        if (newlyPressedButtons_player1 == BUTTON_RIGHT) {
            gameType = 0x01;
        } else if (newlyPressedButtons_player1 == BUTTON_LEFT) { /* @rightNotPressed */
            gameType = 0x00;
        }
        /* @leftNotPressed */
        if (newlyPressedButtons_player1 == BUTTON_DOWN) {
            if (musicType != 0x03) {
                musicType++;
                setMusicTrack(musicType);
            }
            /* (at the last track the original also skips the up check) */
        } else if (newlyPressedButtons_player1 == BUTTON_UP) { /* @downNotPressed */
            if (musicType) {
                musicType--;
                setMusicTrack(musicType);
            }
        }
        if (newlyPressedButtons_player1 == BUTTON_START) { /* @upNotPressed */
            gameMode++;
            PT_RETURN(0x02);
        }
        if (newlyPressedButtons_player1 == BUTTON_B) { /* @startNotPressed */
            frameCounter[1] = 0;
            gameMode--;
            PT_RETURN(0x02);
        }
        /* @bNotPressed: stages the two blinking cursors (left out) */
        WAIT_RESET_OAM(0x83D3);
    }
    PT_END;
}

/*
 * gameMode_levelMenu_handleLevelHeightNavigation: the d-pad moves the level
 * cursor over the 2 x 5 grid of levels 0-9 or, in B-type after A switches
 * to it, the height cursor over the 2 x 3 grid of heights 0-5. Works on the
 * copies in startLevel, startHeight and selectingLevelOrHeight. (Then it
 * stages the blinking cursors, left out.)
 */
static void gameMode_levelMenu_handleLevelHeightNavigation(void)
{
    if (newlyPressedButtons == BUTTON_RIGHT) {
        if (selectingLevelOrHeight) {
            if (startHeight != 0x05) /* @rightPressedForHeightSelection */
                startHeight++;
        } else if (startLevel != 0x09) {
            startLevel++;
        }
    }
    if (newlyPressedButtons == BUTTON_LEFT) { /* @checkLeftPressed */
        if (selectingLevelOrHeight) {
            if (startHeight) /* @leftPressedForHeightSelection */
                startHeight--;
        } else if (startLevel) {
            startLevel--;
        }
    }
    if (newlyPressedButtons == BUTTON_DOWN) { /* @checkDownPressed */
        if (selectingLevelOrHeight) {
            if (sign_set(startHeight, 0x03)) /* @downPressedForHeightSelection */
                startHeight = (uint8_t)(startHeight + 3);
        } else if (sign_set(startLevel, 0x05)) {
            startLevel = (uint8_t)(startLevel + 0x05);
        }
    }
    if (newlyPressedButtons == BUTTON_UP) { /* @checkUpPressed */
        if (selectingLevelOrHeight) {
            if (!sign_set(startHeight, 0x03)) /* @upPressedForHeightSelection */
                startHeight = (uint8_t)(startHeight - 3);
        } else if (!sign_set(startLevel, 0x05)) {
            startLevel = (uint8_t)(startLevel - 0x05);
        }
    }
    if (gameType && newlyPressedButtons == BUTTON_A) { /* @checkAPressed */
        selectingLevelOrHeight ^= 0x01;
    }
}

/*
 * gameMode_levelMenu (3): chooses the starting level (and for B-type the
 * height) and shows the high scores. Start begins the game (Start with A
 * held adds 10 to the level); B goes back. While waiting, it keeps choosing
 * random holes for two-player garbage, which steps the random number
 * generator twice or more a frame. Leaves A = 2.
 */
int gameMode_levelMenu(void)
{
    PT_BEGIN(PT_gameMode_levelMenu);
    /* updateAudio2 */
    renderMode = 0x01;
    WAIT_DISABLE_RENDERING(0x83E8);
    disableNmi();
    /* menu_palette, level_menu_nametable, (A-type) height_menu_nametablepalette_patch,
     * then showHighScores: high_scores_nametable and three entries */
    busy(PALETTE_CYCLES + NAMETABLE_CYCLES + (gameType ? 0 : 400) + 900 + 3 * 400);
    waitForVBlankAndEnableNmi();
    WAIT_RESET_OAM(0x8411);
    WAIT_ENABLE_RENDERING(0x841E);
    WAIT_RESET_OAM(0x8421);
    originalY = 0;
    dropSpeed = 0;
    while (player1_startLevel >= 0x0A) /* @forceStartLevelToRange: after Start + A */
        player1_startLevel = (uint8_t)(player1_startLevel - 0x0A);

    for (;;) { /* gameMode_levelMenu_processPlayer1Navigation */
        activePlayer = 0;
        startLevel = player1_startLevel;
        startHeight = player1_startHeight;
        selectingLevelOrHeight = originalY;
        newlyPressedButtons = newlyPressedButtons_player1;
        gameMode_levelMenu_handleLevelHeightNavigation();
        player1_startLevel = startLevel;
        player1_startHeight = startHeight;
        originalY = selectingLevelOrHeight;
        if (newlyPressedButtons_player1 == BUTTON_START) {
            if (heldButtons_player1 == BUTTON_A + BUTTON_START)
                player1_startLevel = (uint8_t)(player1_startLevel + 0x0A);
            gameModeState = 0; /* @startAndANotPressed */
            gameMode++;
            PT_RETURN(0x02);
        }
        if (newlyPressedButtons_player1 == BUTTON_B) { /* @checkBPressed */
            gameMode--;
            PT_RETURN(0x02);
        }
        uint8_t a;
        do { /* @chooseRandomHole_player1 */
            generateNextPseudorandomNumber(A_rng_seed, 2);
            a = rng_seed[0] & 0x0F;
        } while (!sign_set(a, 0x0A));
        player1_garbageHole = a;
        do { /* @chooseRandomHole_player2 */
            generateNextPseudorandomNumber(A_rng_seed, 2);
            a = rng_seed[0] & 0x0F;
        } while (!sign_set(a, 0x0A));
        player2_garbageHole = a;
        WAIT_RESET_OAM(0x84AA);
    }
    PT_END;
}

/* gameMode_startDemo (6): sets up an A-type game at level 0 as the demo
 * (gameMode 5); branchOnGameMode then runs the play handler at once. */
void gameMode_startDemo(void)
{
    gameType = 0;
    player1_startLevel = 0;
    gameModeState = 0;
    player1_playState = 0;
    gameMode = 0x05;
}

/* setMusicTrack: asks the audio engine for a music track. The decomp has no
 * audio; what is left is what the original leaves in A: gameMode, or $FF in the
 * demo (where the music is silent). */
void setMusicTrack(uint8_t a)
{
    (void)a;
    regA = gameMode == 0x05 ? 0xFF : gameMode;
}
