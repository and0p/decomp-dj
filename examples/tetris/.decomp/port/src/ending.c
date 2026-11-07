/*
 * The endings. A-type, with 30,000 points or more: a rocket launch, bigger
 * for bigger scores (selectEndingScreen). B-type, on reaching 0 lines: the
 * level and height bonuses are counted up over an animation chosen by the
 * level: at level 9 (or 19) a concert of Nintendo characters in front of the
 * cathedral, more of them the higher the starting height, while the
 * cathedral's domes lift off as rockets; at other levels a parade of
 * characters or vehicles across the screen.
 *
 * ending and ending_customVars mean different things in each ending; the
 * comments say which.
 */
#include "game.h"

#define wr(addr, v) poke((addr), (uint8_t)(v))

static void endingAnimationA(void);
static void endingAnimationB(void);
static void render_ending(void);
static void ending_initTypeBVars(void);
static void patchToPpu(void);

#define bTypeLevelBonus generalCounter4
#define bTypeHeightBonus generalCounter5

/* endingAnimation: plays the ending for the game type. */
void endingAnimation(void)
{
    spriteIndexInOamContentLookup = 0x02;
    renderMode = 0x04;
    if (gameType)
        endingAnimationB();
    else
        endingAnimationA();
}

/* render_endingUnskippable: plays A frames of the ending animation. */
static void render_endingUnskippable(uint8_t a)
{
    sleepCounter = a;
    do { /* @loopUntilEnoughFrames */
        render_ending();
        updateAudioWaitForNmiAndResetOamStaging();
    } while (sleepCounter);
}

/* add100Points: adds 100 (BCD) to totalScore. The argument in A is unused:
 * the routine loads 1 itself. Its BCD fixup checks the hundreds digit only
 * after a carry out of the tens. */
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
    soundEffectSlot1Init = 0x01;
    return bonus;
}

/*
 * endingAnimationB: B-type's end screen. The level bonus is the level's
 * last digit times 1,000 points and the height bonus the starting height
 * times 1,000; each is added 100 points at a time while the animation runs.
 * totalScore holds the running total while player1_score shows 0; Start
 * leaves, with the total as the score.
 */
static void endingAnimationB(void)
{
    levelNumber = romTable(ROM_levelDisplayTable, player1_levelNumber) & 0x0F;
    totalScore[2] = 0;
    totalScore[1] = 0;
    totalScore[0] = 0;
    bTypeLevelBonus = (uint8_t)(levelNumber << 4);
    bTypeHeightBonus = (uint8_t)(player1_startHeight << 4);
    updateAudioWaitForNmiAndDisablePpuRendering();
    disableNmi();
    if (levelNumber == 0x09) { /* the concert, for level 9 or 19 */
        changeCHRBank0(CHR_TYPEB_ENDING);
        changeCHRBank1(CHR_TYPEB_ENDING);
        bulkCopyToPpu(0x9E80, ROM_type_b_lvl9_ending_nametable);
    } else { /* @checkPenguinOrOstrichEnding */
        uint8_t x = CHR_GAME; /* the penguins (level 2) and ostriches (6) are in the game's tiles */
        if (levelNumber != 0x02 && levelNumber != 0x06)
            x = CHR_TYPEA_ENDING;
        changeCHRBank0(x); /* @normalEnding */
        changeCHRBank1(CHR_TYPEA_ENDING);
        bulkCopyToPpu(0x9E9F, ROM_type_b_ending_nametable);
    }
    /* @startAnimation */
    bulkCopyToPpu(0x9EA4, ROM_ending_palette);
    ending_initTypeBVars();
    waitForVBlankAndEnableNmi();
    updateAudioWaitForNmiAndResetOamStaging();
    updateAudioWaitForNmiAndEnablePpuRendering();
    updateAudioWaitForNmiAndResetOamStaging();
    renderMode = 0x04;
    setMusicTrack(0x0A);
    render_endingUnskippable(ENDING_SLEEP_TIME_1);
    totalScore[0] = player1_score[0];
    totalScore[1] = player1_score[1];
    totalScore[2] = player1_score[2];
    soundEffectSlot1Init = 0x02;
    player1_score[0] = 0;
    player1_score[1] = 0;
    player1_score[2] = 0;
    render_endingUnskippable(ENDING_SLEEP_TIME_2);
    if (bTypeLevelBonus) {
        do { /* @addLevelBonus */
            bTypeLevelBonus = countDownBonus(bTypeLevelBonus);
            render_endingUnskippable(ENDING_SLEEP_TIME_3);
        } while (bTypeLevelBonus);
        render_endingUnskippable(ENDING_SLEEP_TIME_2);
    }
    if (bTypeHeightBonus) { /* @checkForHeightBonus */
        do { /* @addHeightBonus */
            bTypeHeightBonus = countDownBonus(bTypeHeightBonus);
            render_endingUnskippable(ENDING_SLEEP_TIME_3);
        } while (bTypeHeightBonus);
        soundEffectSlot1Init = 0x02;
        render_endingUnskippable(ENDING_SLEEP_TIME_2);
    }
    do { /* @startNotPressed */
        render_ending();
        updateAudioWaitForNmiAndResetOamStaging();
    } while (newlyPressedButtons_player1 != BUTTON_START);
    levelNumber = player1_levelNumber;
    score[0] = totalScore[0];
    score[1] = totalScore[1];
    score[2] = totalScore[2];
}

