/* The log callback dj gives a core (`retro_log_printf_t`). It is C because stable Rust can't
   define a variadic function, and the arguments have to be read here: where they are passed
   differs by system (on the stack on arm64 macOS, in registers elsewhere). Fills in the
   format and hands the text to `dj_core_log_text` in host.rs. */
#include <stdarg.h>
#include <stdio.h>

void dj_core_log_text(unsigned level, const char *text);

void dj_core_log(unsigned level, const char *fmt, ...)
{
    char text[4096];
    va_list va;
    if (!fmt || level == 0) return; /* debug messages: too many */
    va_start(va, fmt);
    vsnprintf(text, sizeof text, fmt, va);
    va_end(va);
    dj_core_log_text(level, text);
}
