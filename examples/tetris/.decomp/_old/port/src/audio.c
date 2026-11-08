/*
 * The audio engine: sound effects in five slots (0 noise, 1 square 1, 2 and
 * 4 the DMC, 3 triangle) and the music player. updateAudio runs once a frame
 * from the main thread, before it waits for the NMI.
 *
 * Sound effects are requested by writing a number to soundEffectSlotNInit;
 * the engine runs that effect's init routine, then its playing routine every
 * frame while soundEffectSlotNPlaying holds the number. The routines are
 * found through tables of addresses in ROM (computeSoundEffMethod), and the
 * original jumps to them indirectly; this port looks the address up the same
 * way and calls the C function for it (callAudioRoutine).
 *
 * Music: musicTrack requests a track (1-10; $FF stops). Each track has a
 * script per channel: a list of routines, each a stream of bytecode
 * instructions (notes, durations, loops, instrument changes), interpreted
 * by updateMusicFrame_updateChannel.
 *
 * Several routines here pass results to each other in registers. The C
 * versions take and return those values; the comments name the registers.
 */
#include "game.h"

#define wr(addr, v) poke((addr), (uint8_t)(v))

/* Arrays the original indexes by slot or channel with a scalar label. */
#define soundEffectSlotInit(x) R(A_soundEffectSlot0Init + (x))
#define soundEffectSlotPlaying(x) R(A_soundEffectSlot0Playing + (x))
#define soundEffectSlotSecondaryCounter(x) R(A_soundEffectSlot0SecondaryCounter + (x))
#define soundEffectSlotTertiaryCounter(x) R(A_soundEffectSlot0TertiaryCounter + (x))
#define soundEffectSlotTmp(x) R(A_soundEffectSlot0Tmp + (x))

static void callAudioRoutine(uint16_t addr);
static void updateMusic(void);
static void muteAudio(void);

/* ------------------------------------------------------------------------ */
/* Entry points                                                             */
/* ------------------------------------------------------------------------ */

/* updateAudio2 (canon updateAudio): silences everything: jumps to
 * soundEffectSlot2_makesNoSound. */
void updateAudio2(void) { callAudioRoutine(ROM_soundEffectSlot2_makesNoSound); }

/* LE1D8 (via LE006): power-on audio setup: enables the four tone channels. */
void LE006(void)
{
    wr(SND_CHN, 0x0F);
    soundRngSeed[0] = 0x55;
    callAudioRoutine(ROM_soundEffectSlot2_makesNoSound);
}

/* ------------------------------------------------------------------------ */
/* Sound effect plumbing                                                    */
/* ------------------------------------------------------------------------ */

/*
 * copyToApuChannel: copies four bytes of channel settings from $E1yy (the
 * sound effects' init data; `data` is its address) to the APU channel at
 * $4000+a. AUDIOTMP1-4 hold the two pointers.
 */
static void copyToApuChannel(uint8_t a, uint16_t data)
{
    AUDIOTMP1 = a;
    AUDIOTMP2 = 0x40;
    AUDIOTMP3 = (uint8_t)data;
    AUDIOTMP4 = ROM_soundEffectSlot0_gameOverCurtainInitData >> 8;
    for (uint8_t y = 0; y != 0x04; y++) /* @copyByte */
        wr((uint16_t)(ptr16(A_AUDIOTMP1) + y), prg8((uint16_t)(ptr16(A_AUDIOTMP3) + y)));
}
static void copyToSq1Channel(uint16_t data) { copyToApuChannel(0x00, data); }
static void copyToSq2Channel(uint16_t data) { copyToApuChannel(0x04, data); }
static void copyToTriChannel(uint16_t data) { copyToApuChannel(0x08, data); }
static void copyToNoiseChannel(uint16_t data) { copyToApuChannel(0x0C, data); }

/*
 * computeSoundEffMethod: looks up entry a - 1 of the table at $E0xx
 * (AUDIOTMP1 is the low byte) and leaves the routine's address in
 * AUDIOTMP3/4. Past entry 16 it gives $E091, an RTS. currentAudioSlot ends
 * up as a (it is counted down during the search, then restored).
 */
static void computeSoundEffMethod(uint8_t a)
{
    currentAudioSlot = a;
    AUDIOTMP2 = ROM_soundEffectSlot0Init_table >> 8;
    uint8_t y = 0;
    for (;;) { /* @whileYNot2TimesA */
        currentAudioSlot--;
        if (!currentAudioSlot) { /* @copyAddr */
            AUDIOTMP3 = prg8((uint16_t)(ptr16(A_AUDIOTMP1) + y));
            y++;
            AUDIOTMP4 = prg8((uint16_t)(ptr16(A_AUDIOTMP1) + y));
            break;
        }
        y = (uint8_t)(y + 2);
        if (y == 0x22) {
            AUDIOTMP3 = 0x91;
            AUDIOTMP4 = ROM_soundEffectSlot0Init_table >> 8;
            break;
        }
    }
    currentAudioSlot = a; /* @ret */
}

/* advanceAudioSlotFrame: counts a frame of the current slot's effect.
 * Returns nonzero (the original's Z flag) when the effect's frame count has
 * been reached, and restarts the count. */
static int advanceAudioSlotFrame(void)
{
    uint8_t x = currentSoundEffectSlot;
    R(A_soundEffectSlot0FrameCounter + x)++;
    if (R(A_soundEffectSlot0FrameCounter + x) != R(A_soundEffectSlot0FrameCount + x))
        return 0;
    R(A_soundEffectSlot0FrameCounter + x) = 0;
    return 1;
}

/*
 * updateSoundEffectSlotShared: runs slot x: its init routine (from the table
 * at initTable) if soundEffectSlotNInit is set this frame, else its playing
 * routine (from playingTable) if one is playing.
 */
static void updateSoundEffectSlotShared(uint8_t x, uint16_t initTable, uint16_t playingTable)
{
    AUDIOTMP1 = (uint8_t)initTable;
    currentSoundEffectSlot = x;
    uint8_t a = soundEffectSlotInit(x);
    if (!a) { /* @primaryIsEmpty */
        a = soundEffectSlotPlaying(x);
        if (!a)
            return;
        AUDIOTMP1 = (uint8_t)playingTable;
    }
    computeSoundEffMethod(a); /* @computeAndExecute */
    callAudioRoutine(ptr16(A_AUDIOTMP3));
}