/* render_mode_ending_animation: the NMI's work in the endings: the score
 * and, for B-type, the two bonuses and the total. */
void render_mode_ending_animation(void)
{
    wr(PPUADDR, 0x20);
    wr(PPUADDR, 0x8E);
    twoDigsToPPU(player1_score[2]);
    twoDigsToPPU(player1_score[1]);
    twoDigsToPPU(player1_score[0]);
    if (gameType) {
        wr(PPUADDR, 0x20);
        wr(PPUADDR, 0xB0);
        twoDigsToPPU(bTypeLevelBonus);
        wr(PPUADDR, 0x20);
        wr(PPUADDR, 0xD0);
        twoDigsToPPU(bTypeHeightBonus);
        wr(PPUADDR, 0x21);
        wr(PPUADDR, 0x2E);
        twoDigsToPPU(totalScore[2]);
        twoDigsToPPU(totalScore[1]);
        twoDigsToPPU(totalScore[0]);
    }
    wr(PPUSCROLL, 0); /* L9FE9 */
    wr(PPUSCROLL, 0);
}

/* ------------------------------------------------------------------------ */
/* B-type                                                                   */
/* ------------------------------------------------------------------------ */

/*
 * ending_typeBConcertPatchToPpuForHeight: the concert stage has room for six
 * performers; for each starting height below 5 the missing ones' places are
 * patched over. Entry `ending` (height + 1) runs its patch and every later
 * one, falling through the original's code.
 */
static void ending_typeBConcertPatchToPpuForHeight(void)
{
    static const uint16_t patches[6] = {
        ROM_ending_patchToPpu_typeBConcertHeightUnused, ROM_ending_patchToPpu_typeBConcertHeight0,
        ROM_ending_patchToPpu_typeBConcertHeight1,      ROM_ending_patchToPpu_typeBConcertHeight2,
        ROM_ending_patchToPpu_typeBConcertHeight3,      ROM_ending_patchToPpu_typeBConcertHeight4,
    };
    switch_s_plus_2a(ending, (uint16_t)(ROM_ending_typeBConcertPatchToPpuForHeight + 5));
    for (uint8_t i = ending; i < 6; i++) {
        R(A_patchToPpuAddr + 1) = (uint8_t)(patches[i] >> 8);
        R(A_patchToPpuAddr) = (uint8_t)patches[i];
        patchToPpu();
    }
}

/* patchToPpu: writes the patch at patchToPpuAddr: a PPU address, tiles, $FE
 * then another address, ..., $FD to end. */
static void patchToPpu(void)
{
    uint16_t p = ptr16(A_patchToPpuAddr);
    uint8_t y = 0;
    for (;;) { /* @patchAddr */
        wr(PPUADDR, peek((uint16_t)(p + y)));
        y++;
        wr(PPUADDR, peek((uint16_t)(p + y)));
        y++;
        for (;;) { /* @patchValue */
            uint8_t a = peek((uint16_t)(p + y));
            y++;
            if (a == 0xFE)
                break;
            if (a == 0xFD)
                return;
            wr(PPUDATA, a);
        }
    }
}

/*
 * ending_initTypeBVars: sets up the B-type animation. For the concert,
 * patches the stage for the height and sets the four domes' heights
 * (ending_customVars+3..+6, from LA73D). For the parade, sets each
 * performer's x (ending_customVars+1 onward; the first starts at its entry
 * point LA75D, the rest wait off screen at LA767).
 */
