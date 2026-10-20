//! Pixel identicons for agent sessions: a small mirrored pattern and a hue,
//! derived locally from the provider and the full session id so that a
//! conversation keeps its picture whatever its name, status or place.

/// Cells per side.
pub const SIDE: usize = 5;

/// A 5×5 pattern, mirrored left to right, and its hue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identicon {
    /// Rows from the top; `cells[row][column]`.
    pub cells: [[bool; SIDE]; SIDE],
    /// Hue in degrees, 0 to 359.
    pub hue: u16,
}

impl Identicon {
    /// The picture of one conversation. The provider is part of the input,
    /// so the same id under two providers gets two pictures; the whole id is,
    /// so ids that share a prefix do not.
    pub fn new(provider: &str, session: &str) -> Self {
        let bits = hash(provider, session);
        let mut cells = [[false; SIDE]; SIDE];
        for (row, line) in cells.iter_mut().enumerate() {
            for column in 0..=SIDE / 2 {
                let on = bits >> (row * (SIDE / 2 + 1) + column) & 1 == 1;
                line[column] = on;
                line[SIDE - 1 - column] = on;
            }
        }
        // An empty picture would look like a missing one.
        if cells.iter().flatten().all(|on| !on) {
            cells[SIDE / 2][SIDE / 2] = true;
        }
        Self {
            cells,
            hue: ((bits >> 24) % 360) as u16,
        }
    }
}

/// FNV-1a over `provider`, a separator and `session`, then splitmix64's
/// finalizer so that every input bit reaches every output bit. Written out
/// instead of using `std`'s hashers, whose output may change between Rust
/// versions: a picture must stay the same across releases.
fn hash(provider: &str, session: &str) -> u64 {
    let mut state: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in provider
        .bytes()
        .chain(std::iter::once(0))
        .chain(session.bytes())
    {
        state ^= u64::from(byte);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    state = (state ^ (state >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    state = (state ^ (state >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    state ^ (state >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn picture(icon: &Identicon) -> String {
        icon.cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|on| if *on { '#' } else { '.' })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn pictures_never_change_between_releases() {
        // Pinned values: changing the algorithm would give every existing
        // conversation a new face.
        let uuid = "0199c2d4-5b1e-7f3a-9c60-2a8e4d1b7f05";
        let claude = Identicon::new("claude", uuid);
        let codex = Identicon::new("codex", uuid);
        assert_eq!(picture(&claude), "##.##/.###./.#.#./##.##/#.#.#");
        assert_eq!(claude.hue, 10);
        assert_eq!(picture(&codex), ".#.#./...../#...#/##.##/.#.#.");
        assert_eq!(codex.hue, 113);
    }

    #[test]
    fn the_same_conversation_always_gets_the_same_picture() {
        let a = Identicon::new("claude", "session-1");
        assert_eq!(a, Identicon::new("claude", "session-1"));
    }

    #[test]
    fn pictures_mirror_left_to_right() {
        for n in 0..200 {
            let icon = Identicon::new("codex", &format!("id-{n}"));
            for row in icon.cells {
                for column in 0..SIDE {
                    assert_eq!(row[column], row[SIDE - 1 - column]);
                }
            }
            assert!(icon.cells.iter().flatten().any(|on| *on));
            assert!(icon.hue < 360);
        }
    }

    #[test]
    fn the_provider_and_the_whole_id_both_count() {
        // The same id under two providers.
        let differing = (0..64)
            .filter(|n| {
                let id = format!("0199c2d4-5b1e-7f3a-9c60-{n:012x}");
                Identicon::new("claude", &id) != Identicon::new("codex", &id)
            })
            .count();
        assert!(differing >= 60, "{differing} of 64 differ");

        // Ids that share all but their last characters.
        let prefix = "0199c2d4-5b1e-7f3a-9c60-2a8e4d1b7f";
        let pictures: HashSet<_> = (0..256)
            .map(|n| {
                let icon = Identicon::new("claude", &format!("{prefix}{n:02x}"));
                (picture(&icon), icon.hue)
            })
            .collect();
        assert!(pictures.len() >= 240, "{} of 256 differ", pictures.len());
    }

    #[test]
    fn a_name_cannot_blur_into_the_id() {
        // "ab" + "c" and "a" + "bc" are different conversations.
        assert_ne!(Identicon::new("ab", "c"), Identicon::new("a", "bc"));
    }
}
