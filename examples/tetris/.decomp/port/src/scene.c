/*
 * The game state as a 3D scene (tetris_scene_build).
 *
 * Which screen is up follows the original: renderMode says which kind of
 * screen the NMI is drawing (0 the legal and title screens, 1 the menus, 2
 * the name entry, 3 play and the demo, 4 the endings), gameMode which one,
 * and currentPpuMask whether the picture is on at all (it is off while a new
 * screen is drawn). Everything is placed where the original puts it: text at
 * its nametable row and column, the pieces where their sprites go.
 */
#include <string.h>

#include "font8x8_basic.h"
#include "tetris/scene.h"

/* The level colours (colorTable in the original): per level, mod 10, the
 * background, white, and the two block colours. */
static const uint8_t colorTable[10][4] = {
    {0x0F, 0x30, 0x21, 0x12}, {0x0F, 0x30, 0x29, 0x1A}, {0x0F, 0x30, 0x24, 0x14}, {0x0F, 0x30, 0x2A, 0x12},
    {0x0F, 0x30, 0x2B, 0x15}, {0x0F, 0x30, 0x22, 0x2B}, {0x0F, 0x30, 0x00, 0x16}, {0x0F, 0x30, 0x05, 0x13},
    {0x0F, 0x30, 0x16, 0x12}, {0x0F, 0x30, 0x27, 0x16},
};

/* NES palette entries used for the rest */
#define C_BLACK 0x0F
#define C_WHITE 0x30
#define C_GREY 0x00
#define C_LIGHT_GREY 0x10
#define C_ORANGE 0x27
#define C_RED 0x16

#define TEXT_DEPTH 0.4f
#define LIFT 0.6f /* how far a selected item comes forward */

/* The characters of high score names, as highScoreCharToTile has them. */
static const char highScoreChars[] = " ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789,/()\". ";

static struct tetris_scene *S;
static const struct tetris_vars *V;

static uint32_t nes(uint8_t c) { return tetris_nes_color(c); }

static struct tetris_scene_item *add(uint8_t kind, float x, float y, float z, float w, float h, float d,
                                     uint32_t face, uint32_t side)
{
    if (S->count >= TETRIS_SCENE_MAX)
        return NULL;
    struct tetris_scene_item *it = &S->items[S->count++];
    it->kind = kind;
    it->x = x, it->y = y, it->z = z;
    it->w = w, it->h = h, it->d = d;
    it->face = face, it->side = side;
    return it;
}

/* ---- text ----------------------------------------------------------------- */

/* One character, its top left at tile (col, row), `scale` tiles a side. Each
 * row of the glyph's pixels becomes boxes, one per run of set pixels. */
static void glyph(float col, float row, char c, uint32_t color, float z, float scale, float depth)
{
    if ((unsigned char)c >= 128 || c == ' ')
        return;
    const float px = scale / 8.0f;
    for (int y = 0; y < 8; y++) {
        uint8_t bits = (uint8_t)font8x8_basic[(int)c][y];
        for (int x = 0; x < 8;) {
            if (!(bits >> x & 1)) {
                x++;
                continue;
            }
            int x0 = x;
            while (x < 8 && (bits >> x & 1))
                x++;
            add(TETRIS_SCENE_BOX, col + x0 * px, row + y * px, z, (x - x0) * px, px, depth, color, color);
        }
    }
}

static void textAt(float col, float row, const char *s, uint32_t color, float z, float scale)
{
    for (; *s; s++, col += scale)
        glyph(col, row, *s, color, z, scale, TEXT_DEPTH * scale);
}

static void text(int col, int row, const char *s) { textAt((float)col, (float)row, s, nes(C_WHITE), 0, 1); }

/* BCD bytes, most significant first, as digits. */
static void bcdText(int col, int row, const uint8_t *bytes, int n, uint32_t color)
{
    char buf[16];
    for (int i = 0; i < n; i++) {
        buf[2 * i] = (char)('0' + (bytes[i] >> 4 & 0x0F) % 10);
        buf[2 * i + 1] = (char)('0' + (bytes[i] & 0x0F) % 10);
    }
    buf[2 * n] = 0;
    textAt((float)col, (float)row, buf, color, 0, 1);
}