static void ending_initTypeBVars(void)
{
    ending = 0;
    ending_customVars = 0;
    ending_typeBCathedralFrameDelayCounter = 0;
    spriteIndexInOamContentLookup = 0x02;
    if (levelNumber == 0x09) {
        ending = (uint8_t)(player1_startHeight + 1);
        ending_typeBConcertPatchToPpuForHeight();
        ending = 0;
        R(A_ending_customVars + 2) = 0;
        for (int i = 0; i < 4; i++)
            R(A_ending_customVars + 3 + i) = romTable(ROM_LA73D, (uint8_t)i);
        return;
    }
    /* @notLevel9 */
    uint8_t a = romTable(ROM_LA767, levelNumber);
    for (int i = 2; i <= 6; i++)
        R(A_ending_customVars + i) = a;
    R(A_ending_customVars + 1) = romTable(ROM_LA75D, levelNumber);
}

/* LA6BC: the cathedral's domes take off one after another: when the frame
 * count in ending_customVars reaches a dome's start time (LA735) the dome
 * starts to rise, slowly at first, then faster, with a jet below it. */
static void LA6BC(void)
{
    for (uint8_t x = 0; x != 0x04; x++) { /* LA6BE */
        if (romTable(ROM_LA735, x) == ending_customVars && R(A_ending_customVars + 3 + x)) {
            R(A_ending_customVars + 3 + x)--;
            ending_customVars++;
        }
    }
    ending_currentSprite = 0;
    do { /* LA6D9 */
        uint8_t x = ending_currentSprite;
        if (R(A_ending_customVars + 3 + x)) {
            generalCounter = R(A_ending_customVars + 3 + x);
            if (romTable(ROM_LA73D, x) != generalCounter) { /* off the ground: rising */
                soundEffectSlot0Init = 0x03;
                generalCounter--;
                if (generalCounter < 0xA0)
                    generalCounter--;
            }
            /* LA6F7 */
            R(A_ending_customVars + 3 + x) = generalCounter;
            spriteYOffset = generalCounter;
            spriteXOffset = romTable(ROM_domeNumberToXOffsetTable, x);
            spriteIndexInOamContentLookup = romTable(ROM_domeNumberToSpriteTable, x);
            loadSpriteIntoOamStaging();
            x = ending_currentSprite;
            generalCounter = R(A_ending_customVars + 3 + x);
            if (romTable(ROM_LA73D, x) != generalCounter) {
                spriteXOffset = (uint8_t)(romTable(ROM_LA745, x) + spriteXOffset);
                spriteIndexInOamContentLookup = (uint8_t)(((frameCounter[0] & 0x02) >> 1) + 0x51);
                loadSpriteIntoOamStaging();
            }
        }
        ending_currentSprite++; /* LA72C */
    } while (ending_currentSprite != 0x04);
}

/*
 * ending_typeBConcert: level 9's ending. The performers for each height
 * are added in a chain: height 5 has Mario, Luigi and Peach, then Bowser,
 * Donkey Kong, Samus, Link and Kid Icarus, which every height has. Each
 * steps between two frames on frameCounter; Luigi jumps on a ten-step cycle
 * counted in `ending`, and ending_customVars counts his jumps (it is also
 * the domes' clock in LA6BC).
 */
