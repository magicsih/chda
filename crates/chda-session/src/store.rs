//! One writer, atomic private files and one validated previous checkpoint.
use crate::{SessionRecord, protocol::now};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    pub record: SessionRecord,
    pub output: String,
}
pub(crate) struct Store {
    root: PathBuf,
}
impl Store {
    pub fn new(data: &Path) -> io::Result<Self> {
        let root = data.join("session-service");
        chda_pty::local::private_directory(&root)?;
        chda_pty::local::private_directory(&root.join("sessions"))?;
        Ok(Self { root })
    }
    pub fn save<T: Serialize + serde::de::DeserializeOwned>(
        &self,
        path: &Path,
        value: &T,
    ) -> io::Result<()> {
        self.save_with(path, value, chda_agents::ipc::write_private_atomic)
    }
    fn save_with<T: Serialize + serde::de::DeserializeOwned>(
        &self,
        path: &Path,
        value: &T,
        write: impl Fn(&Path, &[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        if let Ok(old) = fs::read(path)
            && old == bytes
        {
            return Ok(());
        }
        if let Ok(old) = fs::read(path)
            && serde_json::from_slice::<T>(&old).is_ok()
        {
            write(&path.with_extension("previous"), &old)?;
        }
        write(path, &bytes)
    }
    pub fn read<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Option<T> {
        if let Ok(bytes) = fs::read(path) {
            if let Ok(value) = serde_json::from_slice(&bytes) {
                return Some(value);
            }
            // Keep the malformed file available for diagnosis; never replace a
            // valid backup with it during recovery.
            let _ = fs::rename(path, path.with_extension("corrupt"));
        }
        fs::read(path.with_extension("previous"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }
    pub fn layout(&self) -> Option<chda_core::SavedSession> {
        self.read(&self.root.join("layout.json"))
    }
    pub fn layout_saved_at(&self) -> Option<u64> {
        fs::metadata(self.root.join("layout.json"))
            .ok()?
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64)
    }
    pub fn save_layout(&self, layout: &chda_core::SavedSession) -> io::Result<()> {
        self.save(&self.root.join("layout.json"), layout)
    }
    pub fn checkpoint(&self, checkpoint: &mut Checkpoint, text_saved: bool) -> io::Result<()> {
        let previous = checkpoint.record.saved_at;
        if text_saved {
            checkpoint.record.saved_at = Some(now());
        }
        let path = self
            .root
            .join("sessions")
            .join(format!("{}.json", checkpoint.record.identity.session));
        if let Err(e) = self.save(&path, checkpoint) {
            checkpoint.record.saved_at = previous;
            return Err(e);
        }
        Ok(())
    }
    pub fn load(&self) -> Vec<Checkpoint> {
        let Ok(files) = fs::read_dir(self.root.join("sessions")) else {
            return vec![];
        };
        let mut names = std::collections::BTreeSet::new();
        for file in files.flatten() {
            let path = file.path();
            if matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("json" | "previous")
            ) {
                names.insert(path.with_extension("json"));
            }
        }
        names
            .into_iter()
            .filter_map(|p| self.read::<Checkpoint>(&p))
            .filter(|c| valid_id(&c.record.identity.session))
            .map(|mut c| {
                c.record.running = false;
                c.record.pid = None;
                c
            })
            .collect()
    }
    /// Import the old manifest once. No executable, argv or environment is
    /// evaluated by migration. Keep the exact original bytes as a private backup.
    pub fn migrate(&self, data: &Path) -> io::Result<Option<chda_core::SavedSession>> {
        if let Some(layout) = self.layout() {
            return Ok(Some(layout));
        }
        let legacy = data.join("session.json");
        if legacy.exists() && !self.root.join("legacy-session.json").exists() {
            chda_agents::ipc::write_private_atomic(
                &self.root.join("legacy-session.json"),
                &fs::read(&legacy)?,
            )?;
        }
        let layout = chda_core::SavedSession::load(data).or_else(|| {
            let bytes = fs::read(&legacy).ok()?;
            let value = serde_json::from_slice::<serde_json::Value>(&bytes).ok()?;
            let (layout, rejected) = salvage_legacy(value);
            if !rejected.is_empty() {
                // The byte-for-byte original is already backed up. Keep the
                // unreadable entries separate from the usable layout as well.
                let _ = self.save(&self.root.join("legacy-rejected.json"), &rejected);
            }
            layout
        });
        if let Some(layout) = &layout {
            self.save_layout(layout)?;
        }
        Ok(layout)
    }
}

fn salvage_legacy(
    value: serde_json::Value,
) -> (Option<chda_core::SavedSession>, Vec<serde_json::Value>) {
    let active = value
        .get("active_window")
        .and_then(|n| n.as_u64())
        .unwrap_or(0) as usize;
    let windows = value
        .get("windows")
        .and_then(|w| w.as_array())
        .cloned()
        .unwrap_or_else(|| vec![value]);
    let mut result = chda_core::SavedSession::default();
    let mut rejected = Vec::new();
    for (index, window) in windows.into_iter().enumerate() {
        let Some(tabs) = window.get("tabs").and_then(|t| t.as_array()) else {
            rejected.push(window);
            continue;
        };
        let old_active = window.get("active").and_then(|a| a.as_u64()).unwrap_or(0) as usize;
        let mut saved = chda_core::SavedWindow {
            bounds: window
                .get("bounds")
                .and_then(|b| serde_json::from_value(b.clone()).ok()),
            ..Default::default()
        };
        for (tab_index, tab) in tabs.iter().enumerate() {
            match serde_json::from_value::<chda_core::SavedTab>(tab.clone()) {
                Ok(tab) => {
                    if tab_index == old_active {
                        saved.active = saved.tabs.len();
                    }
                    saved.tabs.push(tab);
                }
                Err(_) => rejected.push(tab.clone()),
            }
        }
        if !saved.tabs.is_empty() {
            if index == active {
                result.active_window = result.windows.len();
            }
            result.windows.push(saved);
        }
    }
    ((!result.windows.is_empty()).then_some(result), rejected)
}
pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simulated_storage_full_does_not_replace_a_good_checkpoint() {
        let root =
            std::env::temp_dir().join(format!("cs-full-{}", chda_pty::local::token().unwrap()));
        let store = Store::new(&root).unwrap();
        let path = store.root.join("full.json");
        store.save(&path, &vec![String::from("retained")]).unwrap();
        let result = store.save_with(&path, &vec![String::from("replacement")], |p, bytes| {
            if p.extension().and_then(|e| e.to_str()) == Some("previous") {
                chda_agents::ipc::write_private_atomic(p, bytes)
            } else {
                Err(io::Error::from_raw_os_error(28))
            }
        });
        assert!(result.is_err());
        assert_eq!(
            store.read::<Vec<String>>(&path),
            Some(vec!["retained".into()])
        );
        assert_eq!(
            store.read::<Vec<String>>(&path.with_extension("previous")),
            Some(vec!["retained".into()])
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corruption_recovers_the_last_good_checkpoint_without_replacing_it() {
        let root =
            std::env::temp_dir().join(format!("cs-store-{}", chda_pty::local::token().unwrap()));
        let store = Store::new(&root).unwrap();
        let path = store.root.join("sample.json");
        store.save(&path, &vec![String::from("first")]).unwrap();
        store.save(&path, &vec![String::from("second")]).unwrap();
        fs::write(&path, b"{interrupted").unwrap();
        assert_eq!(store.read::<Vec<String>>(&path), Some(vec!["first".into()]));
        assert!(path.with_extension("corrupt").is_file());
        store.save(&path, &vec![String::from("recovered")]).unwrap();
        assert_eq!(
            store.read::<Vec<String>>(&path),
            Some(vec!["recovered".into()])
        );
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(
                &fs::read(path.with_extension("previous")).unwrap()
            )
            .unwrap(),
            vec!["first"]
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn migration_backs_up_the_old_file_and_does_not_start_commands() {
        let root =
            std::env::temp_dir().join(format!("cs-migrate-{}", chda_pty::local::token().unwrap()));
        fs::create_dir_all(&root).unwrap();
        let bytes=br#"{"tabs":[{"root":{"kind":"pane","cwd":"/missing","agent":"minimax","session":"exact"},"focused":0}],"active":0}"#;
        fs::write(root.join("session.json"), bytes).unwrap();
        let store = Store::new(&root).unwrap();
        let migrated = store.migrate(&root).unwrap().unwrap();
        assert_eq!(migrated.windows.len(), 1);
        assert!(store.load().is_empty());
        assert_eq!(
            fs::read(store.root.join("legacy-session.json")).unwrap(),
            bytes
        );
        assert_eq!(fs::read(root.join("session.json")).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_damaged_legacy_tab_does_not_discard_other_windows_or_tabs() {
        let root =
            std::env::temp_dir().join(format!("cs-salvage-{}", chda_pty::local::token().unwrap()));
        fs::create_dir_all(&root).unwrap();
        let good = serde_json::json!({"root":{"kind":"pane","cwd":"/missing"},"focused":0});
        let bad = serde_json::json!({"root":{"kind":"unknown"},"focused":0});
        let bytes = serde_json::to_vec(&serde_json::json!({"windows":[{"active":1,"tabs":[bad.clone(),good.clone()]},{"active":0,"tabs":[good]}],"active_window":1})).unwrap();
        fs::write(root.join("session.json"), &bytes).unwrap();
        let store = Store::new(&root).unwrap();
        let saved = store.migrate(&root).unwrap().unwrap();
        assert_eq!(saved.windows.len(), 2);
        assert_eq!(saved.active_window, 1);
        assert_eq!(saved.windows[0].tabs.len(), 1);
        assert_eq!(
            store.read::<Vec<serde_json::Value>>(&store.root.join("legacy-rejected.json")),
            Some(vec![bad])
        );
        assert!(store.load().is_empty());
        assert_eq!(
            fs::read(store.root.join("legacy-session.json")).unwrap(),
            bytes
        );
        fs::remove_dir_all(root).unwrap();
    }
}
