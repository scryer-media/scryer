// Nothing is generated here. The script exists so Cargo sets `OUT_DIR`,
// which the unit tests use as the root of their shared Wasmtime cache.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
