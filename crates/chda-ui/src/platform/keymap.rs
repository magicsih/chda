//! Translate application shortcuts while leaving ordinary terminal controls free.

#[cfg(target_os = "linux")]
const SHORTCUTS: &[(&str, &str)] = &[
    ("cmd-ctrl-left", "ctrl-alt-shift-left"),
    ("cmd-ctrl-right", "ctrl-alt-shift-right"),
    ("cmd-ctrl-up", "ctrl-alt-shift-up"),
    ("cmd-ctrl-down", "ctrl-alt-shift-down"),
    ("cmd-ctrl-=", "ctrl-alt-shift-="),
    ("cmd-shift-enter", "ctrl-shift-enter"),
    ("cmd-shift-]", "ctrl-pagedown"),
    ("cmd-shift-[", "ctrl-pageup"),
    ("cmd-shift-d", "ctrl-shift-e"),
    ("cmd-shift-g", "ctrl-alt-g"),
    ("cmd-shift-o", "ctrl-shift-o"),
    ("cmd-shift-n", "ctrl-shift-n"),
    ("cmd-shift-p", "ctrl-shift-p"),
    ("cmd-shift-a", "ctrl-shift-a"),
    ("cmd-shift-,", "ctrl-shift-,"),
    ("cmd-alt-left", "ctrl-shift-left"),
    ("cmd-alt-right", "ctrl-shift-right"),
    ("cmd-alt-up", "ctrl-shift-up"),
    ("cmd-alt-down", "ctrl-shift-down"),
    ("cmd-click", "ctrl-click"),
    ("cmd-up", "ctrl-shift-pageup"),
    ("cmd-down", "ctrl-shift-pagedown"),
    ("cmd-c", "ctrl-shift-c"),
    ("cmd-v", "ctrl-shift-v"),
    ("cmd-f", "ctrl-shift-f"),
    ("cmd-g", "ctrl-shift-g"),
    ("cmd-q", "ctrl-shift-q"),
    ("cmd-t", "ctrl-shift-t"),
    ("cmd-w", "ctrl-shift-w"),
    ("cmd-d", "ctrl-shift-d"),
    ("cmd-b", "ctrl-shift-b"),
    ("cmd-n", "ctrl-alt-n"),
    ("cmd-m", "ctrl-shift-m"),
    ("cmd-1", "alt-1"),
    ("cmd-2", "alt-2"),
    ("cmd-3", "alt-3"),
    ("cmd-4", "alt-4"),
    ("cmd-5", "alt-5"),
    ("cmd-6", "alt-6"),
    ("cmd-7", "alt-7"),
    ("cmd-8", "alt-8"),
    ("cmd-9", "alt-9"),
    ("cmd-]", "alt-]"),
    ("cmd-[", "alt-["),
    ("cmd-=", "ctrl-="),
    ("cmd-+", "ctrl-+"),
    ("cmd--", "ctrl--"),
    ("cmd-0", "ctrl-0"),
    ("cmd-,", "ctrl-,"),
];

pub(crate) fn hide_key_bindings() -> Vec<gpui::KeyBinding> {
    #[cfg(target_os = "linux")]
    {
        // GPUI's Linux hide operations are unsupported.
        Vec::new()
    }
    #[cfg(not(target_os = "linux"))]
    {
        vec![
            gpui::KeyBinding::new("cmd-h", crate::menus::Hide, None),
            gpui::KeyBinding::new("cmd-alt-h", crate::menus::HideOthers, None),
        ]
    }
}

pub(crate) fn shortcut(key: &str) -> &str {
    #[cfg(target_os = "linux")]
    {
        SHORTCUTS
            .iter()
            .find_map(|(from, to)| (*from == key).then_some(*to))
            // Text-field Select All has no terminal application binding.
            .unwrap_or(if key == "cmd-a" { "ctrl-a" } else { key })
    }
    #[cfg(not(target_os = "linux"))]
    {
        key
    }
}

pub(crate) fn shortcut_hint(text: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        SHORTCUTS
            .iter()
            .fold(text.to_owned(), |text, (from, to)| text.replace(from, to))
    }
    #[cfg(not(target_os = "linux"))]
    {
        text.to_owned()
    }
}

pub(crate) fn link_modifier(modifiers: gpui::Modifiers) -> bool {
    #[cfg(target_os = "linux")]
    {
        modifiers.control
    }
    #[cfg(not(target_os = "linux"))]
    {
        modifiers.platform
    }
}

#[cfg(test)]
pub(crate) fn link_modifiers() -> gpui::Modifiers {
    #[cfg(target_os = "linux")]
    {
        gpui::Modifiers {
            control: true,
            ..Default::default()
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        gpui::Modifiers::command()
    }
}

pub(crate) fn consumes_terminal_key(key: &gpui::Keystroke) -> bool {
    #[cfg(target_os = "linux")]
    {
        SHORTCUTS
            .iter()
            .filter(|(from, _)| *from != "cmd-click")
            .any(|(_, binding)| {
                gpui::Keystroke::parse(binding)
                    .is_ok_and(|bound| bound.key == key.key && bound.modifiers == key.modifiers)
            })
            || key.modifiers.platform
    }
    #[cfg(not(target_os = "linux"))]
    {
        key.modifiers.platform
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn desktop_shortcuts_do_not_capture_shell_control_keys() {
        for binding in ["ctrl-c", "ctrl-a", "ctrl-d", "ctrl-z", "ctrl-r"] {
            assert!(
                !consumes_terminal_key(&gpui::Keystroke::parse(binding).unwrap()),
                "{binding}"
            );
        }
        for binding in [
            "ctrl-shift-c",
            "ctrl-shift-v",
            "ctrl-shift-t",
            "ctrl-shift-w",
            "ctrl-shift-p",
            "alt-1",
        ] {
            assert!(
                consumes_terminal_key(&gpui::Keystroke::parse(binding).unwrap()),
                "{binding}"
            );
        }
        assert_eq!(shortcut("cmd-t"), "ctrl-shift-t");
        assert_eq!(shortcut("cmd-shift-d"), "ctrl-shift-e");
        assert_eq!(shortcut("cmd-a"), "ctrl-a");
        assert_eq!(
            shortcut_hint("Split down (cmd-shift-d), then cmd-click"),
            "Split down (ctrl-shift-e), then ctrl-click"
        );
        assert!(link_modifier(link_modifiers()));
        assert!(!link_modifier(gpui::Modifiers::command()));
        assert!(hide_key_bindings().is_empty());
        for binding in ["ctrl-shift-h", "ctrl-alt-shift-h"] {
            assert!(!consumes_terminal_key(
                &gpui::Keystroke::parse(binding).unwrap()
            ));
        }
    }
}
