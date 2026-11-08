/*
 * Checking the decomp against a real Tetris frame by frame
 * (tetris_check_nes_parity).
 *
 * Every frame is judged on two things:
 *
 * - Where the code is. The original's stack says which routines are part-way
 *   through and where each will continue (see snapshot.c); the decomp keeps
 *   the same thing in ctl. They must agree, so a frame lost or gained (one
 *   side still drawing a screen while the other waits for the next frame) is
 *   a mismatch on the frame it happens.
 * - The RAM, byte for byte, except what the decomp doesn't keep (notKept).
 *
 * In the first two frames after power-on the original waits for vertical
 * blanks inline in reset, with nothing on the stack, so only RAM is judged.
 *
 * On the few frames where the original is part-way through drawing a screen
 * with NMIs off (more than a frame of work: the power-on clear, the level
 * menu and the name entry screen), its RAM is caught mid-routine, while the
 * decomp has done all of that work at once and is waiting for the next frame
 * (ctl.framesBehind counts the frames it is ahead). The bytes that routine
 * writes are in flight on that frame, so they are left out of that frame's
 * comparison, and only those: inFlight lists them for each screen, measured
 * by running the original's code (tools/measure_draws.py --writes). The next
 * frame compares them again.
 */
#include <stdio.h>
#include <string.h>

#include "game.h"

struct span {
    uint16_t lo, hi; /* inclusive */
};

/* What the decomp leaves out. The stack is judged as the code position. */
static const struct span notKept[] = {
    {0x0100, 0x01FF}, /* the stack */
    {0x0200, 0x02FF}, /* oamStaging */
    {0x00A0, 0x00A2}, /* spriteXOffset, spriteYOffset, spriteIndexInOamContentLookup */
    {0x00A8, 0x00AC}, /* generalCounter-generalCounter5: what loading sprites leaves */
    {0x00AE, 0x00AE}, /* originalY: the same */
    {0x00B3, 0x00B3}, /* oamStagingLength */
    {0x00E0, 0x00EF}, /* the audio engine's zero page */
    {0x0680, 0x06FF}, /* the audio engine's RAM */
};

/* The bytes each long screen draw writes, from the end of its frame's NMI to
 * the next wait for a frame, by the decomp's resume point after it. */
static const struct span bootDraw[] = {{0x0000, 0x0002}, {0x0033, 0x0033}, {0x00FE, 0x00FF}, {0x0400, 0x05FF}};
static const struct span levelMenuDraw[] = {{0x0000, 0x0001}, {0x0005, 0x0006}, {0x0033, 0x0033}, {0x00FE, 0x00FF}};
static const struct span highScoreDraw[] = {
    {0x0000, 0x0001}, {0x0005, 0x0006}, {0x0033, 0x0033}, {0x00BD, 0x00BD}, {0x00FE, 0x00FF}};
static const struct {
    uint16_t resume;
    const char *what;
    const struct span *bytes;
    size_t n;
} inFlight[] = {
    {0x811F, "the power-on clear", bootDraw, sizeof bootDraw / sizeof bootDraw[0]},
    {0x8411, "the level menu's draw", levelMenuDraw, sizeof levelMenuDraw / sizeof levelMenuDraw[0]},
    {0xA24A, "the name entry screen's draw", highScoreDraw, sizeof highScoreDraw / sizeof highScoreDraw[0]},
};

static int inSpans(const struct span *s, size_t n, uint16_t a)
{
    for (size_t i = 0; i < n; i++)
        if (a >= s[i].lo && a <= s[i].hi)
            return 1;
    return 0;
}

static int resumesAt(const struct tetris *t, uint16_t site)
{
    for (int i = 0; i < TETRIS_THREADS; i++)
        if (t->ctl.resume[i] == site)
            return 1;
    return 0;
}

int tetris_check_nes_parity(const struct tetris *t, const uint8_t nes[0x800], char *why, size_t whyLen)
{
    static struct tetris there; /* the original's state, as the decomp would hold it */
    char reason[160];
    int loaded = tetris_load_nes_snapshot(&there, nes, reason, sizeof reason) == 0;
    const struct span *flight = NULL;
    size_t flightN = 0;
    const char *drawing = NULL;

    if (t->ctl.framesBehind) {
        for (size_t i = 0; i < sizeof inFlight / sizeof inFlight[0]; i++)
            if (resumesAt(t, inFlight[i].resume)) {
                flight = inFlight[i].bytes;
                flightN = inFlight[i].n;
                drawing = inFlight[i].what;
            }
        if (!drawing) {
            snprintf(why, whyLen, "the decomp is a frame ahead in a screen draw it has no list of in-flight bytes for");
            return 1;
        }
        if (loaded) {
            snprintf(why, whyLen, "frame timing: the NES is waiting for the next frame, but should still be in %s", drawing);
            return 1;
        }
    } else if (t->ctl.waiting == 2) {
        /* reset's two waits for a vertical blank at power-on, before anything
         * is on the stack: the code position isn't in RAM to compare */
        if (loaded) {
            snprintf(why, whyLen, "frame timing: the decomp is still powering on, but the NES isn't");
            return 1;
        }
        drawing = "power-on";
    } else {
        if (!loaded) {
            snprintf(why, whyLen, "frame timing: the decomp is waiting for the next frame, but the NES isn't (%s)", reason);
            return 1;
        }
        for (int i = 0; i < TETRIS_THREADS; i++)
            if (there.ctl.resume[i] != t->ctl.resume[i]) {
                snprintf(why, whyLen, "code position: a routine continues at $%04X on the NES, $%04X in the decomp",
                         there.ctl.resume[i], t->ctl.resume[i]);
                return 1;
            }
        for (size_t i = 0; i < sizeof t->ctl.dispatched; i++)
            if (there.ctl.dispatched[i] != t->ctl.dispatched[i]) {
                snprintf(why, whyLen, "code position: jump table %zu runs a different handler", i);
                return 1;
            }
    }

    size_t o = 0;
    int differ = 0;
    for (uint16_t a = 0; a < 0x800; a++) {
        if (inSpans(notKept, sizeof notKept / sizeof notKept[0], a) || inSpans(flight, flightN, a))
            continue;
        if (nes[a] == t->ram.b[a])
            continue;
        if (differ < 6 && o < whyLen)
            o += (size_t)snprintf(why + o, whyLen - o, "%s$%04X NES %02X decomp %02X", differ ? ", " : "RAM differs: ", a,
                                  nes[a], t->ram.b[a]);
        differ++;
    }
    if (differ) {
        if (differ > 6 && o < whyLen)
            snprintf(why + o, whyLen - o, " (%d bytes in all)", differ);
        return 1;
    }
    if (flight)
        snprintf(why, whyLen, "matched, but for the bytes %s is part-way through writing", drawing);
    else if (drawing)
        snprintf(why, whyLen, "matched RAM; at power-on the code position isn't on the stack yet");
    else if (whyLen)
        why[0] = 0;
    return 0;
}
