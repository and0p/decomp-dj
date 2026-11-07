/*
 * Play: the game modes 4 (play, ending and high score) and 5 (demo), which
 * step through gameModeState, and the per-player states (playState) that
 * move, rotate, drop and lock pieces, clear lines and keep score.
 *
 * The active player's data is the 32 bytes at $40 (tetriminoX onward). The
 * game copies a player's block in from $60 or $80, runs its playState, and
 * copies it back. Two-player games exist in the code but cannot be started
 * from the menus; that code is translated but has not been run.
 */
#include "game.h"

#define wr(addr, v) poke((addr), (uint8_t)(v))

/* A byte of the playfield being processed, (playfieldAddr),y. */
#define PF(y) R(ptr16(A_playfieldAddr) + (uint8_t)(y))

static void playState_unassignOrientationId(void);
static void playState_playerControlsActiveTetrimino(void);
static void playState_lockTetrimino(void);
static void playState_checkForCompletedRows(void);
static void playState_noop(void);
static void playState_updateLinesAndStatistics(void);
static void playState_bTypeGoalCheck(void);
static void playState_receiveGarbage(void);
static void playState_spawnNextTetrimino(void);
static void playState_updateGameOverCurtain(void);
static void playState_incrementPlayState(void);
static void shift_tetrimino(void);
static void rotate_tetrimino(void);
static void drop_tetrimino(void);
static void initPlayfieldIfTypeB(void);
static void addHoldDownPoints(void);
static void addLineClearPoints(void);

/* ------------------------------------------------------------------------ */
/* gameModeState                                                            */
/* ------------------------------------------------------------------------ */

/*
 * gameModeState_initGameBackground (state 0): draws the playfield screen:
 * the game's tiles and palette, the nametable, the "A-TYPE" or "B-TYPE"
 * label, the top score for the game type and, for B-type, the height box.
 * Sets both players to playState 1 at their starting level.
 * Returns A = player2_startLevel, left from the last load.
 */
static uint8_t gameModeState_initGameBackground(void)
{
    updateAudioWaitForNmiAndDisablePpuRendering();
    disableNmi();
    changeCHRBank0(CHR_GAME);
    changeCHRBank1(CHR_GAME);
    bulkCopyToPpu(0x8600, ROM_game_palette);
    bulkCopyToPpu(0x8605, ROM_game_nametable);
    wr(PPUADDR, 0x20);
    wr(PPUADDR, 0x83);
    if (!gameType) {
        wr(PPUDATA, 0x0A); /* "A" */
        wr(PPUADDR, 0x20);
        wr(PPUADDR, 0xB8);
        twoDigsToPPU(highScoreScoresA[0]);
        twoDigsToPPU(highScoreScoresA[1]);
        twoDigsToPPU(highScoreScoresA[2]);
    } else { /* @typeB */
        wr(PPUDATA, 0x0B); /* "B" */
        wr(PPUADDR, 0x20);
        wr(PPUADDR, 0xB8);
        twoDigsToPPU(highScoreScoresB[0]);
        twoDigsToPPU(highScoreScoresB[1]);
        twoDigsToPPU(highScoreScoresB[2]);
        /* game_typeb_nametable_patch: address, tiles, $FE for a new address, $FD to end */
        uint8_t x = 0;
        for (;;) { /* @nextPpuAddress */
            wr(PPUADDR, romTable(ROM_game_typeb_nametable_patch, x++));
            wr(PPUADDR, romTable(ROM_game_typeb_nametable_patch, x++));
            uint8_t a;
            while ((a = romTable(ROM_game_typeb_nametable_patch, x++)) != 0xFE && a != 0xFD)
                wr(PPUDATA, a); /* @nextPpuData */
            if (a == 0xFD)
                break;
        }
        /* @endOfPpuPatching: the height digit */
        wr(PPUADDR, 0x23);
        wr(PPUADDR, 0x3B);
        wr(PPUDATA, startHeight & 0x0F);
    }
    /* gameModeState_initGameBackground_finish */
    waitForVBlankAndEnableNmi();
    updateAudioWaitForNmiAndResetOamStaging();
    updateAudioWaitForNmiAndEnablePpuRendering();
    updateAudioWaitForNmiAndResetOamStaging();
    player1_playState = 0x01;
    player2_playState = 0x01;
    player1_levelNumber = player1_startLevel;
    player2_levelNumber = player2_startLevel;
    gameModeState++;
    return player2_startLevel;
}

/*
 * gameModeState_initGameState (state 1): clears the playfield and the
 * statistics, puts both players' pieces at the top, deals the first piece
 * and the next one, sets B-type's 25 lines and garbage, and starts the music.
 * Returns A from setMusicTrack.
 */
static uint8_t gameModeState_initGameState(void)
{
    memset_page(tileEmpty, 0x04, 0x04);
    for (uint8_t x = 0x0F; x; x--) /* @initStatsByType: $03F0-$03FD, and $03FE */
        R(A_statsByType - 1 + x) = 0;
    player1_tetriminoX = 0x05;
    player2_tetriminoX = 0x05;
    player1_tetriminoY = 0;
    player2_tetriminoY = 0;
    player1_vramRow = 0;
    player2_vramRow = 0;
    player1_fallTimer = 0;
    player2_fallTimer = 0;
    pendingGarbage = 0;
    pendingGarbageInactivePlayer = 0;
    player1_score[0] = player1_score[1] = player1_score[2] = 0;
    player2_score[0] = player2_score[1] = player2_score[2] = 0;
    player1_lines[0] = player1_lines[1] = 0;
    player2_lines[0] = player2_lines[1] = 0;
    twoPlayerPieceDelayCounter = 0;
    for (int i = 0; i < 4; i++)
        lineClearStatsByType[i] = 0;
    allegro = 0;
    demo_heldButtons = 0;
    demo_repeats = 0;
    demoIndex = 0;
    R(A_demoButtonsAddr) = 0;
    spawnID = 0;
    R(A_demoButtonsAddr + 1) = ROM_demoButtonsTable >> 8;
    renderMode = 0x03;
    player1_autorepeatY = INITIAL_AUTOREPEAT_Y;
    player2_autorepeatY = INITIAL_AUTOREPEAT_Y;
    uint8_t a = chooseNextTetrimino();
    player1_currentPiece = a;
    player2_currentPiece = a;
    incrementPieceStat(a);
    generateNextPseudorandomNumber(A_rng_seed, 2);
    a = chooseNextTetrimino();
    nextPiece = a;
    twoPlayerPieceDelayPiece = a;
    if (gameType) {
        player1_lines[0] = 0x25;
        player2_lines[0] = 0x25;
    }
    /* @skipTypeBInit */
    outOfDateRenderFlags = RENDER_STATS + RENDER_SCORE + RENDER_LEVEL + RENDER_LINES;
    updateAudioWaitForNmiAndResetOamStaging();
    initPlayfieldIfTypeB();
    setMusicTrack(prg8((uint16_t)(ROM_musicSelectionTable + musicType)));
    gameModeState++;
    /* setMusicTrack leaves A = gameMode, or $FF in the demo */
    return gameMode == 0x05 ? 0xFF : gameMode;
}

