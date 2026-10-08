//! NUL-safe paths and numbered rows from one read-only Git diff snapshot.
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
    Header,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub hunk: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffFile {
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    pub status: String,
    pub binary: bool,
    pub lines: Vec<DiffLine>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewDiff {
    pub base: String,
    pub revision: String,
    pub files: Vec<DiffFile>,
}
pub fn review_base_command(worktree: &Path, base: &str) -> io::Result<Command> {
    if base.is_empty() || base.starts_with('-') || base.contains(['\0', '\n', '\r']) {
        return Err(io::Error::other("Invalid review base reference"));
    }
    let mut command = Command::new(crate::cli::git_binary());
    command
        .current_dir(worktree)
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "merge-base",
            "--",
            "HEAD",
            base,
        ])
        .env("GIT_TERMINAL_PROMPT", "0");
    Ok(command)
}
pub fn review_diff_command(worktree: &Path, base: &str) -> io::Result<Command> {
    if !matches!(base.len(), 40 | 64) || !base.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("Diff base must be a resolved commit"));
    }
    let mut c = Command::new(crate::cli::git_binary());
    c.current_dir(worktree)
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "diff.suppressBlankEmpty=false",
            "diff",
            "--raw",
            "-z",
            "--patch",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--find-renames",
            "--unified=3",
            base,
            "--",
        ])
        .env("GIT_TERMINAL_PROMPT", "0");
    Ok(c)
}
pub fn parse_review_diff(base: &str, bytes: &[u8]) -> io::Result<ReviewDiff> {
    let revision = gix::objs::compute_hash(gix::hash::Kind::Sha1, gix::objs::Kind::Blob, bytes)
        .map_err(io::Error::other)?
        .to_string();
    let mut result = ReviewDiff {
        base: base.into(),
        revision,
        files: Vec::new(),
    };
    if bytes.is_empty() {
        return Ok(result);
    }
    let split = bytes
        .windows(2)
        .position(|b| b == [0, 0])
        .ok_or_else(|| io::Error::other("Incomplete raw diff paths"))?;
    let mut raw = bytes[..split].split(|b| *b == 0);
    while let Some(header) = raw.next() {
        let header = std::str::from_utf8(header).map_err(io::Error::other)?;
        let fields: Vec<_> = header.split_ascii_whitespace().collect();
        if fields.len() != 5 || !fields[0].starts_with(':') {
            return Err(io::Error::other("Invalid raw diff record"));
        }
        let status = fields[4];
        let path = raw
            .next()
            .filter(|p| !p.is_empty())
            .ok_or_else(|| io::Error::other("Missing diff path"))?;
        let new = if status.starts_with(['R', 'C']) {
            raw.next()
                .filter(|p| !p.is_empty())
                .ok_or_else(|| io::Error::other("Missing rename destination"))?
        } else {
            path
        };
        result.files.push(DiffFile {
            old_path: gix::path::try_from_bstr(gix::bstr::BStr::new(path))
                .map_err(io::Error::other)?
                .into_owned(),
            new_path: gix::path::try_from_bstr(gix::bstr::BStr::new(new))
                .map_err(io::Error::other)?
                .into_owned(),
            status: status.into(),
            binary: false,
            lines: Vec::new(),
        });
    }
    let patches = &bytes[split + 2..];
    // Hunk content is prefixed with a space, plus or minus. A bare diff header
    // therefore separates files without parsing quoted or newline-containing paths.
    let mut chunks = Vec::new();
    let mut at = 0;
    for i in 0..patches.len() {
        if (i == 0 || patches[i - 1] == b'\n') && patches[i..].starts_with(b"diff --git ") {
            if i > at {
                chunks.push(&patches[at..i]);
            }
            at = i;
        }
    }
    if at < patches.len() {
        chunks.push(&patches[at..]);
    }
    if chunks.len() != result.files.len() {
        return Err(io::Error::other("Diff paths and patches do not match"));
    }
    for (file, chunk) in result.files.iter_mut().zip(chunks) {
        parse_patch(file, chunk)?;
    }
    Ok(result)
}
fn range(word: &str, prefix: char) -> io::Result<(u32, u32)> {
    let word = word
        .strip_prefix(prefix)
        .ok_or_else(|| io::Error::other("Invalid diff range"))?;
    let (start, count) = word.split_once(',').unwrap_or((word, "1"));
    Ok((
        start.parse().map_err(io::Error::other)?,
        count.parse().map_err(io::Error::other)?,
    ))
}
fn parse_patch(file: &mut DiffFile, chunk: &[u8]) -> io::Result<()> {
    let Ok(text) = std::str::from_utf8(chunk) else {
        file.binary = true;
        return Ok(());
    };
    if text
        .lines()
        .any(|l| l.starts_with("Binary files ") || l == "GIT binary patch")
    {
        file.binary = true;
        return Ok(());
    }
    let (mut old, mut new, mut old_left, mut new_left, mut hunk) =
        (0_u32, 0_u32, 0_u32, 0_u32, 0_usize);
    for line in text.split_terminator('\n') {
        if line.starts_with("@@ ") {
            if old_left != 0 || new_left != 0 {
                return Err(io::Error::other("Truncated diff hunk"));
            }
            let mut parts = line.split_ascii_whitespace();
            parts.next();
            (old, old_left) = range(parts.next().unwrap_or(""), '-')?;
            (new, new_left) = range(parts.next().unwrap_or(""), '+')?;
            if parts.next() != Some("@@") {
                return Err(io::Error::other("Invalid diff hunk"));
            }
            hunk += 1;
            file.lines.push(DiffLine {
                kind: DiffLineKind::Header,
                text: line.into(),
                old: None,
                new: None,
                hunk,
            });
            continue;
        }
        if hunk == 0 {
            continue;
        }
        if line.starts_with('\\') {
            file.lines.push(DiffLine {
                kind: DiffLineKind::Header,
                text: line.into(),
                old: None,
                new: None,
                hunk,
            });
            continue;
        }
        let (kind, old_line, new_line) = match line.as_bytes().first() {
            Some(b' ') => (DiffLineKind::Context, Some(old), Some(new)),
            Some(b'+') => (DiffLineKind::Added, None, Some(new)),
            Some(b'-') => (DiffLineKind::Removed, Some(old), None),
            _ => return Err(io::Error::other("Invalid diff line")),
        };
        if old_line.is_some() {
            old_left = old_left
                .checked_sub(1)
                .ok_or_else(|| io::Error::other("Invalid old line count"))?;
            old = old
                .checked_add(1)
                .ok_or_else(|| io::Error::other("Diff line overflow"))?;
        }
        if new_line.is_some() {
            new_left = new_left
                .checked_sub(1)
                .ok_or_else(|| io::Error::other("Invalid new line count"))?;
            new = new
                .checked_add(1)
                .ok_or_else(|| io::Error::other("Diff line overflow"))?;
        }
        file.lines.push(DiffLine {
            kind,
            text: line[1..].into(),
            old: old_line,
            new: new_line,
            hunk,
        });
    }
    if old_left != 0 || new_left != 0 {
        return Err(io::Error::other("Truncated diff hunk"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nul_paths_and_renames_keep_old_new_line_numbers() {
        let bytes=b":100644 100644 abc def R069\0old name\nfile\0new name\nfile\0\0diff --git ignored ignored\nsimilarity index 69%\n@@ -2,2 +2,3 @@\n same\n-old\n+new\n+extra\n";
        let d = parse_review_diff("base", bytes).unwrap();
        assert_eq!(d.files.len(), 1);
        let f = &d.files[0];
        assert_eq!(f.old_path, Path::new("old name\nfile"));
        assert_eq!(f.new_path, Path::new("new name\nfile"));
        assert_eq!((f.lines[1].old, f.lines[1].new), (Some(2), Some(2)));
        assert_eq!((f.lines[2].old, f.lines[2].new), (Some(3), None));
        assert_eq!((f.lines[3].old, f.lines[3].new), (None, Some(3)));
        assert_eq!((f.lines[4].old, f.lines[4].new), (None, Some(4)));
    }
    #[test]
    fn deletion_and_multiple_hunks_keep_sides_and_reject_truncation() {
        let deleted=b":100644 000000 abc 000 D\0deleted.txt\0\0diff --git a/deleted.txt b/deleted.txt\n@@ -1,2 +0,0 @@\n-old\n-last\n";
        let d = parse_review_diff("base", deleted).unwrap();
        assert_eq!(
            (d.files[0].lines[2].old, d.files[0].lines[2].new),
            (Some(2), None)
        );
        assert!(parse_review_diff("base", &deleted[..deleted.len() - 6]).is_err());
        let two=b":100644 100644 abc def M\0two.txt\0\0diff --git a/two.txt b/two.txt\n@@ -1 +1 @@\n-old\n+new\n@@ -20 +20 @@\n-later\n+updated\n";
        let d = parse_review_diff("base", two).unwrap();
        assert_eq!(d.files[0].lines[2].hunk, 1);
        assert_eq!(d.files[0].lines[5].hunk, 2);
        assert_eq!(d.files[0].lines[5].new, Some(20));
    }
}