static void updateSoundEffectSlot2(void)
{
    updateSoundEffectSlotShared(0x02, ROM_soundEffectSlot2Init_table, ROM_soundEffectSlot2Init_table);
}
static void updateSoundEffectSlot3(void)
{
    updateSoundEffectSlotShared(0x03, ROM_soundEffectSlot3Init_table, ROM_soundEffectSlot3Playing_table);
}
/* updateSoundEffectSlot1: square 1, unless the (unused) DMC slot 4 is busy. */
static void updateSoundEffectSlot1(void)
{
    if (soundEffectSlot4Playing)
        return;
    updateSoundEffectSlotShared(0x01, ROM_soundEffectSlot1Init_table, ROM_soundEffectSlot1Playing_table);
}
static void updateSoundEffectSlot0(void)
{
    updateSoundEffectSlotShared(0x00, ROM_soundEffectSlot0Init_table, ROM_soundEffectSlot0Playing_table);
}

/*
 * initSoundEffectShared: starts an effect in the current slot: a frames per
 * step, the channel set up from the init data at `data` (none if its low
 * byte is 0), and the slot's counters cleared. The playing routine runs from
 * next frame.
 */
static void initSoundEffectShared(uint8_t a, uint16_t data)
{
    uint8_t x = currentSoundEffectSlot;
    R(A_soundEffectSlot0FrameCount + x) = a;
    R(0x06C7 + x) = x;
    if ((uint8_t)data) {
        switch (x) {
        case 0: copyToNoiseChannel(data); break; /* @slot0 */
        case 1: copyToSq1Channel(data); break;
        case 2: copyToSq2Channel(data); break;
        case 3: copyToTriChannel(data); break;
        default: return;
        }
    }
    /* @continue */
    soundEffectSlotPlaying(x) = currentAudioSlot;
    R(A_soundEffectSlot0FrameCounter + x) = 0;
    soundEffectSlotSecondaryCounter(x) = 0;
    soundEffectSlotTertiaryCounter(x) = 0;
    soundEffectSlotTmp(x) = 0;
    resetSq12ForMusic = 0;
}

/* ------------------------------------------------------------------------ */
/* Silence and pause                                                        */
/* ------------------------------------------------------------------------ */

/* muteAudio: volume 0 on every channel. */
static void muteAudio(void)
{
    wr(DMC_RAW, 0);
    wr(SQ1_VOL, 0x10);
    wr(SQ2_VOL, 0x10);
    wr(NOISE_VOL, 0x10);
    wr(TRI_LINEAR, 0);
}

/* LE253: stops every effect and the music (soundEffectSlot0Playing to
 * currentlyPlayingMusicTrack). */
static void LE253(void)
{
    musicChanInhibit[0] = 0;
    musicChanInhibit[1] = 0;
    musicChanInhibit[2] = 0;
    musicStagingNoiseLo = 0;
    resetSq12ForMusic = 0;
    for (uint8_t y = 0; y != 0x06; y++) /* LE265 */
        soundEffectSlotPlaying(y) = 0;
}

/* muteAudioAndClearTriControl */
static void muteAudioAndClearTriControl(void)
{
    muteAudio();
    wr(DMC_RAW, 0);
    musicChanControl[2] = 0;
}

/* soundEffectSlot2_makesNoSound: stops all sound. Slot 2's effect 1, and
 * what updateAudio2 does. */
static void soundEffectSlot2_makesNoSound(void)
{
    LE253();
    muteAudioAndClearTriControl();
}

/* updateAudio_pause: while paused (musicStagingNoiseHi = 5) the music stops
 * and square 1 plays the pause jingle: six short notes alternating between
 * two pitches, then silence. */
static void updateAudio_pause(void)
{
    if (!audioInitialized) { /* initAudioAndMarkInited */
        audioInitialized++;
        muteAudio();
        musicPauseSoundEffectLengthCounter = 0;
        return;
    }
    if (musicPauseSoundEffectLengthCounter == 0x12)
        return;
    if ((musicPauseSoundEffectLengthCounter & 0x03) == 0x03) {
        musicPauseSoundEffectCounter++;
        copyToSq1Channel((musicPauseSoundEffectCounter & 0x01) ? ROM_music_pause_sq1_odd : ROM_music_pause_sq1_even);
    }
    musicPauseSoundEffectLengthCounter++; /* @incAndRet */
}

/*
 * updateAudio: the frame's audio. Disables the APU frame IRQ, plays the pause
 * jingle if paused, otherwise runs the four effect slots and the music, then
 * clears this frame's requests ($06F0-$06F5: the five Init bytes and
 * musicTrack).
 */
void updateAudio(void)
{
    wr(JOY2_APUFC, 0xC0);
    if (musicStagingNoiseHi == 0x05) {
        updateAudio_pause();
        return;
    }
    audioInitialized = 0;
    musicPauseSoundEffectCounter = 0;
    updateSoundEffectSlot2();
    updateSoundEffectSlot0();
    updateSoundEffectSlot3();
    updateSoundEffectSlot1();
    updateMusic();
    for (uint8_t x = 0x06; x; x--) /* @clearSoundEffectSlotsInit */
        R(0x06EF + x) = 0;
}

/* ------------------------------------------------------------------------ */
/* Slot 0: noise                                                            */
/* ------------------------------------------------------------------------ */

static void soundEffectSlot0_makesNoSound(void) { initSoundEffectShared(0x10, 0x0000); }

static void stopSoundEffectSlot0(void)
{
    soundEffectSlot0Playing = 0;
    wr(NOISE_VOL, 0x10);
}

/* advanceSoundEffectSlot0WithoutUpdate: an effect with nothing to update but
 * its length. */
static void advanceSoundEffectSlot0WithoutUpdate(void)
{
    if (advanceAudioSlotFrame())
        stopSoundEffectSlot0();
}

/* soundEffectSlot0_gameOverCurtainInit: the curtain's rumble. */
static void soundEffectSlot0_gameOverCurtainInit(void)
{
    initSoundEffectShared(0x40, ROM_soundEffectSlot0_gameOverCurtainInitData);
}

/* soundEffectSlot0_endingRocketInit: the rockets' roar. */
static void soundEffectSlot0_endingRocketInit(void)
{
    initSoundEffectShared(0x20, ROM_soundEffectSlot0_endingRocketInitData);
}

/* getSoundEffectNoiseNibble: nibble soundEffectSlot0SecondaryCounter of the
 * table at $E1xx (noiselo_table or noisevol_table), high nibble first. */
