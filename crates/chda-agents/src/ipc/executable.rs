//! Durable entry point for hooks, MCP and status-line helpers.
#[cfg(any(target_os = "linux", test))]
use std::path::Path;
use std::path::PathBuf;

pub fn helper_executable() -> std::io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    {
        let directory = std::env::var_os("APPDIR").map(PathBuf::from);
        let image = std::env::var_os("APPIMAGE").map(PathBuf::from);
        Ok(helper_path(
            &executable,
            directory.as_deref(),
            image.as_deref(),
        ))
    }
    #[cfg(not(target_os = "linux"))]
    Ok(executable)
}

#[cfg(any(target_os = "linux", test))]
fn helper_path(executable: &Path, directory: Option<&Path>, image: Option<&Path>) -> PathBuf {
    // An editor launched from another AppImage can pass its environment to
    // chda. Use APPIMAGE only when APPDIR contains this exact packaged binary.
    if let (Some(directory), Some(image)) = (directory, image)
        && directory.join("com.magicsih.chda.desktop").is_file()
        && directory
            .join("usr/bin/chda")
            .canonicalize()
            .ok()
            .as_deref()
            == Some(executable)
        && image.is_absolute()
        && image.is_file()
        && let Ok(image) = image.canonicalize()
    {
        return image;
    }
    executable.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appimage_helpers_keep_the_original_image_after_its_extract_directory_disappears() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "chda-helper-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let directory = root.join("temporary mount");
        let executable = directory.join("usr/bin/chda");
        let image = root.join("chda package 世界.AppImage");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, b"fixture").unwrap();
        std::fs::write(&image, b"fixture").unwrap();
        std::fs::write(directory.join("com.magicsih.chda.desktop"), b"fixture").unwrap();
        let executable = executable.canonicalize().unwrap();
        let expected = image.canonicalize().unwrap();
        let actual = helper_path(&executable, Some(&directory), Some(&image));
        assert_eq!(helper_path(&executable, None, None), executable);
        let portable = root.join("portable-chda");
        std::fs::write(&portable, b"fixture").unwrap();
        let portable = portable.canonicalize().unwrap();
        assert_eq!(
            helper_path(&portable, Some(&directory), Some(&image)),
            portable,
            "a portable executable must ignore an inherited AppImage environment"
        );
        std::fs::remove_file(directory.join("com.magicsih.chda.desktop")).unwrap();
        assert_eq!(
            helper_path(&executable, Some(&directory), Some(&image)),
            executable
        );
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(actual, expected);
        assert!(actual.is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
