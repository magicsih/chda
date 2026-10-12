//! Refuse modified or unprepared terminal engine source.
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn entries(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read prepared Ghostty source") {
        let path = entry.expect("read source entry").path();
        let kind = fs::symlink_metadata(&path)
            .expect("read source metadata")
            .file_type();
        if kind.is_dir() {
            if dir == root
                && matches!(
                    path.file_name().and_then(|n| n.to_str()),
                    Some(".zig-cache" | "zig-out" | "zig-pkg")
                )
            {
                continue;
            }
            entries(root, &path, out);
        } else {
            out.push(path);
        }
    }
}

fn name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .to_str()
        .expect("UTF-8 source path")
        .replace('\\', "/")
}

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let resources = root.join("resources/ghostty");
    let manifest_path = resources.join("source.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
    let patch_path = resources.join(manifest["patch"].as_str().unwrap());
    println!("cargo:rerun-if-changed={}", patch_path.display());
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(patch_path).unwrap())),
        manifest["patch_sha256"].as_str().unwrap(),
        "Ghostty patch integrity mismatch"
    );
    println!("cargo:rerun-if-env-changed=GHOSTTY_SOURCE_DIR");
    let source = root
        .join("target/ghostty-source")
        .join(&manifest["source_sha256"].as_str().unwrap()[..16]);
    assert!(
        source.is_dir() && !source.is_symlink(),
        "Ghostty source is not prepared; run python3 scripts/prepare-ghostty.py"
    );
    assert_eq!(
        PathBuf::from(env::var_os("GHOSTTY_SOURCE_DIR").expect("required source override"))
            .canonicalize()
            .unwrap(),
        source.canonicalize().unwrap(),
        "build must use chda's prepared Ghostty source"
    );
    let mut paths = Vec::new();
    entries(&source, &source, &mut paths);
    paths.sort_by_key(|path| name(&source, path));
    let mut digest = Sha256::new();
    for path in paths {
        let symlink = path.is_symlink();
        let data = if symlink {
            fs::read_link(&path)
                .unwrap()
                .to_str()
                .expect("UTF-8 symlink")
                .as_bytes()
                .to_vec()
        } else {
            fs::read(&path).unwrap()
        };
        digest.update(if symlink { b"L" } else { b"F" });
        digest.update(name(&source, &path));
        digest.update([0]);
        digest.update(Sha256::digest(data));
    }
    assert_eq!(
        format!("{:x}", digest.finalize()),
        manifest["source_sha256"].as_str().unwrap(),
        "Ghostty source integrity mismatch; run python3 scripts/prepare-ghostty.py --check"
    );
    // Watch source entries but not Zig's mutable output/cache directories.
    for entry in fs::read_dir(&source).unwrap() {
        let path = entry.unwrap().path();
        if !matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some(".zig-cache" | "zig-out" | "zig-pkg")
        ) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    let zig = Command::new("zig")
        .arg("version")
        .output()
        .expect("Zig 0.16.0 must be on PATH");
    assert!(zig.status.success());
    assert_eq!(
        String::from_utf8(zig.stdout).unwrap().trim(),
        manifest["zig_version"].as_str().unwrap(),
        "use the pinned Zig toolchain"
    );
}
