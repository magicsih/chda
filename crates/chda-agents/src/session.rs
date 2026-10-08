//! Session transcripts: shared types and the index cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::AgentId;

/// Session identifier as the agent names it (a UUID for both agents).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// One past or current agent session, enough for a list entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: SessionId,
    pub agent: AgentId,
    pub cwd: PathBuf,
    /// Milliseconds since the Unix epoch.
    pub started_at: u64,
    /// When the transcript was last written, i.e. the last message, in
    /// milliseconds since the Unix epoch.
    pub last_active_at: u64,
    /// First user prompt, at most 120 characters.
    pub snippet: String,
    pub message_count: usize,
    pub file: PathBuf,
    /// Tokens used, per model, when the transcript records them.
    #[serde(default)]
    pub usage: crate::Usage,
}

/// File name of the persisted cache inside the data directory.
const CACHE_FILE: &str = "session-index.json";

/// Bumped when the cached data changes shape or meaning.
const CACHE_VERSION: u32 = 3;

/// What is known about one transcript file at a given mtime and size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    mtime_ms: u64,
    len: u64,
    /// Directory the session ran in, from the head of the file.
    cwd: Option<PathBuf>,
    /// Full parse, done only once the session's directory was wanted.
    /// `Some(None)`: parsed, not a session.
    parsed: Option<Option<AgentSession>>,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: HashMap<PathBuf, Entry>,
}

/// Index cache keyed by file, mtime and size. Re-indexing reads only files
/// that changed, and fully parses only sessions whose directory is wanted.
/// It is saved to the data directory so a restart starts warm.
#[derive(Debug, Default)]
pub struct SessionCache {
    entries: HashMap<PathBuf, Entry>,
    dirty: bool,
}

impl SessionCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The cache saved in `data_dir`, or an empty one.
    pub fn load(data_dir: &Path) -> Self {
        let entries = std::fs::read(data_dir.join(CACHE_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok())
            .filter(|c| c.version == CACHE_VERSION)
            .map(|c| c.entries)
            .unwrap_or_default();
        Self {
            entries,
            dirty: false,
        }
    }

    /// Write the cache to `data_dir` if it changed since it was loaded or
    /// last saved. Entries of files that no longer exist are dropped.
    pub fn save(&mut self, data_dir: &Path) -> std::io::Result<()> {
        let before = self.entries.len();
        self.entries.retain(|path, _| path.exists());
        if !self.dirty && self.entries.len() == before {
            return Ok(());
        }
        std::fs::create_dir_all(data_dir)?;
        let file = CacheFile {
            version: CACHE_VERSION,
            entries: std::mem::take(&mut self.entries),
        };
        let json = serde_json::to_vec(&file);
        self.entries = file.entries;
        let path = data_dir.join(CACHE_FILE);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json?)?;
        std::fs::rename(tmp, path)?;
        self.dirty = false;
        Ok(())
    }

    /// The session in `file` when its directory passes `wanted`. `cwd` reads
    /// the directory from the head of the file and `parse` reads the whole
    /// file; each runs only when the file changed since it last ran.
    pub fn session(
        &mut self,
        file: &Path,
        wanted: impl Fn(&Path) -> bool,
        cwd: impl FnOnce() -> Option<PathBuf>,
        parse: impl FnOnce() -> Option<AgentSession>,
    ) -> Option<AgentSession> {
        let meta = std::fs::metadata(file).ok()?;
        let mtime_ms = meta
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis() as u64;
        let len = meta.len();
        let fresh = self
            .entries
            .get(file)
            .is_some_and(|e| e.mtime_ms == mtime_ms && e.len == len);
        if !fresh {
            self.entries.insert(
                file.to_path_buf(),
                Entry {
                    mtime_ms,
                    len,
                    cwd: cwd(),
                    parsed: None,
                },
            );
            self.dirty = true;
        }
        let entry = self.entries.get_mut(file)?;
        if !entry.cwd.as_deref().is_some_and(&wanted) {
            return None;
        }
        if entry.parsed.is_none() {
            entry.parsed = Some(parse());
            self.dirty = true;
        }
        entry.parsed.clone().flatten()
    }
}

/// The working directory recorded near the top of a transcript: the first
/// JSON line with a `cwd` field, at the top level or under `payload`.
pub(crate) fn head_cwd(file: &Path) -> Option<PathBuf> {
    use std::io::BufRead;
    const MAX_LINES: usize = 50;
    let reader = std::io::BufReader::new(std::fs::File::open(file).ok()?);
    let finder = memchr::memmem::Finder::new(b"\"cwd\"");
    for line in reader.split(b'\n').take(MAX_LINES).filter_map(Result::ok) {
        if finder.find(&line).is_none() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&line) else {
            continue;
        };
        let cwd = v
            .get("cwd")
            .or_else(|| v.get("payload").and_then(|p| p.get("cwd")))
            .and_then(serde_json::Value::as_str);
        if let Some(cwd) = cwd {
            return Some(PathBuf::from(cwd));
        }
    }
    None
}

