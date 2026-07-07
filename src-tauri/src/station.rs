//! `.station` host filtering (PRD §6).
//!
//! A `.station` file is UTF-8 text, one allowed hostname per line:
//! - case-insensitive, exact match against the current hostname
//! - blank lines ignored; lines whose first non-space char is `#` are comments
//! - absent file  -> Inherit (no opinion; ancestor decides)
//! - empty file   -> Hide (shows on NO host)
//! - non-empty    -> Show iff the current hostname is listed, else Hide
//!
//! A non-matching `.station` on a parent directory stops scanning the whole
//! subtree (handled in `menu_tree`).

use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationVerdict {
    /// `.station` lists this host (or file is absent — ancestor decides).
    Show,
    /// `.station` excludes this host (or is empty).
    Hide,
    /// No `.station` here — defer to ancestor.
    Inherit,
}

/// Parse `.station` bytes into a lowercased set of allowed hostnames.
/// Non-UTF8 input yields an empty set (caller treats as "non-UTF8, degrade").
pub fn parse_station_bytes(bytes: &[u8]) -> HashSet<String> {
    let mut set = HashSet::new();
    let Ok(text) = std::str::from_utf8(bytes) else {
        return set;
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        set.insert(line.to_lowercase());
    }
    set
}

/// Decide visibility for a single directory based on its own `.station` file.
pub fn verdict_for_dir(dir: &Path, hostname_lower: &str) -> StationVerdict {
    let station_path = dir.join(".station");
    match std::fs::read(&station_path) {
        Err(_) => StationVerdict::Inherit,
        Ok(bytes) => {
            if std::str::from_utf8(&bytes).is_err() {
                log::warn!(
                    ".station not valid UTF-8 at {}: treating as inherit",
                    dir.display()
                );
                return StationVerdict::Inherit;
            }
            let set = parse_station_bytes(&bytes);
            if set.is_empty() {
                StationVerdict::Hide
            } else if set.contains(hostname_lower) {
                StationVerdict::Show
            } else {
                StationVerdict::Hide
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_lowercase_and_ignores_comments_blanks() {
        let set = parse_station_bytes(b"DESKTOP-ABC\n  \n# comment\nTim-Laptop\n");
        assert!(set.contains("desktop-abc"));
        assert!(set.contains("tim-laptop"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn non_utf8_is_empty() {
        let set = parse_station_bytes(&[0xFF, 0xFE, 0x00]);
        assert!(set.is_empty());
    }

    #[test]
    fn empty_bytes_is_empty() {
        assert!(parse_station_bytes(b"").is_empty());
        assert!(parse_station_bytes(b"\n\n  \n# only comments\n").is_empty());
    }
}
