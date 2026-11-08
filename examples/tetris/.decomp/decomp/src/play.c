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

/* A byte of the playfield being processed, (playfieldAddr),y. */
#define PF(y) R(ptr16(A_playfieldAddr) + (uint8_t)(y))

/* What copying the game's nametable and palette takes, roughly. */
#define GAME_SCREEN_CYCLES (1024 * 16 + 800 + 32 * 16 + 200)

static uint8_t gameMode_playAndEndingHighScore(void);
static void playState_unassignOrientationId(void);
static void playState_playerControlsActiveTetrimino(void);
static void playState_lockTetrimino(void);
static void playState_checkForCompletedRows(void);
static void playState_noop(void);
static void playState_updateLinesAndStatistics(void);
static void playState_receiveGarbage(void);
static void playState_spawnNextTetrimino(void);
static void playState_incrementPlayState(void);
static void shift_tetrimino(void);
static void rotate_tetrimino(void);
static void drop_tetrimino(void);
static void addHoldDownPoints(void);
static void addLineClearPoints(void);

/* ------------------------------------------------------------------------ */
/* gameModeState                                                            */
/* ------------------------------------------------------------------------ */

/* gameMode_playAndEndingHighScore_jmp: the entry for game modes 4, 5 and 6
 * in branchOnGameMode's table, a JSR to the real handler. */
int gameMode_playAndEndingHighScore_jmp(void)
{
    PT_BEGIN(PT_gameMode_playAndEndingHighScore_jmp);
    PT_CALL(0x815F, gameMode_playAndEndingHighScore());
    PT_END;
}

/*
 * gameModeState_initGameBackground (state 0): draws the playfield screen:
 * the game's tiles and palette, the nametable, the "A-TYPE" or "B-TYPE"
 * label, the top score for the game type and, for B-type, the height box.
 * Sets both players to playState 1 at their starting level.
 * Leaves A = player2_startLevel, from the last load.
 */
int gameModeState_initGameBackground(void)
{
    PT_BEGIN(PT_gameModeState_initGameBackground);
    WAIT_DISABLE_RENDERING(0x85F2);
    disableNmi();
    /* game_palette, game_nametable, the type, the top score; for B-type,
     * game_typeb_nametable_patch and the height */
    busy(GAME_SCREEN_CYCLES + 300 + (gameType ? 600 : 0));
    screen_clearPlayfields();
    /* gameModeState_initGameBackground_finish */
    waitForVBlankAndEnableNmi();
    WAIT_RESET_OAM(0x8698);
    WAIT_ENABLE_RENDERING(0x869B);
    WAIT_RESET_OAM(0x869E);
    player1_playState = 0x01;
    player2_playState = 0x01;
    player1_levelNumber = player1_startLevel;
    player2_levelNumber = player2_startLevel;
    gameModeState++;
    PT_RETURN(player2_startLevel);
    PT_END;
}

/*
 * gameModeState_initGameState (state 1): clears the playfield and the
 * statistics, puts both players' pieces at the top, deals the first piece
 * and the next one, sets B-type's 25 lines and garbage, and starts the music.
 * Leaves A from setMusicTrack.
 */
int gameModeState_initGameState(void)
{
    PT_BEGIN(PT_gameModeState_initGameState);
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
    demoButtonsAddr[0] = 0;
    spawnID = 0;
    demoButtonsAddr[1] = ROM_demoButtonsTable >> 8;
    renderMode = 0x03;
    player1_autorepeatY = INITIAL_AUTOREPEAT_Y;
    player2_autorepeatY = INITIAL_AUTOREPEAT_Y;
    {
        uint8_t a = chooseNextTetrimino();
        player1_currentPiece = a;
        player2_currentPiece = a;
        incrementPieceStat(a);
        generateNextPseudorandomNumber(A_rng_seed, 2);
        a = chooseNextTetrimino();
        nextPiece = a;
        twoPlayerPieceDelayPiece = a;
    }
    if (gameType) {
        player1_lines[0] = 0x25;
        player2_lines[0] = 0x25;
    }
    /* @skipTypeBInit */
    outOfDateRenderFlags = RENDER_STATS + RENDER_SCORE + RENDER_LEVEL + RENDER_LINES;
    WAIT_RESET_OAM(0x8767);
    PT_CALL(0x876A, initPlayfieldIfTypeB());
    setMusicTrack(musicType);
    gameModeState++;
    PT_RETURN(regA); /* from setMusicTrack */
    PT_END;
}

