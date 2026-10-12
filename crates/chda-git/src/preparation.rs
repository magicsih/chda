use std::{
    io,
    path::{Path, PathBuf},
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
/// Reject tracked or nonignored copy sources, preserving the rejected entry.
pub fn validate_preparation_sources(primary: &Path, paths: &[PathBuf]) -> io::Result<()> {
    let repo = gix::discover(primary).map_err(io::Error::other)?;
    let index = repo.index_or_empty().map_err(io::Error::other)?;
    let ignore_case = repo
        .config_snapshot()
        .boolean("core.ignorecase")
        .unwrap_or(false);
    let normalize = |bytes: &[u8]| {
        if ignore_case {
            bytes.to_ascii_lowercase()
        } else {
            bytes.to_vec()
        }
    };
    let tracked: std::collections::HashSet<Vec<u8>> = index
        .entries()
        .iter()
        .map(|entry| normalize(entry.path(&index).as_ref()))
        .collect();
    let mut ignores = repo
        .excludes(
            &index,
            None,
            gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
        )
        .map_err(io::Error::other)?;
    for path in paths {
        let name = path
            .to_str()
            .ok_or_else(|| io::Error::other("Preparation paths must be UTF-8"))?;
        let git_path = path
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let reason = if tracked.contains(&normalize(git_path.as_bytes())) {
            Some("the source is tracked by Git")
        } else if !ignores
            .at_path(path, None)
            .map_err(|_| io::Error::other(format!("Cannot inspect Git ignore rules for {name:?}")))?
            .is_excluded()
        {
            Some("the source is not gitignored")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(io::Error::other(format!("Cannot copy {name:?}: {reason}")));
        }
    }
    Ok(())
}
