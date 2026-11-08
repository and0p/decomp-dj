/*
 * Shared definitions for the decomp.
 *
 * Conventions:
 *
 * - Every routine is named after its label in the reference disassembly
 *   (CelestialAmber/TetrisNESDisasm) and sits in the file for its area.
 * - Game state lives in the game's RAM (T->ram), under the disassembly's names
 *   (ram_names.h), at the original addresses. C locals stand in for the 6502's
 *   registers. Scratch bytes (tmp1, generalCounter...) are written as the
 *   original writes them: their leftover values are part of the state.
 * - Where the original hands a result to its caller in A, the C function
 *   leaves it in regA (or returns it, for routines that never wait).
 * - Arithmetic is 8-bit and wraps, as on the 6502; comparisons that the
 *   original makes with BPL/BMI after CMP use sign_set().
 * - What only drives hardware is left out: PPU writes, sprites, the mapper,
 *   the audio engine. Where that work took long enough to matter (it decides
 *   which frame the game resumes in), busy() counts its cycles instead.
 *
 * Waiting for a frame: the original's main loop and many routines wait for
 * the NMI in the middle of their work. Here those routines are resumable: a
 * routine that waits saves where it is (its resume point) and returns
 * PT_RUNNING to its caller, which saves its own point and returns, up to
 * tetris_tick. The next tick runs the NMI and calls back down the same path,
 * and each routine jumps to its resume point (the switch in PT_BEGIN). The
 * resume points are the addresses the original's JSRs push on the stack, so
 * that a snapshot of the original's stack page says where to resume.
 * A resumable routine must not keep anything in C locals across a wait.
 */
#ifndef TETRIS_GAME_H
#define TETRIS_GAME_H

#include <stdint.h>

#include "tetris/tetris.h"
#include "data.h"
#include "ram_names.h"

/* The game being run. Set by the entry points in tetris.h. */
extern struct tetris *T;
/* The A register, for the results the original passes back in it. */
extern uint8_t regA;

/* RAM by address, for the original's indexed accesses (`sta soundEffectSlot0Init,x`). */
#define R(addr) (T->ram.b[(uint16_t)(addr) & 0x7FF])
/* A 16-bit pointer kept in RAM, low byte first. */
static inline uint16_t ptr16(uint16_t zp) { return (uint16_t)(R(zp) | (R(zp + 1) << 8)); }

/* True when bit 7 of (a - m) is set: the N flag after CMP, tested with BMI (set) and BPL (clear). */
static inline int sign_set(uint8_t a, uint8_t m) { return ((uint8_t)(a - m) & 0x80) != 0; }

/* ADC with the carry flag, where the original lets a carry flow into the next addition. */
static inline uint8_t adc8(uint8_t a, uint8_t m, uint8_t *c)
{
    unsigned s = (unsigned)a + m + *c;
    *c = (uint8_t)(s >> 8);
    return (uint8_t)s;
}

/* ---- Constants (constants.asm) ---- */

#define BUTTON_A TETRIS_BUTTON_A
#define BUTTON_B TETRIS_BUTTON_B
#define BUTTON_SELECT TETRIS_BUTTON_SELECT
#define BUTTON_START TETRIS_BUTTON_START
#define BUTTON_UP TETRIS_BUTTON_UP
#define BUTTON_DOWN TETRIS_BUTTON_DOWN
#define BUTTON_LEFT TETRIS_BUTTON_LEFT
#define BUTTON_RIGHT TETRIS_BUTTON_RIGHT
#define BUTTON_DPAD (BUTTON_UP | BUTTON_DOWN | BUTTON_LEFT | BUTTON_RIGHT)

#define RENDER_HIGH_SCORE_LETTER 0x80
#define RENDER_STATS 0x40
#define RENDER_SCORE 0x04
#define RENDER_LEVEL 0x02
#define RENDER_LINES 0x01

