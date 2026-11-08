/*
 * System code: power-on and reset, the frame (tetris_tick and the NMI), the
 * main loop and its jump table, the random number generator, the
 * controllers, and the waits.
 */
#include <string.h>

#include "game.h"

struct tetris *T;
uint8_t regA;

/* A resume point that isn't one of the routine's: a bug (or a bad snapshot). */
void pt_bad_resume(int routine) { (void)routine; }

const pt_fn pt_routines[PT_COUNT] = {
    [PT_reset] = reset,
    [PT_gameMode_legalScreen] = gameMode_legalScreen,
    [PT_gameMode_titleScreen] = gameMode_titleScreen,
    [PT_gameMode_gameTypeMenu] = gameMode_gameTypeMenu,
    [PT_gameMode_levelMenu] = gameMode_levelMenu,
    [PT_gameMode_playAndEndingHighScore_jmp] = gameMode_playAndEndingHighScore_jmp,
    [PT_gameModeState_initGameBackground] = gameModeState_initGameBackground,
    [PT_gameModeState_initGameState] = gameModeState_initGameState,
    [PT_initPlayfieldIfTypeB] = initPlayfieldIfTypeB,
    [PT_gameModeState_handleGameOver] = gameModeState_handleGameOver,
    [PT_handleHighScoreIfNecessary] = handleHighScoreIfNecessary,
    [PT_gameModeState_updatePlayer1] = gameModeState_updatePlayer1,
    [PT_gameModeState_updatePlayer2] = gameModeState_updatePlayer2,
    [PT_playState_updateGameOverCurtain] = playState_updateGameOverCurtain,
    [PT_playState_bTypeGoalCheck] = playState_bTypeGoalCheck,
    [PT_gameModeState_startButtonHandling] = gameModeState_startButtonHandling,
    [PT_sleep_for_14_vblanks] = sleep_for_14_vblanks,
    [PT_sleep_for_a_vblanks] = sleep_for_a_vblanks,
    [PT_endingAnimation] = endingAnimation,
    [PT_render_endingUnskippable] = render_endingUnskippable,
};

/* ------------------------------------------------------------------------ */
/* The frame                                                                */
/* ------------------------------------------------------------------------ */

/* CPU cycles in an NTSC frame (341 * 262 / 3). */
#define FRAME_CYCLES 29781
/* Roughly where in the frame the game resumes after the NMI. */
#define NMI_CYCLES 2500

void tetris_init(struct tetris *t)
{
    memset(t, 0, sizeof *t);
    T = t;
    screen_clearPlayfields();
}

/*
 * One frame. The NES runs the NMI at the start of the vertical blank (if the
 * game has it turned on), and the main loop, which was waiting for it, carries
 * on until it waits again. A frame in which the game is still busy with work
 * from the one before (drawing a new screen with NMIs off) does nothing.
 */
void tetris_tick(struct tetris *t, uint8_t pad1, uint8_t pad2)
{
    T = t;
    t->ctl.pad[0] = pad1;
    t->ctl.pad[1] = pad2;
    if (t->ctl.framesBehind) {
        t->ctl.framesBehind--;
        return;
    }
    if (nmiEnabled())
        nmi();
    else if (t->ctl.waiting == 1)
        return; /* waiting for an NMI that can't come: the original hangs too */
    t->ctl.cycles = NMI_CYCLES;
    t->ctl.waiting = 0;
    reset(); /* continues where the game waited */
}

/*
 * busy: the original spends `cycles` here on work this port leaves out
 * (copying a screen to the PPU). With NMIs off that can take the game past
 * the next vertical blank, and the frames it misses are frames the decomp
 * must sit out too, so it counts them in framesBehind.
 */
void busy(uint32_t cycles)
{
    T->ctl.cycles += cycles;
    while (T->ctl.cycles >= FRAME_CYCLES) {
        T->ctl.cycles -= FRAME_CYCLES;
        if (!nmiEnabled())
            T->ctl.framesBehind++;
    }
}

/* ------------------------------------------------------------------------ */
/* Reset and boot                                                           */
/* ------------------------------------------------------------------------ */