static uint8_t getSoundEffectNoiseNibble(uint16_t table)
{
    AUDIOTMP1 = (uint8_t)table;
    AUDIOTMP2 = ROM_noiselo_table >> 8;
    uint8_t x = soundEffectSlot0SecondaryCounter;
    AUDIOTMP5 = prg8((uint16_t)(ptr16(A_AUDIOTMP1) + (x >> 1)));
    if (x & 0x01)
        return AUDIOTMP5 & 0x0F;
    return AUDIOTMP5 >> 4; /* @shift4 */
}

/* updateSoundEffectSlot0_apu: the curtain's rumble: each frame a new noise
 * period and volume from noiselo_table and noisevol_table. */
static void updateSoundEffectSlot0_apu(void)
{
    if (advanceAudioSlotFrame()) {
        stopSoundEffectSlot0();
        return;
    }
    /* updateSoundEffectNoiseAudio */
    wr(NOISE_LO, getSoundEffectNoiseNibble(ROM_noiselo_table)); /* loadNoiseLo, setNoiseLo */
    wr(NOISE_VOL, getSoundEffectNoiseNibble(ROM_noisevol_table) | 0x10);
    soundEffectSlot0SecondaryCounter++;
}

/* ------------------------------------------------------------------------ */
/* Slot 1: square 1, the game's main effects                                */
/* ------------------------------------------------------------------------ */

/* LE33B: is a line clear, Tetris or level up effect playing? (Z set in the
 * original.) Moves, rotations and locks don't interrupt those. */
static int LE33B(void)
{
    uint8_t a = soundEffectSlot1Playing;
    return a == 0x04 || a == 0x06 || a == 0x09 || a == 0x0A;
}

static void soundEffectSlot1Playing_stop(void)
{
    wr(SQ1_VOL, 0x10);
    musicChanInhibit[0] = 0;
    soundEffectSlot1Playing = 0;
    resetSq12ForMusic++; /* the music must rewrite square 1 */
}

static void soundEffectSlot1Playing_advance(void)
{
    if (advanceAudioSlotFrame())
        soundEffectSlot1Playing_stop();
}

/* soundEffectSlot1_chirpChirpPlaying (unused): eight frames of volume steps,
 * a second note, then off. */
static void soundEffectSlot1_chirpChirpPlaying(void)
{
    if (soundEffectSlot1TertiaryCounter) {
        soundEffectSlot1SecondaryCounter++;
        if (soundEffectSlot1SecondaryCounter == 0x16)
            soundEffectSlot1Playing_stop();
        return;
    }
    /* @stage1 */
    wr(SQ1_VOL, romTable(ROM_soundEffectSlot1_chirpChirpSq1Vol_table, soundEffectSlot1SecondaryCounter & 0x03));
    soundEffectSlot1SecondaryCounter++;
    if (soundEffectSlot1SecondaryCounter != 0x08)
        return;
    soundEffectSlot1TertiaryCounter++;
    copyToSq1Channel(ROM_soundEffectSlot1Playing_chirpChirpStage2);
}

/* soundEffectSlot1_chirpChirpInit (unused). It passes on whatever A held,
 * which is the effect's number (8). */
static void soundEffectSlot1_chirpChirpInit(void)
{
    initSoundEffectShared(currentAudioSlot, ROM_soundEffectSlot1_chirpChirpInitData);
}

static void soundEffectSlot1_lockTetriminoInit(void)
{
    if (LE33B())
        return;
    initSoundEffectShared(0x0F, ROM_soundEffectSlot1_lockTetriminoInitData);
}

static void soundEffectSlot1_shiftTetriminoInit(void)
{
    if (LE33B())
        return;
    initSoundEffectShared(0x02, ROM_soundEffectSlot1_shiftTetriminoInitData);
}

/* soundEffectSlot1_menuOptionSelectPlaying: two notes. */
static void soundEffectSlot1_menuOptionSelectPlaying(void)
{
    if (!advanceAudioSlotFrame())
        return;
    soundEffectSlot1SecondaryCounter++;
    if (soundEffectSlot1SecondaryCounter == 0x02) {
        soundEffectSlot1Playing_stop();
        return;
    }
    copyToSq1Channel(ROM_soundEffectSlot1Playing_menuOptionSelectStage2); /* @stage2 */
}

static void soundEffectSlot1_menuOptionSelectInit(void)
{
    initSoundEffectShared(0x03, ROM_soundEffectSlot1_menuOptionSelectInitData);
}

/* soundEffectSlot1_rotateTetriminoPlaying: three short notes. */
static void soundEffectSlot1_rotateTetriminoPlaying(void)
{
    if (!advanceAudioSlotFrame())
        return;
    uint8_t a = soundEffectSlot1SecondaryCounter++;
    if (soundEffectSlot1SecondaryCounter == 0 || a == 0x02) { /* the first test never passes */
        copyToSq1Channel(ROM_soundEffectSlot1Playing_rotateTetriminoStage3); /* @stage3 */
    } else if (a == 0x01) {
        copyToSq1Channel(ROM_soundEffectSlot1_rotateTetriminoInitData); /* @stage2 */
    } else if (a == 0x03) {
        soundEffectSlot1Playing_stop();
    }
}

/* soundEffectSlot1_rotateTetriminoInit: starts, and runs its first frame at once. */
static void soundEffectSlot1_rotateTetriminoInit(void)
{
    if (LE33B())
        return;
    initSoundEffectShared(0x04, ROM_soundEffectSlot1_rotateTetriminoInitData); /* via LE417 */
    soundEffectSlot1_rotateTetriminoPlaying();
}

static void soundEffectSlot1_lineClearingInit(void);

/*
 * LE442: a step of the line clear family of effects: reloads square 1 from
 * the effect's data, then sets the pitch from soundEffectSlot1_lineClearing_lo
 * (the effect's start, in soundEffectSlot1TertiaryCounter, plus the step) and
 * the volume from soundEffectSlot1_lineClearing_vol. At volume 0 the effect
 * ends; a Tetris's chime then continues with the line clearing sound.
 */
static void LE442(uint16_t data)
{
    copyToSq1Channel(data);
    uint8_t y = (uint8_t)(soundEffectSlot1TertiaryCounter + soundEffectSlot1SecondaryCounter);
    wr(SQ1_LO, romTable(ROM_soundEffectSlot1_lineClearing_lo, y));
    uint8_t vol = romTable(ROM_soundEffectSlot1_lineClearing_vol, soundEffectSlot1SecondaryCounter);
    wr(SQ1_VOL, vol);
    if (vol) {
        soundEffectSlot1SecondaryCounter++; /* LE46F */
        return;
    }
    if (soundEffectSlot1Playing == 0x04) {
        currentAudioSlot = 0x09;
        soundEffectSlot1_lineClearingInit();
        return;
    }
    soundEffectSlot1Playing_stop(); /* LE46C */
}

