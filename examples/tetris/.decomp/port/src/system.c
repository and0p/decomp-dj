/*
 * System code: reset and boot, the NMI, the main loop, the jump-table dispatch,
 * the PPU copy routines, the random number generator, controller reading, the
 * mapper (MMC1) and the routines that wait for the next frame.
 */
#include "game.h"

uint8_t tetris_ram[0x800];

static const uint8_t *prg;

void tetris_set_prg(const uint8_t *prg32k) { prg = prg32k; }

uint8_t prg8(uint16_t addr) { return prg[addr & 0x7FFF]; }
uint16_t prg16(uint16_t addr) { return (uint16_t)(prg8(addr) | (prg8((uint16_t)(addr + 1)) << 8)); }

uint8_t peek(uint16_t addr)
{
    if (addr < 0x2000)
        return R(addr);
    if (addr >= 0x8000)
        return prg8(addr);
    return hw_read(addr);
}

void poke(uint16_t addr, uint8_t value)
{
    if (addr < 0x2000) {
        R(addr) = value;
        return;
    }
    hw_write(addr, value);
}

/* Shorthands for the hardware writes and reads below. */
#define wr(addr, v) poke((addr), (uint8_t)(v))
#define rd(addr) hw_read(addr)

/* `inc initRam` and `inc reset`: an INC of a ROM byte writes it back plus one.
 * Both bytes have bit 7 set, so the write resets the MMC1 shift register. */
static void incRomByte(uint16_t addr) { wr(addr, prg8(addr) + 1); }
void incrementInitRam(void) { incRomByte(ROM_initRam); }

/* ------------------------------------------------------------------------ */
/* Reset and boot                                                           */
/* ------------------------------------------------------------------------ */

static void initRamContinued(void);
static void mainLoop(void);

/*
 * reset: the reset vector. Turns the PPU off, waits for two vertical blanks
 * so that the PPU has warmed up, puts the mapper in a known state (32 KB PRG,
 * 4 KB CHR banks, one-screen mirroring) and selects the title screen's tiles,
 * then clears RAM in initRam.
 */
void tetris_reset(void)
{
    wr(PPUCTRL, 0);
    wr(PPUMASK, 0);
    while (!(rd(PPUSTATUS) & 0x80)) /* @vsyncWait1 */
        hw_idle();
    while (!(rd(PPUSTATUS) & 0x80)) /* @vsyncWait2 */
        hw_idle();
    incRomByte(ROM_reset);
    setMMC1Control(MMC1_4KCHR_32KPRG_H_MIRROR);
    changeCHRBank0(CHR_TITLE_MENU);
    changeCHRBank1(CHR_TITLE_MENU);
    changePRGBank(PRG_32K_BANK);
    initRam();
}

/* initRam: the first byte of PRG ROM. Besides being the entry point after
 * reset, it is the byte that `inc initRam` rewrites to reset the mapper. */
void initRam(void)
{
    initRamContinued();
}

/*
 * initRamContinued: clears $0000-$06FF (everything but the high score table),
 * then checks initMagic, five bytes that only a previous run of the game can
 * have left in RAM. If they are missing this is a cold boot, so the high score
 * table is filled with its defaults; after a press of Reset the scores
 * survive. Then it seeds the random number generator, sets up the PPU and the
 * audio, clears all four nametables and both playfields, turns the screen on
 * and runs the main loop.
 */
