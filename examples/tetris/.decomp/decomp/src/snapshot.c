/*
 * Taking over from a running original (tetris_load_nes_snapshot).
 *
 * The RAM carries all of the game's variables, so most of the work is a
 * copy. What it doesn't carry by name is where the code is: which routines
 * are part-way through, and where they will continue. On the NES that is the
 * stack. The game sets the stack pointer to $FF at reset and never moves it
 * otherwise, so the routines that are part-way through have pushed their
 * return addresses at $01FE-$01FF, $01FC-$01FD and so on down, in the order
 * they were called. Each return address is one of the decomp's resume points
 * (sites.h), which names the routine and where in it to continue; the walk
 * ends at the first one that waits for the NMI. (Jump tables don't push
 * anything: switch_s_plus_2a pops its own return address, so a handler's
 * calls follow its dispatcher's.)
 */
#include <stdio.h>
#include <string.h>

#include "game.h"
#include "sites.h"

static const struct pt_site *findSite(uint16_t ret)
{
    for (size_t i = 0; i < sizeof pt_sites / sizeof pt_sites[0]; i++)
        if (pt_sites[i].ret == ret)
            return &pt_sites[i];
    return NULL;
}

/* The jump table a call site goes through, if any. */
static int dispatchOf(uint16_t site)
{
    switch (site) {
    case 0x813A: return DISPATCH_GAME_MODE;       /* branchOnGameMode */
    case 0x815F: return DISPATCH_GAME_MODE_STATE; /* gameMode_playAndEndingHighScore */
    case 0x8179:                                  /* branchOnPlayStatePlayer1 */
    case 0x8191: return DISPATCH_PLAY_STATE;      /* branchOnPlayStatePlayer2 */
    default: return -1;
    }
}

int tetris_load_nes_snapshot(struct tetris *t, const uint8_t nesRam[0x800], char *why, size_t whyLen)
{
    static struct tetris next; /* not on the stack: it is a few KB */
    memset(&next, 0, sizeof next);
    memcpy(next.ram.b, nesRam, 0x800);

    uint16_t sp = 0x1FF;
    int parentDispatch = -1;
    int expect = PT_reset;
    for (int depth = 0;; depth++) {
        if (sp < 0x101 || depth >= TETRIS_THREADS) {
            snprintf(why, whyLen, "no wait for the next frame found on the stack");
            return 1;
        }
        uint16_t ret = (uint16_t)(nesRam[sp - 1] | (nesRam[sp] << 8));
        const struct pt_site *site = findSite(ret);
        if (!site) {
            snprintf(why, whyLen, "the original is between frames: $%04X at $%04X on the stack is not a wait",
                     ret, sp - 1);
            return 1;
        }
        if (parentDispatch >= 0)
            next.ctl.dispatched[parentDispatch] = (uint8_t)(site->routine + 1);
        else if (site->routine != expect) {
            snprintf(why, whyLen, "unexpected return address $%04X at $%04X on the stack", ret, sp - 1);
            return 1;
        }
        next.ctl.resume[site->routine] = ret;
        if (site->kind == SITE_WAIT)
            break;
        parentDispatch = dispatchOf(ret);
        /* the routine a non-dispatching call site calls */
        switch (ret) {
        case 0x823A: case 0x9A57: case 0xA420: expect = PT_sleep_for_a_vblanks; break;
        case 0xA417: expect = PT_sleep_for_14_vblanks; break;
        case 0x876A: expect = PT_initPlayfieldIfTypeB; break;
        case 0x9A5A: case 0xA423: expect = PT_endingAnimation; break;
        case 0x9CF0: expect = PT_handleHighScoreIfNecessary; break;
        case 0x9EC5: case 0x9EE3: case 0x9F08: case 0x9F11: case 0x9F36: case 0x9F44: case 0xA95C:
            expect = PT_render_endingUnskippable;
            break;
        default: expect = -1; break;
        }
        sp = (uint16_t)(sp - 2);
    }

    next.ctl.waiting = 1;
    /* The screen: the playfields as they are in RAM, and the pause screen if
     * the game is in the pause loop. */
    memcpy(next.screen.field[0], &nesRam[A_playfield], 200);
    memcpy(next.screen.field[1], &nesRam[A_playfieldForSecondPlayer], 200);
    next.screen.paused = next.ctl.resume[PT_gameModeState_startButtonHandling] == 0xA3DB;
    *t = next;
    return 0;
}