static void soundEffectSlot1_tetrisAchievedInit(void)
{
    initSoundEffectShared(SFX_TETRIS_INIT, ROM_soundEffectSlot1_tetrisAchievedInitData);
    soundEffectSlot1TertiaryCounter = 0x10; /* LE437 */
}

static void soundEffectSlot1_tetrisAchievedPlaying(void)
{
    if (advanceAudioSlotFrame())
        LE442(ROM_soundEffectSlot1_tetrisAchievedInitData);
}

static void soundEffectSlot1_lineCompletedInit(void)
{
    initSoundEffectShared(SFX_LINE_COMPLETE_INIT, ROM_soundEffectSlot1_lineCompletedInitData);
    soundEffectSlot1TertiaryCounter = 0x08;
}

static void soundEffectSlot1_lineCompletedPlaying(void)
{
    if (advanceAudioSlotFrame())
        LE442(ROM_soundEffectSlot1_lineCompletedInitData);
}

static void soundEffectSlot1_lineClearingInit(void)
{
    initSoundEffectShared(SFX_LINECLEAR_INIT, ROM_soundEffectSlot1_lineClearingInitData);
    soundEffectSlot1TertiaryCounter = 0x00;
}

static void soundEffectSlot1_lineClearingPlaying(void)
{
    if (advanceAudioSlotFrame())
        LE442(ROM_soundEffectSlot1_lineClearingInitData);
}

/* soundEffectSlot1_menuScreenSelectInit: a falling tone, starting at the
 * period in its data's third byte. */
static void soundEffectSlot1_menuScreenSelectInit(void)
{
    initSoundEffectShared(0x03, ROM_soundEffectSlot1_menuScreenSelectInitData);
    soundEffectSlot1SecondaryCounter = prg8(ROM_soundEffectSlot1_menuScreenSelectInitData + 2);
}

/* soundEffectSlot1_menuScreenSelectPlaying: four steps, each lowering the
 * period by a sixteenth of itself, plus one. */
static void soundEffectSlot1_menuScreenSelectPlaying(void)
{
    if (!advanceAudioSlotFrame())
        return;
    soundEffectSlot1TertiaryCounter++;
    if (soundEffectSlot1TertiaryCounter == 0x04) {
        soundEffectSlot1Playing_stop();
        return;
    }
    /* LE493 */
    soundEffectSlot1Tmp = soundEffectSlot1SecondaryCounter >> 4;
    soundEffectSlot1SecondaryCounter = (uint8_t)(soundEffectSlot1SecondaryCounter - soundEffectSlot1Tmp - 1);
    wr(SQ1_LO, soundEffectSlot1SecondaryCounter);
    wr(SQ1_HI, 0x28); /* LE4AC */
}

/* soundEffectSlot1_levelUpPlaying: a run of notes from
 * soundEffectSlot1_levelUp_lo, ending at a 0. */
static void soundEffectSlot1_levelUpPlaying(void)
{
    if (!advanceAudioSlotFrame())
        return;
    uint8_t y = soundEffectSlot1SecondaryCounter++;
    uint8_t a = romTable(ROM_soundEffectSlot1_levelUp_lo, y);
    if (!a) {
        soundEffectSlot1Playing_stop(); /* LE4E9 */
        return;
    }
    wr(SQ1_LO, a);
    wr(SQ1_HI, 0x28);
}

static void soundEffectSlot1_levelUpInit(void)
{
    initSoundEffectShared(SFX_LEVELUP_INIT, ROM_soundEffectSlot1_levelUpInitData);
}

/* ------------------------------------------------------------------------ */
/* Slots 2 and 4: the DMC (unused), and slot 3: triangle                   */
/* ------------------------------------------------------------------------ */

/* LE51B: plays a DMC sample: length a, start y, rate x. */
static void LE51B(uint8_t a, uint8_t y, uint8_t x)
{
    wr(DMC_LEN, a);
    wr(DMC_START, y);
    wr(DMC_FREQ, x);
    wr(SND_CHN, 0x0F);
    wr(DMC_RAW, 0);
    wr(SND_CHN, 0x1F);
}
/* soundEffectSlot2_mediumBuzz (unused; assembled as .byte in the disassembly) */
static void soundEffectSlot2_mediumBuzz(void) { LE51B(0x3F, 0x60, 0x0F); }
/* soundEffectSlot2_lowBuzz (unused) */
static void soundEffectSlot2_lowBuzz(void) { LE51B(0x3F, 0x60, 0x0E); }

/* soundEffectSlot3_donk (unused) */
static void soundEffectSlot3_donk(void) { initSoundEffectShared(0x02, ROM_soundEffectSlot3_donkInitData); }

static void soundEffectSlot3Playing_stop(void)
{
    wr(TRI_LINEAR, 0);
    musicChanInhibit[2] = 0;
    soundEffectSlot3Playing = 0;
    wr(TRI_HI, 0x18);
}

static void soundEffectSlot3Playing_advance(void)
{
    if (advanceAudioSlotFrame())
        soundEffectSlot3Playing_stop();
}

/* updateSoundEffectSlot3_apu: steps through trilo_table's pitches. */
static void updateSoundEffectSlot3_apu(void)
{
    if (!advanceAudioSlotFrame())
        return;
    uint8_t y = soundEffectSlot3SecondaryCounter++;
    uint8_t a = romTable(ROM_trilo_table, y);
    if (!a) {
        soundEffectSlot3Playing_stop();
        return;
    }
    wr(TRI_LO, a);
    soundEffectSlot3TertiaryCounter = a;
    wr(TRI_HI, prg8(ROM_soundEffectSlot3_fallingAlienInitData + 3));
}

/* soundEffectSlot3_fallingAlien (unused) */
static void soundEffectSlot3_fallingAlien(void)
{
    initSoundEffectShared(0x06, ROM_soundEffectSlot3_fallingAlienInitData);
    soundEffectSlot3TertiaryCounter = prg8(ROM_soundEffectSlot3_fallingAlienInitData + 2);
}

/*
 * callAudioRoutine: the original's `jmp (AUDIOTMP3)`: runs the routine at a
 * ROM address taken from one of the sound effect tables.
 */
