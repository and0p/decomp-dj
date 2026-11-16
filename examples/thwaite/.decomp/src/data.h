/* Tables converted from Thwaite's source by tools/gen_assets.py. */
#ifndef THW_DATA_H
#define THW_DATA_H

/* The 8 KB of CHR: two pattern tables of 256 tiles, 16 bytes a tile (two bit planes). */
extern const unsigned char thw_chr[8192];

/* The NES master palette the baseline draws with (Mesen's default), as 0xRRGGBB. */
extern const unsigned thw_master_palette[64];

#endif
