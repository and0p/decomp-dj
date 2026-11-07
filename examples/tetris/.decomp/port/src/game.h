/*
 * Shared definitions for the Tetris port.
 *
 * Conventions used throughout:
 *
 * - Every routine is named after its label in the reference disassembly
 *   (CelestialAmber/TetrisNESDisasm) and sits in the file for its area.
 * - Game state lives only in tetris_ram, at the original addresses (ram_map.h).
 *   C locals stand in for the 6502's registers, never for RAM variables, so the
 *   RAM matches the original's byte for byte at the end of every frame.
 * - Scratch bytes (tmp1, generalCounter, ...) are written exactly as the original
 *   writes them, even where C would not need them: their leftover values are
 *   part of the game's state.
 * - Where the original passes a result in a register that a caller tests (the
 *   main loop compares A with gameModeState, for example), the C function
 *   returns that value. Comments say so.
 * - Tables, nametables and music come from the PRG ROM by address
 *   (rom_labels.h). The source carries no data from the ROM.
 * - Arithmetic is 8-bit and wraps, as on the 6502: values are uint8_t, and
 *   comparisons that the original makes with BPL/BMI after CMP use sign_set().
 */
#ifndef TETRIS_GAME_H
#define TETRIS_GAME_H

#include <stdint.h>
#include "tetris/port.h"
#include "tetris/hw.h"
#include "ram_map.h"
#include "rom_labels.h"

/* ---- Hardware registers (constants.asm) ---- */
#define PPUCTRL 0x2000
#define PPUMASK 0x2001
#define PPUSTATUS 0x2002
#define OAMADDR 0x2003
#define OAMDATA 0x2004
#define PPUSCROLL 0x2005
#define PPUADDR 0x2006
#define PPUDATA 0x2007
#define SQ1_VOL 0x4000
#define SQ1_SWEEP 0x4001
#define SQ1_LO 0x4002
#define SQ1_HI 0x4003
#define SQ2_VOL 0x4004
#define SQ2_SWEEP 0x4005
#define SQ2_LO 0x4006
#define SQ2_HI 0x4007
#define TRI_LINEAR 0x4008
#define TRI_LO 0x400A
#define TRI_HI 0x400B
#define NOISE_VOL 0x400C
#define NOISE_LO 0x400E
#define NOISE_HI 0x400F
#define DMC_FREQ 0x4010
#define DMC_RAW 0x4011
#define DMC_START 0x4012
#define DMC_LEN 0x4013
#define OAMDMA 0x4014
#define SND_CHN 0x4015
#define JOY1 0x4016
#define JOY2_APUFC 0x4017
#define MMC1_Control 0x9FFF
#define MMC1_CHR0 0xBFFF
#define MMC1_CHR1 0xDFFF
#define MMC1_PRG 0xFFF0

#define BUTTON_A 0x80
#define BUTTON_B 0x40
#define BUTTON_SELECT 0x20
#define BUTTON_START 0x10
#define BUTTON_UP 0x08
#define BUTTON_DOWN 0x04
#define BUTTON_LEFT 0x02
#define BUTTON_RIGHT 0x01
#define BUTTON_DPAD (BUTTON_UP | BUTTON_DOWN | BUTTON_LEFT | BUTTON_RIGHT)

#define MMC1_4KCHR_32KPRG_H_MIRROR 0x10
#define PRG_32K_BANK 0x00

#define CHR_TITLE_MENU 0x00
#define CHR_TYPEB_ENDING 0x01
#define CHR_TYPEA_ENDING 0x02
#define CHR_GAME 0x03

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
#define SFX_LEVELUP_INIT 0x06
#define SFX_LINE_COMPLETE_INIT 0x05
#define SFX_LINECLEAR_INIT 0x04
#define SFX_TETRIS_INIT 0x05

/* Tiles */
#define tile1 0x7B
#define tile2 0x7C
#define tile3 0x7D
#define tileEmpty 0xEF  /* blank tile used for empty squares */
#define tileHidden 0xFF /* blank tile used to hide minos */

/* Orientation IDs (currentPiece and nextPiece hold these) */
#define tDown 0x02
#define jDown 0x07
#define zHoriz 0x08
#define oFixed 0x0A
#define sHoriz 0x0B
#define lDown 0x0E
#define iHoriz 0x12
#define hidden 0x13

/* ---- Memory access ---- */

/* RAM by address, for indexed accesses such as `sta soundEffectSlot0Init,x`. */
#define R(addr) (tetris_ram[(uint16_t)(addr) & 0x7FF])