/*
 * reset: the reset vector, then initRam and initRamContinued, then the main
 * loop, which never ends: on the NES this is the bottom of the stack.
 *
 * reset turns the PPU off and waits for two vertical blanks so that the PPU
 * has warmed up, then sets up the mapper (left out here).
 *
 * initRamContinued clears $0000-$06FF (everything but the high score table),
 * then checks initMagic, five bytes that only a previous run of the game can
 * have left in RAM. If they are missing this is a cold boot, so the high score
 * table gets its defaults; after a press of Reset the scores survive. Then it
 * seeds the random number generator, turns NMIs on, clears the four
 * nametables (with NMIs off again) and both playfields, and starts the main
 * loop on the legal screen.
 *
 * @mainLoop runs the handler for the current game mode, over and over. A
 * handler that has finished its work for this frame returns with A equal to
 * gameModeState, and the loop then waits for the next frame; one that returns
 * anything else is run again at once. (Most handlers return a value that just
 * happens to be left in A, so this test is subtle: see the gameModeState
 * handlers.) The loop also ends the demo when its recorded input runs out.
 */
int reset(void)
{
    PT_BEGIN(PT_reset);
    PT_WAIT_VBLANK(1); /* @vsyncWait1 */
    PT_WAIT_VBLANK(2); /* @vsyncWait2 */

    /* initRamContinued: @zeroOutPages, pages 6 down to 0. The loop leaves
     * tmp1 = 0 and tmp2 = $FF. */
    memset(T->ram.b, 0, 0x700);
    tmp2 = 0xFF;
    busy(0x700 * 13);

    if (!(initMagic[0] == 0x12 && initMagic[1] == 0x34 && initMagic[2] == 0x56 && initMagic[3] == 0x78 &&
          initMagic[4] == 0x9A)) {
        /* @initHighScoreTable: copies defaultHighScoresTable (names, scores and
         * levels, which follow each other in RAM) up to its $FF end marker */
        for (uint8_t x = 0; defaultHighScoresTable[x] != 0xFF; x++)
            R(A_highScoreNames + x) = defaultHighScoresTable[x];
        initMagic[0] = 0x12; /* @continueColdBootInit */
        initMagic[1] = 0x34;
        initMagic[2] = 0x56;
        initMagic[3] = 0x78;
        initMagic[4] = 0x9A;
    }

    /* @continueWarmBootInit */
    rng_seed[0] = 0x89;
    rng_seed[1] = 0x88;
    ppuScrollX = 0;
    ppuScrollY = 0;
    currentPpuCtrl = 0x90; /* NMI on, background tiles from $1000 */
    /* LE006 and updateAudio2 set up the audio engine. Values left on the stack
     * page by this point; nothing reads them: */
    stack[0] = 0xC0;
    stack[1] = 0x80;
    stack[3] = 0x35;
    stack[4] = 0xAC;
    WAIT_DISABLE_RENDERING(0x80F9);
    disableNmi();
    busy(4 * (0x400 * 9 + 0x40 * 8)); /* LAA82 x4: clear the nametables */
    memset_page(tileEmpty, 0x04, 0x05); /* playfield and playfieldForSecondPlayer */
    waitForVBlankAndEnableNmi();
    WAIT_RESET_OAM(0x811F);
    WAIT_ENABLE_RENDERING(0x8122);
    WAIT_RESET_OAM(0x8125);
    unused_0E = 0x0E;
    gameModeState = 0;
    gameMode = 0;
    numberOfPlayers = 1;
    frameCounter[1] = 0;

    for (;;) { /* @mainLoop */
        PT_CALL(0x813A, branchOnGameMode());
        if (regA == gameModeState)
            WAIT_RESET_OAM(0x8141);
        /* @checkForDemoDataExhaustion */
        if (gameMode == 0x05 && demoButtonsAddr[1] == (ROM_demoTetriminoTypeTable >> 8)) {
            demoButtonsAddr[1] = ROM_demoButtonsTable >> 8;
            frameCounter[1] = 0;
            gameMode = 1;
        }
    }
    PT_END;
}

/* ------------------------------------------------------------------------ */
/* NMI and dispatch                                                         */
/* ------------------------------------------------------------------------ */

/*
 * nmi: runs at the start of every vertical blank while NMIs are on. On the
 * NES it copies what the main loop has prepared to the PPU; here render()
 * keeps the parts of that which the game depends on. It counts down
 * sleepCounter (stopping at 0), counts frames, steps the random number
 * generator, tells the main loop that a frame has passed
 * (verticalBlankingInterval) and reads the controllers.
 */
