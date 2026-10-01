//! Shell integration: scripts that make the shell report its prompt
//! boundaries (OSC 133), working directory (OSC 7) and title.
//!
//! The scripts ship inside the binary and are written to a per-user data
//! directory on first use. zsh picks them up through `ZDOTDIR`, the same
//! mechanism Ghostty and Kitty use; the scripts themselves are chda's own.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const ZSH_ZSHENV: &str = include_str!("../shell-integration/zsh/.zshenv");
const ZSH_INTEGRATION: &str = include_str!("../shell-integration/zsh/chda-integration");

/// Which shells to integrate with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellIntegration {
    /// Integrate when the shell is recognized (currently zsh).
    #[default]
    Detect,
    None,
}

/// Write the scripts under `data_dir` and return the environment variables
/// that make `shell` load them. Empty when the shell is not supported.
pub fn env_for(
    mode: ShellIntegration,
    shell: &Path,
    data_dir: &Path,
) -> io::Result<Vec<(String, String)>> {
    if mode == ShellIntegration::None {
        return Ok(Vec::new());
    }
    let name = shell.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match name {
        "zsh" => {
            let dir = install(data_dir)?.join("zsh");
            let mut env = vec![("ZDOTDIR".to_owned(), dir.to_string_lossy().into_owned())];
            if let Some(old) = std::env::var_os("ZDOTDIR") {
                env.push((
                    "CHDA_ZSH_ZDOTDIR".to_owned(),
                    old.to_string_lossy().into_owned(),
                ));
            }
            Ok(env)
        }
        _ => Ok(Vec::new()),
    }
}

/// Write the bundled scripts to `data_dir/shell-integration`, replacing
/// stale copies. Returns that directory.
fn install(data_dir: &Path) -> io::Result<PathBuf> {
    let root = data_dir.join("shell-integration");
    let zsh = root.join("zsh");
    fs::create_dir_all(&zsh)?;
    write_if_changed(&zsh.join(".zshenv"), ZSH_ZSHENV)?;
    write_if_changed(&zsh.join("chda-integration"), ZSH_INTEGRATION)?;
    Ok(root)
}

fn write_if_changed(path: &Path, contents: &str) -> io::Result<()> {
    if fs::read_to_string(path).ok().as_deref() == Some(contents) {
        return Ok(());
    }
    fs::write(path, contents)
}

/// Per-user data directory for chda.
pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(dir).join("chda"));
    }
    let home = std::env::var_os("HOME")?;
    let home = PathBuf::from(home);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/chda"))
    } else {
        Some(home.join(".local/share/chda"))
    }
}

/// The user's login shell, from `$SHELL`.
pub fn login_shell() -> Option<PathBuf> {
    std::env::var_os("SHELL").map(PathBuf::from)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn zsh_gets_a_zdotdir_with_the_scripts_installed() {
        let dir = std::env::temp_dir().join(format!("chda-shell-{}", std::process::id()));
        let env = env_for(ShellIntegration::Detect, Path::new("/bin/zsh"), &dir).unwrap();
        let zdotdir = env
            .iter()
            .find(|(k, _)| k == "ZDOTDIR")
            .map(|(_, v)| v)
            .unwrap();
        assert!(Path::new(zdotdir).join(".zshenv").is_file());
        assert!(Path::new(zdotdir).join("chda-integration").is_file());
        assert!(
            env_for(ShellIntegration::Detect, Path::new("/bin/fish"), &dir)
                .unwrap()
                .is_empty()
        );
        assert!(
            env_for(ShellIntegration::None, Path::new("/bin/zsh"), &dir)
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
