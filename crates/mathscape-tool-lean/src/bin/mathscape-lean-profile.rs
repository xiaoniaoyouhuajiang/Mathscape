use std::{env, path::PathBuf, process::ExitCode};

use mathscape_tool_lean::{LeanProfileStore, ManagedLeanProfileSpec};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mathscape-lean-profile: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("register") => {
            let store_root = required(&mut args, "store root")?;
            let id = required(&mut args, "profile id")?;
            let project_root = required(&mut args, "project root")?;
            let toolchain = required(&mut args, "toolchain")?;
            let mathlib_reference = required(&mut args, "mathlib reference")?;
            if args.next().is_some() {
                return Err("unexpected extra arguments".into());
            }
            let store = LeanProfileStore::new(PathBuf::from(store_root))?;
            let spec = ManagedLeanProfileSpec::new(
                id,
                PathBuf::from(project_root),
                toolchain,
                mathlib_reference,
            )?;
            let profile = store.register_existing(&spec)?;
            println!("{}", serde_json::to_string_pretty(profile.metadata())?);
            Ok(())
        }
        Some("load") => {
            let store_root = required(&mut args, "store root")?;
            let id = required(&mut args, "profile id")?;
            if args.next().is_some() {
                return Err("unexpected extra arguments".into());
            }
            let store = LeanProfileStore::new(PathBuf::from(store_root))?;
            println!("{}", serde_json::to_string_pretty(&store.load(&id)?)?);
            Ok(())
        }
        _ => Err("usage: mathscape-lean-profile register <store-root> <id> <project-root> <toolchain> <mathlib-reference> | load <store-root> <id>".into()),
    }
}

fn required(
    args: &mut impl Iterator<Item = String>,
    field: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    args.next().ok_or_else(|| format!("missing {field}").into())
}