static void callAudioRoutine(uint16_t addr)
{
    switch (addr) {
    case ROM_soundEffectSlot0_makesNoSound: soundEffectSlot0_makesNoSound(); break;
    case ROM_soundEffectSlot0_gameOverCurtainInit: soundEffectSlot0_gameOverCurtainInit(); break;
    case ROM_soundEffectSlot0_endingRocketInit: soundEffectSlot0_endingRocketInit(); break;
    case ROM_advanceSoundEffectSlot0WithoutUpdate: advanceSoundEffectSlot0WithoutUpdate(); break;
    case ROM_updateSoundEffectSlot0_apu: updateSoundEffectSlot0_apu(); break;
    case ROM_soundEffectSlot1_menuOptionSelectInit: soundEffectSlot1_menuOptionSelectInit(); break;
    case ROM_soundEffectSlot1_menuScreenSelectInit: soundEffectSlot1_menuScreenSelectInit(); break;
    case ROM_soundEffectSlot1_shiftTetriminoInit: soundEffectSlot1_shiftTetriminoInit(); break;
    case ROM_soundEffectSlot1_tetrisAchievedInit: soundEffectSlot1_tetrisAchievedInit(); break;
    case ROM_soundEffectSlot1_rotateTetriminoInit: soundEffectSlot1_rotateTetriminoInit(); break;
    case ROM_soundEffectSlot1_levelUpInit: soundEffectSlot1_levelUpInit(); break;
    case ROM_soundEffectSlot1_lockTetriminoInit: soundEffectSlot1_lockTetriminoInit(); break;
    case ROM_soundEffectSlot1_chirpChirpInit: soundEffectSlot1_chirpChirpInit(); break;
    case ROM_soundEffectSlot1_lineClearingInit: soundEffectSlot1_lineClearingInit(); break;
    case ROM_soundEffectSlot1_lineCompletedInit: soundEffectSlot1_lineCompletedInit(); break;
    case ROM_soundEffectSlot1_menuOptionSelectPlaying: soundEffectSlot1_menuOptionSelectPlaying(); break;
    case ROM_soundEffectSlot1_menuScreenSelectPlaying: soundEffectSlot1_menuScreenSelectPlaying(); break;
    case ROM_soundEffectSlot1Playing_advance: soundEffectSlot1Playing_advance(); break;
    case ROM_soundEffectSlot1_tetrisAchievedPlaying: soundEffectSlot1_tetrisAchievedPlaying(); break;
    case ROM_soundEffectSlot1_rotateTetriminoPlaying: soundEffectSlot1_rotateTetriminoPlaying(); break;
    case ROM_soundEffectSlot1_levelUpPlaying: soundEffectSlot1_levelUpPlaying(); break;
    case ROM_soundEffectSlot1_chirpChirpPlaying: soundEffectSlot1_chirpChirpPlaying(); break;
    case ROM_soundEffectSlot1_lineClearingPlaying: soundEffectSlot1_lineClearingPlaying(); break;
    case ROM_soundEffectSlot1_lineCompletedPlaying: soundEffectSlot1_lineCompletedPlaying(); break;
    case ROM_soundEffectSlot3_fallingAlien: soundEffectSlot3_fallingAlien(); break;
    case ROM_soundEffectSlot3_donk: soundEffectSlot3_donk(); break;
    case ROM_updateSoundEffectSlot3_apu: updateSoundEffectSlot3_apu(); break;
    case ROM_soundEffectSlot3Playing_advance: soundEffectSlot3Playing_advance(); break;
    case ROM_soundEffectSlot2_makesNoSound: soundEffectSlot2_makesNoSound(); break;
    case ROM_soundEffectSlot2_lowBuzz: soundEffectSlot2_lowBuzz(); break;
    case ROM_soundEffectSlot2_mediumBuzz: soundEffectSlot2_mediumBuzz(); break;
    default:
        /* $E091 (an RTS, for numbers past the end of a table), or bytes read
         * past a table's end, which only an out-of-range request reaches. */
        break;
    }
}

/* ------------------------------------------------------------------------ */
/* Music                                                                    */
/* ------------------------------------------------------------------------ */

static void updateMusicFrame(void);

/*
 * loadMusicTrack: starts track musicTrack_dec + 1. Copies its 10-byte header
 * from musicDataTable (note and duration table offsets, then a script pointer
 * per channel; $FFxx means the channel is unused), clears the channels'
 * positions, and points each channel at its script's first routine
 * (derefNextAddr).
 */
static void loadMusicTrack(void)
{
    muteAudioAndClearTriControl();
    currentlyPlayingMusicTrack = currentAudioSlot;
    uint8_t y = romTable(ROM_musicDataTableIndex, musicTrack_dec);
    for (uint8_t x = 0; x != 0x0A; x++, y++) /* @copyByteToMusicData */
        R(A_musicDataNoteTableOffset + x) = romTable(ROM_musicDataTable, y);
    for (int i = 0; i < 4; i++)
        musicChanNoteDurationRemaining[i] = 0x01;
    music_unused2 = 0;
    for (y = 0x08; y; y--) /* @zeroFillDeref: musicDataChanPtrOff and musicDataChanInstructionOffset */
        R(A_musicDataChanPtrDeref + 7 + y) = 0;
    for (uint8_t x = 0; x != 0x08; x = (uint8_t)(x + 2)) { /* derefNextAddr */
        musicChanTmpAddr[0] = musicDataChanPtr[x];
        uint8_t hi = musicDataChanPtr[x + 1];
        if (hi == 0xFF) { /* copyFFFFToDeref */
            musicDataChanPtrDeref[x] = 0xFF;
            musicDataChanPtrDeref[x + 1] = 0xFF; /* storeDeref1AndContinue */
            continue;
        }
        musicChanTmpAddr[1] = hi;
        uint8_t off = musicDataChanPtrOff[0]; /* always channel 0's: all are 0 here */
        musicDataChanPtrDeref[x] = peek((uint16_t)(ptr16(A_musicChanTmpAddr) + off));
        musicDataChanPtrDeref[x + 1] = peek((uint16_t)(ptr16(A_musicChanTmpAddr) + (uint8_t)(off + 1)));
    }
}

/* updateMusic: starts a newly requested track, stops the music for $FF, or
 * plays a frame of the current one. */
static void updateMusic(void)
{
    uint8_t a = musicTrack;
    if (a == 0xFF) {
        soundEffectSlot2_makesNoSound(); /* updateMusic_noSoundJmp */
        return;
    }
    if (a == 0x00) { /* @checkIfAlreadyPlaying */
        if (currentlyPlayingMusicTrack)
            updateMusicFrame();
        return;
    }
    currentAudioSlot = a;
    musicTrack_dec = (uint8_t)(a - 1);
    musicStagingSq1Sweep[0] = 0x7F;
    musicStagingSq1Sweep[1] = 0x7F;
    loadMusicTrack();
    updateMusicFrame(); /* @updateFrame */
}