static void numberText(int col, int row, unsigned v, int digits, uint32_t color)
{
    char buf[8];
    buf[digits] = 0;
    for (int i = digits - 1; i >= 0; i--, v /= 10)
        buf[i] = (char)('0' + v % 10);
    textAt((float)col, (float)row, buf, color, 0, 1);
}

/* An item the player is choosing: comes forward and takes the level's colour,
 * pulsing while it is the one being changed. */
static void selected(float col, float row, const char *s, int active)
{
    float lift = active && (V->frameCounter[0] & 0x10) ? LIFT * 0.5f : LIFT;
    uint8_t c = colorTable[V->startLevel % 10][2];
    textAt(col, row, s, nes(active ? C_ORANGE : c), -lift, 1);
}

/* ---- blocks ---------------------------------------------------------------- */

static uint8_t levelColors(void) { return (uint8_t)(V->player1_levelNumber % 10); }

/* A square with tile `tile` at (col, row), `size` tiles a side. */
static void block(float col, float row, uint8_t tile, float size)
{
    const uint8_t *c = colorTable[levelColors()];
    uint32_t face, side;
    switch (tile) {
    case 0x7B: face = nes(C_WHITE), side = nes(c[3]); break; /* white, framed in the second colour */
    case 0x7C: face = side = nes(c[2]); break;
    case 0x7D: face = side = nes(c[3]); break;
    case 0x4F: face = nes(c[2]), side = nes(c[3]); break; /* the game over curtain */
    default:
        if (tile <= 0x23) { /* digits and letters, as B-type's success message writes them */
            char ch = (char)(tile < 10 ? '0' + tile : 'A' + tile - 10);
            glyph(col, row, ch, nes(C_WHITE), 0, size, 0.5f);
        }
        return;
    }
    add(TETRIS_SCENE_BLOCK, col, row, 0, size, size, size, face, side);
}

/* A piece of orientation `o` with its centre square's top left at (col, row). */
static void piece(float col, float row, uint8_t o, float size, float clipRow)
{
    int8_t rows[4], cols[4];
    uint8_t tiles[4];
    if (!tetris_piece_squares(o, rows, cols, tiles))
        return;
    for (int i = 0; i < 4; i++) {
        float r = row + rows[i] * size;
        if (tiles[i] != 0xFF && r >= clipRow)
            block(col + cols[i] * size, r, tiles[i], size);
    }
}

/* ---- screens --------------------------------------------------------------- */

static void legalScreen(void)
{
    text(8, 6, "TM AND (C) 1987");
    text(5, 8, "V/O ELECTRONORGTECHNICA");
    text(11, 10, "(\"ELORG\")");
    text(3, 12, "TETRIS LICENSED TO NINTENDO");
    text(7, 14, "(C) 1989 NINTENDO");
    text(6, 16, "ALL RIGHTS RESERVED");
    text(4, 18, "ORIGINAL CONCEPT,DESIGN");
    text(11, 20, "AND PROGRAM");
    text(7, 22, "BY ALEXEY PAZHITNOV");
}

static void titleScreen(void)
{
    textAt(7, 5, "TETRIS", nes(C_RED), 0, 3);
    text(11, 20, "PUSH START");
}

static void typeText(int col, int row)
{
    textAt((float)col, (float)row, V->gameType ? "B-TYPE" : "A-TYPE", nes(C_WHITE), 0, 1);
}

static void gameTypeMenu(void)
{
    text(5, 4, "GAME TYPE");
    if (V->gameType)
        text(9, 8, "A-TYPE"), selected(21, 8, "B-TYPE", 1);
    else
        selected(9, 8, "A-TYPE", 1), text(21, 8, "B-TYPE");
    text(5, 13, "MUSIC TYPE");
    static const char *music[4] = {"MUSIC  1", "MUSIC  2", "MUSIC  3", "  OFF"};
    for (int i = 0; i < 4; i++) {
        if (i == (V->musicType & 3))
            selected(14, (float)(18 + 2 * i), music[i], 1);
        else
            text(14, 18 + 2 * i, music[i]);
    }
}

/* The high score table for the game type, rows 18-24 (the level menu and the
 * name entry). The entry being named shows its letter forward. */
