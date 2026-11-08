use std::{env, path::PathBuf, process::ExitCode};

use mathscape_tool_lean::{DeclarationExportRequest, DeclarationExporter, LeanProfileStore};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mathscape-lean-export: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("exact") {
        return Err(
            "usage: mathscape-lean-export exact <store-root> <profile-id> <declaration>...".into(),
        );
    }
    let store_root = required(&mut args, "store root")?;
    let profile_id = required(&mut args, "profile id")?;
    let names = args.collect::<Vec<_>>();
    if names.is_empty() {
        return Err("at least one declaration is required".into());
    }
    let store = LeanProfileStore::new(PathBuf::from(store_root))?;
    let profile = store.open(&profile_id)?;
    let request = DeclarationExportRequest::exact_names(names)?;
    let result = DeclarationExporter::default().export(&profile, &request)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn required(
    args: &mut impl Iterator<Item = String>,
    field: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    args.next().ok_or_else(|| format!("missing {field}").into())
}
