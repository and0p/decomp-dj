cmake_minimum_required(VERSION 3.16)
project(thwaite_decomp C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_VISIBILITY_PRESET hidden)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()

# The build id dj records: thwaite-decomp@<commit, 8 hex digits>, with -dirty for
# uncommitted changes under this folder.
execute_process(COMMAND git rev-parse --short=8 HEAD
                WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
                OUTPUT_VARIABLE THW_COMMIT OUTPUT_STRIP_TRAILING_WHITESPACE ERROR_QUIET)
execute_process(COMMAND git status --porcelain -- .
                WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
                OUTPUT_VARIABLE THW_DIRTY OUTPUT_STRIP_TRAILING_WHITESPACE ERROR_QUIET)
if(NOT THW_COMMIT)
  set(THW_COMMIT "dev")
endif()
if(THW_DIRTY)
  set(THW_COMMIT "${THW_COMMIT}-dirty")
endif()

add_library(thwaite_decomp_libretro SHARED
  src/libretro.c
  src/events.c
  src/main.c
  src/title.c
  src/pads.c
  src/compare.c
  src/gfx.c
  src/gl.c
  src/data/chr.c
  src/data/palette.c
  src/data/map.c
  src/data/texts.c
)
set_target_properties(thwaite_decomp_libretro PROPERTIES PREFIX "")
target_compile_definitions(thwaite_decomp_libretro PRIVATE THW_BUILD_ID="thwaite-decomp@${THW_COMMIT}")
# A build with a bug planted on purpose, to check that parity catches it (README).
option(THW_BREAK_ON_PURPOSE "Plant a known bug" OFF)
if(THW_BREAK_ON_PURPOSE)
  target_compile_definitions(thwaite_decomp_libretro PRIVATE THW_BREAK_ON_PURPOSE)
endif()
if(MSVC)
  target_compile_options(thwaite_decomp_libretro PRIVATE /W4)
else()
  target_compile_options(thwaite_decomp_libretro PRIVATE -Wall -Wextra -Wno-unused-parameter)
endif()
# The libretro entry points are exported by libretro.h's RETRO_API (visibility default).