/* A byte of PRG ROM by CPU address ($8000-$FFFF). */
uint8_t prg8(uint16_t addr);
/* A little-endian word of PRG ROM by CPU address. */
uint16_t prg16(uint16_t addr);
/* A 16-bit pointer stored in RAM (low byte first), as `(zp),y` reads it. */
static inline uint16_t ptr16(uint16_t zp) { return (uint16_t)(R(zp) | (R(zp + 1) << 8)); }
/* The CPU bus: RAM, ROM, or hardware. Used for the original's indirect accesses,
 * which can reach any of them. */
uint8_t peek(uint16_t addr);
void poke(uint16_t addr, uint8_t value);

/* True when bit 7 of (a - m) is set: the N flag after `CMP`, which the original
 * tests with BMI (set) and BPL (clear). */
static inline int sign_set(uint8_t a, uint8_t m) { return ((uint8_t)(a - m) & 0x80) != 0; }

/* ADC and SBC with the carry flag, for the places where the original lets a
 * carry from one addition flow into the next. */
static inline uint8_t adc8(uint8_t a, uint8_t m, uint8_t *c)
{
    unsigned s = (unsigned)a + m + *c;
    *c = (uint8_t)(s >> 8);
    return (uint8_t)s;
}
static inline uint8_t sbc8(uint8_t a, uint8_t m, uint8_t *c)
{
    unsigned d = (unsigned)a - m - (1u - *c);
    *c = (uint8_t)(d < 0x100); /* carry set means no borrow */
    return (uint8_t)d;
}
/* ASL and ROL with the carry flag. */
static inline uint8_t asl8(uint8_t a, uint8_t *c)
{
    *c = a >> 7;
    return (uint8_t)(a << 1);
}
static inline uint8_t rol8(uint8_t a, uint8_t *c)
{
    uint8_t out = a >> 7;
    a = (uint8_t)((a << 1) | *c);
    *c = out;
    return a;
}

/* A signed byte from a ROM table (orientationTable's offsets are -2..2). */
static inline uint8_t romTable(uint16_t table, uint8_t index) { return prg8((uint16_t)(table + index)); }

/* ---- Routines (by file) ---- */

/* system.c */
void initRam(void);
void render(void);
uint8_t branchOnGameMode(void);
void switch_s_plus_2a(uint8_t index, uint16_t table);
void bulkCopyToPpu(uint16_t callSite, uint16_t data);
void copyToPpu(void);
void generateNextPseudorandomNumber(uint8_t x, uint8_t y);
void copyOamStagingToOam(void);
void pollController_actualRead(void);
void addExpansionPortInputAsControllerInput(void);
void pollController(void);
void diffOldAndNewButtons(void);
void memset_ppu_page_and_more(uint8_t a, uint8_t x, uint8_t y);
void LAA82(uint8_t a);
void memset_page(uint8_t a, uint8_t x, uint8_t y);
void setMMC1Control(uint8_t a);
void changeCHRBank0(uint8_t a);
void changeCHRBank1(uint8_t a);
void changePRGBank(uint8_t a);
void incrementInitRam(void);
uint8_t updateAudioWaitForNmiAndResetOamStaging(void);
uint8_t updateAudioAndWaitForNmi(void);
void updateAudioWaitForNmiAndDisablePpuRendering(void);
void updateAudioWaitForNmiAndEnablePpuRendering(void);
void waitForVBlankAndEnableNmi(void);
void disableNmi(void);
void copyCurrentScrollAndCtrlToPPU(void);
void sleep_for_14_vblanks(void);
void sleep_for_a_vblanks(uint8_t a);
void twoDigsToPPU(uint8_t a);

/* menus.c */
uint8_t gameMode_legalScreen(void);
uint8_t gameMode_titleScreen(void);
void render_mode_legal_and_title_screens(void);
uint8_t gameMode_gameTypeMenu(void);
uint8_t gameMode_levelMenu(void);
void render_mode_menu_screens(void);
uint8_t gameMode_startDemo(void);
void pollControllerButtons(void);
void demoButtonsTable_indexIncr(void);
void setMusicTrack(uint8_t a);

/* play.c */
uint8_t gameMode_playAndEndingHighScore(void);
uint8_t chooseNextTetrimino(void);
void incrementPieceStat(uint8_t a);
uint8_t isPositionValid(void);
void updatePlayfield(void);
void updateMusicSpeed(void);
uint8_t stageSpriteForNextPiece(void);
void stageSpriteForCurrentPiece(void);
uint8_t loadSpriteIntoOamStaging(void);

/* render.c */
void render_mode_play_and_demo(void);
void copyPlayfieldRowToVRAM(void);
void updateLineClearingAnimation(void);
void updatePaletteForLevel(void);

/* highscore.c */
void showHighScores(void);
void handleHighScoreIfNecessary(void);
void render_mode_congratulations_screen(void);

/* ending.c */
void endingAnimation(void);
void render_mode_ending_animation(void);

/* audio.c */
void updateAudio(void);
void updateAudio2(void);
void LE006(void);

#endif