/* musicLoOffset_setLo: applies a pitch offset to the channel's staged period
 * (AUDIOTMP2), unless a sound effect has the channel. */
static void musicLoOffset_setLo(uint8_t x, uint8_t a)
{
    if (musicChanInhibit[x])
        return;
    wr((uint16_t)(SQ1_LO + musicChannelOffset), (uint8_t)(a + AUDIOTMP2));
}

/* musicLoOffset_setLoAndSaveFrameCounter: as musicLoOffset_setLo, after
 * saving y (the table position, which may have wrapped) as the frame count. */
static void musicLoOffset_setLoAndSaveFrameCounter(uint8_t x, uint8_t y, uint8_t a)
{
    musicChanLoFrameCounter[x] = y;
    musicLoOffset_setLo(x, a);
}

/*
 * updateMusicFrame_setChanLoOffset: vibrato and pitch bends. The high three
 * bits of the channel's control byte (AUDIOTMP1) choose a table of period
 * offsets, stepped by musicChanLoFrameCounter (AUDIOTMP3): $20 and $E0 a
 * trill that loops, $A0 a trill after a slow start, $40 a fall that bounces,
 * $60 a constant -2, $80 and $C0 a fall from +9 (none for note $46).
 */
static void updateMusicFrame_setChanLoOffset(uint8_t x)
{
    AUDIOTMP3 = musicChanLoFrameCounter[x];
    uint8_t a, y;
    switch (AUDIOTMP1) {
    case 0xA0: /* @A */
        y = (AUDIOTMP3 == 0x2B) ? 0x21 : AUDIOTMP3;
        musicLoOffset_setLoAndSaveFrameCounter(x, y, romTable(ROM_loOffSlowStartTrillTable, y));
        return;
    case 0x60: /* musicLoOffset_minus2_6 */
        musicLoOffset_setLo(x, 0xFE);
        return;
    case 0x40: /* musicLoOffset_4 */
        y = AUDIOTMP3;
        a = (y >= 0x10) ? 0xF6 : romTable(ROM_loOffDescendToNeg11BounceToNeg9Table, y);
        musicLoOffset_setLo(x, a);
        return;
    case 0x80:
    case 0xC0: /* musicLoOffset_8AndC */
        y = (AUDIOTMP3 == 0x31) ? 0x27 : AUDIOTMP3;
        a = romTable(ROM_loOff9To0FallTable, y);
        if (musicChanNote[x] == 0x46)
            a = 0x00;
        musicLoOffset_setLoAndSaveFrameCounter(x, y, a); /* LE613 */
        return;
    default: /* @2AndE */
        y = (AUDIOTMP3 == 0x0A) ? 0x00 : AUDIOTMP3;
        musicLoOffset_setLoAndSaveFrameCounter(x, y, romTable(ROM_loOffTrillNeg2To2Table, y));
        return;
    }
}

/* updateMusicFrame_setChanLo: the pitch effect for channel x, every frame of
 * a note. Not for the noise channel, nor in track 1. */
static void updateMusicFrame_setChanLo(uint8_t x)
{
    if (currentlyPlayingMusicTrack == 0x01 || x == 0x03)
        return;
    uint8_t a = musicChanControl[x] & 0xE0;
    if (!a)
        return;
    AUDIOTMP1 = a;
    if (musicChanNote[x] != 0x02) { /* note 2 is a rest */
        AUDIOTMP2 = R(A_musicStagingSq1Lo + musicChannelOffset);
        updateMusicFrame_setChanLoOffset(x);
    }
    musicChanLoFrameCounter[x]++; /* @incAndRet */
}

/* initSq12IfTrashedBySoundEffect: after a sound effect used a square channel,
 * restores the music's settings (resetSq12ForMusic: 1 square 1, 2 both). */
static void initSq12IfTrashedBySoundEffect(void)
{
    uint8_t a = resetSq12ForMusic;
    if (!a)
        return;
    if (a != 0x01) {
        wr(SQ2_SWEEP, 0x7F);
        wr(SQ2_LO, musicStagingSq2Lo);
        wr(SQ2_HI, musicStagingSq2Hi);
    }
    wr(SQ1_SWEEP, 0x7F); /* @setSq1 */
    wr(SQ1_LO, musicStagingSq1Lo);
    wr(SQ1_HI, musicStagingSq1Hi);
    resetSq12ForMusic = 0;
}

/*
 * updateMusicFrame_setChanVol: the volume envelope for the square channels
 * (x = 0, 1). The low five bits of the control byte choose an envelope from
 * musicChanVolControlTable, a string of volume nibbles stepped once a frame;
 * $FF holds the channel's volume, $F0 silences it. The nibble replaces the
 * low half of musicChanVolume.
 */
static void updateMusicFrame_setChanVol(uint8_t x)
{
    if (x >= 0x02)
        return;
    uint8_t ctrl = musicChanControl[x] & 0x1F;
    if (!ctrl)
        return;
    AUDIOTMP2 = ctrl;
    uint8_t y;
    int advance = 1;
    if (musicChanNote[x] == 0x02) {
        y = 0x10; /* @muteAndAdvanceFrame */
    } else {
        y = 0;
        while (--AUDIOTMP2) /* @controlMinus1Times2_storeToY */
            y = (uint8_t)(y + 2);
        AUDIOTMP3 = romTable(ROM_musicChanVolControlTable, y); /* @loadFromTable */
        AUDIOTMP4 = romTable(ROM_musicChanVolControlTable, (uint8_t)(y + 1));
        AUDIOTMP5 = peek((uint16_t)(ptr16(A_AUDIOTMP3) + (musicChanVolFrameCounter[x] >> 1)));
        if (AUDIOTMP5 == 0xFF) { /* @constVolAtEnd */
            y = musicChanVolume[x];
            if (!y)
                y = 0x10;
            advance = 0;
        } else if (AUDIOTMP5 == 0xF0) { /* @muteAtEnd */
            y = 0x10;
            advance = 0;
        } else {
            if (!(musicChanVolFrameCounter[x] & 0x01))
                AUDIOTMP5 >>= 4;
            AUDIOTMP1 = AUDIOTMP5 & 0x0F; /* @useNibbleFromTable */
            y = (musicChanVolume[x] & 0xF0) | AUDIOTMP1;
        }
    }
    if (advance)
        musicChanVolFrameCounter[x]++; /* @advanceFrameAndSetVol */
    if (musicChanInhibit[x]) /* @setVol */
        return;
    wr((uint16_t)(SQ1_VOL + musicChannelOffset), y);
}

