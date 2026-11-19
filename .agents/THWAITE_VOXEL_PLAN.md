# Thwaite decomp: a voxel render style

**Status: done.**

How the build differs from the plan below:

- **Camera:**
  - It frames the screen's nearer edge, not the houses: that edge's row just fills the
    width and sits on the screen's edge, and the far edge recedes. Framing on the houses
    left a band of nothing under the ground.
  - A negative tilt puts the camera above. 22 (below) stays the default, because from
    above the playfield floats in the middle of the picture.
- **Backdrop:** a rectangle the size of the screen, behind every slab, with black around
  it. The plan cleared to the backdrop colour, which made the cutscene's black letterbox
  look like a slab floating in blue sky.
- **Meshes:**
  - no back faces, since the camera is always in front;
  - one quad per pixel face, not merged into strips;
  - instances sorted by tile in an integer texture, one `glDrawArraysInstanced` per tile.
    Instanced attributes need GL 3.3, and the context is 3.2.
- **Outline:** an option in percent.

**Checked with `thwaite_pictures`:**
- With tilt 0, a 10° field of view and no outline, the voxel picture sampled at each 4x4
  block's centre equals the flat picture on the title, play, pause, practice and a
  cutscene. The only exception is the cutscene's trees, 8 pixels (below).
- `dj parity --all` passes in the same time, because it asks for no video.
- **Not looked at in dj-gui itself:** it fits the comparison picture to its pane by the
  texture's size.

An optional way for the decomp (`examples/thwaite/.decomp`) to draw its picture. It extrudes
every tile and sprite one voxel deep and draws them in perspective. Each voxel gets a
subtle outline. The colours stay the NES's: palette indices, looked up when drawn.

It is presentation only. `struct gfx` (the tile map, palette, OAM, scroll and splits) is
still the whole input, and the game logic doesn't change. Parity is unaffected, because it
compares RAM, not pictures.

## The option

- A new libretro variable, `thwaite_decomp_render`: `flat|voxel`. Flat is the default and
  is today's renderer, unchanged.
- It is set for the project in `examples/thwaite/config.toml`, under the decomp's
  `[[compare]]` entry:

  ```toml
  [compare.options]
  thwaite_decomp_render = "voxel"
  ```

- `thwaite_pictures` checks pixel parity of the flat picture. It forces `flat` (or reads
  an override), so it stays a pixel test.
- The core reads the variable at load and again when the frontend says variables changed.
  Switching between frames works because both renderers draw from the same `struct gfx`.
- Output size:
  - The geometry is base 256x240 and max 1024x960. dj sizes its GL context from the max
    (`player/src/libretro.rs`), so both styles fit in one context.
  - Voxel mode draws at 4x (1024x960) and reports it with `SET_GEOMETRY` when the option
    changes.
  - The hardware render callback asks for a depth buffer (`hw.depth = true`).

## What becomes voxels

| Layer | In voxel mode |
| --- | --- |
| Backdrop (palette entry 0) | The clear colour: the flat sky behind everything, never extruded |
| BG tiles | Every opaque pixel (colour index 1-3) is a voxel. Slab from z = 0 to z = 1 (in NES pixels) |
| Sprites in front of the BG | Slab from z = 1 to z = 2, nudged toward the camera by OAM index (lower index nearer), so the console's "lowest index wins" mostly holds |
| Sprites behind the BG (attr 0x20) | Slab from z = -1 to z = 0: hidden by BG voxels, seen through the BG's transparent pixels, as on the console |

- Stars, houses, silos, the ground, text, missiles, explosions, the cutscene's trees and
  villagers all extrude. The night sky, being the backdrop, doesn't.
- Text sits 1 px deep on the BG plane. It stays readable from the camera's tilt, which is
  gentle (below).

## Geometry: meshes built from CHR

CHR is ROM, so the meshes are built once at `gl_init`:

- **One mesh per tile** (512: both pattern tables), in tile-local coordinates (0-8, 0-8,
  0-1). It has:
  - a front face and a back face for each opaque pixel, merged by row into strips where
    the colour index runs on (fewer vertices; the colour still comes from the CHR texture,
    below);
  - a side face wherever an opaque pixel borders a transparent pixel or the tile's edge.
    Faces between two opaque pixels are never built, so the inside is hollow.
- Each vertex carries:
  - its position, in tile pixels;
  - the face's normal, as an index (0-5: front, back, ±x, ±y);
  - the texel it belongs to (0-7, 0-7). This is the "UV" that the colour and the outline
    come from.
- Flips are done in the vertex shader (mirror x or y of the position and texel), so a tile
  needs only one mesh.
- All 512 tile meshes share one vertex buffer, with a table of `(first, count)` per tile.
- Size: a busy tile is at most about 64 faces × 6 directions. Thwaite's tiles come to a few
  hundred kB in all.

**Instances, every frame, built on the CPU from `struct gfx`:**

- **BG:** one per visible map cell: up to 33×31 = about 1,000, counting the partly
  scrolled column and row. Each carries the cell position after scroll, the tile, the
  pattern table and the palette.
  - The splits (the title's table switch at a row, the cutscene's scroll reset at row 18)
    are applied here, row by row, exactly as the flat shader does.
  - Cells whose tile has no opaque pixels are skipped, so the empty sky costs nothing.
- **Sprites:** one per sprite in OAM with y < 0xEF, at most 64. Each carries x, y, tile,
  palette, flips, priority and OAM index.