static void initRamContinued(void)
{
    uint8_t y = 0;
    tmp2 = 0x06;
    tmp1 = 0x00;
    do { /* @zeroOutPages: page tmp2, offsets 0, $FF..$01, for pages 6 down to 0 */
        do {
            R(ptr16(A_tmp1) + y) = 0;
            y--;
        } while (y);
        tmp2--;
    } while (!(tmp2 & 0x80));
    hw_cycles(0x700 * 13);

    if (!(initMagic[0] == 0x12 && initMagic[1] == 0x34 && initMagic[2] == 0x56 && initMagic[3] == 0x78 &&
          initMagic[4] == 0x9A)) {
        /* @initHighScoreTable: copy defaultHighScoresTable up to its $FF end marker. (The
         * `ldx #$00` above the label is skipped by the jmp, but X is already 0 here,
         * from reset's `ldx #$00`... and from the clearing loop never touching X.) */
        for (uint8_t x = 0;; x++) {
            uint8_t a = prg8(ROM_defaultHighScoresTable + x);
            if (a == 0xFF)
                break;
            highScoreNames[x] = a;
        }
        initMagic[0] = 0x12; /* @continueColdBootInit */
        initMagic[1] = 0x34;
        initMagic[2] = 0x56;
        initMagic[3] = 0x78;
        initMagic[4] = 0x9A;
    }

    /* @continueWarmBootInit */
    rng_seed[0] = 0x89;
    rng_seed[1] = 0x88;
    ppuScrollX = 0;
    wr(PPUSCROLL, 0);
    ppuScrollY = 0;
    wr(PPUSCROLL, 0);
    currentPpuCtrl = 0x90; /* NMI on, background tiles from $1000 */
    wr(PPUCTRL, 0x90);
    wr(PPUMASK, 0x06);
    LE006();
    updateAudio2();
    /* Values left on the stack page; nothing reads them. */
    stack[0] = 0xC0;
    stack[1] = 0x80;
    stack[3] = 0x35;
    stack[4] = 0xAC;
    updateAudioWaitForNmiAndDisablePpuRendering();
    disableNmi();
    LAA82(0x20);
    LAA82(0x24);
    LAA82(0x28);
    LAA82(0x2C);
    memset_page(tileEmpty, 0x04, 0x05); /* playfield and playfieldForSecondPlayer */
    waitForVBlankAndEnableNmi();
    updateAudioWaitForNmiAndResetOamStaging();
    updateAudioWaitForNmiAndEnablePpuRendering();
    updateAudioWaitForNmiAndResetOamStaging();
    unused_0E = 0x0E;
    gameModeState = 0;
    gameMode = 0;
    numberOfPlayers = 1;
    frameCounter[1] = 0;
    mainLoop();
}

/*
 * @mainLoop: runs the handler for the current game mode, over and over. A
 * handler that has finished its work for this frame returns with A equal to
 * gameModeState, and the loop then waits for the next frame; one that returns
 * anything else is run again at once. (Most handlers return a value that just
 * happens to be left in A, so this test is subtle: see the comments on the
 * gameModeState handlers.)
 *
 * The loop also ends the demo when its recorded input runs out, going back to
 * the title screen.
 */
static void mainLoop(void)
{
    for (;;) {
        uint8_t a = branchOnGameMode();
        if (a == gameModeState)
            updateAudioWaitForNmiAndResetOamStaging();
        /* @checkForDemoDataExhaustion */
        if (gameMode == 0x05 && R(A_demoButtonsAddr + 1) == (ROM_demoTetriminoTypeTable >> 8)) {
            R(A_demoButtonsAddr + 1) = ROM_demoButtonsTable >> 8;
            frameCounter[1] = 0;
            gameMode = 1;
        }
    }
}

/* ------------------------------------------------------------------------ */
/* NMI and dispatch                                                         */
/* ------------------------------------------------------------------------ */

/*
 * nmi: runs at the start of every vertical blank while NMIs are on. It
 * copies what the main loop has prepared to the PPU (render, then the sprite
 * DMA), counts down sleepCounter (stopping at 0), counts frames, steps the
 * random number generator, resets the scroll, tells the main loop that a
 * frame has passed (verticalBlankingInterval) and reads the controllers.
 */
void tetris_nmi(void)
{
    oamStagingLength = 0;
    render();
    sleepCounter--;
    if (sleepCounter == 0xFF)
        sleepCounter++;
    copyOamStagingToOam();
    uint16_t f = (uint16_t)(frameCounter[0] | (frameCounter[1] << 8)) + 1;
    frameCounter[0] = (uint8_t)f;
    frameCounter[1] = (uint8_t)(f >> 8);
    generateNextPseudorandomNumber(A_rng_seed, 2);
    ppuScrollX = 0;
    wr(PPUSCROLL, 0);
    ppuScrollY = 0;
    wr(PPUSCROLL, 0);
    verticalBlankingInterval = 1;
    pollControllerButtons();
}