/* musicGetNextInstructionByte: the next byte of channel x's routine. */
static uint8_t musicGetNextInstructionByte(uint8_t x)
{
    uint8_t y = musicDataChanInstructionOffset[x]++;
    return peek((uint16_t)(ptr16(A_musicChanTmpAddr) + y));
}

/* playNoise: a drum: noise volume, period and length from noises_table, unless
 * sound effect slot 0 has the noise channel. */
static void playNoise(uint8_t y)
{
    if (soundEffectSlot0Playing)
        return;
    wr(NOISE_VOL, romTable(ROM_noises_table, y));
    wr(NOISE_LO, romTable(ROM_noises_table, (uint8_t)(y + 1)));
    wr(NOISE_HI, romTable(ROM_noises_table, (uint8_t)(y + 2)));
}

/* playDmc: a sampled drum, chosen by the top bits of the noise channel's
 * note ($40 or $80). It sets up the sample but leaves the DMC channel
 * disabled in SND_CHN, so the samples are never heard. */
static void playDmc(uint8_t y)
{
    uint8_t a;
    switch (y & 0xC0) {
    case 0x40: /* @loadDmc0 */
        AUDIOTMP2 = 0x0E;
        a = 0x07;
        y = 0x00;
        break;
    case 0x80: /* @loadDmc1 */
        AUDIOTMP2 = 0x0E;
        a = 0x0F;
        y = 0x02;
        break;
    default:
        return;
    }
    wr(DMC_LEN, a); /* @loadIntoDmc */
    wr(DMC_START, y);
    if (R(0x06F7))
        return;
    wr(DMC_FREQ, AUDIOTMP2);
    wr(SND_CHN, 0x0F);
    wr(DMC_RAW, 0);
    wr(SND_CHN, 0x1F);
}

/* Results of updateMusicFrame_progLoadRoutine. */
enum { PROG_CONTINUE, PROG_END };

/*
 * updateMusicFrame_progLoadRoutine: points channel x at the routine at entry
 * y of its script (musicChanTmpAddr). An entry $FFxx jumps to another script
 * (progLoadNextScript, for looping songs); $00xx ends the music
 * (progEnd: everything is silenced and the frame's music update ends).
 */
static int updateMusicFrame_progLoadRoutine(uint8_t x, uint8_t y)
{
    for (;;) {
        uint8_t x2 = (uint8_t)(x << 1);
        musicDataChanPtrDeref[x2] = peek((uint16_t)(ptr16(A_musicChanTmpAddr) + y));
        y++;
        uint8_t hi = peek((uint16_t)(ptr16(A_musicChanTmpAddr) + y));
        musicDataChanPtrDeref[x2 + 1] = hi;
        if (hi == 0x00) { /* updateMusicFrame_progEnd */
            soundEffectSlot2_makesNoSound();
            return PROG_END;
        }
        if (hi != 0xFF)
            break;
        /* updateMusicFrame_progLoadNextScript */
        y++;
        musicDataChanPtr[x2] = peek((uint16_t)(ptr16(A_musicChanTmpAddr) + y));
        y++;
        musicDataChanPtr[x2 + 1] = peek((uint16_t)(ptr16(A_musicChanTmpAddr) + y));
        musicChanTmpAddr[0] = musicDataChanPtr[x2];
        musicChanTmpAddr[1] = musicDataChanPtr[x2 + 1];
        y = 0;
        musicDataChanPtrOff[x] = 0;
    }
    musicDataChanInstructionOffset[x] = 0;
    musicChanNoteDurationRemaining[x] = 0x01;
    return PROG_CONTINUE;
}

/* updateMusicFrame_progNextRoutine: channel x's routine has ended: on to the
 * next entry of its script. */
static int updateMusicFrame_progNextRoutine(uint8_t x)
{
    uint8_t x2 = (uint8_t)(x << 1);
    musicChanTmpAddr[0] = musicDataChanPtr[x2];
    musicChanTmpAddr[1] = musicDataChanPtr[x2 + 1];
    musicDataChanPtrOff[x] = (uint8_t)(musicDataChanPtrOff[x] + 2);
    return updateMusicFrame_progLoadRoutine(x, musicDataChanPtrOff[x]);
}

/*
 * The instruction interpreter for channel x, when its note has run out. Runs
 * instructions until one starts a note:
 *
 *   $00        end of routine: next routine of the script
 *   $9F c v    set the control byte (envelope and pitch effect) and volume
 *   $9E d      set the duration table offset
 *   $9C n      set the note table offset (transpose; bit 7 = down)
 *   $C0-$FE    start a loop: run the following instructions (n & $3F) times
 *   $FF        end of loop
 *   $B0-$BF    a duration (noteDurationTable, plus the offset), then a note
 *   other      a note (an index into noteToWaveTable), with the last duration
 *
 * For the noise channel a note is a drum (playNoise, playDmc). Returns
 * PROG_END if the music ended.
 */
