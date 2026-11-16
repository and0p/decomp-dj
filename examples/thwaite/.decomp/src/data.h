/* Tables converted from Thwaite's source by tools/gen_assets.py. */
#ifndef THW_DATA_H
#define THW_DATA_H

/* The 8 KB of CHR: two pattern tables of 256 tiles, 16 bytes a tile (two bit planes). */
extern const unsigned char thw_chr[8192];

/* The NES master palette the baseline draws with (Mesen's default), as 0xRRGGBB. */
extern const unsigned thw_master_palette[64];

/* Screen texts as tile numbers: 0x0A ends a row, 0 ends the text (texts.txt). */
extern const unsigned char thw_todo_txt[];
extern const unsigned char thw_practice_txt[];
/* The tips across the top of the playfield (tips.txt), by number. */
extern const unsigned char *const thw_tips[];
extern const unsigned thw_tip_count;

/* The cutscenes (cutscripts.txt), compressed as the ROM holds them: `thw_cut_data` is the
 * ROM's bytes from CPU address `thw_cut_data_addr`, and `thw_cut_scripts` gives each
 * script's address. Byte-pair codes ($80 and up) expand through `thw_dte_replacements`. */
extern const unsigned char thw_dte_replacements[];
extern const unsigned short thw_cut_data_addr;
extern const unsigned thw_cut_data_size;
extern const unsigned char thw_cut_data[];
extern const unsigned short thw_cut_scripts[];
extern const unsigned thw_cut_script_count;

#endif
