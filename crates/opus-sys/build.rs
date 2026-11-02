use std::path::PathBuf;

fn main() {
    // 预编译的静态库随仓库分发（lib/opus.lib），链接指令只有这两行——
    // 没有 cmake，也没有编译步骤
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("lib");
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=opus");
    println!("cargo:rerun-if-changed=lib/opus.lib");
}
