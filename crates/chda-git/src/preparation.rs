use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};
/// The actual shared repository directory, for separately stored preparation consent.
pub fn repository_directory(primary: &Path) -> io::Result<PathBuf> {
    let repo = gix::discover(primary).map_err(io::Error::other)?;
    let root = crate::main_worktree(primary)?.canonicalize()?;
    if primary.canonicalize()? != root {
        return Err(io::Error::other(
            "Preparation sources must belong to the primary checkout",
        ));
    }
    repo.common_dir().canonicalize()
}
/// Default check-ignore omits tracked paths, including force-added ignored files.
pub fn preparation_ignored_command(primary: &Path) -> Command {
    let mut command = Command::new(crate::cli::git_binary());
    command
        .current_dir(primary)
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "check-ignore",
            "--stdin",
            "-z",
        ])
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}
