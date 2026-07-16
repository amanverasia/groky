//! Deterministic provider catalog generator.
//!
//! Reads a raw models.dev document from a local file (`--input`) or a URL
//! (`--fetch`, requires the `generator` feature), normalizes it, applies the
//! committed overrides, and writes canonical pretty JSON with exactly one
//! trailing newline to `--output`. With `--check` it compares bytes against
//! the existing output instead of writing.

use std::process::ExitCode;

use xai_grok_catalog::{NormalizationLimits, apply_patch, load_overrides, normalize_models_dev};

struct Args {
    input: Option<String>,
    fetch: Option<String>,
    output: String,
    check: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut input = None;
    let mut fetch = None;
    let mut output = None;
    let mut check = false;
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--input" => input = Some(argv.next().ok_or("--input requires a path")?),
            "--fetch" => fetch = Some(argv.next().ok_or("--fetch requires a url")?),
            "--output" => output = Some(argv.next().ok_or("--output requires a path")?),
            "--check" => check = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if input.is_some() == fetch.is_some() {
        return Err("exactly one of --input or --fetch is required".to_string());
    }
    Ok(Args {
        input,
        fetch,
        output: output.ok_or("--output is required")?,
        check,
    })
}

#[cfg(feature = "generator")]
fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| format!("failed to build http client: {err}"))?
        .get(url)
        .send()
        .map_err(|err| format!("failed to fetch {url}: {err}"))?
        .error_for_status()
        .map_err(|err| format!("failed to fetch {url}: {err}"))?;
    response
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|err| format!("failed to read body from {url}: {err}"))
}

#[cfg(not(feature = "generator"))]
fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    Err(format!(
        "--fetch {url} requires the `generator` feature: \
         cargo run -p xai-grok-catalog --features generator --bin generate_catalog"
    ))
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let raw = match (&args.input, &args.fetch) {
        (Some(path), None) => {
            std::fs::read(path).map_err(|err| format!("failed to read {path}: {err}"))?
        }
        (None, Some(url)) => fetch_bytes(url)?,
        _ => unreachable!("parse_args enforces exactly one source"),
    };

    let catalog = normalize_models_dev(&raw, NormalizationLimits::default())
        .map_err(|err| format!("normalization failed: {err}"))?;
    let catalog =
        apply_patch(catalog, load_overrides()).map_err(|err| format!("overrides failed: {err}"))?;

    let mut text = serde_json::to_string_pretty(&catalog)
        .map_err(|err| format!("serialization failed: {err}"))?;
    text.push('\n');

    if args.check {
        let existing = std::fs::read(&args.output)
            .map_err(|err| format!("failed to read {}: {err}", args.output))?;
        if existing != text.as_bytes() {
            return Err(format!(
                "{} is stale; run scripts/update-provider-catalog.sh",
                args.output
            ));
        }
        println!("Catalog snapshot is current");
    } else {
        std::fs::write(&args.output, text)
            .map_err(|err| format!("failed to write {}: {err}", args.output))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
