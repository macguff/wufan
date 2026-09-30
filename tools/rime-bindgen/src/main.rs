fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bindings = bindgen::Builder::default()
        .header(root.join("crates/rime-sys/wrapper.h").to_string_lossy())
        .clang_arg("--target=x86_64-pc-windows-msvc")
        .allowlist_type("Rime.*")
        .allowlist_function("rime_get_api")
        .derive_default(true)
        .generate_comments(false)
        .generate()
        .expect("generate librime 1.17.0 bindings with LLVM 19.1.7");
    bindings
        .write_to_file(root.join("crates/rime-sys/bindings.rs"))
        .expect("save bindings");
}