void nmi(void)
{
    oamStagingLength = 0;
    render();
    sleepCounter--;
    if (sleepCounter == 0xFF)
        sleepCounter++;
    uint16_t f = (uint16_t)(frameCounter[0] | (frameCounter[1] << 8)) + 1;
    frameCounter[0] = (uint8_t)f;
    frameCounter[1] = (uint8_t)(f >> 8);
    generateNextPseudorandomNumber(A_rng_seed, 2);
    ppuScrollX = 0;
    ppuScrollY = 0;
    verticalBlankingInterval = 1;
    pollControllerButtons();
}

/*
 * Runs a jump table entry that may wait: the first time, `routine`, chosen by
 * the caller from the table index; after a wait, the routine chosen then
 * (the index may have changed meanwhile, as when the NMI ends the demo).
 * switch_s_plus_2a, the original's dispatcher, leaves the entry's address in
 * tmp1/tmp2, which is written here too.
 */
static int dispatch(int slot, int routine, uint16_t entryAddress)
{
    uint8_t *chosen = &T->ctl.dispatched[slot];
    if (!*chosen) {
        *chosen = (uint8_t)(routine + 1);
        tmp1 = (uint8_t)entryAddress;
        tmp2 = (uint8_t)(entryAddress >> 8);
    }
    int status = pt_routines[*chosen - 1]();
    if (status == PT_DONE)
        *chosen = 0;
    return status;
}

/* Routine addresses, for tmp1/tmp2 after a dispatch. */
#define ADDR_gameMode_legalScreen 0x8200
#define ADDR_gameMode_titleScreen 0x824F
#define ADDR_gameMode_gameTypeMenu 0x82D1
#define ADDR_gameMode_levelMenu 0x83D7
#define ADDR_gameMode_playAndEndingHighScore_jmp 0x815D
#define ADDR_gameMode_startDemo 0x9DF6

/*
 * branchOnGameMode: runs the handler for gameMode (0 legal screen, 1 title,
 * 2 game type menu, 3 level menu, 4 play, 5 demo, 6 start demo). gameMode 6
 * (gameMode_startDemo) sets up the demo and jumps to the play handler.
 * Leaves in A what the handler leaves, which the main loop compares with
 * gameModeState.
 */
int branchOnGameMode(void)
{
    if (T->ctl.dispatched[DISPATCH_GAME_MODE])
        return dispatch(DISPATCH_GAME_MODE, 0, 0);
    switch (gameMode) {
    case 0: return dispatch(DISPATCH_GAME_MODE, PT_gameMode_legalScreen, ADDR_gameMode_legalScreen);
    case 1: return dispatch(DISPATCH_GAME_MODE, PT_gameMode_titleScreen, ADDR_gameMode_titleScreen);
    case 2: return dispatch(DISPATCH_GAME_MODE, PT_gameMode_gameTypeMenu, ADDR_gameMode_gameTypeMenu);
    case 3: return dispatch(DISPATCH_GAME_MODE, PT_gameMode_levelMenu, ADDR_gameMode_levelMenu);
    case 4:
    case 5:
        return dispatch(DISPATCH_GAME_MODE, PT_gameMode_playAndEndingHighScore_jmp,
                        ADDR_gameMode_playAndEndingHighScore_jmp);
    default:
        gameMode_startDemo();
        return dispatch(DISPATCH_GAME_MODE, PT_gameMode_playAndEndingHighScore_jmp, ADDR_gameMode_startDemo);
    }
}

/* ------------------------------------------------------------------------ */
/* Helpers                                                                  */
/* ------------------------------------------------------------------------ */

/*
 * generateNextPseudorandomNumber: steps a linear feedback shift register of
 * Y bytes at zero page address X. The new top bit is bit 1 of the first byte
 * XOR bit 1 of the second, and the whole register shifts right by one bit.
 * tmp1 is used as scratch.
 */
void generateNextPseudorandomNumber(uint8_t x, uint8_t y)
{
    tmp1 = R(x) & 0x02;
    uint8_t carry = ((R(x + 1) & 0x02) ^ tmp1) ? 1 : 0;
    do {
        uint8_t v = R(x);
        R(x) = (uint8_t)((v >> 1) | (carry << 7));
        carry = v & 1;
        x++;
    } while (--y);
}

/*
 * memset_page: fills RAM pages X to Y (inclusive) with A. It works from page Y
 * downward, so it leaves tmp1 = 0 and tmp2 = X - 1.
 */
void memset_page(uint8_t a, uint8_t x, uint8_t y)
{
    memset(&T->ram.b[(x << 8) & 0x7FF], a, (size_t)(y - x + 1) << 8);
    tmp1 = 0;
    tmp2 = (uint8_t)(x - 1);
    busy((uint32_t)(y - x + 1) * 256 * 11);
}

