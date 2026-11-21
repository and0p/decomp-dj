/* A tiny libretro core that draws with OpenGL (SET_HW_RENDER), for the player's tests.
 *
 * Each frame clears a 64x48 picture to a colour that changes with the frame count, then
 * paints the image's top-left quarter white, so a test can check colour and orientation.
 * Core options: glcore_profile = core|compat, glcore_origin = bottom|top
 * (bottom_left_origin true or false). RAM byte 0 counts frames; save states hold the RAM.
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define W 64
#define H 48

struct retro_game_info { const char *path; const void *data; size_t size; const char *meta; };
struct retro_system_info { const char *library_name, *library_version, *valid_extensions; bool need_fullpath, block_extract; };
struct retro_game_geometry { unsigned base_width, base_height, max_width, max_height; float aspect_ratio; };
struct retro_system_timing { double fps, sample_rate; };
struct retro_system_av_info { struct retro_game_geometry geometry; struct retro_system_timing timing; };
struct retro_variable { const char *key, *value; };
typedef void (*proc_t)(void);
struct retro_hw_render_callback {
    unsigned context_type;
    void (*context_reset)(void);
    uintptr_t (*get_current_framebuffer)(void);
    proc_t (*get_proc_address)(const char *);
    bool depth, stencil, bottom_left_origin;
    unsigned version_major, version_minor;
    bool cache_context;
    void (*context_destroy)(void);
    bool debug_context;
};

typedef bool (*env_t)(unsigned, void *);
typedef void (*video_t)(const void *, unsigned, unsigned, size_t);
typedef int16_t (*input_t)(unsigned, unsigned, unsigned, unsigned);

static env_t env;
static video_t video;
static input_t input;
static void (*poll_cb)(void);
static uint8_t ram[64];
static struct retro_hw_render_callback hw;
static int resets, destroys;

typedef void (*clear_color_t)(float, float, float, float);
typedef void (*u32_t)(unsigned);
typedef void (*bind_fb_t)(unsigned, unsigned);
typedef void (*rect_t)(int, int, int, int);
static clear_color_t gl_clear_color;
static u32_t gl_clear, gl_enable, gl_disable;
static bind_fb_t gl_bind_framebuffer;
static rect_t gl_viewport, gl_scissor;

#define GL_FRAMEBUFFER 0x8D40
#define GL_COLOR_BUFFER_BIT 0x4000
#define GL_SCISSOR_TEST 0x0C11

static void context_reset(void) {
    gl_clear_color = (clear_color_t)hw.get_proc_address("glClearColor");
    gl_clear = (u32_t)hw.get_proc_address("glClear");
    gl_enable = (u32_t)hw.get_proc_address("glEnable");
    gl_disable = (u32_t)hw.get_proc_address("glDisable");
    gl_bind_framebuffer = (bind_fb_t)hw.get_proc_address("glBindFramebuffer");
    gl_viewport = (rect_t)hw.get_proc_address("glViewport");
    gl_scissor = (rect_t)hw.get_proc_address("glScissor");
    resets++;
}
static void context_destroy(void) { destroys++; }

void retro_set_environment(env_t cb) {
    env = cb;
    static const struct retro_variable vars[] = {
        { "glcore_profile", "Profile; core|compat" },
        { "glcore_origin", "Origin; bottom|top" },
        { NULL, NULL },
    };
    env(16, (void *)vars);
}
void retro_set_video_refresh(video_t cb) { video = cb; }
void retro_set_audio_sample(void (*cb)(int16_t, int16_t)) { (void)cb; }
void retro_set_audio_sample_batch(size_t (*cb)(const int16_t *, size_t)) { (void)cb; }
void retro_set_input_poll(void (*cb)(void)) { poll_cb = cb; }
void retro_set_input_state(input_t cb) { input = cb; }
void retro_init(void) { memset(ram, 0, sizeof ram); }
void retro_deinit(void) {}
unsigned retro_api_version(void) { return 1; }
void retro_get_system_info(struct retro_system_info *info) {
    memset(info, 0, sizeof *info);
    info->library_name = "glcore";
    info->library_version = "1";
    info->valid_extensions = "bin";
}
void retro_get_system_av_info(struct retro_system_av_info *info) {
    info->geometry = (struct retro_game_geometry){ W, H, W, H, 4.0f / 3.0f };
    info->timing = (struct retro_system_timing){ 60.0, 44100.0 };
}
void retro_set_controller_port_device(unsigned port, unsigned device) { (void)port; (void)device; }
void retro_reset(void) { memset(ram, 0, sizeof ram); }

static bool option(const char *key, const char *value) {
    struct retro_variable v = { key, NULL };
    return env(15, &v) && v.value && strcmp(v.value, value) == 0;
}

bool retro_load_game(const struct retro_game_info *game) {
    (void)game;
    memset(&hw, 0, sizeof hw);
    bool compat = option("glcore_profile", "compat");
    hw.context_type = compat ? 1 : 3; /* OPENGL or OPENGL_CORE */
    hw.version_major = compat ? 0 : 3;
    hw.version_minor = compat ? 0 : 2;
    hw.context_reset = context_reset;
    hw.context_destroy = context_destroy;
    hw.bottom_left_origin = !option("glcore_origin", "top");
    hw.depth = true;
    hw.stencil = true;
    return env(14, &hw);
}
void retro_unload_game(void) {}
bool retro_load_game_special(unsigned t, const struct retro_game_info *g, size_t n) { (void)t; (void)g; (void)n; return false; }
unsigned retro_get_region(void) { return 0; }

void retro_run(void) {
    if (poll_cb) poll_cb();
    ram[0]++;
    gl_bind_framebuffer(GL_FRAMEBUFFER, (unsigned)hw.get_current_framebuffer());
    gl_viewport(0, 0, W, H);
    gl_disable(GL_SCISSOR_TEST);
    gl_clear_color((float)(ram[0] % 8) / 7.0f, 0.25f, 0.5f, 1.0f);
    gl_clear(GL_COLOR_BUFFER_BIT);
    /* The image's top-left quarter: the framebuffer's top rows with GL's usual origin,
     * its first rows when the core says it draws top-left first. */
    gl_enable(GL_SCISSOR_TEST);
    gl_scissor(0, hw.bottom_left_origin ? H / 2 : 0, W / 2, H / 2);
    gl_clear_color(1, 1, 1, 1);
    gl_clear(GL_COLOR_BUFFER_BIT);
    gl_disable(GL_SCISSOR_TEST);
    video((const void *)(uintptr_t)-1, W, H, 0);
}

size_t retro_serialize_size(void) { return sizeof ram; }
bool retro_serialize(void *data, size_t size) {
    if (size < sizeof ram) return false;
    memcpy(data, ram, sizeof ram);
    return true;
}
bool retro_unserialize(const void *data, size_t size) {
    if (size < sizeof ram) return false;
    memcpy(ram, data, sizeof ram);
    return true;
}
void retro_cheat_reset(void) {}
void retro_cheat_set(unsigned i, bool e, const char *c) { (void)i; (void)e; (void)c; }
void *retro_get_memory_data(unsigned id) { return id == 2 ? ram : NULL; }
size_t retro_get_memory_size(unsigned id) { return id == 2 ? sizeof ram : 0; }

/* For the tests: how many times the frontend called context_reset and context_destroy. */
int glcore_resets(void) { return resets; }
int glcore_destroys(void) { return destroys; }
