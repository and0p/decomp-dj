/*
 * The NES hardware the libretro core gives the port: the PPU (drawn a whole
 * frame at a time), the MMC1 mapper's CHR banking and mirroring, and the two
 * standard controllers. The APU is not emulated: writes to it are ignored.
 */
#ifndef TETRIS_NES_H
#define TETRIS_NES_H

#include <stdint.h>

#define NES_WIDTH 256
#define NES_HEIGHT 240

struct nes {
    /* cartridge */
    const uint8_t *chr; /* 16 KB of CHR ROM */
    uint8_t mmc1Shift, mmc1Count, mmc1Control, mmc1Chr0, mmc1Chr1, mmc1Prg;

    /* PPU */
    uint8_t ctrl, mask, status, oamAddr, readBuffer;
    uint16_t v, t; /* the PPU's address registers ("loopy" v and t) */
    uint8_t fineX, w;
    uint8_t ciram[0x800]; /* the console's 2 KB of nametable RAM */
    uint8_t palette[0x20];
    uint8_t oam[0x100];

    /* controllers: buttons in NES order (bit 7 A ... bit 0 right) */
    uint8_t buttons[2], shift[2], strobe;
};

void nes_power(struct nes *n, const uint8_t *chr);
/* Register access for the port's hw_read and hw_write. `ram` is the CPU's
 * 2 KB, for sprite DMA. */
uint8_t nes_read(struct nes *n, uint16_t addr);
void nes_write(struct nes *n, uint16_t addr, uint8_t value, const uint8_t *ram);
/* The vertical blank starts: sets the PPUSTATUS flag. */
void nes_vblank(struct nes *n);
/* Draws the frame from the PPU's current state, as XRGB8888. */
void nes_render(const struct nes *n, uint32_t *out /* NES_WIDTH * NES_HEIGHT */);

#endif
