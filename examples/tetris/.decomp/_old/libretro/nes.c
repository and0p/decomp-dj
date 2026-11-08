/*
 * A small model of the NES hardware that Tetris touches. Timing is by whole
 * frames: the picture is drawn from the PPU's state at the end of a frame,
 * which is enough for a game that changes nothing mid-frame.
 */
#include "nes.h"

#include <math.h>
#include <string.h>

void nes_power(struct nes *n, const uint8_t *chr)
{
    memset(n, 0, sizeof *n);
    n->chr = chr;
    n->mmc1Control = 0x0C; /* MMC1 powers on with PRG mode 3 */
}

/* ---- MMC1 ---- */

static void mmc1Write(struct nes *n, uint16_t addr, uint8_t value)
{
    if (value & 0x80) {
        n->mmc1Shift = 0;
        n->mmc1Count = 0;
        n->mmc1Control |= 0x0C;
        return;
    }
    n->mmc1Shift |= (uint8_t)((value & 1) << n->mmc1Count);
    if (++n->mmc1Count < 5)
        return;
    switch ((addr >> 13) & 3) {
    case 0: n->mmc1Control = n->mmc1Shift; break;
    case 1: n->mmc1Chr0 = n->mmc1Shift; break;
    case 2: n->mmc1Chr1 = n->mmc1Shift; break;
    case 3: n->mmc1Prg = n->mmc1Shift; break;
    }
    n->mmc1Shift = 0;
    n->mmc1Count = 0;
}

static uint8_t chrByte(const struct nes *n, uint16_t addr)
{
    unsigned bank;
    if (n->mmc1Control & 0x10) /* two 4 KB banks */
        bank = (addr & 0x1000) ? n->mmc1Chr1 : n->mmc1Chr0;
    else /* one 8 KB bank */
        bank = (n->mmc1Chr0 & 0x1E) | ((addr >> 12) & 1);
    return n->chr[((bank & 0x03) << 12) | (addr & 0x0FFF)]; /* 16 KB: four 4 KB banks */
}

/* Nametable address ($2000-$2FFF) to CIRAM offset, by MMC1 mirroring. */
static uint16_t ciramIndex(const struct nes *n, uint16_t addr)
{
    uint16_t table = (addr >> 10) & 3, offset = addr & 0x3FF;
    switch (n->mmc1Control & 3) {
    case 0: return offset;                                 /* one screen, lower */
    case 1: return (uint16_t)(0x400 | offset);             /* one screen, upper */
    case 2: return (uint16_t)(((table & 1) << 10) | offset); /* vertical */
    default: return (uint16_t)(((table >> 1) << 10) | offset); /* horizontal */
    }
}

static uint8_t paletteIndex(uint16_t addr)
{
    uint8_t i = addr & 0x1F;
    if ((i & 0x13) == 0x10) /* $3F10/$3F14/$3F18/$3F1C mirror the backdrop entries */
        i &= 0x0F;
    return i;
}

static uint8_t ppuRead(struct nes *n, uint16_t addr)
{
    addr &= 0x3FFF;
    if (addr < 0x2000)
        return chrByte(n, addr);
    if (addr < 0x3F00)
        return n->ciram[ciramIndex(n, addr)];
    return n->palette[paletteIndex(addr)];
}

static void ppuWrite(struct nes *n, uint16_t addr, uint8_t value)
{
    addr &= 0x3FFF;
    if (addr < 0x2000)
        return; /* CHR ROM */
    if (addr < 0x3F00)
        n->ciram[ciramIndex(n, addr)] = value;
    else
        n->palette[paletteIndex(addr)] = value & 0x3F;
}

/* ---- Registers ---- */