static void ending_typeBConcert(void)
{
    switch_s_plus_2a(player1_startHeight, (uint16_t)(ROM_ending_typeBConcert + 5));
    switch (player1_startHeight) {
    case 5: /* @marioLuigiPeach */
        spriteXOffset = 0xC8;
        spriteYOffset = 0x47;
        spriteIndexInOamContentLookup = (uint8_t)((((frameCounter[0] & 0x08) >> 1) >> 2) + 0x21);
        loadSpriteIntoOamStaging();
        spriteXOffset = 0xA0;
        spriteIndexInOamContentLookup = 0x27;
        spriteYOffset = romTable(ROM_marioFrameToYOffsetTable, (uint8_t)((frameCounter[0] & 0x18) >> 3));
        if (spriteYOffset != 0x97)
            spriteIndexInOamContentLookup = 0x28;
        loadSpriteIntoOamStaging(); /* @marioFrame1 */
        for (;;) { /* @luigiCalculateFrame */
            spriteXOffset = 0xC0;
            if ((ending >> 3) != 0x0A)
                break;
            ending = 0;
            ending_customVars++;
        }
        { /* @luigiFrameCalculated */
            uint8_t x = ending >> 3;
            spriteYOffset = romTable(ROM_luigiFrameToYOffsetTable, x);
            spriteIndexInOamContentLookup = romTable(ROM_luigiFrameToSpriteTable, x);
            loadSpriteIntoOamStaging();
            ending++;
        }
        /* fall through */
    case 4: /* @bowser */
        spriteXOffset = 0x30;
        spriteYOffset = 0xA7;
        spriteIndexInOamContentLookup = (uint8_t)((((frameCounter[0] & 0x10) >> 1) >> 3) + 0x1F);
        loadSpriteIntoOamStaging();
        /* fall through */
    case 3: /* @donkeyKong */
        spriteXOffset = 0x40;
        spriteYOffset = 0x77;
        spriteIndexInOamContentLookup = (uint8_t)(((frameCounter[0] & 0x10) >> 4) + 0x1D);
        loadSpriteIntoOamStaging();
        /* fall through */
    case 2: /* @samus */
        spriteXOffset = 0xA8;
        spriteYOffset = 0xD7;
        spriteIndexInOamContentLookup = (uint8_t)((((frameCounter[0] & 0x10) >> 1) >> 3) + 0x1A);
        loadSpriteIntoOamStaging();
        /* fall through */
    case 1: /* @link */
        spriteXOffset = 0xC8;
        spriteYOffset = 0xD7;
        spriteIndexInOamContentLookup = (uint8_t)((((frameCounter[0] & 0x10) >> 1) >> 3) + 0x18);
        loadSpriteIntoOamStaging();
        /* fall through */
    case 0: /* @kidIcarus */
        spriteXOffset = 0x28;
        spriteYOffset = 0x77;
        spriteIndexInOamContentLookup = (uint8_t)((((frameCounter[0] & 0x10) >> 1) >> 3) + 0x16);
        loadSpriteIntoOamStaging();
        LA6BC();
        break;
    }
}

/* ending_typeBCathedralSetSprite: steps the parade's two-frame animation at
 * the level's speed (ending counts frames, ending_customVars is the frame). */
static void ending_typeBCathedralSetSprite(void)
{
    ending++;
    uint8_t x = levelNumber;
    if (romTable(ROM_ending_typeBCathedralAnimSpeed, x) == ending) {
        ending_customVars ^= 0x01;
        ending = 0;
    }
    /* @skipAnimSpriteChange */
    spriteIndexInOamContentLookup = (uint8_t)(romTable(ROM_ending_typeBCathedralSpriteTable, x) + ending_customVars);
}

/* ending_computeTypeBCathedralYTableIndex: levelNumber * 6 + ending_currentSprite. */
static uint8_t ending_computeTypeBCathedralYTableIndex(void)
{
    generalCounter = (uint8_t)(levelNumber << 1);
    return (uint8_t)((uint8_t)((uint8_t)(generalCounter << 1) + generalCounter) + ending_currentSprite);
}

/*
 * ending_typeBCathedral: the parade for levels other than 9: up to six
 * performers (one more per starting height) cross the screen in front of the
 * cathedral, each entering when the one before reaches its mark
 * (ending_typeBCathedralXTable). They move ending_typeBCathedralVectorTable
 * pixels every ending_typeBCathedralFrameDelayTable frames. A performer whose
 * x is still LA767's "waiting" value is not drawn.
 */
static void ending_typeBCathedral(void)
{
    ending_typeBCathedralSetSprite();
    ending_typeBCathedralFrameDelayCounter++;
    ending_currentSprite = 0;
    for (;;) { /* @spriteLoop */
        generalCounter = romTable(ROM_LA767, levelNumber);
        uint8_t x = ending_currentSprite;
        uint8_t a = R(A_ending_customVars + 1 + x);
        if (a != generalCounter) {
            spriteXOffset = a;
            x = ending_computeTypeBCathedralYTableIndex();
            spriteYOffset = romTable(ROM_ending_typeBCathedralYTable, x);
            loadSpriteIntoOamStaging();
            if (romTable(ROM_ending_typeBCathedralFrameDelayTable, levelNumber) == ending_typeBCathedralFrameDelayCounter) {
                spriteXOffset = (uint8_t)(romTable(ROM_ending_typeBCathedralVectorTable, levelNumber) + spriteXOffset);
                R(A_ending_customVars + 1 + ending_currentSprite) = spriteXOffset;
                x = ending_computeTypeBCathedralYTableIndex();
                if (romTable(ROM_ending_typeBCathedralXTable, x) == spriteXOffset)
                    R(A_ending_customVars + 1 + (uint8_t)(ending_currentSprite + 1)) = romTable(ROM_LA75D, levelNumber);
            }
        }
        /* @continue */
        generalCounter = ending_currentSprite;
        if (ending_currentSprite == startHeight)
            break;
        ending_currentSprite++;
    }
    /* @done */
    if (romTable(ROM_ending_typeBCathedralFrameDelayTable, levelNumber) == ending_typeBCathedralFrameDelayCounter)
        ending_typeBCathedralFrameDelayCounter = 0;
}

