//! Shell integration: scripts that make the shell report its prompt
//! boundaries (OSC 133), working directory (OSC 7) and title.
//!
//! The scripts ship inside the binary and are written to a per-user data
//! directory on first use. Each shell loads them through a startup hook that
//! leaves the user's dotfiles alone: zsh through `ZDOTDIR`, bash through
//! `--rcfile`, fish through a `vendor_conf.d` directory on `XDG_DATA_DIRS`.
//! The scripts themselves are chda's own.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const ZSH_ZSHENV: &str = include_str!("../shell-integration/zsh/.zshenv");
const ZSH_INTEGRATION: &str = include_str!("../shell-integration/zsh/chda-integration");
const BASH_INTEGRATION: &str = include_str!("../shell-integration/bash/chda-integration");
const FISH_INTEGRATION: &str =
    include_str!("../shell-integration/fish/vendor_conf.d/chda-integration.fish");

/// What the XDG base directory spec assumes when `XDG_DATA_DIRS` is unset.
const DEFAULT_XDG_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// Which shells to integrate with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellIntegration {
    /// Integrate when the shell is recognized: zsh, bash or fish.
    #[default]
    Detect,
    None,
}

/// How to start a shell so that it loads the integration scripts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellLaunch {
    /// Extra arguments for the shell program. When not empty, `env` only
    /// works together with them, so apply both or neither.
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Write the scripts under `data_dir` and return how to start `shell` so it
/// loads them. Empty when the shell is not supported.
pub fn launch_for(
    mode: ShellIntegration,
    shell: &Path,
    data_dir: &Path,
) -> io::Result<ShellLaunch> {
    if mode == ShellIntegration::None {
        return Ok(ShellLaunch::default());
    }
    let name = shell.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let launch = match name {
        // zsh reads its startup files from ZDOTDIR; ours restores the
        // user's ZDOTDIR and loads their files.
        "zsh" => {
            let dir = install(data_dir)?.join("zsh");
            let mut env = vec![("ZDOTDIR".to_owned(), path_string(&dir))];
            if let Some(old) = env_string("ZDOTDIR") {
                env.push(("CHDA_ZSH_ZDOTDIR".to_owned(), old));
            }
            ShellLaunch {
                args: Vec::new(),
                env,
            }
        }
        // bash ignores `--rcfile` in login shells, so it starts as a plain
        // interactive shell; CHDA_BASH_INJECT tells the script to read the
        // login files itself. The script's header explains the choice.
        "bash" => {
            let script = install(data_dir)?.join("bash/chda-integration");
            ShellLaunch {
                args: vec!["--rcfile".to_owned(), path_string(&script)],
                env: vec![("CHDA_BASH_INJECT".to_owned(), "1".to_owned())],
            }
        }
        // fish sources `<dir>/fish/vendor_conf.d/*.fish` for each directory
        // on XDG_DATA_DIRS. The script restores the original value, which
        // chda passes along (empty when it was unset).
        "fish" => {
            let root = install(data_dir)?;
            let old = env_string("XDG_DATA_DIRS").filter(|v| !v.is_empty());
            let dirs = format!(
                "{}:{}",
                path_string(&root),
                old.as_deref().unwrap_or(DEFAULT_XDG_DATA_DIRS)
            );
            ShellLaunch {
                args: Vec::new(),
                env: vec![
                    ("XDG_DATA_DIRS".to_owned(), dirs),
                    (
                        "CHDA_FISH_XDG_DATA_DIRS".to_owned(),
                        old.unwrap_or_default(),
                    ),
                ],
            }
        }
        _ => ShellLaunch::default(),
    };
    Ok(launch)
}

