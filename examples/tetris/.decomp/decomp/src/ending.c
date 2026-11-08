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

/* What drawing an ending's screen takes, roughly: a nametable and a palette. */
#define ENDING_SCREEN_CYCLES (1024 * 16 + 800 + 32 * 16 + 200)

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

/* render_ending: one frame of the ending animation. */
static void render_ending(void)
{
    if (!gameType)
        ending_typeA();
    /* else ending_typeB: the concert or the parade, sprites only */
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
                    busy(400); /* patchToPpu: ending_patchToPpu_typeAOver120k */
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
    busy(ENDING_SCREEN_CYCLES); /* type_a_ending_nametable, ending_palette */
    selectEndingScreen();
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
    /* the concert's or the parade's nametable, ending_palette, and
     * ending_initTypeBVars, which for the concert patches the stage for the
     * height */
    busy(ENDING_SCREEN_CYCLES + (levelNumber == 0x09 ? 600 : 0));
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
