//! Diff algorithms for diffyman: line splitting, comparison keys, line diff
//! behind versioned algorithm IDs, op grouping, inline refinement and (from
//! Phase 1) moved-block detection.
//!
//! This crate knows nothing about the Diff Document JSON; `diffyman` (the
//! engine) maps its results onto `diffyman-model` types.
//!
//! - [`lines`]: LF, CRLF and lone-CR line splitting (Diff Document spec §7.2).
//! - [`key`]: comparison keys (whitespace modes, simple case folding, regex
//!   masks) and token interning.
//! - [`line_diff`]: `text.histogram@1`, `text.myers@1`, `text.patience@1`.
//! - [`ops`]: the full op list and hunk grouping (spec §7.3, §7.4).
//! - [`refine`]: word, char and smart inline spans (`text.refine@1`).
//! - [`moves`]: pure and edited moved blocks (`text.moves@1`).

pub mod key;
pub mod line_diff;
pub mod lines;
pub mod moves;
pub mod ops;
pub mod refine;

/// Versions of the libraries whose behavior defines the algorithm IDs. They
/// must match the exact pins in the workspace `Cargo.toml`.
pub const LIBRARY_VERSIONS: [(&str, &str); 7] = [
    ("gix-imara-diff", "0.3.3"),
    ("similar", "3.2.1"),
    ("unicode-segmentation", "1.13.3"),
    ("icu_casemap", "2.4.0"),
    ("icu_properties", "2.4.0"),
    ("regex", "1.13.1"),
    ("regex-automata", "0.4.18"),
];

/// The Unicode version of the word and grapheme segmentation tables.
pub fn segmentation_unicode_version() -> String {
    let (major, minor, patch) = unicode_segmentation::UNICODE_VERSION;
    format!("{major}.{minor}.{patch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_versions_match_workspace_pins() {
        let manifest = include_str!("../../../Cargo.toml");
        for (name, version) in LIBRARY_VERSIONS {
            let pin = format!("{name} = {{ version = \"={version}\"");
            let plain = format!("{name} = \"={version}\"");
            assert!(
                manifest.contains(&pin) || manifest.contains(&plain),
                "{name} {version} is not the pinned version"
            );
        }
    }

    #[test]
    fn unicode_version_is_reported() {
        assert!(segmentation_unicode_version().starts_with(char::is_numeric));
    }
}
