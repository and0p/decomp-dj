/*
 * The screens before a game: the legal screen, the title screen, the game
 * type and music menu, the level (and height) menu, and the demo's input.
 *
 * Each game mode handler draws its screen with NMIs off, then loops inside
 * itself, one frame per pass, until the player moves on. It returns the A the
 * original leaves, which the main loop compares with gameModeState (0 on
 * these screens); none of these values is 0, so the main loop never waits
 * after them.
 */
#include "game.h"

#define wr(addr, v) poke((addr), (uint8_t)(v))

/* The screens drawn with NMIs off use the same four steps to turn the screen
 * back on: wait for the end of vertical blank, enable NMIs, and over three
 * frames turn rendering on. */
static void finishScreenSetup(void)
{
    waitForVBlankAndEnableNmi();
    updateAudioWaitForNmiAndResetOamStaging();
    updateAudioWaitForNmiAndEnablePpuRendering();
    updateAudioWaitForNmiAndResetOamStaging();
}

/*
 * gameMode_legalScreen (0): the copyright screen. Shown for 255 frames, then
 * up to 255 more, which Start skips. Returns A: BUTTON_START if Start was
 * pressed, else $FF from the last wait.
 */
uint8_t gameMode_legalScreen(void)
{
    updateAudio2();
    renderMode = 0;
    updateAudioWaitForNmiAndDisablePpuRendering();
    disableNmi();
    changeCHRBank0(CHR_TITLE_MENU);
    changeCHRBank1(CHR_TITLE_MENU);
    bulkCopyToPpu(0x8217, ROM_legal_screen_palette);
    bulkCopyToPpu(0x821C, ROM_legal_screen_nametable);
    finishScreenSetup();
    memset_page(0x00, 0x02, 0x02); /* clears oamStaging to 0 */
    sleep_for_a_vblanks(LEGAL_SLEEP_TIME);
    generalCounter = LEGAL_SLEEP_TIME;
    uint8_t a;
    do { /* @waitForStartButton */
        a = newlyPressedButtons_player1;
        if (a == BUTTON_START)
            break;
        a = updateAudioWaitForNmiAndResetOamStaging();
    } while (--generalCounter);
    gameMode++; /* @continueToNextScreen */
    return a;
}

/*
 * gameMode_titleScreen (1): waits for Start, which goes to the game type
 * menu. If nobody presses it for about 21 seconds (frameCounter's high byte
 * reaching 5, counted from 0) the demo starts instead.
 * Returns A = 2 after Start, 6 for the demo.
 */
uint8_t gameMode_titleScreen(void)
{
    updateAudio2();
    renderMode = 0;
    R(0xD0) = 0; /* demo recording off ($FF records the demo; see pollControllerButtons) */
    displayNextPiece = 0;
    updateAudioWaitForNmiAndDisablePpuRendering();
    disableNmi();
    changeCHRBank0(CHR_TITLE_MENU);
    changeCHRBank1(CHR_TITLE_MENU);
    bulkCopyToPpu(0x826A, ROM_menu_palette);
    bulkCopyToPpu(0x826F, ROM_title_screen_nametable);
    finishScreenSetup();
    memset_page(0x00, 0x02, 0x02);
    frameCounter[1] = 0;
    for (;;) { /* @waitForStartButton */
        updateAudioWaitForNmiAndResetOamStaging();
        if (newlyPressedButtons_player1 == BUTTON_START) {
            soundEffectSlot1Init = 0x02; /* @startButtonPressed */
            gameMode++;
            return 0x02;
        }
        if (frameCounter[1] == 0x05) {
            soundEffectSlot1Init = 0x02; /* @timeout */
            gameMode = 0x06;
            return 0x06;
        }
    }
}

/* render_mode_legal_and_title_screens: the NMI's work on those screens: show
 * nametable 0 (PPUCTRL's low bits are cleared in currentPpuCtrl only) and
 * reset the scroll. */
void render_mode_legal_and_title_screens(void)
{
    currentPpuCtrl &= 0xFC;
    ppuScrollX = 0;
    wr(PPUSCROLL, 0);
    ppuScrollY = 0;
    wr(PPUSCROLL, 0);
}

