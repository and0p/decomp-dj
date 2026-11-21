//! Colour for the command line's verdicts (docs/cli.md): red for what failed, green for
//! what passed, yellow for what was left out. On when the stream is a terminal or `CI` is
//! set (CI logs show colour); off with `NO_COLOR`; forced with `CLICOLOR_FORCE`. JSON and
//! the MCP server never use it: they don't print through here.

use std::io::IsTerminal;
use std::sync::OnceLock;

/// How a verdict reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Bad,
    Note,
    Plain,
}

fn set(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty() && v != "0" && v != "false")
}

fn decide(terminal: bool) -> bool {
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    set("CLICOLOR_FORCE") || terminal || set("CI")
}

/// Whether stdout gets colour.
pub fn stdout_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| decide(std::io::stdout().is_terminal()))
}

/// Whether stderr gets colour.
pub fn stderr_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| decide(std::io::stderr().is_terminal()))
}

/// The tone of a verdict word, as the commands print them.
pub fn tone_of(verdict: &str) -> Tone {
    match verdict.trim() {
        "ok" | "passed" | "cleaned" | "recorded" | "cleared" | "done" => Tone::Good,
        "FAILED" | "failed" | "changed" | "dirty" | "crashed" | "hung" | "blocked" | "mismatch" | "earlier" | "refused" => Tone::Bad,
        "new" | "skipped" | "gone" | "stored" => Tone::Note,
        _ => Tone::Plain,
    }
}

fn paint(text: &str, tone: Tone, on: bool) -> String {
    let code = match tone {
        Tone::Good => "32",
        Tone::Bad => "1;31",
        Tone::Note => "33",
        Tone::Plain => return text.to_string(),
    };
    if on {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// `verdict` padded to `width` and coloured by its tone, for stdout.
pub fn verdict(verdict: &str, width: usize) -> String {
    paint(&format!("{verdict:<width$}"), tone_of(verdict), stdout_on())
}

/// A word for stdout, in the tone given.
pub fn out(text: &str, tone: Tone) -> String {
    paint(text, tone, stdout_on())
}

/// A word for stderr, in the tone given.
pub fn err(text: &str, tone: Tone) -> String {
    paint(text, tone, stderr_on())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones_and_padding() {
        assert_eq!(tone_of("FAILED"), Tone::Bad);
        assert_eq!(tone_of("ok  "), Tone::Good);
        assert_eq!(tone_of("new"), Tone::Note);
        assert_eq!(paint("ok      ", Tone::Good, true), "\x1b[32mok      \x1b[0m");
        assert_eq!(paint("ok", Tone::Good, false), "ok");
        assert_eq!(paint("x", Tone::Plain, true), "x");
    }
}
