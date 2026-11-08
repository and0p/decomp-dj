/*
 * The endings. A-type, with 30,000 points or more: a rocket launch, bigger
 * for bigger scores. B-type, on reaching 0 lines: the level and height
 * bonuses are counted up while an animation plays.
 *
 * What is kept is what decides when the game goes on: the frame counts, the
 * bonus count-up, and the rocket's flight (Start only works once it has left
 * the screen). The B-type animations (a concert or a parade of characters)
 * only move sprites, and are left out.
 */
#include "game.h"

#define bTypeLevelBonus generalCounter4
#define bTypeHeightBonus generalCounter5

/* How long the endings' screens take to draw with NMIs off, in CPU cycles
 * from the NMI (see busyUntil). The A-type rocket's grows a little with the
 * ending, and the cathedral (ending 4) patches the picture; B-type's level 9
 * concert patches the stage for the height. */
#define ENDING_A_DRAW(ending) ((ending) == 4 ? 26595 : 24852 + 12 * (ending))
#define ENDING_B_DRAW 25174
#define ENDING_B_LEVEL9_DRAW 27302

/* add100Points: adds 100 (BCD) to totalScore. Its BCD fixup checks the
 * hundreds digit only after a carry out of the tens. */
static void add100Points(void)
{
    uint8_t a = (uint8_t)(0x01 + totalScore[1]);
    totalScore[1] = a;
    a &= 0x0F;
    if (a >= 0x0A) {
        a = (uint8_t)(totalScore[1] + 0x06);
        totalScore[1] = a;
    }
    if ((a & 0xF0) >= 0xA0) { /* L9F76 */
        totalScore[1] = (uint8_t)(totalScore[1] + 0x60);
        totalScore[2]++;
    }
    if ((totalScore[2] & 0x0F) >= 0x0A) /* L9F85 */
        totalScore[2] = (uint8_t)(totalScore[2] + 0x06);
}

/* One step of counting down a bonus (BCD), adding 100 points for it.
 * Returns the bonus left. */
static uint8_t countDownBonus(uint8_t bonus)
{
    bonus--;
    if ((bonus & 0x0F) == 0x0F)
        bonus = (uint8_t)((bonus & 0xF0) | 0x09);
    add100Points(); /* @noLevelCarry / @noHeightCarry */
    return bonus;
}

/*
 * ending_typeA: one frame of the rocket. After a countdown of $F0 frames
 * (ending_customVars+1) it rises: ending_customVars is its y, which falls a
 * pixel every other frame near the ground, then a pixel a frame, then two,
 * until it reaches 0 and the rocket has gone.
 */
static void ending_typeA(void)
{
    if (ending_customVars[0] == 0x00)
        return; /* LAA10 */
    /* (the rocket and its flickering jet are sprites) */
    if (ending_customVars[1] != 0xF0) {
        ending_customVars[1]++; /* LAA0E */
        return;
    }
    if (ending_customVars[0] >= 0xB0 && (frameCounter[0] & 0x01))
        return; /* LAA0B */
    ending_customVars[0]--; /* LA9FC */
    if (ending_customVars[0] < 0x80)
        ending_customVars[0]--;
}

/*
 * ending_initTypeBVars: B-type's ending, before it starts. Level 9 is the
 * concert: the stage is patched for the start height (patchToPpu; the picture
 * isn't kept, only where the patching left patchToPpuAddr), and the four domes
 * start at their resting y. Other levels are the parade: figures walk in from
 * one side, ending_customVars+1 on being the first one's x.
 */
static void ending_initTypeBVars(void)
{
    ending = 0;
    ending_customVars[0] = 0;
    ending_typeBCathedralFrameDelayCounter = 0;
    spriteIndexInOamContentLookup = 0x02;
    if (levelNumber == 0x09) {
        ending = (uint8_t)(player1_startHeight + 1);
        /* ending_typeBConcertPatchToPpuForHeight: from this height's patch on,
         * falling through to height 4's; height 5 has none */
        uint16_t entry = ROM_ending_typeBConcertPatchToPpuForHeight_entries[ending < 7 ? ending : 6];
        tmp1 = (uint8_t)entry;
        tmp2 = (uint8_t)(entry >> 8);
        if (ending <= 0x05) {
            R(A_patchToPpuAddr) = (uint8_t)ROM_ending_patchToPpu_typeBConcertHeight4;
            R(A_patchToPpuAddr + 1) = (uint8_t)(ROM_ending_patchToPpu_typeBConcertHeight4 >> 8);
        }
        ending = 0;
        ending_customVars[2] = 0;
        for (int i = 0; i < 4; i++)
            ending_customVars[3 + i] = LA73D[i];
        return;
    }
    for (int i = 2; i <= 6; i++) /* @notLevel9 */
        ending_customVars[i] = LA767[levelNumber];
    ending_customVars[1] = LA75D[levelNumber];
}

/*
 * The concert's domes (LA6BC): ending_customVars counts up as the domes
 * start; each starts to rise when it reaches the dome's number in LA735,
 * a pixel a frame, then two once above $A0 (with a sound effect, which the
 * decomp doesn't play), until it reaches 0. ending_customVars+3.. are their y.
 */
