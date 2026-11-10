/* The decomp-dj GBA test ROM: a few functions that only run when a button is pressed, in
 * Thumb and ARM, with a jump table, literal pools and a call into IWRAM. It draws nothing;
 * a segment's inputs decide which code runs. */
#include <stdint.h>

#define REG_DISPCNT (*(volatile uint16_t *)0x04000000)
#define REG_VCOUNT (*(volatile uint16_t *)0x04000006)
#define REG_KEYINPUT (*(volatile uint16_t *)0x04000130)
#define BG_PALETTE ((volatile uint16_t *)0x05000000)

enum { KEY_A = 1, KEY_B = 2, KEY_SELECT = 4, KEY_START = 8, KEY_RIGHT = 16, KEY_LEFT = 32 };

volatile uint32_t score;
volatile uint32_t frames;
static const uint16_t colors[8] = { 0x001F, 0x03E0, 0x7C00, 0x03FF, 0x7FE0, 0x7C1F, 0x7FFF, 0x0000 };

extern uint32_t iwram_mix(uint32_t a, uint32_t b);

static void wait_vblank(void) {
    while (REG_VCOUNT >= 160) {}
    while (REG_VCOUNT < 160) {}
}

/* Thumb, with a switch the compiler turns into a jump table. */
__attribute__((noinline)) uint32_t on_a(uint32_t x) {
    switch (x & 7) {
    case 0: return x + 0x1234567;
    case 1: return x * 3;
    case 2: return x ^ 0xDEADBEEF;
    case 3: return x - 99;
    case 4: return x << 2;
    case 5: return x >> 1;
    case 6: return ~x;
    default: return 0x8000001;
    }
}

/* ARM, with conditional instructions. */
__attribute__((noinline, target("arm"))) uint32_t on_b(uint32_t x) {
    if (x > 1000) {
        x -= 1000;
    } else {
        x += 7;
    }
    return x;
}

/* Runs only on Start. */
__attribute__((noinline)) void on_start(void) {
    score = 0;
    BG_PALETTE[0] = colors[7];
}

/* Runs only on Left or Right. */
__attribute__((noinline)) void on_direction(int right) {
    score = iwram_mix(score, right ? 0x55AA55AA : 0x12345678);
}

int main(void) {
    REG_DISPCNT = 0;
    for (;;) {
        wait_vblank();
        ++frames;
        uint32_t keys = ~REG_KEYINPUT & 0x3FF;
        if (keys & KEY_A) score = on_a(score);
        if (keys & KEY_B) score = on_b(score);
        if (keys & KEY_START) on_start();
        if (keys & (KEY_LEFT | KEY_RIGHT)) on_direction(keys & KEY_RIGHT);
        BG_PALETTE[0] = colors[score & 7];
    }
}