/// Never treat a filename suffix or a partial UUID as an exact conversation.
pub(crate) fn transcript_matches_id(file: &Path, agent: AgentId, id: &SessionId) -> bool {
    use std::io::{BufRead, Read};
    let Some(stem) = file.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    if id.0.is_empty() {
        return false;
    }
    if stem == id.0 {
        return true;
    }
    if agent != AgentId::Codex || !stem.starts_with("rollout-") || !stem.ends_with(&id.0) {
        return false;
    }
    let Ok(file) = std::fs::File::open(file) else {
        return false;
    };
    let reader = std::io::BufReader::new(file.take(512 * 1024));
    for line in reader.split(b'\n').take(50).filter_map(Result::ok) {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
            continue;
        };
        if value["type"] == "session_meta" {
            return value["payload"]["id"]
                .as_str()
                .or_else(|| value["payload"]["session_id"].as_str())
                == Some(id.0.as_str());
        }
    }
    false
}

/// Read only a bounded top-level metadata prefix, leaving message bodies alone.
pub(crate) fn head_metadata_field(file: &Path, field: &str) -> Option<String> {
    use serde::de::{MapAccess, Visitor};
    use std::io::Read;
    struct Field<'a> {
        key: &'a str,
        found: &'a mut Option<String>,
    }
    impl<'de> Visitor<'de> for Field<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("session metadata")
        }
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            while let Some(key) = map.next_key::<String>()? {
                if key == self.key {
                    *self.found = Some(map.next_value::<String>()?);
                    // Stop deliberately before serde consumes/validates the
                    // remaining map: this reads metadata, not message bodies.
                    return Err(serde::de::Error::custom("metadata extracted"));
                }
                map.next_value::<serde::de::IgnoredAny>()?;
            }
            Ok(())
        }
    }
    let file = std::fs::File::open(file).ok()?;
    let mut parser =
        serde_json::Deserializer::from_reader(std::io::BufReader::new(file.take(512 * 1024)));
    let mut found = None;
    let _ = serde::Deserializer::deserialize_map(
        &mut parser,
        Field {
            key: field,
            found: &mut found,
        },
    );
    found
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

/// A file's modification time in milliseconds since the Unix epoch.
pub(crate) fn file_mtime_ms(file: &Path) -> Option<u64> {
    std::fs::metadata(file)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
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
    fn resume_requires_the_exact_provider_identity_not_a_filename_suffix() {
        let dir = std::env::temp_dir().join(format!("chda-exact-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let claude = dir.join("other-s1.jsonl");
        std::fs::write(&claude, "{}").unwrap();
        assert!(!transcript_matches_id(
            &claude,
            AgentId::Claude,
            &SessionId("s1".into())
        ));
        assert!(transcript_matches_id(
            &claude,
            AgentId::Claude,
            &SessionId("other-s1".into())
        ));
        let codex = dir.join("rollout-time-parent-session.jsonl");
        std::fs::write(
            &codex,
            r#"{"type":"session_meta","payload":{"id":"parent-session"}}"#,
        )
        .unwrap();
        assert!(transcript_matches_id(
            &codex,
            AgentId::Codex,
            &SessionId("parent-session".into())
        ));
        assert!(!transcript_matches_id(
            &codex,
            AgentId::Codex,
            &SessionId("session".into())
        ));
        assert!(!transcript_matches_id(
            &codex,
            AgentId::Claude,
            &SessionId("parent-session".into())
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cache_parses_only_wanted_sessions_and_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("chda-sesscache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mine = dir.join("mine.jsonl");
        let other = dir.join("other.jsonl");
        std::fs::write(&mine, "{\"type\":\"user\",\"cwd\":\"/src/app/sub\"}\n").unwrap();
        std::fs::write(&other, "{\"payload\":{\"cwd\":\"/elsewhere\"}}\n").unwrap();
        assert_eq!(head_cwd(&other), Some(PathBuf::from("/elsewhere")));
        let wanted = |p: &Path| p.starts_with("/src/app");
        let session = |file: &Path| AgentSession {
            id: SessionId("s".into()),
            agent: AgentId::Claude,
            cwd: head_cwd(file).unwrap(),
            started_at: 1,
            last_active_at: 1,
            snippet: "hi".into(),
            message_count: 1,
            usage: Default::default(),
            file: file.to_path_buf(),
        };
        let parses = std::cell::Cell::new(0);
        let index = |cache: &mut SessionCache, file: &Path| {
            cache.session(
                file,
                wanted,
                || head_cwd(file),
                || {
                    parses.set(parses.get() + 1);
                    Some(session(file))
                },
            )
        };

        let mut cache = SessionCache::new();
        assert!(index(&mut cache, &mine).is_some());
        assert!(index(&mut cache, &other).is_none());
        assert_eq!(parses.get(), 1, "the unwanted session is never parsed");
        cache.save(&dir).unwrap();

        let mut warm = SessionCache::load(&dir);
        assert_eq!(
            index(&mut warm, &mine).map(|s| s.cwd),
            Some("/src/app/sub".into())
        );
        assert!(index(&mut warm, &other).is_none());
        assert_eq!(parses.get(), 1, "a warm cache parses nothing");
        assert!(!warm.dirty);

        std::fs::remove_file(&other).unwrap();
        warm.save(&dir).unwrap();
        assert_eq!(SessionCache::load(&dir).entries.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
