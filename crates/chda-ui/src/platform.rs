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
