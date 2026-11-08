use std::{env, path::PathBuf, process::ExitCode};

use mathscape_tool_lean::{LeanProfileStore, LoogleAdapter, LoogleSearchRequest};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mathscape-lean-search: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("loogle") {
        return Err("usage: mathscape-lean-search loogle <store-root> <profile-id> <loogle-executable> <query>".into());
    }
    let store_root = required(&mut args, "store root")?;
    let profile_id = required(&mut args, "profile id")?;
    let executable = required(&mut args, "Loogle executable")?;
    let query = required(&mut args, "query")?;
    let mut request = LoogleSearchRequest::new(query)?;
    if let Some(option) = args.next() {
        if option == "--index-file" {
            request = request.index_file(required(&mut args, "index file")?);
        } else {
            return Err("unexpected extra arguments".into());
        }
    }
    let store = LeanProfileStore::new(PathBuf::from(store_root))?;
    let profile = store.open(&profile_id)?;
    let result = LoogleAdapter::new(executable)?.search(&profile, &request)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn required(
    args: &mut impl Iterator<Item = String>,
    field: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    args.next().ok_or_else(|| format!("missing {field}").into())
}