/* disableNmi: turns NMIs off, for work that must not be interrupted. */
void disableNmi(void) { currentPpuCtrl &= 0x7F; }

/* waitForVBlankAndEnableNmi: waits until the vertical blank flag reads clear
 * (reading it clears it, so this never waits long), then turns NMIs on. */
void waitForVBlankAndEnableNmi(void) { currentPpuCtrl |= 0x80; }

/* ------------------------------------------------------------------------ */
/* Controllers                                                              */
/* ------------------------------------------------------------------------ */

/*
 * pollController: on the NES, reads both controllers twice and keeps the
 * buttons seen both times (a guard against the DMC glitch corrupting a read),
 * with the Famicom's expansion port controllers counted as the players' own.
 * Here the buttons come from tetris_tick. Then diffOldAndNewButtons: works
 * out which are newly pressed (newlyPressedButtons_playerN) and which are
 * held (heldButtons_playerN).
 */
static void pollController(void)
{
    /* pollController_actualRead leaves the expansion port's (empty) bits in tmp1/tmp2 */
    tmp1 = 0;
    tmp2 = 0;
    generalCounter2 = T->ctl.pad[0];
    generalCounter3 = T->ctl.pad[1];
    for (int x = 1; x >= 0; x--) { /* diffOldAndNewButtons */
        uint8_t now = T->ctl.pad[x];
        R(A_newlyPressedButtons_player1 + x) = (now ^ R(A_heldButtons_player1 + x)) & now;
        R(A_heldButtons_player1 + x) = now;
    }
}

/* demoButtonsTable_indexIncr: steps demoButtonsAddr, a 16-bit pointer into the table. */
static void demoButtonsTable_indexIncr(void)
{
    uint16_t p = (uint16_t)(ptr16(A_demoButtonsAddr) + 1);
    demoButtonsAddr[0] = (uint8_t)p;
    demoButtonsAddr[1] = (uint8_t)(p >> 8);
}

/*
 * The byte demoButtonsAddr points at. It holds a CPU address, as the
 * original's does: normally into the demo's tables in ROM, but the demo's
 * first few frames (before initGameState points it at the table) read from
 * $0000, the start of RAM.
 */
static uint8_t demoButtonsByte(void)
{
    uint16_t a = ptr16(A_demoButtonsAddr);
    if (a < 0x2000)
        return R(a);
    if (a >= ROM_demoButtonsTable && a < ROM_demoButtonsTable + sizeof demoButtonsTable)
        return demoButtonsTable[a - ROM_demoButtonsTable];
    if (a >= ROM_demoTetriminoTypeTable && a < ROM_demoTetriminoTypeTable + sizeof demoTetriminoTypeTable)
        return demoTetriminoTypeTable[a - ROM_demoTetriminoTypeTable];
    return 0; /* elsewhere in ROM: never reached by the original */
}

/*
 * pollControllerButtons: called from the NMI to read the controllers. In the
 * demo the input comes from demoButtonsTable instead: pairs of (buttons held,
 * frames to hold them). Start in the demo goes back to the title screen.
 * (The original can also record a demo, when $D0 is $FF, which nothing in the
 * game sets: it writes the pairs over the table, which on a cartridge is ROM.
 * That is left out.)
 */
void pollControllerButtons(void)
{
    if (gameMode != 0x05) {
        pollController();
        return;
    }
    /* @demoGameMode */
    pollController();
    if (R(0xD0) == 0xFF)
        return; /* @recording */
    if (newlyPressedButtons_player1 == BUTTON_START) { /* @startButtonPressed */
        demoButtonsAddr[1] = ROM_demoButtonsTable >> 8;
        frameCounter[1] = 0;
        gameMode = 0x01;
        return;
    }
    if (demo_repeats) {
        demo_repeats--;
        newlyPressedButtons_player1 = 0; /* @moveInProgress */
    } else { /* @finishedMove */
        generalCounter = demoButtonsByte();
        demoButtonsTable_indexIncr();
        newlyPressedButtons_player1 = (demo_heldButtons ^ generalCounter) & generalCounter;
        demo_heldButtons = generalCounter;
        demo_repeats = demoButtonsByte();
        demoButtonsTable_indexIncr();
        if (demoButtonsAddr[1] == (ROM_demoTetriminoTypeTable >> 8))
            return;
    }
    heldButtons_player1 = demo_heldButtons; /* @holdButtons */
}