- **Drawing:**
  - one `glDrawArraysInstanced` call for each distinct tile mesh used;
  - or, simpler: `glMultiDrawArrays`-free, one draw per instance. About 1,100 draws a
    frame is fine on desktop GL.

  Start with one draw per instance and batch by tile only if it shows up in a profile.

## Shaders (GLSL 150, the existing context)

### Vertex

```
in vec3  a_pos;      // tile-local, pixels; z 0..1
in float a_face;     // 0-5
in vec2  a_texel;    // 0..7
uniform  ivec4 u_inst;   // x, y (screen pixels), tile | table<<8, palette | flips<<2 | layer<<4
uniform  float u_z;      // the slab's base z (layer, plus the OAM nudge)
uniform  mat4  u_view_proj;
out vec2 v_texel_uv;  // the texel, plus the position within it (for the outline)
flat out int v_tile, v_palette, v_face;
```

- Apply the flips to `a_pos.xy` and `a_texel`.
- Place the vertex at `inst.xy + pos.xy` and `u_z + pos.z`.
- Pass on the texel and the position within it: `fract` of the pixel position along the
  face's two in-plane axes.

### Fragment

1. **Palette index:** `texelFetch(u_chr, tile_origin + texel)`. This is the same CHR
   integer texture the flat renderer uses. It is always 1-3 here, since transparent pixels
   have no faces.
2. **Colour:** `u_palette[palette * 4 + index]`, which is the NES colour number from
   `gfx.palette`, uploaded each frame as 32 ints. Then the RGB comes from the 64-entry
   NES palette texture (Mesen's default, as now). Fades, the cutscene's fade-in and
   palette changes work for free.
3. **Light:** a fixed light from the upper left, toward the camera:
   - front face × 1.0;
   - top faces × 1.08 (clamped);
   - left and right faces × 0.8;
   - bottom faces × 0.65.

   Flat shading per face, as in the reference picture. No specular.
4. **Outline:** the "subtle outlines" are a darkening at the edge of each voxel face:

   ```
   vec2 d = min(v_in_voxel, 1.0 - v_in_voxel);           // distance to the voxel's edges
   float e = smoothstep(0.0, u_line, min(d.x, d.y));     // u_line ~ 0.08 of a voxel
   color *= mix(1.0 - u_outline, 1.0, e);                // u_outline ~ 0.18
   ```

   - The edge distance is computed with screen-space derivatives (`fwidth`), so the line
     stays about one output pixel wide at any distance. That is the antialiased, even
     line in the picture.
   - Faces merged into strips still outline each voxel, because the outline comes from the
     position within the voxel, not from the face's edges.
5. Output is opaque, with no blending. The depth test does the layering.

**Explosion and smoke flicker** (`thwaite_decomp_flicker`) are OAM changes from frame to
frame, so they come through as they are. `blend` needs alpha and is left to the flat
renderer for now.

## The camera

Perspective, looking at the z = 0 plane (the BG) from in front and a little below its
middle, so the ground and houses lean toward the viewer. The reference picture's look:

- **FOV:** 50°, wider than a flat view's would be. With the camera close, that shows the
  depth: sides of houses, silos and missiles.
- **Tilt:** about 22° about the x axis. It pivots on the screen's lower third (row 160),
  where the action is, so the ground is nearest and the sky recedes.
- **Distance:** chosen so the plane's 256 px width just fills the view at the pivot row
  ("panned in"). The top corners fall a little outside, cropping some empty sky. The
  status bar (rows 26-27) and the houses stay whole.
- **Fixed:** the camera doesn't follow anything. The cutscene's horizontal scroll moves
  the world under it, as now.
- **Pause (sprite table 0)** and every other screen use the same camera.
- **Options**, if the default needs tuning per taste:
  - `thwaite_decomp_voxel_tilt`: `22|0|10|30`;
  - `thwaite_decomp_voxel_fov`: `50|30|70`.

  0° tilt with a narrow FOV is close to the flat picture, with outlines.

## What doesn't carry over

- **The 8-sprites-a-line limit**, and the low-index quirk where a sprite behind the BG hides
  a higher-numbered one in front:
  - The depth nudge by OAM index keeps the order among sprites.
  - The behind-BG case differs only where the cutscene's tree leaves overlap the trunk.
    Up close in 3D, that reads as the leaves being in front, which is what they are meant
    to be.
  - Thwaite doesn't rely on the 8-a-line dropout for anything the player should see.
- **Exact pixels.** Voxel pictures can't be compared with the original's.
  `thwaite_pictures` keeps checking the flat renderer.

## Steps

1. **Option plumbing.**
   - Add the variable and read it; geometry max 1024x960; `hw.depth`.
   - `SET_GEOMETRY` on change.
   - `thwaite_pictures` forces flat.
2. **Tile meshes from CHR**, with a CPU check: front-face pixel count = opaque pixel count
   for every tile.
3. **Instances and the voxel program.** Front faces only, flat camera (0° tilt, framed to
   256x240).
   - Compare with the flat renderer at 1/4 scale: it should match, apart from the
     outlines, on every frame `thwaite_pictures` takes.
   - This proves layering, palettes, flips and splits before the camera moves.
4. **Sides, lighting, outlines; then the camera.**
   - Pictures of the title, a busy game frame (explosions, MIRVs, balloons), pause, the
     practice menu, and a cutscene mid-scroll.
5. **Config and docs.**
   - `[compare.options]` in `examples/thwaite/config.toml`.
   - The decomp README: the option, and what doesn't carry over.
   - Check that dj-gui shows the 1024x960 picture beside the original's.
   - Check that `dj parity` (which doesn't ask for video) is no slower.
