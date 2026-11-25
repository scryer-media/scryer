// disk-guard: test, example and build-script code may call the filesystem directly.
#![allow(clippy::disallowed_methods)]

fn main() {
    let schema = scryer_plugin_sdk::plugin_sdk_schema_json();
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(path, schema).expect("write plugin SDK schema");
    } else {
        print!("{schema}");
    }
}
