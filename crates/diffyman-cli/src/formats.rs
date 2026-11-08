//! `diffyman formats`: the kinds, adapters and input formats this build
//! compares, as a table or as JSON.

use std::io::Write;

use diffyman_detect::ExtensionClass;
use serde_json::{Value, json};

use crate::output::{Failure, Sink};

/// Archive formats of the tree adapter and how each is recognized (spec §19.1).
const ARCHIVES: [(&str, &str); 4] = [
    ("zip", "starts with 50 4B 03 04 or 50 4B 05 06"),
    ("tar", "ustar at bytes 257 to 261 (POSIX and GNU tar)"),
    ("tar.gz", "gzip (1F 8B) around a tar"),
    ("tar.zst", "zstd (28 B5 2F FD) around a tar"),
];

/// The output formats of `diffyman diff`.
const OUTPUT_FORMATS: [&str; 6] = ["text", "unified", "json", "html", "summary", "json-stream"];

/// Everything `formats` reports, as a JSON value.
pub fn catalog() -> Value {
    let extensions = |class: ExtensionClass| -> Value {
        diffyman_detect::EXTENSIONS
            .iter()
            .filter(|(_, c, _)| *c == class)
            .map(|(ext, _, media)| json!({ "extension": ext, "mediaType": media }))
            .collect()
    };
    json!({
        "schemaVersion": diffyman::model::SCHEMA_VERSION,
        "engine": { "name": diffyman::ENGINE_NAME, "version": diffyman::ENGINE_VERSION },
        "detect": diffyman_detect::DETECT_ID,
        "kinds": [
            {
                "kind": "text",
                "adapter": "text",
                "algorithms": [
                    diffyman::model::Algorithm::Histogram.id(),
                    diffyman::model::Algorithm::Myers.id(),
                    diffyman::model::Algorithm::Patience.id(),
                ],
                "refine": diffyman_algo::refine::REFINE_ID,
                "moves": diffyman_algo::moves::MOVES_ID,
                "inputs": "UTF-8 text or empty input (a UTF-8 BOM is removed); LF, CRLF and CR line endings",
                "extensions": extensions(ExtensionClass::Text),
            },
            {
                "kind": "binary",
                "adapter": "binary",
                "algorithms": [diffyman_adapter_binary::BINARY_ID],
                "inputs": "any bytes: everything that is not UTF-8 text or an archive pair",
                "extensions": extensions(ExtensionClass::Binary),
            },
            {
                "kind": "tree",
                "adapter": "tree",
                "algorithms": [diffyman_adapter_tree::TREE_ID],
                "inputs": "folders, and two archives",
                "archives": ARCHIVES
                    .iter()
                    .map(|(format, magic)| json!({ "format": format, "recognizedBy": magic }))
                    .collect::<Vec<_>>(),
            },
        ],
        "outputFormats": OUTPUT_FORMATS,
    })
}

/// Prints the catalog as indented JSON (`json`) or as a table.
pub fn run(json: bool) -> Result<u8, Failure> {
    let catalog = catalog();
    let mut out = Sink::open(None)?;
    if json {
        serde_json::to_writer_pretty(&mut out, &catalog).map_err(std::io::Error::from)?;
        out.write_all(b"\n")?;
    } else {
        write_table(&mut out, &catalog)?;
    }
    out.finish()?;
    Ok(0)
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn write_table<W: Write>(out: &mut W, catalog: &Value) -> std::io::Result<()> {
    writeln!(
        out,
        "diffyman {} (Diff Document {}), type detection {}",
        catalog["engine"]["version"].as_str().unwrap_or(""),
        catalog["schemaVersion"].as_str().unwrap_or(""),
        catalog["detect"].as_str().unwrap_or("")
    )?;
    let empty = Vec::new();
    for kind in catalog["kinds"].as_array().unwrap_or(&empty) {
        writeln!(out)?;
        writeln!(
            out,
            "{:<8}{}",
            kind["kind"].as_str().unwrap_or(""),
            kind["inputs"].as_str().unwrap_or("")
        )?;
        writeln!(
            out,
            "        algorithms: {}",
            strings(&kind["algorithms"]).join(", ")
        )?;
        if let Some(refine) = kind["refine"].as_str() {
            writeln!(out, "        inline changes: {refine} (--granularity)")?;
        }
        if let Some(moves) = kind["moves"].as_str() {
            writeln!(out, "        moved blocks: {moves} (--detect-moves)")?;
        }
    }
    writeln!(out)?;
    writeln!(
        out,
        "Archives (recognized from their first bytes, never their name):"
    )?;
    for archive in catalog["kinds"][2]["archives"].as_array().unwrap_or(&empty) {
        writeln!(
            out,
            "  {:<9}{}",
            archive["format"].as_str().unwrap_or(""),
            archive["recognizedBy"].as_str().unwrap_or("")
        )?;
    }
    writeln!(out)?;
    writeln!(out, "Media types by extension:")?;
    for kind in catalog["kinds"].as_array().unwrap_or(&empty) {
        let Some(list) = kind["extensions"].as_array() else {
            continue;
        };
        let items: Vec<String> = list
            .iter()
            .map(|e| {
                format!(
                    "{} {}",
                    e["extension"].as_str().unwrap_or(""),
                    e["mediaType"].as_str().unwrap_or("")
                )
            })
            .collect();
        // Wrapped at 80 columns.
        let mut line = format!("  {:<8}", kind["kind"].as_str().unwrap_or(""));
        let mut first = true;
        for item in items {
            if !first && line.len() + item.len() + 2 > 80 {
                writeln!(out, "{line},")?;
                line = " ".repeat(10);
                first = true;
            }
            if !first {
                line.push_str(", ");
            }
            line.push_str(&item);
            first = false;
        }
        writeln!(out, "{line}")?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "Output formats of diffyman diff (--format, --json-stream): {}",
        strings(&catalog["outputFormats"]).join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The IDs listed are the ones documents carry in provenance.
    #[test]
    fn ids_match_provenance() {
        let catalog = catalog();
        let doc = |left: &[u8], right: &[u8], options: &str| {
            let options = diffyman::parse_options(options).unwrap();
            diffyman::diff_document(
                diffyman::Input::new(left),
                diffyman::Input::new(right),
                &options,
            )
            .unwrap()
        };
        let text = doc(
            b"a\n",
            b"b\n",
            r#"{"granularity": "word", "detectMoves": true}"#,
        );
        assert_eq!(
            catalog["kinds"][0]["algorithms"][0],
            text.provenance.algorithm
        );
        assert_eq!(
            catalog["kinds"][0]["refine"],
            text.provenance.extra["refine"]
        );
        assert_eq!(catalog["kinds"][0]["moves"], text.provenance.extra["moves"]);
        assert_eq!(catalog["detect"], text.provenance.extra["detect"]);
        let binary = doc(b"\0\x01", b"\0\x02", "{}");
        assert_eq!(
            catalog["kinds"][1]["algorithms"][0],
            binary.provenance.algorithm
        );
        let empty_zip = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let tree = doc(empty_zip, empty_zip, "{}");
        assert_eq!(
            catalog["kinds"][2]["algorithms"][0],
            tree.provenance.algorithm
        );
    }
}