static void ending_typeBConcertDomes(void)
{
    for (int x = 0; x < 4; x++)
        if (LA735[x] == ending_customVars[0] && ending_customVars[3 + x]) {
            ending_customVars[3 + x]--;
            ending_customVars[0]++;
        }
    for (ending_currentSprite = 0; ending_currentSprite != 4; ending_currentSprite++) {
        uint8_t x = ending_currentSprite, y = ending_customVars[3 + x];
        if (!y)
            continue;
        if (LA73D[x] != y) { /* rising: soundEffectSlot0Init = 3 */
            y--;
            if (y < 0xA0)
                y--;
        }
        ending_customVars[3 + x] = y;
        generalCounter = y;
        /* (the dome, and its exhaust while it rises, are sprites) */
    }
}

/*
 * ending_typeBConcert: the musicians, more of them the higher the start
 * height (a jump table into a run that falls through), then the domes.
 * At height 5 Luigi jumps: `ending` counts his frames, and each time it
 * wraps (ten animation frames of eight) the domes' count goes up by one.
 */
static void ending_typeBConcert(void)
{
    uint8_t h = player1_startHeight;
    uint16_t entry = ROM_ending_typeBConcert_entries[h < 6 ? h : 5];
    tmp1 = (uint8_t)entry;
    tmp2 = (uint8_t)(entry >> 8);
    if (h == 5) { /* @marioLuigiPeach */
        while ((ending >> 3) == 0x0A) { /* @luigiCalculateFrame */
            ending = 0;
            ending_customVars[0]++;
        }
        ending++;
    }
    /* (Mario, Luigi, Peach, Bowser, Donkey Kong, Samus, Link and Pit are
     * sprites) */
    ending_typeBConcertDomes();
}

/*
 * ending_typeBCathedral: the parade. `ending` times the figures' animation
 * (ending_customVars flips between two frames). Every few frames
 * (ending_typeBCathedralFrameDelayTable) each figure that has entered
 * moves by the level's vector, and on reaching its spot in
 * ending_typeBCathedralXTable lets the next one in. There are startHeight + 1
 * of them.
 */
static void ending_typeBCathedral(void)
{
    uint8_t lv = levelNumber;
    ending++; /* ending_typeBCathedralSetSprite */
    if (ending_typeBCathedralAnimSpeed[lv] == ending) {
        ending_customVars[0] ^= 0x01;
        ending = 0;
    }
    ending_typeBCathedralFrameDelayCounter++;
    ending_currentSprite = 0;
    for (;;) { /* @spriteLoop */
        uint8_t cs = ending_currentSprite;
        generalCounter = LA767[lv];
        uint8_t x = ending_customVars[1 + cs];
        if (x != generalCounter && ending_typeBCathedralFrameDelayTable[lv] == ending_typeBCathedralFrameDelayCounter) {
            /* (the figure is a sprite) */
            x = (uint8_t)(x + ending_typeBCathedralVectorTable[lv]);
            ending_customVars[1 + cs] = x;
            if (ending_typeBCathedralXTable[(uint8_t)(lv * 6 + cs)] == x)
                R(A_ending_customVars + 2 + cs) = LA75D[lv]; /* past the end for the sixth: ending_currentSprite */
        }
        generalCounter = ending_currentSprite; /* @continue */
        if (ending_currentSprite == startHeight)
            break;
        ending_currentSprite++;
    }
    if (ending_typeBCathedralFrameDelayTable[lv] == ending_typeBCathedralFrameDelayCounter) /* @done */
        ending_typeBCathedralFrameDelayCounter = 0;
}

/* render_ending: one frame of the ending animation. */
static void render_ending(void)
{
    if (!gameType)
        ending_typeA();
    else if (levelNumber == 0x09)
        ending_typeBConcert();
    else
        ending_typeBCathedral();
}

/* render_endingUnskippable: plays sleepCounter frames of the ending
 * animation (the original takes the count in A). */
int render_endingUnskippable(void)
{
    PT_BEGIN(PT_render_endingUnskippable);
    do { /* @loopUntilEnoughFrames */
        render_ending();
        WAIT_RESET_OAM(0xA804);
    } while (sleepCounter);
    PT_RETURN(sleepCounter);
    PT_END;
}

/*
 * selectEndingScreen: picks the rocket by score: ending 0 from 30,000, 1
 * from 50,000, 2 from 70,000, 3 from 100,000, 4 (the cathedral, which also
 * patches the picture) from 120,000. Sets the rocket's starting y and its
 * countdown.
 */
static void selectEndingScreen(void)
{
    ending = 0;
    if (player1_score[2] >= 0x05) {
        ending = 0x01;
        if (player1_score[2] >= 0x07) {
            ending = 0x02;
            if (player1_score[2] >= 0x10) {
                ending = 0x03;
                if (player1_score[2] >= 0x12) {
                    ending = 0x04;
                    /* and patchToPpu: the cathedral, in the picture */
                    R(A_patchToPpuAddr) = (uint8_t)ROM_ending_patchToPpu_typeAOver120k;
                    R(A_patchToPpuAddr + 1) = (uint8_t)(ROM_ending_patchToPpu_typeAOver120k >> 8);
                }
            }
        }
    }
    /* ending_selected */
    ending_customVars[0] = LAA2A[ending];
    ending_customVars[1] = 0;
}

