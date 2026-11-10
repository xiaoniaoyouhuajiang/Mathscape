//! A stand-in for `loogle --json --interactive` used by the resident-mode
//! tests. No Lean or Mathlib is involved.
//!
//! It checks the argv the resident instance must pass (`--json`,
//! `--interactive`, `--index-mode read`, `--max-results 200`, an existing
//! `--index-file`) and that `LEAN_PATH` is set, then prints
//! `Loogle is ready.` and answers one JSON line per stdin line, like Loogle.
//!
//! Behaviour switches (environment, set by a wrapper script in tests):
//! - `LOOGLE_FIXTURE_START=never_ready|exit|slow:<ms>`
//! - `LOOGLE_FIXTURE_PIDS=<file>`: append this pid on start.
//! - `LOOGLE_FIXTURE_LOG=<file>`: append each received query.
//!
//! Queries: `hang` never answers, `crash` exits 9, `garbage` prints a non-JSON
//! line, `unknown` returns Loogle's error JSON, `none` returns `count: 0` with
//! suggestions, `many` returns `count: 250` with 200 hits; anything else
//! returns `count: 3` with three hits.

use std::{
    env,
    fs::OpenOptions,
    io::{self, BufRead, Write},
    path::Path,
    process::ExitCode,
    thread,
    time::Duration,
};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(problem) = check_args(&args) {
        eprintln!("loogle-fixture: {problem}");
        return ExitCode::from(2);
    }
    if env::var_os("LEAN_PATH").is_none_or(|value| value.is_empty()) {
        eprintln!("loogle-fixture: LEAN_PATH is not set");
        return ExitCode::from(2);
    }
    if let Some(file) = env::var_os("LOOGLE_FIXTURE_PIDS") {
        append(Path::new(&file), &std::process::id().to_string());
    }
    match env::var("LOOGLE_FIXTURE_START").as_deref() {
        Ok("never_ready") => loop {
            thread::sleep(Duration::from_secs(60));
        },
        Ok("exit") => {
            eprintln!("loogle-fixture: index is stale");
            return ExitCode::from(1);
        }
        Ok(other) if other.starts_with("slow:") => {
            let ms = other[5..].parse().unwrap_or(0);
            thread::sleep(Duration::from_millis(ms));
        }
        _ => {}
    }
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "Loogle is ready.");
    let _ = stdout.flush();
    let module = value_after(&args, "--module")
        .unwrap_or("Mathlib")
        .to_owned();
    for line in io::stdin().lock().lines() {
        let Ok(query) = line else { break };
        if query.is_empty() {
            return ExitCode::SUCCESS;
        }
        if let Some(file) = env::var_os("LOOGLE_FIXTURE_LOG") {
            append(Path::new(&file), &query);
        }
        let reply = match query.as_str() {
            "hang" => loop {
                thread::sleep(Duration::from_secs(60));
            },
            "crash" => {
                eprintln!("loogle-fixture: crashed on purpose");
                return ExitCode::from(9);
            }
            "garbage" => "this is not json".to_owned(),
            "unknown" => r#"{"error":"Unknown identifier 'klDiv'","heartbeats":12,"suggestions":["InformationTheory.klDiv"]}"#.to_owned(),
            "none" => r#"{"count":0,"header":"","heartbeats":1,"hits":[],"suggestions":["Nat.add_comm"]}"#.to_owned(),
            "many" => reply(&module, &query, 250, 200),
            _ => reply(&module, &query, 3, 3),
        };
        let _ = writeln!(stdout, "{reply}");
        let _ = stdout.flush();
    }
    ExitCode::SUCCESS
}

fn reply(module: &str, query: &str, count: usize, hits: usize) -> String {
    let hits: Vec<String> = (0..hits)
        .map(|index| {
            format!(
                r#"{{"name":"Fixture.hit{index}","module":"{module}.Fixture","type":"{}","doc":null}}"#,
                query.replace('\\', "\\\\").replace('"', "\\\"")
            )
        })
        .collect();
    format!(
        r#"{{"count":{count},"header":"Found {count}","heartbeats":3,"hits":[{}]}}"#,
        hits.join(",")
    )
}

fn check_args(args: &[String]) -> Result<(), String> {
    for flag in ["--json", "--interactive"] {
        if !args.iter().any(|arg| arg == flag) {
            return Err(format!("missing {flag}"));
        }
    }
    if value_after(args, "--index-mode") != Some("read") {
        return Err("resident mode must use --index-mode read".to_owned());
    }
    if value_after(args, "--max-results") != Some("200") {
        return Err("resident mode must use --max-results 200".to_owned());
    }
    match value_after(args, "--index-file") {
        Some(path) if Path::new(path).is_file() => Ok(()),
        Some(path) => Err(format!("index file {path} is missing")),
        None => Err("missing --index-file".to_owned()),
    }
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn append(path: &Path, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}