/* initPlayfieldIfTypeB: B-type starts with garbage: twelve rows of random
 * squares (each with at least one gap) are added a frame at a time, then
 * rows above the chosen height are cleared again. */
static void initPlayfieldIfTypeB(void)
{
    if (!gameType)
        return; /* endTypeBInit */
    /* initPlayfieldForTypeB */
    generalCounter = 0x0C;
    while (generalCounter) { /* typeBRows */
        generalCounter2 = (uint8_t)(0x14 - generalCounter); /* row */
        player1_vramRow = 0;
        player2_vramRow = 0;
        generalCounter3 = 0x09; /* column */
        for (;;) { /* typeBGarbageInRow */
            generateNextPseudorandomNumber(A_rng_seed, 2);
            generalCounter4 = romTable(ROM_rngTable, rng_seed[0] & 0x07); /* a square, or blank */
            uint8_t y = (uint8_t)(romTable(ROM_multBy10Table, generalCounter2) + generalCounter3);
            playfield[y] = generalCounter4;
            if (!generalCounter3)
                break;
            generalCounter3--;
        }
        uint8_t a;
        do { /* typeBGuaranteeBlank: one random column of the row is always empty */
            generateNextPseudorandomNumber(A_rng_seed, 2);
            a = rng_seed[0] & 0x0F;
        } while (!sign_set(a, 0x0A));
        generalCounter5 = a;
        playfield[(uint8_t)(romTable(ROM_multBy10Table, generalCounter2) + generalCounter5)] = tileEmpty;
        updateAudioWaitForNmiAndResetOamStaging();
        generalCounter--;
    }
    /* initCopyPlayfieldToPlayer2: copies $0401-$04C8 (not $0400) */
    for (uint8_t x = 0xC8; x; x--)
        playfieldForSecondPlayer[x] = playfield[x];
    /* Clear the playfield above the starting height (typeBBlankInitPlayer1/2). */
    uint8_t y = romTable(ROM_typeBBlankInitCountByHeightTable, player1_startHeight);
    do
        playfield[y] = tileEmpty;
    while (y-- != 0x00);
    y = romTable(ROM_typeBBlankInitCountByHeightTable, player2_startHeight);
    do
        playfieldForSecondPlayer[y] = tileEmpty;
    while (y-- != 0x00);
}

/*
 * gameModeState_updateCountersAndNonPlayerState (state 2): selects the
 * game's tiles, counts the fall timers and the two-player piece delay, and
 * toggles the next piece preview on Select.
 * Returns A: 0, or the new displayNextPiece when Select was pressed.
 */
static uint8_t gameModeState_updateCountersAndNonPlayerState(void)
{
    changeCHRBank0(CHR_GAME);
    changeCHRBank1(CHR_GAME);
    oamStagingLength = 0;
    player1_fallTimer++;
    player2_fallTimer++;
    if (twoPlayerPieceDelayCounter)
        twoPlayerPieceDelayCounter++;
    uint8_t a = newlyPressedButtons_player1 & BUTTON_SELECT; /* @checkSelectButtonPressed */
    if (a) {
        a = displayNextPiece ^ 0x01;
        displayNextPiece = a;
    }
    gameModeState++;
    return a;
}

/*
 * gameModeState_handleGameOver (state 3): if the game is over (playState 0),
 * checks for a high score, clears the playfields and goes back to the level
 * menu. Otherwise moves on.
 * Returns A: 3 (gameMode) after a game over, which equals gameModeState and
 * so makes the main loop wait a frame; 1 (numberOfPlayers) otherwise.
 */
static uint8_t gameModeState_handleGameOver(void)
{
    generalCounter2 = 0x05;
    if (player1_playState != 0x00) {
        if (numberOfPlayers == 0x01) {
            gameModeState++; /* @ret */
            return numberOfPlayers;
        }
        generalCounter2 = 0x04;
        if (player2_playState != 0x00) {
            gameModeState++;
            return player2_playState;
        }
    }
    /* @gameOver */
    if (numberOfPlayers != 0x01) {
        gameModeState = 0x09;
        return 0x09;
    }
    /* @onePlayerGameOver */
    renderMode = 0x03;
    if (numberOfPlayers == 0x01)
        handleHighScoreIfNecessary();
    /* @resetGameState */
    player1_playState = 0x01;
    player2_playState = 0x01;
    memset_page(tileEmpty, 0x04, 0x05);
    player1_vramRow = 0;
    player2_vramRow = 0;
    player1_playState = 0x01;
    player2_playState = 0x01;
    updateAudioWaitForNmiAndResetOamStaging();
    gameMode = 0x03;
    return 0x03;
}

/* makePlayer1Active: copies player 1's block ($60) to the active player's ($40). */
static void makePlayer1Active(void)
{
    activePlayer = 0x01;
    R(A_playfieldAddr + 1) = 0x04;
    newlyPressedButtons = newlyPressedButtons_player1;
    heldButtons = heldButtons_player1;
    for (int x = 0x1F; x >= 0; x--)
        R(A_tetriminoX + x) = R(A_player1_tetriminoX + x);
}