/* NTSC timings (the USA cartridge) */
#define DAS_DELAY 0x0A
#define DAS_RESET 0x10
#define ENDING_SLEEP_TIME_1 0x80
#define ENDING_SLEEP_TIME_2 0x40
#define ENDING_SLEEP_TIME_3 0x02
#define INITIAL_AUTOREPEAT_Y 0xA0
#define LEGAL_SLEEP_TIME 0xFF
#define MENU_CURSOR_MASK 0x03

#define tileEmpty 0xEF  /* blank tile used for empty squares */
#define tileHidden 0xFF /* blank tile used to hide minos */
#define hidden 0x13     /* the orientation ID of no piece */

/* PPUCTRL bit 7: the NMI at each vertical blank. currentPpuCtrl is the game's copy. */
#define nmiEnabled() ((currentPpuCtrl & 0x80) != 0)

/* ---- Resumable routines ---- */

/* One for each routine that can wait part-way through. Their resume points are
 * T->ctl.resume[PT_name]. */
enum pt_routine {
    PT_reset, /* reset, initRam, initRamContinued and the main loop */
    PT_gameMode_legalScreen,
    PT_gameMode_titleScreen,
    PT_gameMode_gameTypeMenu,
    PT_gameMode_levelMenu,
    PT_gameMode_playAndEndingHighScore_jmp,
    PT_gameModeState_initGameBackground,
    PT_gameModeState_initGameState,
    PT_initPlayfieldIfTypeB,
    PT_gameModeState_handleGameOver,
    PT_handleHighScoreIfNecessary, /* and highScoreEntryScreen, which it jumps to */
    PT_gameModeState_updatePlayer1,
    PT_gameModeState_updatePlayer2,
    PT_playState_updateGameOverCurtain,
    PT_playState_bTypeGoalCheck,
    PT_gameModeState_startButtonHandling,
    PT_sleep_for_14_vblanks,
    PT_sleep_for_a_vblanks,
    PT_endingAnimation, /* and endingAnimationA/B, which it jumps to */
    PT_render_endingUnskippable,
    PT_COUNT
};
_Static_assert(PT_COUNT <= TETRIS_THREADS, "TETRIS_THREADS");

/* The jump tables whose choice must survive a wait (T->ctl.dispatched). */
enum { DISPATCH_GAME_MODE, DISPATCH_GAME_MODE_STATE, DISPATCH_PLAY_STATE };

#define PT_RUNNING 1
#define PT_DONE 0

/* Opens a resumable routine's body: continues at its resume point, if any. */
#define PT_BEGIN(routine)                                                                          \
    const int pt_self_ = (routine);                                                                \
    switch (T->ctl.resume[pt_self_]) {                                                             \
    case 0:

/* Closes it: the routine is done and returns to its caller. */
#define PT_END                                                                                     \
    break;                                                                                         \
    default:                                                                                       \
        pt_bad_resume(pt_self_);                                                                   \
    }                                                                                              \
    T->ctl.resume[pt_self_] = 0;                                                                   \
    return PT_DONE

/* Returns from a resumable routine, leaving `a` in A. */
#define PT_RETURN(a)                                                                               \
    do {                                                                                           \
        regA = (uint8_t)(a);                                                                       \
        T->ctl.resume[pt_self_] = 0;                                                               \
        return PT_DONE;                                                                            \
    } while (0)

/*
 * Waits for the next NMI, as updateAudioAndWaitForNmi does: clears
 * verticalBlankingInterval, which the NMI sets. `site` is the address the
 * original's JSR to its wait routine pushes. (The wait routines are
 * updateAudioWaitForNmiAndResetOamStaging, which then hides all sprites,
 * updateAudioWaitForNmiAndDisablePpuRendering and ...EnablePpuRendering, which
 * then turn the picture off or on, and updateAudioAndWaitForNmi. All run the
 * audio engine first. The macros below add what they do after the wait.)
 */