/*
 * endingAnimation: plays the ending for the game type, jumping to
 * endingAnimationA or endingAnimationB.
 *
 * endingAnimationA: draws the rocket's screen, then plays the animation for
 * 128 frames, then until the rocket has left and Start is pressed.
 *
 * endingAnimationB: B-type's end screen. The level bonus is the level's
 * last digit times 1,000 points and the height bonus the starting height
 * times 1,000; each is added 100 points at a time, two frames apart.
 * totalScore holds the running total while player1_score shows 0; Start
 * leaves, with the total as the score.
 */
int endingAnimation(void)
{
    PT_BEGIN(PT_endingAnimation);
    spriteIndexInOamContentLookup = 0x02;
    renderMode = 0x04;
    if (gameType)
        goto endingAnimationB;

    /* endingAnimationA */
    WAIT_DISABLE_RENDERING(0xA928);
    disableNmi();
    selectEndingScreen(); /* with type_a_ending_nametable and ending_palette */
    drawLeftovers(0xAD66, 0xA93D);
    busyUntil(ENDING_A_DRAW(ending));
    waitForVBlankAndEnableNmi();
    WAIT_RESET_OAM(0xA948);
    WAIT_ENABLE_RENDERING(0xA94B);
    WAIT_RESET_OAM(0xA94E);
    renderMode = 0x04;
    setMusicTrack(0x0A);
    sleepCounter = ENDING_SLEEP_TIME_1;
    PT_CALL(0xA95C, render_endingUnskippable());
    do { /* @endingSleep */
        render_ending();
        WAIT_RESET_OAM(0xA962);
    } while (ending_customVars[0] || newlyPressedButtons_player1 != BUTTON_START);
    PT_RETURN(regA);

endingAnimationB:
    levelNumber = (uint8_t)(player1_levelNumber < 30 ? levelDisplayTable[player1_levelNumber] & 0x0F : 0);
    totalScore[2] = 0;
    totalScore[1] = 0;
    totalScore[0] = 0;
    bTypeLevelBonus = (uint8_t)(levelNumber << 4);
    bTypeHeightBonus = (uint8_t)(player1_startHeight << 4);
    WAIT_DISABLE_RENDERING(0x9E6C);
    disableNmi();
    /* the concert's or the parade's nametable, ending_palette, then
     * ending_initTypeBVars */
    drawLeftovers(0xAD66, 0x9EA6);
    ending_initTypeBVars();
    busyUntil(levelNumber == 0x09 ? ENDING_B_LEVEL9_DRAW : ENDING_B_DRAW);
    waitForVBlankAndEnableNmi();
    WAIT_RESET_OAM(0x9EB1);
    WAIT_ENABLE_RENDERING(0x9EB4);
    WAIT_RESET_OAM(0x9EB7);
    renderMode = 0x04;
    setMusicTrack(0x0A);
    sleepCounter = ENDING_SLEEP_TIME_1;
    PT_CALL(0x9EC5, render_endingUnskippable());
    totalScore[0] = player1_score[0];
    totalScore[1] = player1_score[1];
    totalScore[2] = player1_score[2];
    player1_score[0] = 0;
    player1_score[1] = 0;
    player1_score[2] = 0;
    sleepCounter = ENDING_SLEEP_TIME_2;
    PT_CALL(0x9EE3, render_endingUnskippable());
    if (bTypeLevelBonus) {
        do { /* @addLevelBonus */
            bTypeLevelBonus = countDownBonus(bTypeLevelBonus);
            sleepCounter = ENDING_SLEEP_TIME_3;
            PT_CALL(0x9F08, render_endingUnskippable());
        } while (bTypeLevelBonus);
        sleepCounter = ENDING_SLEEP_TIME_2;
        PT_CALL(0x9F11, render_endingUnskippable());
    }
    if (bTypeHeightBonus) { /* @checkForHeightBonus */
        do { /* @addHeightBonus */
            bTypeHeightBonus = countDownBonus(bTypeHeightBonus);
            sleepCounter = ENDING_SLEEP_TIME_3;
            PT_CALL(0x9F36, render_endingUnskippable());
        } while (bTypeHeightBonus);
        sleepCounter = ENDING_SLEEP_TIME_2;
        PT_CALL(0x9F44, render_endingUnskippable());
    }
    do { /* @startNotPressed */
        render_ending();
        WAIT_RESET_OAM(0x9F4A);
    } while (newlyPressedButtons_player1 != BUTTON_START);
    levelNumber = player1_levelNumber;
    score[0] = totalScore[0];
    score[1] = totalScore[1];
    score[2] = totalScore[2];
    PT_RETURN(score[2]);
    PT_END;
}