/* makePlayer2Active: copies player 2's block ($80) to the active player's ($40). */
static void makePlayer2Active(void)
{
    activePlayer = 0x02;
    R(A_playfieldAddr + 1) = 0x05;
    newlyPressedButtons = newlyPressedButtons_player2;
    heldButtons = heldButtons_player2;
    for (int x = 0x1F; x >= 0; x--)
        R(A_tetriminoX + x) = R(A_player2_tetriminoX + x);
}

/* savePlayer1State: copies the active block back to player 1's. In a
 * two-player game, also swaps the pending garbage so that what this player
 * sent waits for the other. */
static void savePlayer1State(void)
{
    for (int x = 0x1F; x >= 0; x--)
        R(A_player1_tetriminoX + x) = R(A_tetriminoX + x);
    if (numberOfPlayers == 0x01)
        return;
    uint8_t x = pendingGarbage;
    pendingGarbage = pendingGarbageInactivePlayer;
    pendingGarbageInactivePlayer = x;
}

/* savePlayer2State: as savePlayer1State, for player 2 (always swaps).
 * Returns A, the garbage now pending for this player. */
static uint8_t savePlayer2State(void)
{
    for (int x = 0x1F; x >= 0; x--)
        R(A_player2_tetriminoX + x) = R(A_tetriminoX + x);
    uint8_t x = pendingGarbage;
    pendingGarbage = pendingGarbageInactivePlayer;
    pendingGarbageInactivePlayer = x;
    return pendingGarbage;
}

/* branchOnPlayStatePlayer1 and branchOnPlayStatePlayer2: run the active
 * player's playState. The two tables differ only in entry 1, which in the
 * original are two copies of the same routine. */
static void branchOnPlayState(uint16_t table)
{
    switch_s_plus_2a(playState, table);
    switch (playState) {
    case 0x00: playState_unassignOrientationId(); break;
    case 0x01: playState_playerControlsActiveTetrimino(); break;
    case 0x02: playState_lockTetrimino(); break;
    case 0x03: playState_checkForCompletedRows(); break;
    case 0x04: playState_noop(); break; /* the NMI animates the line clear */
    case 0x05: playState_updateLinesAndStatistics(); break;
    case 0x06: playState_bTypeGoalCheck(); break;
    case 0x07: playState_receiveGarbage(); break;
    case 0x08: playState_spawnNextTetrimino(); break;
    case 0x09: playState_noop(); break;
    case 0x0A: playState_updateGameOverCurtain(); break;
    case 0x0B: playState_incrementPlayState(); break;
    }
}

/* gameModeState_updatePlayer1 (state 4): runs player 1's playState, stages
 * the piece and the next piece preview as sprites.
 * Returns A from stageSpriteForNextPiece. */
static uint8_t gameModeState_updatePlayer1(void)
{
    makePlayer1Active();
    branchOnPlayState((uint16_t)(ROM_branchOnPlayStatePlayer1 + 5));
    stageSpriteForCurrentPiece();
    savePlayer1State();
    uint8_t a = stageSpriteForNextPiece();
    gameModeState++;
    return a;
}

/* gameModeState_updatePlayer2 (state 5): the same for player 2, in
 * two-player games. Returns A: numberOfPlayers in a one-player game. */
static uint8_t gameModeState_updatePlayer2(void)
{
    uint8_t a = numberOfPlayers;
    if (numberOfPlayers == 0x02) {
        makePlayer2Active();
        branchOnPlayState((uint16_t)(ROM_branchOnPlayStatePlayer2 + 5));
        stageSpriteForCurrentPiece();
        a = savePlayer2State();
    }
    gameModeState++; /* @ret */
    return a;
}

/* gameModeState_checkForResetKeyCombo (state 6): A+B+Select+Start held
 * returns to the legal screen. Returns A: heldButtons_player1, or 0 on reset. */
static uint8_t gameModeState_checkForResetKeyCombo(void)
{
    if (heldButtons_player1 == BUTTON_A + BUTTON_B + BUTTON_SELECT + BUTTON_START) {
        updateAudio2(); /* @reset */
        gameMode = 0;
        return 0;
    }
    gameModeState++;
    return heldButtons_player1;
}

/*
 * gameModeState_startButtonHandling (state 7): Start during the demo goes to
 * the title screen. Start during play (except under the game over curtain)
 * pauses: the playfield is hidden, the music is muted and "PAUSE" is shown
 * until Start is pressed again.
 * Returns A, which is never 8 (the new gameModeState): the value left in A on
 * each path.
 */
static uint8_t gameModeState_startButtonHandling(void)
{
    uint8_t a;
    if (gameMode == 0x05 && newlyPressedButtons_player1 == BUTTON_START) {
        gameMode = 0x01;
        a = 0x01;
    } else if (renderMode != 0x03) { /* @checkIfInGame */
        a = renderMode;
    } else if (!(newlyPressedButtons_player1 & BUTTON_START)) {
        a = 0x00;
    } else if (player1_playState == 0x0A) { /* @startPressed */
        a = 0x0A;
    } else { /* @pause */
        musicStagingNoiseHi = 0x05;
        renderMode = 0x00;
        updateAudioAndWaitForNmi();
        wr(PPUMASK, 0x16); /* background off, sprites on */
        memset_page(0xFF, 0x02, 0x02);
        for (;;) { /* @pauseLoop */
            spriteXOffset = 0x70;
            spriteYOffset = 0x77;
            spriteIndexInOamContentLookup = 0x05;
            loadSpriteIntoOamStaging();
            if (newlyPressedButtons_player1 == BUTTON_START)
                break;
            updateAudioWaitForNmiAndResetOamStaging();
        }
        /* @resume */
        wr(PPUMASK, 0x1E);
        musicStagingNoiseHi = 0x00;
        player1_vramRow = 0x00;
        renderMode = 0x03;
        a = 0x03;
    }
    gameModeState++; /* @ret */
    return a;
}