/*
 * switch_s_plus_2a: the jump table dispatcher. The original is called with
 * JSR, followed by a table of routine addresses; it pops its return address,
 * reads entry A of the table and jumps there, so the routine returns to the
 * dispatcher's caller. The C callers switch on the index themselves; this
 * keeps the one trace the original leaves in RAM, the entry's address in
 * tmp1/tmp2.
 */
void switch_s_plus_2a(uint8_t index, uint16_t table)
{
    uint16_t entry = prg16((uint16_t)(table + (uint8_t)(index << 1)));
    tmp1 = (uint8_t)entry;
    tmp2 = (uint8_t)(entry >> 8);
}

/* The jump tables follow `lda zp` (2 bytes) and `jsr switch_s_plus_2a` (3 bytes). */
#define TABLE_AFTER(routine) ((uint16_t)((routine) + 5))

/* render: called from the NMI; draws what the current renderMode needs. */
void render(void)
{
    switch_s_plus_2a(renderMode, TABLE_AFTER(ROM_render));
    switch (renderMode) {
    case 0: render_mode_legal_and_title_screens(); break;
    case 1: render_mode_menu_screens(); break;
    case 2: render_mode_congratulations_screen(); break;
    case 3: render_mode_play_and_demo(); break;
    case 4: render_mode_ending_animation(); break;
    }
}

/* branchOnGameMode: runs the handler for gameMode (0 legal screen, 1 title,
 * 2 game type menu, 3 level menu, 4 play, 5 demo, 6 start demo) and returns
 * the A it leaves, which the main loop compares with gameModeState. */
uint8_t branchOnGameMode(void)
{
    switch_s_plus_2a(gameMode, TABLE_AFTER(ROM_branchOnGameMode));
    switch (gameMode) {
    case 0: return gameMode_legalScreen();
    case 1: return gameMode_titleScreen();
    case 2: return gameMode_gameTypeMenu();
    case 3: return gameMode_levelMenu();
    case 4:
    case 5: return gameMode_playAndEndingHighScore(); /* via gameMode_playAndEndingHighScore_jmp */
    default: return gameMode_startDemo();
    }
}

/* ------------------------------------------------------------------------ */
/* Copying to the PPU                                                       */
/* ------------------------------------------------------------------------ */

/*
 * bulkCopyToPpu: copies a block of PPU data (a palette or a whole nametable)
 * whose address follows the JSR in the original: `jsr bulkCopyToPpu` then
 * `.addr data`. callSite is the JSR's address. The helper that fetches the
 * address (copyAddrAtReturnAddressToTmp_incrReturnAddrBy2) leaves the return
 * address in tmpBulkCopyToPpuReturnAddr and the data address in tmp1/tmp2.
 */
void bulkCopyToPpu(uint16_t callSite, uint16_t data)
{
    uint16_t ret = (uint16_t)(callSite + 2); /* what the JSR pushed */
    R(A_tmpBulkCopyToPpuReturnAddr) = (uint8_t)ret;
    R(A_tmpBulkCopyToPpuReturnAddr + 1) = (uint8_t)(ret >> 8);
    tmp1 = (uint8_t)data;
    tmp2 = (uint8_t)(data >> 8);
    copyToPpu();
}

/*
 * copyToPpu: runs the PPU copy "program" at tmp1/tmp2. Each command is:
 *
 *   hh ll nn data...  write to PPU address hhll. nn bit 7 selects +32 address
 *                     steps (down a column) instead of +1; bit 6 repeats one
 *                     data byte instead of copying several; bits 0-5 are the
 *                     count, 0 meaning 64. After writes to the palette ($3Fxx)
 *                     the PPU address is moved away from it.
 *   $4C lo hi         continue at address hilo, like a JSR (nametables are
 *                     built from shared pieces)
 *   $60               return to after the last $4C
 *   $80-$FF           end
 *
 * The addresses saved by $4C live on the 6502 stack in the original.
 */