/*
 * gameMode_gameTypeMenu (2): chooses A-type or B-type (left and right) and
 * the music (up and down; the menu plays each choice). Start goes on to the
 * level menu, B back to the title. Two blinking cursors mark the choices.
 * Returns A = 2.
 */
uint8_t gameMode_gameTypeMenu(void)
{
    incrementInitRam();
    setMMC1Control(MMC1_4KCHR_32KPRG_H_MIRROR);
    renderMode = 0x01;
    updateAudioWaitForNmiAndDisablePpuRendering();
    disableNmi();
    bulkCopyToPpu(0x82E3, ROM_menu_palette);
    bulkCopyToPpu(0x82E8, ROM_game_type_menu_nametable);
    changeCHRBank0(CHR_TITLE_MENU);
    changeCHRBank1(CHR_TITLE_MENU);
    finishScreenSetup();
    setMusicTrack(romTable(ROM_musicSelectionTable, musicType));
    for (;;) { /* L830B */
        memset_page(0xFF, 0x02, 0x02);
        if (newlyPressedButtons_player1 == BUTTON_RIGHT) {
            gameType = 0x01;
            soundEffectSlot1Init = 0x01;
        } else if (newlyPressedButtons_player1 == BUTTON_LEFT) { /* @rightNotPressed */
            gameType = 0x00;
            soundEffectSlot1Init = 0x01;
        }
        /* @leftNotPressed */
        if (newlyPressedButtons_player1 == BUTTON_DOWN) {
            soundEffectSlot1Init = 0x01;
            if (musicType != 0x03) {
                musicType++;
                setMusicTrack(romTable(ROM_musicSelectionTable, musicType));
            } else {
                goto upNotPressed; /* the original skips the up check, too */
            }
        }
        /* @downNotPressed */
        if (newlyPressedButtons_player1 == BUTTON_UP) {
            soundEffectSlot1Init = 0x01;
            if (musicType) {
                musicType--;
                setMusicTrack(romTable(ROM_musicSelectionTable, musicType));
            }
        }
    upNotPressed:
        if (newlyPressedButtons_player1 == BUTTON_START) {
            soundEffectSlot1Init = 0x02;
            gameMode++;
            return 0x02;
        }
        if (newlyPressedButtons_player1 == BUTTON_B) { /* @startNotPressed */
            soundEffectSlot1Init = 0x02;
            frameCounter[1] = 0;
            gameMode--;
            return 0x02;
        }
        /* @bNotPressed: the game type cursor, at x = $3F + 96 * gameType */
        generalCounter = (uint8_t)(gameType << 1);
        spriteXOffset = (uint8_t)((uint8_t)(((uint8_t)(generalCounter << 1) + generalCounter) << 4) + 0x3F);
        spriteYOffset = 0x3F;
        spriteIndexInOamContentLookup = 0x01;
        if (!(frameCounter[0] & MENU_CURSOR_MASK))
            spriteIndexInOamContentLookup = 0x02; /* blank: the cursor blinks */
        loadSpriteIntoOamStaging(); /* @flickerCursorPair1 */
        /* the music cursor */
        spriteYOffset = (uint8_t)((uint8_t)(musicType << 4) + 0x8F);
        spriteIndexInOamContentLookup = 0x53;
        spriteXOffset = 0x67;
        if (!(frameCounter[0] & MENU_CURSOR_MASK))
            spriteIndexInOamContentLookup = 0x02;
        loadSpriteIntoOamStaging(); /* @flickerCursorPair2 */
        updateAudioWaitForNmiAndResetOamStaging();
    }
}

/*
 * gameMode_levelMenu_handleLevelHeightNavigation: the d-pad moves the level
 * cursor over the 2 x 5 grid of levels 0-9 or, in B-type after A switches
 * to it, the height cursor over the 2 x 3 grid of heights 0-5. Then stages
 * the blinking cursors. Works on the copies in startLevel, startHeight and
 * selectingLevelOrHeight.
 */
