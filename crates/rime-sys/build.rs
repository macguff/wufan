fn main() {
    // Release and CI consume checked-in bindings. Generation is an explicit xtask.
    println!("cargo:rerun-if-changed=bindings.rs");
    println!("cargo:rerun-if-changed=vendor/rime_api.h");
}
