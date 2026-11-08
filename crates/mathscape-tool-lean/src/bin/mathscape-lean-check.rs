use std::{env, fs, path::PathBuf, process::ExitCode};

use mathscape_tool_lean::{
    LeanCancellation, LeanCheckRequest, LeanChecker, LeanProfileStore, LeanReceiptStore,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mathscape-lean-check: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let store_root = required(&mut args, "store root")?;
    let profile_id = required(&mut args, "profile id")?;
    let source_path = PathBuf::from(required(&mut args, "source file")?);
    let source_text = fs::read_to_string(&source_path)?;
    let mut request = LeanCheckRequest::new(source_text)?;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--profile-fingerprint" => {
                request = request.profile_fingerprint(required(&mut args, "profile fingerprint")?);
            }
            "--timeout-ms" => {
                let value = required(&mut args, "timeout milliseconds")?.parse::<u64>()?;
                request = request.timeout_ms(value)?;
            }
            "--max-output-bytes" => {
                let value = required(&mut args, "maximum output bytes")?.parse::<usize>()?;
                request = request.max_output_bytes(value)?;
            }
            _ => return Err(format!("unexpected argument: {option}").into()),
        }
    }
    let store = LeanProfileStore::new(PathBuf::from(store_root))?;
    let profile = store.open(&profile_id)?;
    let result = LeanChecker::new().check(&profile, &request, &LeanCancellation::new())?;
    let receipt = LeanReceiptStore::for_profile(&profile)?.record(&request, &result)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "check": result,
            "receipt": receipt,
        }))?
    );
    Ok(())
}

fn required(
    args: &mut impl Iterator<Item = String>,
    field: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    args.next().ok_or_else(|| format!("missing {field}").into())
}