/* gameModeState_vblankThenRunState2 (state 8): back to state 2. Returns
 * A = 2, which now equals gameModeState, so the main loop waits for the next
 * frame: this is where a frame of play ends. */
static uint8_t gameModeState_vblankThenRunState2(void)
{
    gameModeState = 0x02;
    /* noop_disabledVramRowIncr: an rts; the vramRow stepping it once did is in
     * copyPlayfieldRowToVRAM */
    return 0x02;
}

/* gameMode_playAndEndingHighScore: runs the current gameModeState and returns
 * the A it leaves. In a frame of play, states 2 to 8 run one after another. */
uint8_t gameMode_playAndEndingHighScore(void)
{
    switch_s_plus_2a(gameModeState, (uint16_t)(ROM_gameMode_playAndEndingHighScore + 5));
    switch (gameModeState) {
    case 0: return gameModeState_initGameBackground();
    case 1: return gameModeState_initGameState();
    case 2: return gameModeState_updateCountersAndNonPlayerState();
    case 3: return gameModeState_handleGameOver();
    case 4: return gameModeState_updatePlayer1();
    case 5: return gameModeState_updatePlayer2();
    case 6: return gameModeState_checkForResetKeyCombo();
    case 7: return gameModeState_startButtonHandling();
    case 8: return gameModeState_vblankThenRunState2();
    default:
        /* State 9 (the two-player game over) has no entry: the original jumps
         * through whatever bytes follow the table. Unreachable in one-player
         * games. */
        return 0;
    }
}

/* ------------------------------------------------------------------------ */
/* playState                                                                */
/* ------------------------------------------------------------------------ */

/* playState_unassignOrientationId (0): hides the piece. */
static void playState_unassignOrientationId(void) { currentPiece = hidden; }

/* playState_incrementPlayState (11) */
static void playState_incrementPlayState(void) { playState++; }

/* playState_noop (4 and 9) */
static void playState_noop(void) {}

/* playState_playerControlsActiveTetrimino (1): the player moves the piece,
 * rotates it, and it falls. */
static void playState_playerControlsActiveTetrimino(void)
{
    shift_tetrimino();
    rotate_tetrimino();
    drop_tetrimino();
}

/*
 * rotate_tetrimino: A rotates clockwise, B counterclockwise, using
 * rotationTable (two entries per orientation). A rotation that collides is
 * undone. originalY holds the orientation to restore, despite its name.
 */
static void rotate_tetrimino(void)
{
    originalY = currentPiece;
    uint8_t x = (uint8_t)(currentPiece << 1);
    if ((newlyPressedButtons & BUTTON_A) == BUTTON_A) {
        x++;
        currentPiece = romTable(ROM_rotationTable, x);
    } else if ((newlyPressedButtons & BUTTON_B) == BUTTON_B) { /* @aNotPressed */
        currentPiece = romTable(ROM_rotationTable, x);
    } else {
        return;
    }
    if (isPositionValid()) {
        currentPiece = originalY; /* @restoreOrientationID */
        return;
    }
    soundEffectSlot1Init = 0x05;
}

/*
 * drop_tetrimino: gravity and the down button. At the start of a game
 * (autorepeatY negative) the piece does not fall until the countdown in
 * autorepeatY ends or down is pressed. Holding down (alone on the d-pad)
 * drops a row every other frame and counts holdDownPoints. Otherwise the
 * piece falls when fallTimer reaches the level's framesPerDropTable entry (1
 * above level 28). A piece that cannot fall locks: playState 2.
 */
static void drop_tetrimino(void)
{
    int drop = 0;
    if (autorepeatY & 0x80) {
        if (!(newlyPressedButtons & BUTTON_DOWN)) {
            autorepeatY++; /* @incrementAutorepeatY */
            return;
        }
        autorepeatY = 0;
    }
    /* @notBeginningOfGame */
    if (autorepeatY) { /* @autorepeating */
        if ((heldButtons & BUTTON_DPAD) == BUTTON_DOWN) { /* @downPressed */
            autorepeatY++;
            if (autorepeatY >= 0x03) {
                autorepeatY = 0x01;
                holdDownPoints++;
                drop = 1;
            }
        } else {
            autorepeatY = 0;
            holdDownPoints = 0;
        }
    } else { /* @playing */
        if (!(heldButtons & (BUTTON_LEFT + BUTTON_RIGHT)) && (newlyPressedButtons & BUTTON_DPAD) == BUTTON_DOWN)
            autorepeatY = 0x01;
    }
    if (!drop) { /* @lookupDropSpeed */
        uint8_t a = 0x01;
        if (levelNumber < 0x1D)
            a = romTable(ROM_framesPerDropTable, levelNumber);
        dropSpeed = a;
        if (sign_set(fallTimer, dropSpeed))
            return;
    }
    /* @drop */
    fallTimer = 0;
    originalY = tetriminoY;
    tetriminoY++;
    if (!isPositionValid())
        return;
    tetriminoY = originalY;
    playState = 0x02;
    updatePlayfield();
}

/*
 * shift_tetrimino: left and right, with delayed auto shift. A fresh press
 * moves at once; holding moves again after DAS_RESET frames, then every
 * DAS_RESET - DAS_DELAY frames. Nothing moves while down is held. A move
 * into a wall or block is undone and charges the auto shift fully, so the
 * piece moves again as soon as there is room. originalY holds the old X.
 */
static void shift_tetrimino(void)
{
    originalY = tetriminoX;
    if (heldButtons & BUTTON_DOWN)
        return;
    if (!(newlyPressedButtons & (BUTTON_LEFT + BUTTON_RIGHT))) {
        if (!(heldButtons & (BUTTON_LEFT + BUTTON_RIGHT)))
            return;
        autorepeatX++;
        if (sign_set(autorepeatX, DAS_RESET))
            return;
        autorepeatX = DAS_DELAY;
    } else {
        autorepeatX = 0; /* @resetAutorepeatX */
    }
    /* @buttonHeldDown */
    if (heldButtons & BUTTON_RIGHT) {
        tetriminoX++;
    } else if (heldButtons & BUTTON_LEFT) { /* @notPressingRight */
        tetriminoX--;
    } else {
        return;
    }
    if (isPositionValid()) {
        tetriminoX = originalY; /* @restoreX */
        autorepeatX = DAS_RESET;
        return;
    }
    soundEffectSlot1Init = 0x03;
}