static int processChannelInstructions(uint8_t x)
{
    uint8_t a, y;
    for (;;) { /* @processChannelInstruction */
        a = musicGetNextInstructionByte(x);
        if (a == 0x00) {
            if (updateMusicFrame_progNextRoutine(x) == PROG_END)
                return PROG_END;
            /* progLoadRoutine continues at updateMusicFrame_updateChannel with
             * the duration at 1: the new routine's first instruction is read
             * now, after reloading musicChanTmpAddr. */
            uint8_t x2 = (uint8_t)(x << 1);
            musicChanTmpAddr[0] = musicDataChanPtrDeref[x2];
            musicChanTmpAddr[1] = musicDataChanPtrDeref[x2 + 1];
            if (musicDataChanPtrDeref[x2 + 1] == 0xFF)
                return PROG_CONTINUE; /* (not reachable: progLoadRoutine handled $FF) */
            musicChanNoteDurationRemaining[x]--;
            if (musicChanNoteDurationRemaining[x]) {
                updateMusicFrame_setChanVol(x);
                updateMusicFrame_setChanLo(x);
                return PROG_CONTINUE;
            }
            musicChanVolFrameCounter[x] = 0;
            musicChanLoFrameCounter[x] = 0;
            continue;
        }
        if (a == 0x9F) { /* @setControlAndVolume */
            musicChanControl[x] = musicGetNextInstructionByte(x);
            musicChanVolume[x] = musicGetNextInstructionByte(x);
            continue;
        }
        if (a == 0x9E) { /* @setDurationOffset */
            musicDataDurationTableOffset = musicGetNextInstructionByte(x);
            continue;
        }
        if (a == 0x9C) { /* @setNoteOffset */
            musicDataNoteTableOffset = musicGetNextInstructionByte(x);
            continue;
        }
        y = a;
        if (a == 0xFF) { /* @endLoop */
            if (!musicChanProgLoopCounter[x])
                continue;
            musicChanProgLoopCounter[x]--;
            musicDataChanInstructionOffset[x] = musicDataChanInstructionOffsetBackup[x];
            if (musicDataChanInstructionOffset[x])
                continue;
            /* with a backup of 0 the original falls into @startForLoop, with $FF */
        } else if ((a & 0xC0) != 0xC0) {
            break; /* @noteAndMaybeDuration */
        }
        /* @startForLoop */
        musicChanProgLoopCounter[x] = (uint8_t)((y & 0x3F) - 1);
        musicDataChanInstructionOffsetBackup[x] = musicDataChanInstructionOffset[x];
    }

    /* @noteAndMaybeDuration */
    if ((y & 0xB0) == 0xB0) {
        y = (uint8_t)((y & 0x0F) + musicDataDurationTableOffset);
        y = romTable(ROM_noteDurationTable, y);
        musicChanNoteDuration[x] = y;
        if (x == 0x02) { /* @applyDurationForTri: the triangle's linear counter */
            a = musicChanControl[2] & 0x1F;
            if (!a) {
                a = musicChanControl[2] & 0xC0;
                if (a == 0x00 || a == 0xC0) { /* @useDuration */
                    a = y;
                    if (a) { /* @durationToLinearClock: (duration - 1) * 4 quarter frames, at most $3C */
                        a = (uint8_t)((uint8_t)(a + 0xFF) << 2);
                        if (a >= 0x3C)
                            a = 0x3C;
                    } else {
                        a = 0xFF; /* @highCtrlImpliesOn */
                    }
                } else {
                    a = 0xFF;
                }
            }
            musicChanVolume[2] = a; /* @setTriVolume */
        }
        y = musicGetNextInstructionByte(x); /* @loadNextAsNote */
    }

    /* @processNote */
    musicChanNote[x] = y;
    if (x == 0x03) { /* @playDmcAndNoise */
        playDmc(y);
        playNoise(y & 0x3F);
        musicChanNoteDurationRemaining[x] = musicChanNoteDuration[x];
        return PROG_CONTINUE;
    }
    uint8_t off = musicChannelOffset;
    a = romTable(ROM_noteToWaveTable, (uint8_t)(y + 1));
    if (a) { /* a note, not a rest: stage its period */
        if (musicDataNoteTableOffset & 0x80) {
            AUDIOTMP4 = musicDataNoteTableOffset & 0x7F;
            a = (uint8_t)(y - AUDIOTMP4 - 1); /* one more than the offset: the carry is clear */
        } else { /* @signMagnitudeIsPositive */
            a = (uint8_t)(y + musicDataNoteTableOffset);
        }
        y = a; /* @noteOffsetApplied */
        R(A_musicStagingSq1Lo + off) = romTable(ROM_noteToWaveTable, (uint8_t)(y + 1));
        a = romTable(ROM_noteToWaveTable, y) | 0x08;
        R(A_musicStagingSq1Hi + off) = a;
    }
    /* @determineVolume */
    if (a) {
        AUDIOTMP1 = musicChanVolume[x]; /* @loadVolume */
    } else {
        AUDIOTMP1 = (x == 0x02) ? 0x00 : 0x10; /* a rest */
    }
    /* @checkChanControl: (the inhibit test never passes: musicChanInhibit is always 0) */
    musicChanInhibit[x]--;
    if (x == musicChanInhibit[x]) {
        musicChanInhibit[x]++; /* @channelInhibited */
    } else {
        musicChanInhibit[x]++;
        uint8_t v;
        if (x == 0x02 || !(musicChanControl[x] & 0x1F)) {
            v = AUDIOTMP1; /* @useDirectVolume */
        } else if (AUDIOTMP1 == 0x10) {
            v = 0x10;
        } else {
            v = AUDIOTMP1 & 0xF0; /* an envelope will supply the low half */
            if (!v)
                v = AUDIOTMP1;
        }
        wr((uint16_t)(SQ1_VOL + off), v); /* @setMmio */
        wr((uint16_t)(SQ1_SWEEP + off), musicStagingSq1Sweep[x]);
        wr((uint16_t)(SQ1_LO + off), R(A_musicStagingSq1Lo + off));
        wr((uint16_t)(SQ1_HI + off), R(A_musicStagingSq1Hi + off));
    }
    musicChanNoteDurationRemaining[x] = musicChanNoteDuration[x]; /* @copyDurationToRemaining */
    return PROG_CONTINUE;
}

/*
 * updateMusicFrame: a frame of music, for each of the four channels
 * (musicChannelOffset is the channel's APU register offset, 0/4/8/12).
 * While a note lasts it runs the channel's envelope and pitch effect; when
 * it runs out, the next instructions.
 */
static void updateMusicFrame(void)
{
    initSq12IfTrashedBySoundEffect();
    musicChannelOffset = 0;
    uint8_t x = 0;
    for (;;) { /* updateMusicFrame_updateChannel */
        uint8_t x2 = (uint8_t)(x << 1);
        musicChanTmpAddr[0] = musicDataChanPtrDeref[x2];
        musicChanTmpAddr[1] = musicDataChanPtrDeref[x2 + 1];
        if (musicDataChanPtrDeref[x2 + 1] != 0xFF) { /* $FFxx: the channel is unused */
            musicChanNoteDurationRemaining[x]--;
            if (musicChanNoteDurationRemaining[x]) { /* @updateChannelFrame */
                updateMusicFrame_setChanVol(x);
                updateMusicFrame_setChanLo(x);
            } else {
                musicChanVolFrameCounter[x] = 0;
                musicChanLoFrameCounter[x] = 0;
                if (processChannelInstructions(x) == PROG_END)
                    return;
            }
        }
        /* updateMusicFrame_incSlot */
        x++;
        if (x == 0x04)
            return;
        musicChannelOffset = (uint8_t)(musicChannelOffset + 0x04);
    }
}
