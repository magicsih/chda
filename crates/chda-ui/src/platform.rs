//! Platform-specific UI defaults.

/// Monospace face every install of the platform ships with.
#[cfg(target_os = "macos")]
pub const DEFAULT_MONOSPACE: &str = "Menlo";
#[cfg(target_os = "windows")]
pub const DEFAULT_MONOSPACE: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const DEFAULT_MONOSPACE: &str = "DejaVu Sans Mono";

/// Play the system alert sound.
pub fn beep() {
    #[cfg(target_os = "macos")]
    unsafe {
        NSBeep();
    }
}

#[cfg(target_os = "macos")]
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSBeep();
}

/// Show a user notification. Uses the system scripting bridge on macOS
/// until the app ships as a signed bundle (then UNUserNotification).
pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            esc(body),
            esc(title)
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, body);
    }
}