/*
 * isPositionValid: checks the four squares of currentPiece at tetriminoX,
 * tetriminoY: each must be inside the playfield (rows from -2, which is above
 * the top, to 19; columns 0 to 9) and on an empty square (any tile from
 * tileEmpty up). Returns 0 if the piece fits, $FF if not, and leaves the same
 * in generalCounter. Uses selectingLevelOrHeight and generalCounter2-4 as
 * scratch.
 */
uint8_t isPositionValid(void)
{
    uint8_t c = 0;
    generalCounter = (uint8_t)(tetriminoY << 1);
    uint8_t a = adc8((uint8_t)(tetriminoY << 3), generalCounter, &c); /* Y * 10 */
    generalCounter = adc8(a, tetriminoX, &c);                       /* ... + X */
    generalCounter2 = (uint8_t)(currentPiece << 2);
    uint8_t x = (uint8_t)((currentPiece << 3) + generalCounter2); /* currentPiece * 12 */
    generalCounter3 = 0x04;
    do { /* @checkSquare */
        uint8_t dy = romTable(ROM_orientationTable, x);
        c = 0;
        a = adc8(dy, tetriminoY, &c);
        a = adc8(a, 0x02, &c);
        if (a >= 0x16)
            goto invalid;
        generalCounter4 = (uint8_t)(dy << 1);
        c = 0;
        a = adc8((uint8_t)(dy << 3), generalCounter4, &c);
        selectingLevelOrHeight = (uint8_t)(a + generalCounter);
        x = (uint8_t)(x + 2);
        uint8_t dx = romTable(ROM_orientationTable, x);
        uint8_t y = (uint8_t)(dx + selectingLevelOrHeight);
        if (PF(y) < tileEmpty)
            goto invalid;
        if ((uint8_t)(dx + tetriminoX) >= 0x0A)
            goto invalid;
        x++;
    } while (--generalCounter3);
    generalCounter = 0x00;
    return 0x00;
invalid:
    generalCounter = 0xFF;
    return 0xFF;
}

/*
 * playState_lockTetrimino (2): the piece has landed. If it overlaps blocks
 * (it was dealt where there is no room) the game is over: the curtain falls
 * (playState $0A). Otherwise, once the screen has caught up (vramRow = $20),
 * writes its four squares into the playfield and checks for lines.
 */
static void playState_lockTetrimino(void)
{
    if (isPositionValid()) {
        soundEffectSlot0Init = 0x02;
        playState = 0x0A;
        curtainRow = 0xF0;
        updateAudio2();
        return;
    }
    /* @notGameOver */
    if (sign_set(vramRow, 0x20))
        return;
    uint8_t c = 0;
    generalCounter = (uint8_t)(tetriminoY << 1);
    uint8_t a = adc8((uint8_t)(tetriminoY << 3), generalCounter, &c);
    generalCounter = adc8(a, tetriminoX, &c);
    generalCounter2 = (uint8_t)(currentPiece << 2);
    uint8_t x = (uint8_t)((currentPiece << 3) + generalCounter2);
    generalCounter3 = 0x04;
    do { /* @lockSquare */
        uint8_t dy = romTable(ROM_orientationTable, x);
        generalCounter4 = (uint8_t)(dy << 1);
        selectingLevelOrHeight = (uint8_t)((uint8_t)((uint8_t)(dy << 3) + generalCounter4) + generalCounter);
        x++;
        generalCounter5 = romTable(ROM_orientationTable, x); /* the tile */
        x++;
        uint8_t y = (uint8_t)(romTable(ROM_orientationTable, x) + selectingLevelOrHeight);
        PF(y) = generalCounter5;
        x++;
    } while (--generalCounter3);
    lineIndex = 0;
    updatePlayfield();
    updateMusicSpeed();
    playState++;
}

/*
 * playState_updateGameOverCurtain ($0A): every fourth frame, fills one more
 * playfield row with the curtain tile, from the top (curtainRow starts at
 * -16, which delays it). When all 20 rows are down: with 30,000 points or
 * more the ending plays; otherwise the game waits for Start. Then playState
 * 0 ends the game.
 */
static void playState_updateGameOverCurtain(void)
{
    if (curtainRow != 0x14) {
        if (frameCounter[0] & 0x03)
            return;
        if (!(curtainRow & 0x80)) {
            uint8_t y = romTable(ROM_multBy10Table, curtainRow);
            generalCounter3 = 0;
            currentPiece = hidden;
            do { /* @drawCurtainRow */
                PF(y) = 0x4F;
                y++;
                generalCounter3++;
            } while (generalCounter3 != 0x0A);
            vramRow = curtainRow;
        }
        curtainRow++; /* @incrementCurtainRow */
        return;
    }
    /* @curtainFinished */
    if (numberOfPlayers != 0x02) {
        if (player1_score[2] >= 0x03) {
            sleep_for_a_vblanks(ENDING_SLEEP_TIME_1);
            endingAnimation();
        } else if (newlyPressedButtons_player1 != BUTTON_START) { /* @checkForStartButton */
            return;
        }
    }
    /* @exitGame */
    playState = 0;
    newlyPressedButtons_player1 = 0;
}

/*
 * playState_checkForCompletedRows (3): runs once for each of the four rows
 * the piece can cover (lineIndex 0-3, from two rows above tetriminoY). A
 * full row is recorded in completedRow and removed at once: everything above
 * it moves down a row and the top row is cleared (the screen shows the
 * clearing animation over the old picture meanwhile). After the fourth row,
 * sends garbage (two-player), and moves to the animation (playState 4), or
 * straight to 5 if no line was made.
 */
