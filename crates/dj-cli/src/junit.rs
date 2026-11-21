//! JUnit XML reports (`--junit FILE`, or `DJ_JUNIT=DIR`) for CI systems: one test case per
//! segment (docs/ci.md). GitLab, Jenkins, Azure DevOps, CircleCI and Buildkite read them as
//! test results; GitHub through a reporter action.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use dj_core::collection::Collection;

/// The folder every command writes its report into when `--junit` isn't given.
pub const DIR_ENV: &str = "DJ_JUNIT";

pub enum Outcome {
    Pass,
    /// `message`: one line; `body`: the details.
    Fail { message: String, body: String },
    Skip { message: String },
}

pub struct Case {
    /// The project's name, so reports from several projects stay apart.
    pub classname: String,
    /// The segment's short id and title.
    pub name: String,
    /// The segment file, relative to the working directory where it can be.
    pub file: String,
    /// The segment file's line for the frame a failure is at.
    pub line: Option<usize>,
    pub seconds: f64,
    pub outcome: Outcome,
}

impl Case {
    /// A case for segment `id`.
    pub fn segment(col: &Collection, id: &str, seconds: f64, outcome: Outcome) -> Case {
        let title = col.replays.get(id).map(|r| r.title()).unwrap_or("");
        Case { classname: col.config.name.clone(), name: format!("{} {title}", &id[..8.min(id.len())]).trim_end().to_string(), file: segment_file(col, id), line: None, seconds, outcome }
    }
}

/// Where to write a report: `explicit` (`--junit FILE`), else `$DJ_JUNIT/<file_name>`, else
/// nowhere.
pub fn target(explicit: Option<&Path>, file_name: &str) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    std::env::var_os(DIR_ENV).filter(|v| !v.is_empty()).map(|d| PathBuf::from(d).join(file_name))
}

/// A file name made from a `[[compare]]` entry's name: letters, digits, `-` and `_` kept.
pub fn file_part(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

/// The segment's file, relative to the working directory when it is inside it: what a CI
/// reporter resolves against the checkout.
pub fn segment_file(col: &Collection, id: &str) -> String {
    let Some(p) = col.paths.get(id) else { return String::new() };
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
    let rel = std::env::current_dir().ok().and_then(|cwd| std::fs::canonicalize(cwd).ok()).and_then(|cwd| abs.strip_prefix(&cwd).ok().map(Path::to_path_buf));
    rel.unwrap_or(abs).to_string_lossy().replace('\\', "/")
}

/// The segment file's line for `frame`: the last body line at or before it, or None when
/// the segment has no line by then.
pub fn segment_line(col: &Collection, id: &str, frame: u64) -> Option<usize> {
    col.replays.get(id)?.lines.iter().take_while(|l| l.frame <= frame).last().map(|l| l.line_no)
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 allows no other control characters.
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Writes one `<testsuite>` named `suite`, holding `cases`, to `path`.
pub fn write(path: &Path, suite: &str, cases: &[Case]) -> Result<()> {
    let failures = cases.iter().filter(|c| matches!(c.outcome, Outcome::Fail { .. })).count();
    let skipped = cases.iter().filter(|c| matches!(c.outcome, Outcome::Skip { .. })).count();
    let time: f64 = cases.iter().map(|c| c.seconds).sum();
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x += &format!("<testsuites name=\"{}\" tests=\"{}\" failures=\"{failures}\" errors=\"0\" skipped=\"{skipped}\" time=\"{time:.3}\">\n", esc(suite), cases.len());
    x += &format!("  <testsuite name=\"{}\" tests=\"{}\" failures=\"{failures}\" errors=\"0\" skipped=\"{skipped}\" time=\"{time:.3}\">\n", esc(suite), cases.len());
    for c in cases {
        let line = c.line.map(|l| format!(" line=\"{l}\"")).unwrap_or_default();
        x += &format!("    <testcase classname=\"{}\" name=\"{}\" file=\"{}\"{line} time=\"{:.3}\"", esc(&c.classname), esc(&c.name), esc(&c.file), c.seconds);
        match &c.outcome {
            Outcome::Pass => x += "/>\n",
            Outcome::Fail { message, body } => x += &format!(">\n      <failure message=\"{}\" type=\"failure\">{}</failure>\n    </testcase>\n", esc(message), esc(body)),
            Outcome::Skip { message } => x += &format!(">\n      <skipped message=\"{}\"/>\n    </testcase>\n", esc(message)),
        }
    }
    x += "  </testsuite>\n</testsuites>\n";
    if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    std::fs::write(path, x).with_context(|| format!("writing {}", path.display()))?;
    eprintln!("wrote {} ({} test cases, {failures} failed, {skipped} skipped)", path.display(), cases.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_names() {
        assert_eq!(esc("a<b & \"c\"\u{1}"), "a&lt;b &amp; &quot;c&quot;\\x01");
        assert_eq!(file_part("mesen (same build)"), "mesen--same-build-");
    }
}
