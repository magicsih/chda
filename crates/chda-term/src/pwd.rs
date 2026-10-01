//! OSC 7 working-directory reports.

use std::path::PathBuf;

/// Parse an OSC 7 value such as `file://host/Users/me` or
/// `kitty-shell-cwd://host/tmp` into a local path. Reports for other hosts
/// are ignored.
pub fn parse_pwd_report(value: &str) -> Option<PathBuf> {
    let rest = value
        .strip_prefix("file://")
        .or_else(|| value.strip_prefix("kitty-shell-cwd://"))?;
    let i = rest.find('/')?;
    let (host, path) = (&rest[..i], &rest[i..]);
    if !host.is_empty() && host != "localhost" && !is_this_host(host) {
        return None;
    }
    let decoded = percent_decode(path);
    (!decoded.is_empty()).then(|| PathBuf::from(decoded))
}

fn is_this_host(host: &str) -> bool {
    let Ok(ours) = std::env::var("HOSTNAME").or_else(|_| hostname()) else {
        return true;
    };
    let short = |h: &str| h.split('.').next().unwrap_or(h).to_ascii_lowercase();
    short(host) == short(&ours)
}

fn hostname() -> Result<String, std::env::VarError> {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .ok_or(std::env::VarError::NotPresent)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_and_kitty_schemes_with_percent_escapes() {
        assert_eq!(
            parse_pwd_report("file://localhost/Users/me/a%20b").as_deref(),
            Some(std::path::Path::new("/Users/me/a b"))
        );
        assert_eq!(
            parse_pwd_report("kitty-shell-cwd:///tmp/x").as_deref(),
            Some(std::path::Path::new("/tmp/x"))
        );
        assert_eq!(parse_pwd_report("file://otherhost.example/tmp"), None);
        assert_eq!(parse_pwd_report("http://x/y"), None);
    }
}