static void playState_checkForCompletedRows(void)
{
    if (sign_set(vramRow, 0x20))
        return;
    /* @updatePlayfieldComplete */
    uint8_t a = (uint8_t)(tetriminoY - 0x02);
    if (a & 0x80)
        a = 0x00;
    a = (uint8_t)(a + lineIndex); /* @yInRange */
    generalCounter2 = a;
    generalCounter = (uint8_t)(a << 1);
    generalCounter = (uint8_t)((uint8_t)(a << 3) + generalCounter);
    uint8_t y = generalCounter;
    int complete = 1;
    for (int x = 0x0A; x; x--) { /* @checkIfRowComplete */
        if (PF(y) == tileEmpty) {
            complete = 0;
            break;
        }
        y++;
    }
    if (complete) {
        soundEffectSlot1Init = 0x0A;
        completedLines++;
        completedRow[lineIndex] = generalCounter2;
        y = (uint8_t)(generalCounter - 1);
        do { /* @movePlayfieldDownOneRow: the original borrows playfieldAddr's low byte as the +10 */
            uint8_t v = PF(y);
            playfieldAddr[0] = 0x0A;
            R(ptr16(A_playfieldAddr) + y) = v;
            playfieldAddr[0] = 0x00;
            y--;
        } while (y != 0xFF);
        for (y = 0; y != 0x0A; y++) /* @clearRowTopRow */
            PF(y) = tileEmpty;
        currentPiece = hidden;
    } else { /* @rowNotComplete */
        completedRow[lineIndex] = 0;
    }
    /* @incrementLineIndex */
    lineIndex++;
    if (sign_set(lineIndex, 0x04))
        return;
    pendingGarbageInactivePlayer = (uint8_t)(romTable(ROM_garbageLines, completedLines) + pendingGarbageInactivePlayer);
    vramRow = 0;
    rowY = 0;
    if (completedLines == 0x04)
        soundEffectSlot1Init = 0x04;
    playState++; /* @skipTetrisSoundEffect */
    if (completedLines)
        return;
    playState++;
    soundEffectSlot1Init = 0x07;
}

/* playState_receiveGarbage (7): two-player only. Pushes the playfield up by
 * the pending garbage and fills the bottom rows with blocks, leaving a hole
 * at garbageHole. */
static void playState_receiveGarbage(void)
{
    if (numberOfPlayers != 0x01 && pendingGarbage) {
        if (sign_set(vramRow, 0x20))
            return; /* @delay */
        uint8_t y = 0;
        generalCounter2 = romTable(ROM_multBy10Table, pendingGarbage);
        generalCounter = 0;
        do { /* @shiftPlayfieldUp */
            uint8_t v = PF(generalCounter2);
            y = generalCounter;
            PF(y) = v;
            generalCounter++;
            generalCounter2++;
        } while (generalCounter2 != 0xC8);
        y++;
        uint8_t x = 0;
        do { /* @fillGarbage */
            PF(y) = (x == garbageHole) ? 0xFF : 0x78;
            x++;
            if (x == 0x0A)
                x = 0;
            y++;
        } while (y != 0xC8);
        pendingGarbage = 0;
        vramRow = 0;
    }
    playState++; /* @ret */
}

/* incrementLines: adds one line (BCD), and goes up a level when the tens of
 * lines pass the level number (levelNumber < lines / 10). */
static void incrementLines(void)
{
    lines[0]++;
    if (!sign_set(lines[0] & 0x0F, 0x0A)) {
        lines[0] = (uint8_t)(lines[0] + 0x06);
        if ((lines[0] & 0xF0) >= 0xA0) {
            lines[0] &= 0x0F;
            lines[1]++;
        }
    }
    /* L9BC7 */
    if (lines[0] & 0x0F)
        return;
    /* L9BD0: generalCounter = lines >> 4, the tens of lines in BCD */
    generalCounter2 = lines[1];
    generalCounter = lines[0];
    for (int i = 0; i < 4; i++) {
        uint8_t c = generalCounter2 & 1;
        generalCounter2 >>= 1;
        generalCounter = (uint8_t)((generalCounter >> 1) | (c << 7));
    }
    if (!sign_set(levelNumber, generalCounter))
        return;
    /* @incrementLevel */
    levelNumber++;
    soundEffectSlot1Init = 0x06;
    outOfDateRenderFlags |= RENDER_LEVEL;
}

/*
 * playState_updateLinesAndStatistics (5): after the lines have gone: counts
 * the clear by type (single to Tetris), adds the lines (A-type) or takes
 * them off the 25 still to go (B-type), raises the level, and scores.
 */
static void playState_updateLinesAndStatistics(void)
{
    updateMusicSpeed();
    if (!completedLines) {
        addHoldDownPoints();
        return;
    }
    /* @linesCleared */
    uint8_t x = (uint8_t)(completedLines - 1);
    lineClearStatsByType[x]++;
    if (!sign_set(lineClearStatsByType[x] & 0x0F, 0x0A))
        lineClearStatsByType[x] = (uint8_t)(lineClearStatsByType[x] + 0x06);
    /* @noCarry */
    outOfDateRenderFlags |= RENDER_LINES;
    if (gameType) {
        generalCounter = completedLines;
        lines[0] = (uint8_t)(lines[0] - generalCounter);
        if (lines[0] & 0x80) {
            lines[0] = 0;
        } else if (!sign_set(lines[0] & 0x0F, 0x0A)) { /* @checkForBorrow */
            lines[0] = (uint8_t)(lines[0] - 0x06);
        }
        addHoldDownPoints();
        return;
    }
    /* @gameTypeA */
    x = completedLines;
    do
        incrementLines();
    while (--x); /* L9BFB */
    addHoldDownPoints();
}

/* addHoldDownPoints: one point for each row dropped by holding down, less
 * one, if at least two. The BCD fixup only covers the low byte's digits and
 * a carry into the next byte. */
static void addHoldDownPoints(void)
{
    if (!sign_set(holdDownPoints, 0x02)) {
        score[0]--;
        uint8_t a = (uint8_t)(holdDownPoints + score[0]);
        score[0] = a;
        if ((a & 0x0F) >= 0x0A)
            score[0] = (uint8_t)(score[0] + 0x06);
        if ((score[0] & 0xF0) >= 0xA0) { /* L9C18 */
            score[0] = (uint8_t)((score[0] & 0xF0) + 0x60);
            score[1]++;
        }
        outOfDateRenderFlags |= RENDER_SCORE; /* L9C27 */
    }
    addLineClearPoints();
}

