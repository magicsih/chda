//! Readable random branch names (`brisk-otter`) from bundled word lists.

use std::hash::{BuildHasher, Hasher};

const ADJECTIVES: &str = include_str!("words/adjectives.txt");
const NOUNS: &str = include_str!("words/nouns.txt");

/// How many random picks to try before adding a number.
const ATTEMPTS: usize = 32;

/// A random `adjective-noun` name for which `taken` is false. After a few
/// collisions a number is appended (`brisk-otter-2`), so this always ends.
pub fn random_branch_name(taken: impl Fn(&str) -> bool) -> String {
    let adjectives: Vec<&str> = words(ADJECTIVES);
    let nouns: Vec<&str> = words(NOUNS);
    let mut last = String::new();
    for _ in 0..ATTEMPTS {
        let n = random();
        let name = format!(
            "{}-{}",
            adjectives[(n % adjectives.len() as u64) as usize],
            nouns[((n >> 32) % nouns.len() as u64) as usize]
        );
        if !taken(&name) {
            return name;
        }
        last = name;
    }
    (2..)
        .map(|i| format!("{last}-{i}"))
        .find(|name| !taken(name))
        .unwrap_or(last)
}

fn words(list: &str) -> Vec<&str> {
    list.lines()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect()
}

/// A random number from the standard library's per-process hash keys, so
/// no RNG dependency is needed for picking words.
fn random() -> u64 {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(name: &str) -> bool {
        name.split('-').count() >= 2
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    }

    #[test]
    fn names_are_readable_and_avoid_taken_ones() {
        let name = random_branch_name(|_| false);
        assert!(valid(&name), "{name}");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            seen.insert(random_branch_name(|_| false));
        }
        assert!(seen.len() > 40, "names vary: {seen:?}");

        let first = random_branch_name(|_| false);
        let other = random_branch_name(|n| n == first);
        assert_ne!(other, first);
    }

    #[test]
    fn a_number_is_added_when_every_pick_collides() {
        let name = random_branch_name(|n| n.split('-').count() == 2);
        assert!(name.ends_with("-2"), "{name}");
        assert!(valid(&name));
    }

    #[test]
    fn word_lists_are_branch_safe_and_unique() {
        for list in [ADJECTIVES, NOUNS] {
            let w = words(list);
            assert!(w.len() >= 100);
            let unique: std::collections::HashSet<_> = w.iter().collect();
            assert_eq!(unique.len(), w.len());
            assert!(w.iter().all(|w| w.chars().all(|c| c.is_ascii_lowercase())));
        }
    }
}