/* render_ending: one frame of the ending animation's sprites (ending_typeA or
 * ending_typeB). */
static void ending_typeA(void);
static void render_ending(void)
{
    if (!gameType) {
        ending_typeA();
        return;
    }
    /* ending_typeB */
    if (levelNumber == 0x09)
        ending_typeBConcert(); /* @typeBConcert */
    else
        ending_typeBCathedral();
}

/* ------------------------------------------------------------------------ */
/* A-type                                                                   */
/* ------------------------------------------------------------------------ */

/*
 * selectEndingScreen: picks the rocket by score: ending 0 from 30,000, 1
 * from 50,000, 2 from 70,000, 3 from 100,000, 4 (the cathedral, which also
 * patches the picture) from 120,000. ending_customVars is the rocket's y,
 * ending_customVars+1 the countdown before launch.
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
                    R(A_patchToPpuAddr + 1) = ROM_ending_patchToPpu_typeAOver120k >> 8;
                    R(A_patchToPpuAddr) = (uint8_t)ROM_ending_patchToPpu_typeAOver120k;
                    patchToPpu();
                }
            }
        }
    }
    /* ending_selected */
    ending_customVars = romTable(ROM_LAA2A, ending);
    R(A_ending_customVars + 1) = 0;
}

/* endingAnimationA: A-type's ending: waits for the rocket to leave the
 * screen, then for Start. */
static void endingAnimationA(void)
{
    updateAudioWaitForNmiAndDisablePpuRendering();
    disableNmi();
    changeCHRBank0(CHR_TYPEA_ENDING);
    changeCHRBank1(CHR_TYPEA_ENDING);
    bulkCopyToPpu(0xA936, ROM_type_a_ending_nametable);
    bulkCopyToPpu(0xA93B, ROM_ending_palette);
    selectEndingScreen();
    waitForVBlankAndEnableNmi();
    updateAudioWaitForNmiAndResetOamStaging();
    updateAudioWaitForNmiAndEnablePpuRendering();
    updateAudioWaitForNmiAndResetOamStaging();
    renderMode = 0x04;
    setMusicTrack(0x0A);
    render_endingUnskippable(ENDING_SLEEP_TIME_1);
    do { /* @endingSleep */
        render_ending();
        updateAudioWaitForNmiAndResetOamStaging();
    } while (ending_customVars || newlyPressedButtons_player1 != BUTTON_START);
}

/* ending_typeA: one frame of the rocket: drawn with its flickering jet; after
 * a countdown of $F0 frames it rises, a pixel every other frame near the
 * ground, then faster, until its y reaches 0. */
static void ending_typeA(void)
{
    if (ending_customVars == 0x00)
        return; /* LAA10 */
    spriteYOffset = ending_customVars;
    uint8_t x = ending;
    spriteXOffset = romTable(ROM_rocketToXOffsetTable, x);
    spriteIndexInOamContentLookup = romTable(ROM_rocketToSpriteTable, x);
    loadSpriteIntoOamStaging();
    generalCounter = (uint8_t)(ending << 1);
    x = (uint8_t)(((frameCounter[0] & 0x02) >> 1) + generalCounter);
    spriteIndexInOamContentLookup = romTable(ROM_rocketToJetSpriteTable, x);
    spriteXOffset = (uint8_t)(romTable(ROM_rocketToJetXOffsetTable, ending) + spriteXOffset);
    loadSpriteIntoOamStaging();
    if (R(A_ending_customVars + 1) != 0xF0) {
        R(A_ending_customVars + 1)++; /* LAA0E */
        return;
    }
    if (ending_customVars >= 0xB0 && (frameCounter[0] & 0x01))
        return; /* LAA0B */
    soundEffectSlot0Init = 0x03; /* LA9FC */
    ending_customVars--;
    if (ending_customVars < 0x80)
        ending_customVars--;
}
