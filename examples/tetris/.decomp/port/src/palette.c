/*
 * The NES palette, computed rather than copied: the 2C02 makes each colour
 * as an NTSC signal (a square wave between two voltage levels, its phase the
 * hue), and this decodes that signal the way a TV would. After Bisqwit's
 * palette generator (https://bisqwit.iki.fi/utils/nespalette.php), without
 * colour emphasis.
 */
#include <math.h>

#include "tetris/scene.h"

static uint32_t compute(int pixel)
{
    static const float black = 0.518f, white = 1.962f;
    static const float levels[8] = {0.350f, 0.518f, 0.962f, 1.550f, 1.094f, 1.506f, 1.962f, 1.962f};
    int color = pixel & 0x0F;
    int level = color < 0x0E ? (pixel >> 4) & 3 : 1;
    float lo_hi[2] = {levels[level + 4 * (color == 0x0)], levels[level + 4 * (color < 0x0D)]};
    float y = 0, i = 0, q = 0;
    for (int p = 0; p < 12; p++) {
        int high = (color + p + 8) % 12 < 6;
        float v = (lo_hi[high] - black) / (white - black) / 12.0f;
        y += v;
        i += v * cosf(3.14159265f * (float)p / 6.0f);
        q += v * sinf(3.14159265f * (float)p / 6.0f);
    }
    float rgb[3] = {
        y + 0.946882f * i + 0.623557f * q,
        y - 0.274788f * i - 0.635691f * q,
        y - 1.108545f * i + 1.709007f * q,
    };
    uint32_t out = 0;
    for (int c = 0; c < 3; c++) {
        float f = rgb[c] <= 0 ? 0 : powf(rgb[c], 2.2f / 1.8f);
        int v = (int)(255.95f * f);
        out = out << 8 | (uint32_t)(v < 0 ? 0 : v > 255 ? 255 : v);
    }
    return out;
}

uint32_t tetris_nes_color(uint8_t index)
{
    static uint32_t table[64];
    static int ready;
    if (!ready) {
        for (int p = 0; p < 64; p++)
            table[p] = compute(p);
        ready = 1;
    }
    return table[index & 0x3F];
}