void copyToPpu(void)
{
    uint16_t saved[8];
    int depth = 0;
    uint32_t cycles = 0;
    for (;;) {
        rd(PPUSTATUS); /* resets the PPUADDR latch */
        uint8_t y = 0;
        uint8_t a = peek((uint16_t)(ptr16(A_tmp1) + y));
        if (a & 0x80)
            break;
        if (a == 0x60) {
            uint16_t p = saved[--depth & 7];
            tmp2 = (uint8_t)(p >> 8);
            tmp1 = (uint8_t)p;
            y = 2;
        } else if (a == 0x4C) {
            saved[depth++ & 7] = ptr16(A_tmp1);
            uint8_t lo = peek((uint16_t)(ptr16(A_tmp1) + 1));
            uint8_t hi = peek((uint16_t)(ptr16(A_tmp1) + 2));
            tmp2 = hi;
            tmp1 = lo;
            continue;
        } else { /* LAA9E */
            uint8_t high = a;
            wr(PPUADDR, high);
            y++;
            wr(PPUADDR, peek((uint16_t)(ptr16(A_tmp1) + y)));
            y++;
            uint8_t len = peek((uint16_t)(ptr16(A_tmp1) + y));
            currentPpuCtrl = (len & 0x80) ? (currentPpuCtrl | 0x04) : (currentPpuCtrl & 0xFB);
            wr(PPUCTRL, currentPpuCtrl);
            int repeat = (len & 0x40) != 0;
            uint8_t count = len & 0x3F;
            unsigned n = count ? count : 64;
            if (repeat)
                y++;
            for (unsigned i = 0; i < n; i++) {
                if (!repeat)
                    y++;
                wr(PPUDATA, peek((uint16_t)(ptr16(A_tmp1) + y)));
            }
            cycles += 18 * n + 60;
            if (high == 0x3F) {
                wr(PPUADDR, 0x3F);
                wr(PPUADDR, 0);
                wr(PPUADDR, 0);
                wr(PPUADDR, 0);
            }
        }
        /* LAAE6: step past the command */
        uint16_t p = (uint16_t)(ptr16(A_tmp1) + y + 1);
        tmp1 = (uint8_t)p;
        tmp2 = (uint8_t)(p >> 8);
    }
    hw_cycles(cycles);
}

/* twoDigsToPPU: writes a BCD byte to PPUDATA as two digit tiles, high digit first. */
void twoDigsToPPU(uint8_t a)
{
    generalCounter = a;
    wr(PPUDATA, (a >> 4) & 0x0F);
    wr(PPUDATA, generalCounter & 0x0F);
}

/*
 * memset_ppu_page_and_more: fills PPU memory. With A >= $20 it fills the
 * nametable at A00 (1 KB, including its attribute table) with X, then
 * overwrites the attribute table ($23C0 and so on) with Y. With A < $20 (a
 * pattern table) it fills Y pages with X. The arguments are kept in
 * tmp1-tmp3.
 */
void memset_ppu_page_and_more(uint8_t a, uint8_t x, uint8_t y)
{
    tmp1 = a;
    tmp2 = x;
    tmp3 = y;
    rd(PPUSTATUS);
    currentPpuCtrl &= 0xFB;
    wr(PPUCTRL, currentPpuCtrl);
    wr(PPUADDR, tmp1);
    wr(PPUADDR, 0);
    unsigned pages = (a >= 0x20) ? 4 : tmp3;
    if (pages == 0)
        pages = 256;
    for (unsigned i = 0; i < pages * 256u; i++)
        wr(PPUDATA, tmp2);
    if (tmp1 >= 0x20) {
        wr(PPUADDR, (uint8_t)(tmp1 + 3)); /* adc #$02 with the carry set */
        wr(PPUADDR, 0xC0);
        for (int i = 0; i < 0x40; i++)
            wr(PPUDATA, tmp3);
    }
    hw_cycles(pages * 256u * 10 + 0x40 * 8);
}

/* LAA82: clears nametable A to tile $FF with attribute 0. */
void LAA82(uint8_t a) { memset_ppu_page_and_more(a, 0xFF, 0x00); }

/*
 * memset_page: fills RAM pages X to Y (inclusive) with A. It works from page Y
 * downward, so it leaves tmp1 = 0 and tmp2 = X - 1.
 */
