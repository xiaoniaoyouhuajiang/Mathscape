use std::io;
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let mut mode = "normal".to_owned();
    let mut log = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" => {
                println!("opencode 0.0.0-mathscape-fixture");
                return Ok(());
            }
            "--mode" => mode = args.next().unwrap_or_default(),
            "--log" => log = args.next().map(PathBuf::from),
            _ => {}
        }
    }
    mathscape_agent::serve_acp_fixture(
        io::stdin().lock(),
        io::stdout().lock(),
        &mode,
        log.as_deref(),
    )
}
