//! What every crate published to crates.io carries. A crate under `crates/`
//! is published unless its manifest says `publish = false`; each published
//! one needs its own README and copies of both license texts equal to the
//! repository's, listed in `include` so that `cargo package` takes them, and
//! the documentation link. Workspace crates depend on each other at the exact
//! workspace version.

use std::fs;
use std::path::{Path, PathBuf};

/// The crates published at the current version, by folder name. A crate
/// added to or removed from this list is a decision about the release.
const PUBLISHED: &[&str] = &[
    "diffyman-adapter-binary",
    "diffyman-adapter-tree",
    "diffyman-algo",
    "diffyman-cli",
    "diffyman-detect",
    "diffyman-engine",
    "diffyman-model",
    "diffyman-render",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn is_published(manifest: &str) -> bool {
    !manifest
        .lines()
        .any(|line| line.trim() == "publish = false")
}

#[test]
fn published_crates_are_the_listed_ones() {
    let mut published: Vec<String> = fs::read_dir(repo().join("crates"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|dir| dir.join("Cargo.toml").is_file())
        .filter(|dir| is_published(&read(&dir.join("Cargo.toml"))))
        .map(|dir| dir.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    published.sort();
    assert_eq!(published, PUBLISHED, "crates without `publish = false`");
}

#[test]
fn published_crates_carry_readme_and_license_texts() {
    let root = repo();
    for name in PUBLISHED {
        let dir = root.join("crates").join(name);
        let manifest = read(&dir.join("Cargo.toml"));
        for field in [
            "documentation.workspace = true",
            "readme = \"README.md\"",
            "keywords = [",
            "categories = [",
        ] {
            assert!(
                manifest.contains(field),
                "{name}: Cargo.toml has no `{field}`"
            );
        }
        let include = manifest
            .lines()
            .find(|line| line.starts_with("include = ["))
            .unwrap_or_else(|| panic!("{name}: Cargo.toml has no one-line `include`"));
        for file in ["/src/**", "/README.md", "/LICENSE-MIT", "/LICENSE-APACHE"] {
            assert!(
                include.contains(&format!("\"{file}\"")),
                "{name}: `include` lacks {file}"
            );
        }
        assert!(
            !read(&dir.join("README.md")).trim().is_empty(),
            "{name}: README.md is empty"
        );
        for license in ["LICENSE-MIT", "LICENSE-APACHE"] {
            let copy =
                fs::read(dir.join(license)).unwrap_or_else(|e| panic!("{name}/{license}: {e}"));
            let original = fs::read(root.join(license)).unwrap();
            assert!(
                copy == original,
                "{name}/{license} differs from the repository's {license}"
            );
        }
    }
    // The Apache-2.0-only Histogram files keep their NOTICE inside src/.
    assert!(
        root.join("crates/diffyman-algo/src/line_diff/histogram/NOTICE")
            .is_file()
    );
}

#[test]
fn workspace_crates_are_pinned_to_the_workspace_version() {
    let manifest = read(&repo().join("Cargo.toml"));
    let version = manifest
        .lines()
        .skip_while(|line| *line != "[workspace.package]")
        .find_map(|line| line.strip_prefix("version = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("workspace version");
    let pins: Vec<&str> = manifest
        .lines()
        .filter(|line| line.contains("path = \"crates/"))
        .collect();
    assert!(
        pins.len() >= PUBLISHED.len(),
        "workspace dependencies on the crates"
    );
    for line in pins {
        assert!(
            line.contains(&format!("version = \"={version}\"")),
            "not pinned to ={version}: {line}"
        );
    }
}