static void gameMode_levelMenu_handleLevelHeightNavigation(void)
{
    if (newlyPressedButtons == BUTTON_RIGHT) {
        soundEffectSlot1Init = 0x01;
        if (selectingLevelOrHeight) {
            if (startHeight != 0x05) /* @rightPressedForHeightSelection */
                startHeight++;
        } else if (startLevel != 0x09) {
            startLevel++;
        }
    }
    if (newlyPressedButtons == BUTTON_LEFT) { /* @checkLeftPressed */
        soundEffectSlot1Init = 0x01;
        if (selectingLevelOrHeight) {
            if (startHeight) /* @leftPressedForHeightSelection */
                startHeight--;
        } else if (startLevel) {
            startLevel--;
        }
    }
    if (newlyPressedButtons == BUTTON_DOWN) { /* @checkDownPressed */
        soundEffectSlot1Init = 0x01;
        if (selectingLevelOrHeight) {
            if (sign_set(startHeight, 0x03)) /* @downPressedForHeightSelection */
                startHeight = (uint8_t)(startHeight + 3);
        } else if (sign_set(startLevel, 0x05)) {
            startLevel = (uint8_t)(startLevel + 0x05);
        }
    }
    if (newlyPressedButtons == BUTTON_UP) { /* @checkUpPressed */
        soundEffectSlot1Init = 0x01;
        if (selectingLevelOrHeight) {
            if (!sign_set(startHeight, 0x03)) /* @upPressedForHeightSelection */
                startHeight = (uint8_t)(startHeight - 3);
        } else if (!sign_set(startLevel, 0x05)) {
            startLevel = (uint8_t)(startLevel - 0x05);
        }
    }
    if (gameType && newlyPressedButtons == BUTTON_A) { /* @checkAPressed */
        soundEffectSlot1Init = 0x01;
        selectingLevelOrHeight ^= 0x01;
    }
    /* @showSelection: the cursor being moved blinks; the other stays lit */
    if (selectingLevelOrHeight || (frameCounter[0] & MENU_CURSOR_MASK)) {
        /* @showSelectionLevel */
        spriteYOffset = romTable(ROM_levelToSpriteYOffset, startLevel);
        spriteIndexInOamContentLookup = 0x00;
        spriteXOffset = romTable(ROM_levelToSpriteXOffset, startLevel);
        if (activePlayer == 0x01)
            spriteYOffset = (uint8_t)(spriteYOffset + 0x50);
        loadSpriteIntoOamStaging(); /* @stageLevelSelectCursor */
    }
    /* @skipShowingSelectionLevel */
    if (!gameType)
        return;
    if (selectingLevelOrHeight && !(frameCounter[0] & MENU_CURSOR_MASK))
        return;
    /* @showSelectionHeight */
    spriteYOffset = romTable(ROM_heightToPpuHighAddr, startHeight);
    spriteIndexInOamContentLookup = 0x00;
    spriteXOffset = romTable(ROM_heightToPpuLowAddr, startHeight);
    if (activePlayer == 0x01)
        spriteYOffset = (uint8_t)(spriteYOffset + 0x50);
    loadSpriteIntoOamStaging(); /* @stageHeightSelectCursor */
}

/*
 * gameMode_levelMenu (3): chooses the starting level (and for B-type the
 * height) and shows the high scores. Start begins the game (Start with A
 * held adds 10 to the level); B goes back. While waiting, it keeps choosing
 * random holes for two-player garbage.
 * Returns A = 2.
 */
uint8_t gameMode_levelMenu(void)
{
    incrementInitRam();
    setMMC1Control(MMC1_4KCHR_32KPRG_H_MIRROR);
    updateAudio2();
    renderMode = 0x01;
    updateAudioWaitForNmiAndDisablePpuRendering();
    disableNmi();
    changeCHRBank0(CHR_TITLE_MENU);
    changeCHRBank1(CHR_TITLE_MENU);
    bulkCopyToPpu(0x83F6, ROM_menu_palette);
    bulkCopyToPpu(0x83FB, ROM_level_menu_nametable);
    if (!gameType)
        bulkCopyToPpu(0x8404, ROM_height_menu_nametablepalette_patch); /* hides the height box */
    showHighScores(); /* @skipTypeBHeightDisplay */
    waitForVBlankAndEnableNmi();
    updateAudioWaitForNmiAndResetOamStaging();
    wr(PPUSCROLL, 0);
    wr(PPUSCROLL, 0);
    updateAudioWaitForNmiAndEnablePpuRendering();
    updateAudioWaitForNmiAndResetOamStaging();
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
            soundEffectSlot1Init = 0x02;
            gameMode++;
            return 0x02;
        }
        if (newlyPressedButtons_player1 == BUTTON_B) { /* @checkBPressed */
            soundEffectSlot1Init = 0x02;
            gameMode--;
            return 0x02;
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
        updateAudioWaitForNmiAndResetOamStaging();
    }
}