fn env_string(key: &str) -> Option<String> {
    std::env::var_os(key).map(|v| v.to_string_lossy().into_owned())
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Write the bundled scripts to `data_dir/shell-integration`, replacing
/// stale copies. Returns that directory.
fn install(data_dir: &Path) -> io::Result<PathBuf> {
    let root = data_dir.join("shell-integration");
    let zsh = root.join("zsh");
    let bash = root.join("bash");
    let fish = root.join("fish/vendor_conf.d");
    for dir in [&zsh, &bash, &fish] {
        fs::create_dir_all(dir)?;
    }
    write_if_changed(&zsh.join(".zshenv"), ZSH_ZSHENV)?;
    write_if_changed(&zsh.join("chda-integration"), ZSH_INTEGRATION)?;
    write_if_changed(&bash.join("chda-integration"), BASH_INTEGRATION)?;
    write_if_changed(&fish.join("chda-integration.fish"), FISH_INTEGRATION)?;
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

/// Shell binaries for tests that spawn real shells.
#[cfg(all(test, unix))]
pub(crate) mod test_shells {
    use std::path::PathBuf;

    /// Every distinct bash on the usual paths, so both the macOS bash 3.2
    /// and a newer one get covered when installed.
    pub fn bashes() -> Vec<PathBuf> {
        existing(&[
            "/bin/bash",
            "/opt/homebrew/bin/bash",
            "/usr/local/bin/bash",
            "/usr/bin/bash",
        ])
    }

    pub fn fish() -> Option<PathBuf> {
        existing(&[
            "/opt/homebrew/bin/fish",
            "/usr/local/bin/fish",
            "/usr/bin/fish",
        ])
        .into_iter()
        .next()
    }

    fn existing(candidates: &[&str]) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = Vec::new();
        for path in candidates {
            if let Ok(real) = std::fs::canonicalize(path)
                && !found
                    .iter()
                    .any(|f| std::fs::canonicalize(f).ok() == Some(real.clone()))
            {
                found.push(PathBuf::from(path));
            }
        }
        found
    }

    /// A fresh, canonical directory to use as `HOME`.
    pub fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("chda-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    pub fn hostname() -> String {
        String::from_utf8(
            std::process::Command::new("hostname")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::test_shells::{bashes, fish, hostname, temp_home};
    use super::*;
    use chda_pty::{Pty, PtySize, SpawnOptions};
    use std::io::Read;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn launch_settings_per_shell() {
        let dir = temp_home("shell-launch");
        let launch = launch_for(ShellIntegration::Detect, Path::new("/bin/zsh"), &dir).unwrap();
        assert!(launch.args.is_empty());
        let zdotdir = lookup(&launch, "ZDOTDIR").unwrap();
        assert!(Path::new(zdotdir).join(".zshenv").is_file());
        assert!(Path::new(zdotdir).join("chda-integration").is_file());

        let launch = launch_for(ShellIntegration::Detect, Path::new("/bin/bash"), &dir).unwrap();
        assert_eq!(launch.args[0], "--rcfile");
        assert!(Path::new(&launch.args[1]).is_file());
        assert_eq!(lookup(&launch, "CHDA_BASH_INJECT"), Some("1"));

        let launch =
            launch_for(ShellIntegration::Detect, Path::new("/usr/bin/fish"), &dir).unwrap();
        assert!(launch.args.is_empty());
        let dirs = lookup(&launch, "XDG_DATA_DIRS").unwrap();
        let first = dirs.split(':').next().unwrap();
        assert!(
            Path::new(first)
                .join("fish/vendor_conf.d/chda-integration.fish")
                .is_file()
        );
        assert!(lookup(&launch, "CHDA_FISH_XDG_DATA_DIRS").is_some());

        assert_eq!(
            launch_for(ShellIntegration::Detect, Path::new("/bin/tcsh"), &dir).unwrap(),
            ShellLaunch::default()
        );
        assert_eq!(
            launch_for(ShellIntegration::None, Path::new("/bin/zsh"), &dir).unwrap(),
            ShellLaunch::default()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    fn lookup<'a>(launch: &'a ShellLaunch, key: &str) -> Option<&'a str> {
        launch
            .env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// A shell on a PTY whose raw output is collected, so tests can look for
    /// the exact escape sequences the scripts write.
    struct Transcript {
        pty: Pty,
        out: Arc<Mutex<Vec<u8>>>,
        /// Primary device attribute queries answered so far; fish 4 waits
        /// for the reply before showing a prompt.
        answered: usize,
    }

    impl Transcript {
        fn spawn(command: &[String], home: &Path, launch: &ShellLaunch) -> Self {
            let command: Vec<&str> = command.iter().map(String::as_str).collect();
            let home_str = home.to_string_lossy();
            let mut env: Vec<(&str, &str)> = vec![
                ("HOME", &home_str),
                ("TERM", "xterm-256color"),
                ("BASH_SILENCE_DEPRECATION_WARNING", "1"),
            ];
            env.extend(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            let pty = Pty::spawn(
                PtySize {
                    cols: 120,
                    rows: 40,
                    pixel_width: 0,
                    pixel_height: 0,
                },
                SpawnOptions {
                    command: Some(&command),
                    cwd: Some(home),
                    env: &env,
                },
            )
            .unwrap();
            let out = Arc::new(Mutex::new(Vec::new()));
            let mut reader = pty.reader().unwrap();
            let sink = out.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    sink.lock().unwrap().extend_from_slice(&buf[..n]);
                }
            });
            Self {
                pty,
                out,
                answered: 0,
            }
        }

        fn count(&self, needle: &str) -> usize {
            let out = self.out.lock().unwrap();
            out.windows(needle.len())
                .filter(|w| *w == needle.as_bytes())
                .count()
        }

        /// Wait until `needle` has appeared `times` times.
        fn wait_for(&mut self, needle: &str, times: usize) {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let queries = self.count("\x1b[c") + self.count("\x1b[0c");
                while self.answered < queries {
                    self.pty.write_all(b"\x1b[?62;22c").unwrap();
                    self.answered += 1;
                }
                if self.count(needle) >= times {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "waiting for {needle:?} x{times}; output {:?}",
                    String::from_utf8_lossy(&self.out.lock().unwrap())
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        fn send(&mut self, text: &str) {
            self.pty.write_all(text.as_bytes()).unwrap();
        }

        fn finish(mut self) -> String {
            self.send("exit\n");
            let status = self.pty.wait().unwrap();
            assert!(status.success(), "{status:?}");
            // Let the reader drain what the shell wrote last.
            std::thread::sleep(Duration::from_millis(100));
            String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
        }
    }

    fn bash_version(bash: &Path) -> (u32, u32) {
        let out = std::process::Command::new(bash)
            .args(["-c", "echo ${BASH_VERSINFO[0]} ${BASH_VERSINFO[1]}"])
            .output()
            .unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let mut parts = text.split_whitespace().map(|p| p.parse().unwrap());
        (parts.next().unwrap(), parts.next().unwrap())
    }

    #[test]
    fn bash_integration_writes_marks_and_keeps_user_config() {
        // PROMPT_COMMAND as a string, and as an array where bash allows it.
        let cases = bashes().into_iter().flat_map(|bash| {
            let mut forms = vec![(bash.clone(), "PROMPT_COMMAND='user_pc=$((user_pc+1))'")];
            if bash_version(&bash) >= (5, 1) {
                forms.push((bash, "PROMPT_COMMAND=('user_pc=$((user_pc+1))')"));
            }
            forms
        });
        for (bash, prompt_command) in cases {
            let home = temp_home("bash-transcript");
            fs::write(
                home.join(".bash_profile"),
                "echo profile-loaded\n. \"$HOME/.bashrc\"\n",
            )
            .unwrap();
            fs::write(
                home.join(".bashrc"),
                format!(
                    "echo bashrc-loaded\n\
                     PS1='chda$ '\n\
                     {prompt_command}\n\
                     trap 'user_dbg=yes' DEBUG\n"
                ),
            )
            .unwrap();
            let launch = launch_for(ShellIntegration::Detect, &bash, &home).unwrap();
            let mut command = vec![bash.to_string_lossy().into_owned()];
            command.extend(launch.args.iter().cloned());
            let mut t = Transcript::spawn(&command, &home, &launch);

            t.wait_for("chda$ ", 1);
            // An empty command line is not a command: no C or D.
            t.send("\n");
            t.wait_for("chda$ ", 2);
            t.send("false\n");
            t.wait_for("\x1b]133;D;1\x07", 1);
            assert_eq!(t.count("\x1b]133;C\x07"), 1, "{}", bash.display());
            t.send("echo \"pc=$user_pc dbg=$user_dbg inject=${CHDA_BASH_INJECT-none}\"\n");
            t.wait_for("inject=", 2);
            t.wait_for("chda$ ", 4);
            let out = t.finish();

            let host = hostname();
            let home_str = home.to_string_lossy();
            for needle in [
                "profile-loaded",
                "bashrc-loaded",
                "\x1b]133;A\x07",
                "chda$ \x1b]133;B\x07",
                "\x1b]2;~\x07",
                &format!("\x1b]7;file://{host}{home_str}\x07"),
                "\x1b]133;D;0\x07",
                // The user's PROMPT_COMMAND ran before each of the three
                // prompts ahead of the echo, and their DEBUG trap still runs.
                "pc=3 dbg=yes inject=none",
            ] {
                assert!(
                    out.contains(needle),
                    "{} ({prompt_command}): missing {needle:?} in {out:?}",
                    bash.display()
                );
            }
            fs::remove_dir_all(&home).unwrap();
        }
    }

    #[test]
    fn fish_integration_writes_marks_and_keeps_user_config() {
        let Some(fish) = fish() else {
            return;
        };
        // Native: fish 4 writes the marks itself (ST-terminated) and the
        // script must stay out of the way. Fallback: with fish's own marks
        // off, as in fish 3, the script writes them (BEL-terminated).
        for native in [true, false] {
            let home = temp_home(if native { "fish-native" } else { "fish-own" });
            let config = home.join(".config/fish");
            fs::create_dir_all(&config).unwrap();
            fs::write(
                config.join("config.fish"),
                "echo config-loaded\n\
                 set -g fish_greeting\n\
                 function fish_prompt; echo -n 'chda> '; end\n",
            )
            .unwrap();
            let launch = launch_for(ShellIntegration::Detect, &fish, &home).unwrap();
            let mut command = vec![fish.to_string_lossy().into_owned(), "-l".into()];
            if !native {
                command.extend(["--features".into(), "no-mark-prompt".into()]);
            }
            let mut t = Transcript::spawn(&command, &home, &launch);

            t.wait_for("chda> ", 1);
            t.send("false\n");
            t.wait_for("\x1b]133;D;1", 1);
            t.send(
                "set -q CHDA_FISH_XDG_DATA_DIRS; or echo clean-(echo var); \
                 string match -q '*shell-integration*' -- \"$XDG_DATA_DIRS\"; \
                 or echo clean-(echo xdg)\n",
            );
            t.wait_for("clean-xdg", 1);
            let out = t.finish();

            let host = hostname();
            let home_str = home.to_string_lossy();
            for needle in [
                "config-loaded",
                "clean-var",
                "clean-xdg",
                "\x1b]133;A",
                "\x1b]133;B",
                "\x1b]133;C",
                &format!("\x1b]7;file://{host}{home_str}"),
            ] {
                assert!(out.contains(needle), "missing {needle:?} in {out:?}");
            }
            let own = [
                "\x1b]133;A\x07",
                "\x1b]133;B\x07",
                "\x1b]133;C\x07",
                "\x1b]133;D;1\x07",
            ];
            for needle in own {
                assert_eq!(
                    out.contains(needle),
                    !native,
                    "native={native} {needle:?} in {out:?}"
                );
            }
            fs::remove_dir_all(&home).unwrap();
        }
    }
}
