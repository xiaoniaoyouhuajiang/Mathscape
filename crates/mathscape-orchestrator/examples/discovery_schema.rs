fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&mathscape_orchestrator::discovery_output_schema()).unwrap()
    );
}
