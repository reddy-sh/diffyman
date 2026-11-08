//! Diffs two files and prints one rendering, for trying the renderers and for
//! the unified-diff compatibility check (`tools/compare_unified.py`).
//!
//! ```text
//! cargo run -p diffyman-render --example render -- unified LEFT RIGHT [--options FILE] [--context N] [--color]
//! ```
//!
//! The format is `unified`, `html` or `summary`.

use std::io::Write as _;
use std::process::ExitCode;

use diffyman::Input;
use diffyman_render as render;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("render: {e}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [format, left_path, right_path, rest @ ..] = args.as_slice() else {
        return Err("usage: render unified|html|summary LEFT RIGHT [--options FILE] [--context N] [--color]".to_owned());
    };
    let mut options_json = String::from("{}");
    let mut context = None;
    let mut color = false;
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--options" => {
                let path = it.next().ok_or("--options needs a file")?;
                options_json = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            }
            "--context" => {
                let n = it.next().ok_or("--context needs a number")?;
                context = Some(n.parse::<u32>().map_err(|e| format!("--context: {e}"))?);
            }
            "--color" => color = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let left = std::fs::read(left_path).map_err(|e| format!("{left_path}: {e}"))?;
    let right = std::fs::read(right_path).map_err(|e| format!("{right_path}: {e}"))?;
    let options = diffyman::parse_options(&options_json).map_err(|e| e.to_string())?;
    let doc = diffyman::diff_document(
        Input::named(&left, left_path),
        Input::named(&right, right_path),
        &options,
    )
    .map_err(|e| e.to_string())?;
    let text = match format.as_str() {
        "unified" => render::unified(
            &doc,
            &left,
            &right,
            &render::UnifiedOptions {
                context,
                color,
                ..render::UnifiedOptions::default()
            },
        ),
        "html" => render::html(
            &doc,
            &left,
            &right,
            &render::HtmlOptions {
                context,
                ..render::HtmlOptions::default()
            },
        ),
        "summary" => Ok(render::summary(&doc, &render::SummaryOptions::default())),
        other => return Err(format!("unknown format {other}")),
    }
    .map_err(|e| e.to_string())?;
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())
}
