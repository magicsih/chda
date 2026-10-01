//! Session transcripts: shared types and the mtime/size cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::AgentId;

/// Session identifier as the agent names it (a UUID for both agents).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

/// One past or current agent session, enough for a list entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSession {
    pub id: SessionId,
    pub agent: AgentId,
    pub cwd: PathBuf,
    /// Milliseconds since the Unix epoch.
    pub started_at: u64,
    /// First user prompt, at most 120 characters.
    pub snippet: String,
    pub message_count: usize,
    pub file: PathBuf,
}

/// Caches parse results by file mtime and size so re-indexing only reads
/// files that changed.
#[derive(Debug, Default)]
pub struct SessionCache {
    entries: HashMap<PathBuf, (SystemTime, u64, Option<AgentSession>)>,
}

impl SessionCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_parse(
        &mut self,
        file: &Path,
        parse: impl FnOnce() -> Option<AgentSession>,
    ) -> Option<AgentSession> {
        let meta = std::fs::metadata(file).ok()?;
        let key = (meta.modified().ok()?, meta.len());
        if let Some((m, len, cached)) = self.entries.get(file)
            && (*m, *len) == key
        {
            return cached.clone();
        }
        let parsed = parse();
        self.entries
            .insert(file.to_path_buf(), (key.0, key.1, parsed.clone()));
        parsed
    }
}

/// Iterate the lines of a transcript that contain any of `needles`, without
/// reading the whole file into memory. Lines that fail to decode are skipped.
pub(crate) fn matching_lines(
    file: &Path,
    needles: &'static [&'static str],
) -> std::io::Result<impl Iterator<Item = String>> {
    use std::io::BufRead;
    let reader = std::io::BufReader::with_capacity(256 * 1024, std::fs::File::open(file)?);
    let finders: Vec<memchr::memmem::Finder<'static>> = needles
        .iter()
        .map(|n| memchr::memmem::Finder::new(n.as_bytes()))
        .collect();
    Ok(reader
        .split(b'\n')
        .filter_map(Result::ok)
        .filter_map(move |line| {
            if !finders.iter().any(|f| f.find(&line).is_some()) {
                return None;
            }
            String::from_utf8(line).ok()
        }))
}

/// All `.jsonl` files under `root`, recursively.
pub(crate) fn jsonl_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                out.push(path);
            }
        }
    }
    out
}

/// Truncate to `max` characters on a char boundary, single line.
pub(crate) fn snippet(text: &str, max: usize) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.chars().take(max).collect()
}

/// Parse an RFC 3339 timestamp like `2026-09-09T12:42:15.371Z` to epoch ms.
pub(crate) fn parse_rfc3339_ms(s: &str) -> Option<u64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let time = time.trim_end_matches('Z');
    let (hms, frac) = match time.split_once('.') {
        Some((h, f)) => (h, f),
        None => (time, ""),
    };
    let mut t = hms.split(':').map(|p| p.parse::<i64>());
    let (h, min, sec) = (t.next()?.ok()?, t.next()?.ok()?, t.next()?.ok()?);
    let ms: i64 = frac
        .chars()
        .take(3)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
        * 10i64.pow(3u32.saturating_sub(frac.len().min(3) as u32));
    // Days from civil, per Howard Hinnant.
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + min * 60 + sec;
    u64::try_from(secs * 1000 + ms).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_to_epoch_ms() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2026-09-09T12:42:15.371Z"),
            Some(1_788_957_735_371)
        );
        assert_eq!(parse_rfc3339_ms("nope"), None);
    }
}