/*
 * addLineClearPoints: adds pointsTable[completedLines] (BCD: 0, 40, 100, 300,
 * 1200) once per level, plus once: levelNumber + 1 times. The score stops at
 * 999999. Then clears completedLines and moves to the next playState.
 */
static void addLineClearPoints(void)
{
    holdDownPoints = 0;
    generalCounter = (uint8_t)(levelNumber + 1);
    do { /* L9C37 */
        uint8_t x = (uint8_t)(completedLines << 1);
        uint8_t a = (uint8_t)(romTable(ROM_pointsTable, x) + score[0]);
        score[0] = a;
        if (a >= 0xA0) {
            score[0] = (uint8_t)(a + 0x60);
            score[1]++;
        }
        x++; /* L9C4E */
        score[1] = (uint8_t)(romTable(ROM_pointsTable, x) + score[1]);
        if ((score[1] & 0x0F) >= 0x0A)
            score[1] = (uint8_t)(score[1] + 0x06);
        if ((score[1] & 0xF0) >= 0xA0) { /* L9C64 */
            score[1] = (uint8_t)(score[1] + 0x60);
            score[2]++;
        }
        if ((score[2] & 0x0F) >= 0x0A) /* L9C75 */
            score[2] = (uint8_t)(score[2] + 0x06);
        if ((score[2] & 0xF0) >= 0xA0) { /* L9C84 */
            score[0] = 0x99;
            score[1] = 0x99;
            score[2] = 0x99;
        }
    } while (--generalCounter); /* L9C94 */
    outOfDateRenderFlags |= RENDER_SCORE;
    completedLines = 0;
    playState++;
}

/*
 * playState_bTypeGoalCheck (6): in B-type, when no lines are left to go,
 * shows the success message in the playfield, plays the ending and ends the
 * game.
 */
static void playState_bTypeGoalCheck(void)
{
    if (!gameType || lines[0]) {
        playState++; /* playState_bTypeGoalCheck_ret */
        return;
    }
    /* @gameOver */
    setMusicTrack(0x02);
    uint8_t y = 0x46;
    uint8_t a;
    for (uint8_t x = 0; (a = romTable(ROM_typebSuccessGraphic, x)) != 0x80; x++, y++)
        PF(y) = a; /* @copySuccessGraphic */
    player1_vramRow = 0; /* @graphicCopied */
    sleep_for_14_vblanks();
    renderMode = 0;
    sleep_for_a_vblanks(0x80);
    endingAnimation();
    playState = 0;
    gameModeState++;
}

/*
 * playState_spawnNextTetrimino (8): once the screen has caught up, the next
 * piece enters at the top in its spawn orientation, counts in the statistics,
 * and a new next piece is chosen. In two-player games the players share the
 * piece sequence, with a delay so that neither gets ahead.
 */
static void playState_spawnNextTetrimino(void)
{
    if (sign_set(vramRow, 0x20))
        return;
    if (numberOfPlayers != 0x01) {
        if (twoPlayerPieceDelayCounter == 0x00) {
            twoPlayerPieceDelayCounter++;
            twoPlayerPieceDelayPlayer = activePlayer;
            twoPlayerPieceDelayPiece = chooseNextTetrimino();
            return;
        }
        /* @twoPlayerWaiting */
        if (twoPlayerPieceDelayPlayer != activePlayer || twoPlayerPieceDelayCounter != 0x1C)
            return;
    }
    /* @spawnPiece */
    twoPlayerPieceDelayCounter = 0;
    fallTimer = 0;
    tetriminoY = 0;
    playState = 0x01;
    tetriminoX = 0x05;
    currentPiece = romTable(ROM_spawnOrientationFromOrientation, nextPiece);
    incrementPieceStat(currentPiece);
    if (numberOfPlayers != 0x01)
        nextPiece = twoPlayerPieceDelayPiece;
    else
        nextPiece = chooseNextTetrimino(); /* @onePlayerPieceSelection */
    autorepeatY = 0; /* @resetDownHold */
}

/*
 * chooseNextTetrimino: returns the next piece's spawn orientation. The demo
 * takes its pieces from demoTetriminoTypeTable. Otherwise pickRandomTetrimino:
 * a piece from the random seed plus a spawn counter, rerolled once (from a
 * fresh random number, and without the possibility of the same piece) if it
 * repeats the last one. spawnID holds the last piece.
 */
uint8_t chooseNextTetrimino(void)
{
    if (gameMode == 0x05) {
        uint8_t x = demoIndex++;
        uint8_t a = (uint8_t)((romTable(ROM_demoTetriminoTypeTable, x) >> 4) & 0x07);
        return romTable(ROM_spawnTable, a);
    }
    /* pickRandomTetrimino, @realStart */
    spawnCount++;
    uint8_t a = (uint8_t)((rng_seed[0] + spawnCount) & 0x07);
    if (a != 0x07) {
        a = romTable(ROM_spawnTable, a);
        if (a != spawnID) {
            spawnID = a; /* useNewSpawnID */
            return a;
        }
    }
    /* @invalidIndex */
    generateNextPseudorandomNumber(A_rng_seed, 2);
    a = (uint8_t)((rng_seed[0] & 0x07) + spawnID);
    while (a >= 0x07) /* L992A */
        a = (uint8_t)(a - 0x07);
    a = romTable(ROM_spawnTable, a); /* L9934 */
    spawnID = a;
    return a;
}

/* incrementPieceStat: adds one (BCD) to the count of the piece type of
 * orientation A, in statsByType, and marks the statistics for redrawing. */