#define PT_WAIT_NMI(site)                                                                          \
    do {                                                                                           \
        verticalBlankingInterval = 0;                                                              \
        T->ctl.resume[pt_self_] = (site);                                                          \
        T->ctl.waiting = 1;                                                                        \
        return PT_RUNNING;                                                                         \
    case (site):;                                                                                  \
    } while (0)

/* updateAudioWaitForNmiAndResetOamStaging: leaves A = $FF. */
#define WAIT_RESET_OAM(site)                                                                       \
    do {                                                                                           \
        PT_WAIT_NMI(site);                                                                         \
        regA = 0xFF;                                                                               \
    } while (0)
/* updateAudioWaitForNmiAndDisablePpuRendering: the picture goes off. */
#define WAIT_DISABLE_RENDERING(site)                                                               \
    do {                                                                                           \
        PT_WAIT_NMI(site);                                                                         \
        currentPpuMask &= 0xE1;                                                                    \
    } while (0)
/* updateAudioWaitForNmiAndEnablePpuRendering: the picture comes on. */
#define WAIT_ENABLE_RENDERING(site)                                                                \
    do {                                                                                           \
        PT_WAIT_NMI(site);                                                                         \
        currentPpuMask |= 0x1E;                                                                    \
    } while (0)

/* Waits for a vertical blank with NMIs off (reset's warm-up loops). */
#define PT_WAIT_VBLANK(point)                                                                      \
    do {                                                                                           \
        T->ctl.resume[pt_self_] = (point);                                                         \
        T->ctl.waiting = 2;                                                                        \
        return PT_RUNNING;                                                                         \
    case (point):;                                                                                 \
    } while (0)

/* Calls a resumable routine (an expression returning PT_RUNNING or PT_DONE)
 * from the JSR whose pushed address is `site`. */
#define PT_CALL(site, call)                                                                        \
    do {                                                                                           \
        T->ctl.resume[pt_self_] = (site);                                                          \
    case (site):                                                                                   \
        if ((call) == PT_RUNNING)                                                                  \
            return PT_RUNNING;                                                                     \
    } while (0)

void pt_bad_resume(int routine);

/* Resumable routines, by enum, for the jump tables. */
typedef int (*pt_fn)(void);
extern const pt_fn pt_routines[PT_COUNT];

/* ---- Routines (by file) ---- */

/* system.c */
int reset(void);
int branchOnGameMode(void);
void generateNextPseudorandomNumber(uint8_t x, uint8_t y);
void memset_page(uint8_t a, uint8_t x, uint8_t y);
void waitForVBlankAndEnableNmi(void);
void disableNmi(void);
void busy(uint32_t cycles);
void nmi(void);

/* menus.c */
int gameMode_legalScreen(void);
int gameMode_titleScreen(void);
int gameMode_gameTypeMenu(void);
int gameMode_levelMenu(void);
void gameMode_startDemo(void);
void pollControllerButtons(void);
void setMusicTrack(uint8_t a);

/* play.c */
int gameMode_playAndEndingHighScore_jmp(void);
int gameModeState_initGameBackground(void);
int gameModeState_initGameState(void);
int initPlayfieldIfTypeB(void);
int gameModeState_handleGameOver(void);
int gameModeState_updatePlayer1(void);
int gameModeState_updatePlayer2(void);
int playState_updateGameOverCurtain(void);
int playState_bTypeGoalCheck(void);
int gameModeState_startButtonHandling(void);
int sleep_for_14_vblanks(void);
int sleep_for_a_vblanks(void);
uint8_t chooseNextTetrimino(void);
void incrementPieceStat(uint8_t a);
uint8_t isPositionValid(void);
void updatePlayfield(void);
void updateMusicSpeed(void);

/* render.c */
void render(void);
void screen_clearPlayfields(void);

/* highscore.c */
int handleHighScoreIfNecessary(void);

/* ending.c */
int endingAnimation(void);
int render_endingUnskippable(void);

#endif