void memset_page(uint8_t a, uint8_t x, uint8_t y)
{
    tmp2 = y;
    uint8_t pagesLeft = (uint8_t)(x - y - 1); /* counts up to 0 */
    tmp1 = 0;
    uint8_t yy = 0;
    unsigned pages = 0;
    do {
        do {
            R(ptr16(A_tmp1) + yy) = a;
            yy--;
        } while (yy);
        tmp2--;
        pages++;
    } while (++pagesLeft);
    if (pages > 1)
        hw_cycles(pages * 256u * 11);
}

/* ------------------------------------------------------------------------ */
/* Random numbers                                                           */
/* ------------------------------------------------------------------------ */

/*
 * generateNextPseudorandomNumber: steps a linear feedback shift register of
 * Y bytes at zero page address X. The new top bit is bit 1 of the first byte
 * XOR bit 1 of the second, and the whole register shifts right by one bit.
 * tmp1 is used as scratch.
 */
void generateNextPseudorandomNumber(uint8_t x, uint8_t y)
{
    tmp1 = R(x) & 0x02;
    uint8_t carry = ((R(x + 1) & 0x02) ^ tmp1) ? 1 : 0;
    do {
        uint8_t v = R(x);
        R(x) = (uint8_t)((v >> 1) | (carry << 7));
        carry = v & 1;
        x++;
    } while (--y);
}

/* ------------------------------------------------------------------------ */
/* Sprites and controllers                                                  */
/* ------------------------------------------------------------------------ */

/* copyOamStagingToOam: sprite DMA from oamStaging ($0200). */
void copyOamStagingToOam(void)
{
    wr(OAMADDR, 0);
    wr(OAMDMA, 0x02);
}

/*
 * pollController_actualRead: strobes the controllers and reads eight bits
 * from each port. Bit 0 of each read is the standard controller, which goes
 * to newlyPressedButtons_player1/2; bit 1 is the expansion port (the Famicom's
 * extra controllers), which goes to tmp1/tmp2.
 */
void pollController_actualRead(void)
{
    uint8_t x = joy1Location;
    wr(JOY1, x + 1);
    wr(JOY1, x);
    for (int i = 0; i < 8; i++) {
        uint8_t a = rd(JOY1);
        newlyPressedButtons_player1 = (uint8_t)((newlyPressedButtons_player1 << 1) | (a & 1));
        tmp1 = (uint8_t)((tmp1 << 1) | ((a >> 1) & 1));
        a = rd(JOY2_APUFC);
        newlyPressedButtons_player2 = (uint8_t)((newlyPressedButtons_player2 << 1) | (a & 1));
        tmp2 = (uint8_t)((tmp2 << 1) | ((a >> 1) & 1));
    }
}

/* addExpansionPortInputAsControllerInput: a Famicom expansion controller counts
 * as the player's controller. */
void addExpansionPortInputAsControllerInput(void)
{
    newlyPressedButtons_player1 |= tmp1;
    newlyPressedButtons_player2 |= tmp2;
}

/*
 * pollController: reads the controllers twice and keeps only the buttons
 * seen both times (a guard against the DMC glitch corrupting a read), then
 * works out which buttons are newly pressed.
 */
void pollController(void)
{
    pollController_actualRead();
    addExpansionPortInputAsControllerInput();
    generalCounter2 = newlyPressedButtons_player1;
    generalCounter3 = newlyPressedButtons_player2;
    pollController_actualRead();
    addExpansionPortInputAsControllerInput();
    newlyPressedButtons_player1 &= generalCounter2;
    newlyPressedButtons_player2 &= generalCounter3;
    diffOldAndNewButtons();
}

/* diffOldAndNewButtons: on entry newlyPressedButtons_playerN holds the buttons
 * down now. Afterward it holds only those not down last frame, and
 * heldButtons_playerN holds all those down now. */
void diffOldAndNewButtons(void)
{
    for (int x = 1; x >= 0; x--) {
        uint8_t now = R(A_newlyPressedButtons_player1 + x);
        R(A_newlyPressedButtons_player1 + x) = (now ^ R(A_heldButtons_player1 + x)) & now;
        R(A_heldButtons_player1 + x) = now;
    }
}

/* ------------------------------------------------------------------------ */
/* Mapper                                                                   */
/* ------------------------------------------------------------------------ */

/* The MMC1's registers are loaded one bit at a time, low bit first, by five
 * writes; the fifth write's address selects the register. */