uint8_t nes_read(struct nes *n, uint16_t addr)
{
    if (addr >= 0x2000 && addr < 0x4000) {
        switch (addr & 7) {
        case 2: {
            uint8_t s = n->status;
            n->status &= 0x7F;
            n->w = 0;
            return s;
        }
        case 4: return n->oam[n->oamAddr];
        case 7: {
            uint16_t a = n->v & 0x3FFF;
            uint8_t value = n->readBuffer;
            n->readBuffer = ppuRead(n, a);
            if (a >= 0x3F00)
                value = ppuRead(n, a);
            n->v = (uint16_t)(n->v + ((n->ctrl & 0x04) ? 32 : 1));
            return value;
        }
        default: return 0;
        }
    }
    if (addr == 0x4016 || addr == 0x4017) {
        int p = addr & 1;
        if (n->strobe)
            return (uint8_t)(0x40 | (n->buttons[p] >> 7));
        uint8_t bit = n->shift[p] >> 7;
        n->shift[p] = (uint8_t)((n->shift[p] << 1) | 1); /* reads after the eighth return 1 */
        return (uint8_t)(0x40 | bit);
    }
    return 0;
}

void nes_write(struct nes *n, uint16_t addr, uint8_t value, const uint8_t *ram)
{
    if (addr >= 0x8000) {
        mmc1Write(n, addr, value);
        return;
    }
    if (addr >= 0x2000 && addr < 0x4000) {
        switch (addr & 7) {
        case 0:
            n->ctrl = value;
            n->t = (uint16_t)((n->t & 0xF3FF) | ((value & 3) << 10));
            break;
        case 1: n->mask = value; break;
        case 3: n->oamAddr = value; break;
        case 4: n->oam[n->oamAddr++] = value; break;
        case 5:
            if (!n->w) {
                n->t = (uint16_t)((n->t & 0xFFE0) | (value >> 3));
                n->fineX = value & 7;
            } else {
                n->t = (uint16_t)((n->t & 0x8C1F) | ((value & 7) << 12) | ((value & 0xF8) << 2));
            }
            n->w ^= 1;
            break;
        case 6:
            if (!n->w) {
                n->t = (uint16_t)((n->t & 0x00FF) | ((value & 0x3F) << 8));
            } else {
                n->t = (uint16_t)((n->t & 0xFF00) | value);
                n->v = n->t;
            }
            n->w ^= 1;
            break;
        case 7:
            ppuWrite(n, n->v, value);
            n->v = (uint16_t)(n->v + ((n->ctrl & 0x04) ? 32 : 1));
            break;
        }
        return;
    }
    if (addr == 0x4014) { /* sprite DMA, from RAM only (the game uses page 2) */
        if (value < 0x20)
            for (int i = 0; i < 256; i++)
                n->oam[(uint8_t)(n->oamAddr + i)] = ram[((value << 8) | i) & 0x7FF];
        return;
    }
    if (addr == 0x4016) {
        n->strobe = value & 1;
        if (n->strobe) {
            n->shift[0] = n->buttons[0];
            n->shift[1] = n->buttons[1];
        }
        return;
    }
    /* $4000-$4013, $4015, $4017: the APU, not emulated */
}

void nes_vblank(struct nes *n) { n->status |= 0x80; }

/* ---- Picture ---- */

/* The NTSC palette, computed the way the PPU's video signal produces it
 * (after Bisqwit's palette generator, as described on the NESdev wiki),
 * rather than copied from a palette file. */
static uint32_t nesColor(uint8_t pixel)
{
    static const float levels[8] = {0.350f, 0.518f, 0.962f, 1.550f, 1.094f, 1.506f, 1.962f, 1.962f};
    const float black = 0.518f, white = 1.962f;
    int color = pixel & 0x0F, level = color < 0x0E ? (pixel >> 4) & 3 : 1;
    float lohi[2] = {levels[level + 4 * (color == 0x0)], levels[level + 4 * (color < 0x0D)]};
    float y = 0, i = 0, q = 0;
    for (int p = 0; p < 12; p++) {
        float v = (lohi[(color + p + 8) % 12 < 6] - black) / (white - black);
        y += v;
        i += v * cosf(3.14159265f * p / 6);
        q += v * sinf(3.14159265f * p / 6);
    }
    y /= 12;
    i /= 12;
    q /= 12;
    float rgb[3] = {y + 0.946882f * i + 0.623557f * q, y - 0.274788f * i - 0.635691f * q,
                    y - 1.108545f * i + 1.709007f * q};
    uint32_t out = 0;
    for (int c = 0; c < 3; c++) {
        float f = rgb[c] <= 0 ? 0 : powf(rgb[c], 2.2f / 1.8f);
        int v = (int)(255.95f * f);
        out = (out << 8) | (uint32_t)(v < 0 ? 0 : v > 255 ? 255 : v);
    }
    return out;
}