static void highScoreTable(int entering)
{
    text(10, 18, "NAME");
    text(16, 18, "SCORE");
    text(23, 18, "LV");
    for (int i = 0; i < 3; i++) {
        int e = (V->gameType ? 4 : 0) + i, row = 20 + 2 * i;
        char n[2] = {(char)('1' + i), 0};
        text(7, row, n);
        for (int k = 0; k < 6; k++) {
            uint8_t ch = V->highScoreNames[e * 6 + k];
            char s[2] = {ch < sizeof highScoreChars - 1 ? highScoreChars[ch] : ' ', 0};
            if (entering && i == (V->highScoreEntryRawPos & 3) && k == V->highScoreEntryNameOffsetForLetter) {
                if (s[0] == ' ')
                    s[0] = '_';
                selected((float)(9 + k), (float)row, s, 1);
            } else {
                text(9 + k, row, s);
            }
        }
        bcdText(16, row, &V->highScoreScoresA[e * 3], 3, nes(C_WHITE));
        numberText(23, row, V->highScoreLevels[e], 2, nes(C_WHITE));
    }
}

static void levelMenu(void)
{
    typeText(13, 3);
    text(9, 8, "LEVEL");
    for (int l = 0; l < 10; l++) {
        char s[2] = {(char)('0' + l), 0};
        float col = (float)(7 + 2 * (l % 5)), row = (float)(11 + 2 * (l / 5));
        if (l == V->startLevel % 10)
            selected(col, row, s, V->selectingLevelOrHeight == 0);
        else
            textAt(col, row, s, nes(C_WHITE), 0, 1);
    }
    if (V->gameType) {
        text(20, 8, "HEIGHT");
        for (int h = 0; h < 6; h++) {
            char s[2] = {(char)('0' + h), 0};
            float col = (float)(20 + 2 * (h % 3)), row = (float)(11 + 2 * (h / 3));
            if (h == V->startHeight % 6)
                selected(col, row, s, V->selectingLevelOrHeight == 1);
            else
                textAt(col, row, s, nes(C_WHITE), 0, 1);
        }
    }
    highScoreTable(0);
}

static void congratulationsScreen(void)
{
    typeText(13, 3);
    text(9, 7, "CONGRATULATIONS");
    text(11, 10, "YOU ARE A");
    text(9, 12, "TETRIS MASTER.");
    text(5, 15, "PLEASE ENTER YOUR NAME");
    highScoreTable(1);
}

static int tetrisFlash(void)
{
    return V->completedLines == 4 && !(V->frameCounter[0] & 0x03);
}

static void playScreen(const struct tetris *t)
{
    const uint32_t white = nes(C_WHITE);

    /* walls: either side of the playfield (columns 12-21, rows 6-25) and under it */
    uint32_t wall = nes(tetrisFlash() ? C_WHITE : C_GREY);
    add(TETRIS_SCENE_BOX, 11, 6, 0, 1, 20, 1, wall, wall);
    add(TETRIS_SCENE_BOX, 22, 6, 0, 1, 20, 1, wall, wall);
    add(TETRIS_SCENE_BOX, 11, 26, 0, 12, 1, 1, wall, wall);

    /* the counters */
    textAt(3, 4, V->gameType ? "B-TYPE" : "A-TYPE", white, 0, 1);
    text(13, 3, "LINES-");
    char lines[4] = {(char)('0' + (V->player1_lines[1] & 0x0F) % 10), (char)('0' + (V->player1_lines[0] >> 4) % 10),
                     (char)('0' + (V->player1_lines[0] & 0x0F) % 10), 0};
    text(19, 3, lines);
    text(24, 4, "TOP");
    bcdText(24, 5, &V->highScoreScoresA[V->gameType ? 12 : 0], 3, white);
    text(24, 7, "SCORE");
    const uint8_t score[3] = {V->player1_score[2], V->player1_score[1], V->player1_score[0]};
    bcdText(24, 8, score, 3, white);
    text(24, 13, "NEXT");
    text(24, 20, "LEVEL");
    numberText(25, 21, V->player1_levelNumber, 2, white);
    if (V->gameType) {
        text(24, 24, "HEIGHT");
        numberText(27, 25, V->startHeight & 0x0F, 1, white);
    }

    /* statistics: each piece and how many have come (BCD, three digits) */
    textAt(2.5f, 9.5f, "STATISTICS", white, 0, 0.7f);
    for (int i = 0; i < 7; i++) {
        float size = 0.6f;
        uint8_t o = tetris_piece_spawn_orientation((uint8_t)i);
        piece(4.5f - size / 2, (float)(12 + 2 * i) - size / 2, o, size, -100);
        char n[4] = {(char)('0' + (V->statsByType[2 * i + 1] & 0x0F) % 10),
                     (char)('0' + (V->statsByType[2 * i] >> 4) % 10),
                     (char)('0' + (V->statsByType[2 * i] & 0x0F) % 10), 0};
        textAt(6, (float)(12 + 2 * i) - 0.5f, n, nes(colorTable[levelColors()][2]), 0, 1);
    }

    /* the playfield as the screen shows it, then the falling piece (drawn
     * every frame, as a sprite) and the next piece */
    for (int r = 0; r < 20; r++)
        for (int c = 0; c < 10; c++) {
            uint8_t tile = t->screen.field[0][r * 10 + c];
            if (tile != 0xEF && tile != 0xFF)
                block((float)(12 + c), (float)(6 + r), tile, 1);
        }
    piece((float)(12 + V->player1_tetriminoX), (float)(6 + V->player1_tetriminoY), V->player1_currentPiece, 1, 6);
    if (!V->displayNextPiece)
        piece(25, 15, V->nextPiece, 1, -100);
}

