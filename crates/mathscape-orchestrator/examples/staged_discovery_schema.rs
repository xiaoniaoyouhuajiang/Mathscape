fn main() {
    let schema = match std::env::args().nth(1).as_deref() {
        Some("units") => mathscape_orchestrator::unit_output_schema(),
        Some("facts") => mathscape_orchestrator::fact_output_schema(),
        Some("repair") => mathscape_orchestrator::repair_output_schema(),
        Some("markdown-patch") => mathscape_orchestrator::markdown_patch_output_schema(),
        _ => {
            eprintln!("usage: staged_discovery_schema units|facts|repair|markdown-patch");
            std::process::exit(2);
        }
    };
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