void incrementPieceStat(uint8_t a)
{
    uint8_t x = (uint8_t)(romTable(ROM_tetriminoTypeFromOrientation, a) << 1);
    generalCounter = (uint8_t)(statsByType[x] + 1);
    if (!sign_set(generalCounter & 0x0F, 0x0A)) {
        generalCounter = (uint8_t)(generalCounter + 0x06);
        if (generalCounter >= 0xA0) {
            generalCounter = (uint8_t)(generalCounter + 0x60);
            statsByType[x + 1]++;
        }
    }
    statsByType[x] = generalCounter; /* L9996 */
    outOfDateRenderFlags |= RENDER_STATS;
}

/* updatePlayfield: the playfield has changed from two rows above the piece
 * down, so restart the screen copy there if it is further along. */
void updatePlayfield(void)
{
    uint8_t a = (uint8_t)(tetriminoY - 2);
    if (a & 0x80)
        a = 0x00;
    if (sign_set(a, vramRow)) /* @rowInRange */
        vramRow = a;
}

/* updateMusicSpeed: the music speeds up while any block is in row 5 or
 * above (allegro) and slows down again when the stack drops below it. */
void updateMusicSpeed(void)
{
    uint8_t y = romTable(ROM_multBy10Table, 0x05);
    for (uint8_t x = 0x0A; x; x--) { /* @checkForBlockInRow */
        if (PF(y) != tileEmpty) {
            if (allegro) /* @foundBlockInRow */
                return;
            allegro = 0xFF;
            setMusicTrack(romTable(ROM_musicSelectionTable, (uint8_t)(musicType + 0x04)));
            return;
        }
        y++;
    }
    if (!allegro)
        return;
    allegro = 0;
    setMusicTrack(romTable(ROM_musicSelectionTable, musicType));
}

/* ------------------------------------------------------------------------ */
/* Sprites                                                                  */
/* ------------------------------------------------------------------------ */

/*
 * stageSpriteForCurrentPiece: adds the falling piece's four squares to
 * oamStaging. orientationTable gives each square's row offset, tile and
 * column offset; squares above the playfield's top are hidden.
 */
void stageSpriteForCurrentPiece(void)
{
    uint8_t c, a;
    c = (tetriminoX >> 5) & 1; /* the carry left by the third asl */
    generalCounter3 = adc8((uint8_t)(tetriminoX << 3), 0x60, &c); /* x of the center square */
    if (numberOfPlayers != 0x01) {
        c = 1;
        generalCounter3 = sbc8(generalCounter3, 0x40, &c); /* player 1's field is to the left */
        if (activePlayer != 0x01)
            generalCounter3 = adc8(generalCounter3, 0x6F, &c); /* and player 2's to the right */
    }
    /* @calculateYPixel */
    c = 0;
    a = rol8(tetriminoY, &c);
    a = rol8(a, &c);
    a = rol8(a, &c);
    generalCounter4 = adc8(a, 0x2F, &c); /* y of the center square */
    generalCounter5 = currentPiece;
    c = 0;
    a = rol8(generalCounter5, &c);
    a = rol8(a, &c);
    generalCounter = a;
    a = rol8(a, &c);
    uint8_t x = adc8(a, generalCounter, &c); /* currentPiece * 12 */
    uint8_t y = oamStagingLength;
    generalCounter2 = 0x04;
    do { /* @stageMino */
        a = (uint8_t)(romTable(ROM_orientationTable, x) << 3);
        a = (uint8_t)(a + generalCounter4);
        oamStaging[y] = a; /* y */
        originalY = a;
        oamStagingLength++;
        y++;
        x++;
        oamStaging[y] = romTable(ROM_orientationTable, x); /* tile */
        oamStagingLength++;
        y++;
        x++;
        oamStaging[y] = 0x02; /* palette 2, in front */
        if (originalY < 0x2F) { /* above the screen area the playfield uses */
            oamStagingLength++;
            y--;
            oamStaging[y] = 0xFF;
            y = (uint8_t)(y + 2);
            oamStaging[y] = 0x00;
        } else { /* @validYCoordinate */
            oamStagingLength++;
            y++;
            oamStaging[y] = (uint8_t)((uint8_t)(romTable(ROM_orientationTable, x) << 3) + generalCounter3);
        }
        oamStagingLength++; /* @finishLoop */
        y++;
        x++;
    } while (--generalCounter2);
}

/* stageSpriteForNextPiece: the next piece preview, unless turned off with
 * Select. Returns A: displayNextPiece when off, else $FF from
 * loadSpriteIntoOamStaging. */
uint8_t stageSpriteForNextPiece(void)
{
    if (displayNextPiece)
        return displayNextPiece;
    spriteXOffset = 0xC8;
    spriteYOffset = 0x77;
    spriteIndexInOamContentLookup = romTable(ROM_orientationToSpriteTable, nextPiece);
    return loadSpriteIntoOamStaging();
}

/*
 * loadSpriteIntoOamStaging: appends sprite spriteIndexInOamContentLookup to
 * oamStaging at spriteXOffset, spriteYOffset. oamContentLookup points to
 * each sprite's list of four-byte OAM entries (y, tile, attributes, x,
 * relative to the offsets), which ends with $FF. Points generalCounter and
 * generalCounter2 at the list. Returns A = $FF.
 */
uint8_t loadSpriteIntoOamStaging(void)
{
    uint8_t x = (uint8_t)(spriteIndexInOamContentLookup << 1);
    generalCounter = romTable(ROM_oamContentLookup, x);
    generalCounter2 = romTable(ROM_oamContentLookup, (uint8_t)(x + 1));
    x = oamStagingLength;
    uint16_t list = ptr16(A_generalCounter);
    uint8_t y = 0;
    for (;;) { /* @whileNotFF */
        uint8_t a = peek((uint16_t)(list + y));
        if (a == 0xFF)
            return 0xFF;
        oamStaging[x++] = (uint8_t)(a + spriteYOffset);
        y++;
        oamStaging[x++] = peek((uint16_t)(list + y));
        y++;
        oamStaging[x++] = peek((uint16_t)(list + y));
        y++;
        oamStaging[x++] = (uint8_t)(peek((uint16_t)(list + y)) + spriteXOffset);
        y++;
        oamStagingLength = (uint8_t)(oamStagingLength + 0x04);
    }
}
