fn main() {
    let schema = schemars::schema_for!(mathscape_domain::InkLabDocument);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