/* render_mode_menu_screens: the NMI's work on the menus: nametable 0, scroll 0. */
void render_mode_menu_screens(void)
{
    currentPpuCtrl &= 0xFC;
    wr(PPUCTRL, currentPpuCtrl);
    ppuScrollX = 0;
    wr(PPUSCROLL, 0);
    ppuScrollY = 0;
    wr(PPUSCROLL, 0);
}

/* gameMode_startDemo (6): starts an A-type game at level 0 as the demo
 * (gameMode 5), and runs its first state at once. */
uint8_t gameMode_startDemo(void)
{
    gameType = 0;
    player1_startLevel = 0;
    gameModeState = 0;
    player1_playState = 0;
    gameMode = 0x05;
    return gameMode_playAndEndingHighScore(); /* via gameMode_playAndEndingHighScore_jmp */
}

/* setMusicTrack: asks the audio engine for a track (music is silent in the
 * demo). */
void setMusicTrack(uint8_t a)
{
    musicTrack = a;
    if (gameMode == 0x05)
        musicTrack = 0xFF;
}

/* demoButtonsTable_indexIncr: steps demoButtonsAddr, a 16-bit pointer. */
void demoButtonsTable_indexIncr(void)
{
    uint16_t p = (uint16_t)(ptr16(A_demoButtonsAddr) + 1);
    R(A_demoButtonsAddr) = (uint8_t)p;
    R(A_demoButtonsAddr + 1) = (uint8_t)(p >> 8);
}

/*
 * pollControllerButtons: called from the NMI to read the controllers. In the
 * demo the input comes from demoButtonsTable instead: pairs of (buttons held,
 * frames to hold them). Start in the demo goes back to the title screen. The
 * original can also record a demo, when $D0 is $FF (set by nothing in the
 * game): it writes pairs to the table's address, which on a cartridge is ROM.
 */
void pollControllerButtons(void)
{
    if (gameMode != 0x05) {
        pollController();
        return;
    }
    /* @demoGameMode */
    if (R(0xD0) != 0xFF) {
        pollController();
        if (newlyPressedButtons_player1 == BUTTON_START) { /* @startButtonPressed */
            R(A_demoButtonsAddr + 1) = ROM_demoButtonsTable >> 8;
            frameCounter[1] = 0;
            gameMode = 0x01;
            return;
        }
        if (demo_repeats) {
            demo_repeats--;
            newlyPressedButtons_player1 = 0; /* @moveInProgress */
        } else { /* @finishedMove */
            generalCounter = peek(ptr16(A_demoButtonsAddr));
            demoButtonsTable_indexIncr();
            newlyPressedButtons_player1 = (demo_heldButtons ^ generalCounter) & generalCounter;
            demo_heldButtons = generalCounter;
            demo_repeats = peek(ptr16(A_demoButtonsAddr));
            demoButtonsTable_indexIncr();
            if (R(A_demoButtonsAddr + 1) == (ROM_demoTetriminoTypeTable >> 8))
                return;
        }
        heldButtons_player1 = demo_heldButtons; /* @holdButtons */
        return;
    }
    /* @recording */
    pollController();
    if (gameMode != 0x05 || R(0xD0) != 0xFF)
        return;
    if (heldButtons_player1 == demo_heldButtons) {
        demo_repeats++; /* @buttonsNotChanged */
        return;
    }
    poke(ptr16(A_demoButtonsAddr), demo_heldButtons);
    demoButtonsTable_indexIncr();
    poke(ptr16(A_demoButtonsAddr), demo_repeats);
    demoButtonsTable_indexIncr();
    if (R(A_demoButtonsAddr + 1) == (ROM_demoTetriminoTypeTable >> 8))
        return;
    demo_heldButtons = heldButtons_player1;
    demo_repeats = 0;
}