static void mmc1Write(uint16_t reg, uint8_t a)
{
    for (int i = 0; i < 5; i++) {
        wr(reg, a);
        a >>= 1;
    }
}

/* setMMC1Control: mirroring, PRG banking mode and CHR banking mode. */
void setMMC1Control(uint8_t a) { mmc1Write(MMC1_Control, a); }
/* changeCHRBank0: the 4 KB of tiles at PPU $0000. */
void changeCHRBank0(uint8_t a) { mmc1Write(MMC1_CHR0, a); }
/* changeCHRBank1: the 4 KB of tiles at PPU $1000. */
void changeCHRBank1(uint8_t a) { mmc1Write(MMC1_CHR1, a); }
/* changePRGBank: the PRG bank (always 0: the game is one 32 KB bank). */
void changePRGBank(uint8_t a) { mmc1Write(MMC1_PRG, a); }

/* ------------------------------------------------------------------------ */
/* Waiting for the next frame                                               */
/* ------------------------------------------------------------------------ */

/* updateAudioAndWaitForNmi: runs the audio engine for this frame, then waits
 * until the NMI has run. Returns A = 1 (verticalBlankingInterval). */
uint8_t updateAudioAndWaitForNmi(void)
{
    updateAudio();
    verticalBlankingInterval = 0;
    while (!verticalBlankingInterval)
        hw_idle();
    return verticalBlankingInterval;
}

/* updateAudioWaitForNmiAndResetOamStaging: the usual end of a frame. Runs the
 * audio, waits for the NMI, then hides every sprite (fills oamStaging with $FF)
 * so the next frame can stage its own. Returns A = $FF. */
uint8_t updateAudioWaitForNmiAndResetOamStaging(void)
{
    updateAudioAndWaitForNmi();
    memset_page(0xFF, 0x02, 0x02);
    return 0xFF;
}

/* _updatePpuMask */
static void updatePpuMask(uint8_t a)
{
    wr(PPUMASK, a);
    currentPpuMask = a;
}

/* updateAudioWaitForNmiAndDisablePpuRendering: waits a frame, then turns off
 * the background and sprites, so that the PPU can be written at length. */
void updateAudioWaitForNmiAndDisablePpuRendering(void)
{
    updateAudioAndWaitForNmi();
    updatePpuMask(currentPpuMask & 0xE1);
}

/* updateAudioWaitForNmiAndEnablePpuRendering: waits a frame, restores the
 * scroll and PPUCTRL, then turns the background and sprites back on. */
void updateAudioWaitForNmiAndEnablePpuRendering(void)
{
    updateAudioAndWaitForNmi();
    copyCurrentScrollAndCtrlToPPU();
    updatePpuMask(currentPpuMask | 0x1E);
}

/* _updatePpuCtrl */
static void updatePpuCtrl(uint8_t a)
{
    wr(PPUCTRL, a);
    currentPpuCtrl = a;
}

/* waitForVBlankAndEnableNmi: waits until the vertical blank flag reads clear
 * (reading it clears it, so this takes at most two reads), then turns NMIs on. */
void waitForVBlankAndEnableNmi(void)
{
    while (rd(PPUSTATUS) & 0x80)
        ;
    updatePpuCtrl(currentPpuCtrl | 0x80);
}

/* disableNmi: turns NMIs off, for work that must not be interrupted. */
void disableNmi(void) { updatePpuCtrl(currentPpuCtrl & 0x7F); }

/* copyCurrentScrollAndCtrlToPPU: scroll to 0,0 and rewrite PPUCTRL. */
void copyCurrentScrollAndCtrlToPPU(void)
{
    wr(PPUSCROLL, 0);
    wr(PPUSCROLL, 0);
    wr(PPUCTRL, currentPpuCtrl);
}

/* sleep_for_a_vblanks: waits A frames, using the NMI's sleepCounter. */
void sleep_for_a_vblanks(uint8_t a)
{
    sleepCounter = a;
    do
        updateAudioWaitForNmiAndResetOamStaging();
    while (sleepCounter);
}

/* sleep_for_14_vblanks */
void sleep_for_14_vblanks(void) { sleep_for_a_vblanks(0x14); }