static uint32_t paletteRgb[64];

void nes_render(const struct nes *n, uint32_t *out)
{
    if (!paletteRgb[0x30])
        for (int c = 0; c < 64; c++)
            paletteRgb[c] = nesColor((uint8_t)c);

    int showBg = n->mask & 0x08, showSprites = n->mask & 0x10;
    int scrollX = ((n->t >> 10) & 1) * 256 + (n->t & 0x1F) * 8 + n->fineX;
    int scrollY = ((n->t >> 11) & 1) * 240 + ((n->t >> 5) & 0x1F) * 8 + ((n->t >> 12) & 7);
    uint16_t bgTable = (n->ctrl & 0x10) ? 0x1000 : 0x0000;
    uint16_t spTable = (n->ctrl & 0x08) ? 0x1000 : 0x0000;
    int tall = (n->ctrl & 0x20) != 0, height = tall ? 16 : 8;

    for (int y = 0; y < NES_HEIGHT; y++) {
        uint8_t bg[NES_WIDTH];
        memset(bg, 0, sizeof bg);
        if (showBg) {
            int py = (scrollY + y) % 480;
            int ntY = py / 240, row = py % 240;
            for (int x = 0; x < NES_WIDTH; x++) {
                if (x < 8 && !(n->mask & 0x02))
                    continue;
                int px = (scrollX + x) % 512;
                int ntX = px / 256, col = px % 256;
                uint16_t nt = (uint16_t)(0x2000 | ((ntY * 2 + ntX) << 10));
                uint8_t tile = ppuRead((struct nes *)n, (uint16_t)(nt + (row / 8) * 32 + col / 8));
                uint8_t attr = ppuRead((struct nes *)n, (uint16_t)(nt + 0x3C0 + (row / 32) * 8 + col / 32));
                uint8_t pal = (attr >> (((row & 16) >> 2) | ((col & 16) >> 3))) & 3;
                uint16_t a = (uint16_t)(bgTable + tile * 16 + (row & 7));
                int bit = 7 - (col & 7);
                uint8_t c = (uint8_t)(((chrByte(n, a) >> bit) & 1) | (((chrByte(n, (uint16_t)(a + 8)) >> bit) & 1) << 1));
                if (c)
                    bg[x] = (uint8_t)(pal * 4 + c);
            }
        }
        uint8_t line[NES_WIDTH];
        for (int x = 0; x < NES_WIDTH; x++)
            line[x] = bg[x] ? n->palette[bg[x]] : n->palette[0];

        if (showSprites) {
            /* the first eight sprites on the line, drawn so that lower OAM entries win */
            int found[8], count = 0;
            for (int s = 0; s < 64 && count < 8; s++) {
                int top = n->oam[s * 4] + 1;
                if (y >= top && y < top + height)
                    found[count++] = s;
            }
            for (int k = count - 1; k >= 0; k--) {
                const uint8_t *o = &n->oam[found[k] * 4];
                int r = y - (o[0] + 1);
                if (o[2] & 0x80)
                    r = height - 1 - r;
                uint16_t a;
                if (tall)
                    a = (uint16_t)(((o[1] & 1) << 12) + ((o[1] & 0xFE) + (r >= 8)) * 16 + (r & 7));
                else
                    a = (uint16_t)(spTable + o[1] * 16 + r);
                for (int i = 0; i < 8; i++) {
                    int x = o[3] + i;
                    if (x >= NES_WIDTH || (x < 8 && !(n->mask & 0x04)))
                        continue;
                    int bit = (o[2] & 0x40) ? i : 7 - i;
                    uint8_t c = (uint8_t)(((chrByte(n, a) >> bit) & 1) | (((chrByte(n, (uint16_t)(a + 8)) >> bit) & 1) << 1));
                    if (!c)
                        continue;
                    if ((o[2] & 0x20) && bg[x])
                        continue; /* behind the background */
                    line[x] = n->palette[0x10 + (o[2] & 3) * 4 + c];
                }
            }
        }
        for (int x = 0; x < NES_WIDTH; x++)
            out[y * NES_WIDTH + x] = paletteRgb[line[x] & 0x3F];
    }
}