/* initPlayfieldIfTypeB: B-type starts with garbage: twelve rows of random
 * squares (each with at least one gap) are added a frame at a time, then
 * rows above the chosen height are cleared again. */
int initPlayfieldIfTypeB(void)
{
    PT_BEGIN(PT_initPlayfieldIfTypeB);
    if (!gameType)
        PT_RETURN(regA); /* endTypeBInit */
    /* initPlayfieldForTypeB */
    generalCounter = 0x0C;
    while (generalCounter) { /* typeBRows */
        generalCounter2 = (uint8_t)(0x14 - generalCounter); /* row */
        player1_vramRow = 0;
        player2_vramRow = 0;
        generalCounter3 = 0x09; /* column */
        for (;;) { /* typeBGarbageInRow */
            generateNextPseudorandomNumber(A_rng_seed, 2);
            generalCounter4 = rngTable[rng_seed[0] & 0x07]; /* a square, or blank */
            uint8_t y = (uint8_t)(multBy10Table[generalCounter2] + generalCounter3);
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
        playfield[(uint8_t)(multBy10Table[generalCounter2] + generalCounter5)] = tileEmpty;
        WAIT_RESET_OAM(0x8845);
        generalCounter--;
    }
    /* initCopyPlayfieldToPlayer2: copies $0401-$04C8 (not $0400) */
    for (uint8_t x = 0xC8; x; x--)
        playfieldForSecondPlayer[x] = playfield[x];
    /* Clear the playfield above the starting height (typeBBlankInitPlayer1/2). */
    uint8_t y = typeBBlankInitCountByHeightTable[player1_startHeight];
    do
        playfield[y] = tileEmpty;
    while (y-- != 0x00);
    y = typeBBlankInitCountByHeightTable[player2_startHeight];
    do
        playfieldForSecondPlayer[y] = tileEmpty;
    while (y-- != 0x00);
    PT_RETURN(y); /* $FF */
    PT_END;
}

/*
 * gameModeState_updateCountersAndNonPlayerState (state 2): counts the fall
 * timers and the two-player piece delay, and toggles the next piece preview
 * on Select. (It also selects the game's tiles.)
 * Returns A: 0, or the new displayNextPiece when Select was pressed.
 */
static uint8_t gameModeState_updateCountersAndNonPlayerState(void)
{
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
 * Leaves A: 3 (gameMode) after a game over, which equals gameModeState and
 * so makes the main loop wait a frame; 1 (numberOfPlayers) otherwise.
 */
int gameModeState_handleGameOver(void)
{
    PT_BEGIN(PT_gameModeState_handleGameOver);
    generalCounter2 = 0x05;
    if (player1_playState != 0x00) {
        if (numberOfPlayers == 0x01) {
            gameModeState++; /* @ret */
            PT_RETURN(numberOfPlayers);
        }
        generalCounter2 = 0x04;
        if (player2_playState != 0x00) {
            gameModeState++;
            PT_RETURN(player2_playState);
        }
    }
    /* @gameOver */
    if (numberOfPlayers != 0x01) {
        gameModeState = 0x09;
        PT_RETURN(0x09);
    }
    /* @onePlayerGameOver */
    renderMode = 0x03;
    if (numberOfPlayers == 0x01)
        PT_CALL(0x9CF0, handleHighScoreIfNecessary());
    /* @resetGameState */
    player1_playState = 0x01;
    player2_playState = 0x01;
    memset_page(tileEmpty, 0x04, 0x05);
    player1_vramRow = 0;
    player2_vramRow = 0;
    player1_playState = 0x01;
    player2_playState = 0x01;
    WAIT_RESET_OAM(0x9D0E);
    gameMode = 0x03;
    PT_RETURN(0x03);
    PT_END;
}

/* makePlayer1Active: copies player 1's block ($60) to the active player's ($40). */
static void makePlayer1Active(void)
{
    activePlayer = 0x01;
    playfieldAddr[1] = 0x04;
    newlyPressedButtons = newlyPressedButtons_player1;
    heldButtons = heldButtons_player1;
    for (int x = 0x1F; x >= 0; x--)
        R(A_tetriminoX + x) = R(A_player1_tetriminoX + x);
}

/* makePlayer2Active: copies player 2's block ($80) to the active player's ($40). */
static void makePlayer2Active(void)
{
    activePlayer = 0x02;
    playfieldAddr[1] = 0x05;
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

/* The entries of branchOnPlayStatePlayer1's jump table (player 2's differs
 * only in entry 1, a copy of the same routine), for tmp1/tmp2. */
static const uint16_t playStateTable[2][12] = {
    {0x9E2F, 0x81CF, 0x99A2, 0x9A6B, 0x9E39, 0x9B58, 0xA3F2, 0x9B03, 0x988E, 0x9E39, 0x9A11, 0x9E37},
    {0x9E2F, 0x81F6, 0x99A2, 0x9A6B, 0x9E39, 0x9B58, 0xA3F2, 0x9B03, 0x988E, 0x9E39, 0x9A11, 0x9E37},
};

/* branchOnPlayStatePlayer1 and branchOnPlayStatePlayer2: run the active
 * player's playState. States 6 (B-type's goal) and $0A (the curtain) can end
 * in an ending, which waits. */
static int branchOnPlayState(int player)
{
    uint8_t *chosen = &T->ctl.dispatched[DISPATCH_PLAY_STATE];
    if (!*chosen) {
        uint16_t entry = playState < 12 ? playStateTable[player][playState] : 0;
        tmp1 = (uint8_t)entry;
        tmp2 = (uint8_t)(entry >> 8);
        switch (playState) {
        case 0x00: playState_unassignOrientationId(); return PT_DONE;
        case 0x01: playState_playerControlsActiveTetrimino(); return PT_DONE;
        case 0x02: playState_lockTetrimino(); return PT_DONE;
        case 0x03: playState_checkForCompletedRows(); return PT_DONE;
        case 0x04: playState_noop(); return PT_DONE; /* the NMI animates the line clear */
        case 0x05: playState_updateLinesAndStatistics(); return PT_DONE;
        case 0x06: *chosen = PT_playState_bTypeGoalCheck + 1; break;
        case 0x07: playState_receiveGarbage(); return PT_DONE;
        case 0x08: playState_spawnNextTetrimino(); return PT_DONE;
        case 0x09: playState_noop(); return PT_DONE;
        case 0x0A: *chosen = PT_playState_updateGameOverCurtain + 1; break;
        case 0x0B: playState_incrementPlayState(); return PT_DONE;
        default: return PT_DONE; /* beyond the table: not reachable */
        }
    }
    int status = pt_routines[*chosen - 1]();
    if (status == PT_DONE)
        *chosen = 0;
    return status;
}

/* gameModeState_updatePlayer1 (state 4): runs player 1's playState (and
 * stages the piece and the next piece preview as sprites). Leaves A from
 * stageSpriteForNextPiece: displayNextPiece when the preview is off, else $FF. */
int gameModeState_updatePlayer1(void)
{
    PT_BEGIN(PT_gameModeState_updatePlayer1);
    makePlayer1Active();
    PT_CALL(0x8179, branchOnPlayState(0));
    /* stageSpriteForCurrentPiece */
    savePlayer1State();
    gameModeState++;
    PT_RETURN(displayNextPiece ? displayNextPiece : 0xFF); /* stageSpriteForNextPiece */
    PT_END;
}

/* gameModeState_updatePlayer2 (state 5): the same for player 2, in
 * two-player games. Leaves A = numberOfPlayers in a one-player game. */
int gameModeState_updatePlayer2(void)
{
    PT_BEGIN(PT_gameModeState_updatePlayer2);
    regA = numberOfPlayers;
    if (numberOfPlayers == 0x02) {
        makePlayer2Active();
        PT_CALL(0x8191, branchOnPlayState(1));
        /* stageSpriteForCurrentPiece */
        regA = savePlayer2State();
    }
    gameModeState++; /* @ret */
    PT_RETURN(regA);
    PT_END;
}

/* gameModeState_checkForResetKeyCombo (state 6): A+B+Select+Start held
 * returns to the legal screen. Returns A: heldButtons_player1, or 0 on reset. */
static uint8_t gameModeState_checkForResetKeyCombo(void)
{
    if (heldButtons_player1 == BUTTON_A + BUTTON_B + BUTTON_SELECT + BUTTON_START) {
        gameMode = 0; /* @reset (after updateAudio2) */
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
 * Leaves A, which is never 8 (the new gameModeState): the value left in A on
 * each path.
 */
int gameModeState_startButtonHandling(void)
{
    PT_BEGIN(PT_gameModeState_startButtonHandling);
    if (gameMode == 0x05 && newlyPressedButtons_player1 == BUTTON_START) {
        gameMode = 0x01;
        regA = 0x01;
    } else if (renderMode != 0x03) { /* @checkIfInGame */
        regA = renderMode;
    } else if (!(newlyPressedButtons_player1 & BUTTON_START)) {
        regA = 0x00;
    } else if (player1_playState == 0x0A) { /* @startPressed */
        regA = 0x0A;
    } else { /* @pause */
        renderMode = 0x00;
        PT_WAIT_NMI(0xA3B5); /* updateAudioAndWaitForNmi */
        T->screen.paused = 1; /* the background goes off; the "PAUSE" sprite stays */
        for (;;) { /* @pauseLoop */
            if (newlyPressedButtons_player1 == BUTTON_START)
                break;
            WAIT_RESET_OAM(0xA3DB);
        }
        /* @resume */
        T->screen.paused = 0;
        player1_vramRow = 0x00;
        renderMode = 0x03;
        regA = 0x03;
    }
    gameModeState++; /* @ret */
    PT_RETURN(regA);
    PT_END;
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

/* gameMode_playAndEndingHighScore's jump table, for tmp1/tmp2. */
static const uint16_t gameModeStateTable[9] = {0x85F0, 0x86DC, 0x8884, 0x9CBF, 0x8174, 0x8186, 0x9E16, 0xA37F, 0x9E27};

/* gameMode_playAndEndingHighScore: runs the current gameModeState and leaves
 * the A it leaves. In a frame of play, states 2 to 8 run one after another. */
static uint8_t gameMode_playAndEndingHighScore(void)
{
    uint8_t *chosen = &T->ctl.dispatched[DISPATCH_GAME_MODE_STATE];
    if (!*chosen) {
        uint16_t entry = gameModeState < 9 ? gameModeStateTable[gameModeState] : 0;
        tmp1 = (uint8_t)entry;
        tmp2 = (uint8_t)(entry >> 8);
        switch (gameModeState) {
        case 0: *chosen = PT_gameModeState_initGameBackground + 1; break;
        case 1: *chosen = PT_gameModeState_initGameState + 1; break;
        case 2: regA = gameModeState_updateCountersAndNonPlayerState(); return PT_DONE;
        case 3: *chosen = PT_gameModeState_handleGameOver + 1; break;
        case 4: *chosen = PT_gameModeState_updatePlayer1 + 1; break;
        case 5: *chosen = PT_gameModeState_updatePlayer2 + 1; break;
        case 6: regA = gameModeState_checkForResetKeyCombo(); return PT_DONE;
        case 7: *chosen = PT_gameModeState_startButtonHandling + 1; break;
        case 8: regA = gameModeState_vblankThenRunState2(); return PT_DONE;
        default:
            /* State 9 (the two-player game over) has no entry: the original jumps
             * through whatever bytes follow the table. Unreachable in one-player
             * games. */
            regA = 0;
            return PT_DONE;
        }
    }
    int status = pt_routines[*chosen - 1]();
    if (status == PT_DONE)
        *chosen = 0;
    return (uint8_t)status;
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
        currentPiece = rotationTable[x];
    } else if ((newlyPressedButtons & BUTTON_B) == BUTTON_B) { /* @aNotPressed */
        currentPiece = rotationTable[x];
    } else {
        return;
    }
    if (isPositionValid())
        currentPiece = originalY; /* @restoreOrientationID */
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
            a = framesPerDropTable[levelNumber];
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
    }
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
        uint8_t dy = orientationTable[x];
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
        uint8_t dx = orientationTable[x];
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
        playState = 0x0A;
        curtainRow = 0xF0;
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
        uint8_t dy = orientationTable[x];
        generalCounter4 = (uint8_t)(dy << 1);
        selectingLevelOrHeight = (uint8_t)((uint8_t)((uint8_t)(dy << 3) + generalCounter4) + generalCounter);
        x++;
        generalCounter5 = orientationTable[x]; /* the tile */
        x++;
        uint8_t y = (uint8_t)(orientationTable[x] + selectingLevelOrHeight);
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
int playState_updateGameOverCurtain(void)
{
    PT_BEGIN(PT_playState_updateGameOverCurtain);
    if (curtainRow != 0x14) {
        if (frameCounter[0] & 0x03)
            PT_RETURN(regA);
        if (!(curtainRow & 0x80)) {
            uint8_t y = multBy10Table[curtainRow];
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
        PT_RETURN(curtainRow);
    }
    /* @curtainFinished */
    if (numberOfPlayers != 0x02) {
        if (player1_score[2] >= 0x03) {
            sleepCounter = ENDING_SLEEP_TIME_1;
            PT_CALL(0x9A57, sleep_for_a_vblanks());
            PT_CALL(0x9A5A, endingAnimation());
        } else if (newlyPressedButtons_player1 != BUTTON_START) { /* @checkForStartButton */
            PT_RETURN(newlyPressedButtons_player1);
        }
    }
    /* @exitGame */
    playState = 0;
    newlyPressedButtons_player1 = 0;
    PT_RETURN(0);
    PT_END;
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
    pendingGarbageInactivePlayer = (uint8_t)(garbageLines[completedLines] + pendingGarbageInactivePlayer);
    vramRow = 0;
    rowY = 0;
    playState++; /* @skipTetrisSoundEffect */
    if (completedLines)
        return;
    playState++;
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
        generalCounter2 = multBy10Table[pendingGarbage];
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
        uint8_t a = (uint8_t)(pointsTable[x] + score[0]);
        score[0] = a;
        if (a >= 0xA0) {
            score[0] = (uint8_t)(a + 0x60);
            score[1]++;
        }
        x++; /* L9C4E */
        score[1] = (uint8_t)(pointsTable[x] + score[1]);
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
 * writes the success message into the playfield, plays the ending and ends
 * the game.
 */
int playState_bTypeGoalCheck(void)
{
    PT_BEGIN(PT_playState_bTypeGoalCheck);
    if (!gameType || lines[0]) {
        playState++; /* playState_bTypeGoalCheck_ret */
        PT_RETURN(playState);
    }
    /* @gameOver */
    setMusicTrack(0xFF);
    {
        uint8_t y = 0x46;
        for (uint8_t x = 0; typebSuccessGraphic[x] != 0x80; x++, y++)
            PF(y) = typebSuccessGraphic[x]; /* @copySuccessGraphic */
    }
    player1_vramRow = 0; /* @graphicCopied */
    PT_CALL(0xA417, sleep_for_14_vblanks());
    renderMode = 0;
    sleepCounter = 0x80;
    PT_CALL(0xA420, sleep_for_a_vblanks());
    PT_CALL(0xA423, endingAnimation());
    playState = 0;
    gameModeState++;
    PT_RETURN(gameModeState);
    PT_END;
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
    currentPiece = spawnOrientationFromOrientation[nextPiece];
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
        return spawnTable[(demoTetriminoTypeTable[x] >> 4) & 0x07];
    }
    /* pickRandomTetrimino, @realStart */
    spawnCount++;
    uint8_t a = (uint8_t)((rng_seed[0] + spawnCount) & 0x07);
    if (a != 0x07) {
        a = spawnTable[a];
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
    a = spawnTable[a]; /* L9934 */
    spawnID = a;
    return a;
}

/* incrementPieceStat: adds one (BCD) to the count of the piece type of
 * orientation A, in statsByType, and marks the statistics for redrawing. */
void incrementPieceStat(uint8_t a)
{
    uint8_t x = (uint8_t)(tetriminoTypeFromOrientation[a] << 1);
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
 * above (allegro) and slows down again when the stack drops below it. The
 * decomp has no music, but allegro is kept. */
void updateMusicSpeed(void)
{
    uint8_t y = multBy10Table[0x05];
    for (uint8_t x = 0x0A; x; x--) { /* @checkForBlockInRow */
        if (PF(y) != tileEmpty) {
            if (allegro) /* @foundBlockInRow */
                return;
            allegro = 0xFF;
            setMusicTrack(musicType + 0x04);
            return;
        }
        y++;
    }
    if (!allegro)
        return;
    allegro = 0;
    setMusicTrack(musicType);
}

/* sleep_for_14_vblanks: waits 20 frames, using the NMI's sleepCounter. */
int sleep_for_14_vblanks(void)
{
    PT_BEGIN(PT_sleep_for_14_vblanks);
    sleepCounter = 0x14;
    do
        WAIT_RESET_OAM(0xA453);
    while (sleepCounter);
    PT_RETURN(sleepCounter);
    PT_END;
}

/* sleep_for_a_vblanks: waits sleepCounter frames (the original takes the
 * count in A and stores it there first). */
int sleep_for_a_vblanks(void)
{
    PT_BEGIN(PT_sleep_for_a_vblanks);
    do
        WAIT_RESET_OAM(0xA45D);
    while (sleepCounter);
    PT_RETURN(sleepCounter);
    PT_END;
}