static void endingScreen(void)
{
    if (V->gameType)
        return; /* B-type: a concert or a parade, pictures only */
    /* A-type: the rocket, rising once its countdown is over; its sprite's
     * y is ending_customVars (0 when gone) */
    uint8_t y = V->ending_customVars[0];
    if (!y)
        return;
    float row = (float)y / 8.0f;
    const uint32_t body = nes(C_LIGHT_GREY), fin = nes(C_RED);
    add(TETRIS_SCENE_BLOCK, 15.5f, row - 4, 0, 1, 1, 1, nes(C_WHITE), body);
    add(TETRIS_SCENE_BLOCK, 15.5f, row - 3, 0, 1, 1, 1, body, body);
    add(TETRIS_SCENE_BLOCK, 15.5f, row - 2, 0, 1, 1, 1, body, body);
    add(TETRIS_SCENE_BLOCK, 14.5f, row - 1, 0, 1, 1, 1, fin, fin);
    add(TETRIS_SCENE_BLOCK, 16.5f, row - 1, 0, 1, 1, 1, fin, fin);
    if (V->ending_customVars[1] == 0xF0 && (V->frameCounter[0] & 2))
        add(TETRIS_SCENE_BOX, 15.75f, row, 0.25f, 0.5f, 1, 0.5f, nes(C_ORANGE), nes(C_ORANGE));
}

void tetris_scene_build(const struct tetris *t, struct tetris_scene *s)
{
    S = s;
    V = &t->ram.v;
    s->count = 0;
    s->background = nes(C_BLACK);

    /* The camera stands back from the screen's middle (16, 15), a little to
     * the right and above, so the sides and tops of things show. */
    s->eye[0] = 25, s->eye[1] = 10, s->eye[2] = -38;
    s->target[0] = 16, s->target[1] = 15, s->target[2] = 0.5f;
    s->focal = 2.05f;
    s->light[0] = -0.35f, s->light[1] = -0.45f, s->light[2] = -0.82f; /* from the upper left, in front */
    s->ambient = 0.3f;

    if (!(V->currentPpuMask & 0x18))
        return; /* picture off */
    if (t->screen.paused) {
        text(14, 15, "PAUSE");
        return;
    }
    switch (V->renderMode) {
    case 0:
        if (V->gameMode == 0)
            legalScreen();
        else
            titleScreen();
        break;
    case 1:
        if (V->gameMode == 2)
            gameTypeMenu();
        else if (V->gameMode == 3)
            levelMenu();
        break;
    case 2: congratulationsScreen(); break;
    case 3: playScreen(t); break;
    case 4: endingScreen(); break;
    default: break;
    }
}
