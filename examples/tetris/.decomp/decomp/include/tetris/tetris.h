/*
 * Tetris (NES, USA), decompiled to C as a library.
 *
 * The game logic of the original, routine by routine, translated from the
 * annotated disassembly (CelestialAmber/TetrisNESDisasm). It keeps everything
 * that decides what the game does from one frame to the next: the menus and
 * their timers, the pieces and the random number generator, scoring, the
 * demo, the high score table, the endings' timing. It drops what only drives
 * the NES hardware: drawing to the PPU, sprites, the mapper and the audio
 * engine. It needs no ROM.
 *
 * Use:
 *
 *     struct tetris t;
 *     tetris_init(&t);                       // power on
 *     for (;;) {
 *         tetris_tick(&t, pad1, pad2);       // one NES frame
 *         draw(&t);                          // read t.ram.v and t.screen
 *     }
 *
 * All state is in struct tetris, so copying it is a save state.
 */
#ifndef TETRIS_TETRIS_H
#define TETRIS_TETRIS_H

#include <stddef.h>
#include <stdint.h>

#include "tetris/ram.h"

/* Buttons, in the order the NES controller reports them. */
enum {
    TETRIS_BUTTON_A = 0x80,
    TETRIS_BUTTON_B = 0x40,
    TETRIS_BUTTON_SELECT = 0x20,
    TETRIS_BUTTON_START = 0x10,
    TETRIS_BUTTON_UP = 0x08,
    TETRIS_BUTTON_DOWN = 0x04,
    TETRIS_BUTTON_LEFT = 0x02,
    TETRIS_BUTTON_RIGHT = 0x01,
};

/* How many routines can be part-way through, waiting for a frame. */
#define TETRIS_THREADS 32

/*
 * Where the game is in its code, which on the NES is the program counter and
 * the stack. The routines that wait for a frame in the middle are written as
 * resumable functions (see src/game.h); each remembers where it stopped in
 * `resume`, as the address that the original's JSR left on the stack.
 */
struct tetris_ctl {
    uint16_t resume[TETRIS_THREADS]; /* by routine: where to continue, 0 = not running */
    uint8_t dispatched[3];           /* the handler each jump table picked, while it runs */
    uint8_t waiting;                 /* what the game waits for: 1 the NMI, 2 a vertical blank */
    uint8_t framesBehind;            /* frames that passed during long work with NMIs off */
    uint32_t cycles;                 /* estimated CPU cycles since this frame's vertical blank */
    uint8_t pad[2];                  /* the controllers' buttons this frame */
};

/*
 * What the TV shows, where the RAM alone doesn't say. The game draws the
 * playfield to the screen a few rows a frame, and animates line clears on the
 * screen only, so the picture lags the playfield in RAM. Tile numbers are the
 * original's: 0xEF empty, 0x7B-0x7D the three block styles, 0x4F the game
 * over curtain, 0x00-0x09 digits and 0x0A-0x23 letters (B-type's success
 * message is written into the playfield).
 */
struct tetris_screen {
    uint8_t field[2][200]; /* the playfields as drawn, row by row, 10 squares a row */
    uint8_t paused;            /* the pause screen: playfield hidden, "PAUSE" shown */
};

struct tetris {
    union tetris_ram ram;
    struct tetris_ctl ctl;
    struct tetris_screen screen;
};

/* Power on. RAM starts zeroed. */
void tetris_init(struct tetris *t);

/* Runs one frame: the vertical blank's work (the NMI), then the game until it
 * waits for the next frame. pad1 and pad2 are the buttons held (TETRIS_BUTTON_*). */
void tetris_tick(struct tetris *t, uint8_t pad1, uint8_t pad2);

/*
 * Takes over a running original: `nesRam` is the 2 KB of RAM of a real Tetris
 * (USA) at the start of a frame, before its NMI. Works out from the return
 * addresses on the stack page which routines are part-way through and where.
 * Returns 0 on success. Returns 1, with a reason in `why`, when the original is
 * somewhere the decomp can't take over from (between frames' work, as while
 * it draws a new screen with NMIs off); `t` is then unchanged.
 */
int tetris_load_nes_snapshot(struct tetris *t, const uint8_t nesRam[0x800], char *why, size_t whyLen);

/*
 * Compares the game with a real Tetris (USA) at the same frame: `nesRam` is
 * its 2 KB of RAM at the start of the frame, as for tetris_load_nes_snapshot.
 * Judges where the code is (the original's stack against the decomp's resume
 * points, so a frame lost on one side is caught on that frame) and every byte
 * of RAM the decomp keeps. Returns 0 if they match (`why` may then note what
 * wasn't compared), 1 if not, with the difference in `why`. See parity.c.
 */
int tetris_check_nes_parity(const struct tetris *t, const uint8_t nesRam[0x800], char *why, size_t whyLen);

/*
 * Game data that a front end needs to draw the game.
 *
 * tetris_piece_squares: the four squares of a piece orientation (0-0x12, as
 * in currentPiece and nextPiece; 0x13 is "no piece"), each as its row and
 * column offset from the piece's centre square and its tile (0x7B-0x7D, the
 * three block styles, or 0xFF hidden). Returns 0 for an orientation past the
 * table.
 *
 * tetris_piece_spawn_orientation: the orientation piece type 0-6 (T, J, Z,
 * O, S, L, I, as in statsByType) appears in.
 */
int tetris_piece_squares(uint8_t orientation, int8_t rows[4], int8_t cols[4], uint8_t tiles[4]);
uint8_t tetris_piece_spawn_orientation(uint8_t type);

#endif
