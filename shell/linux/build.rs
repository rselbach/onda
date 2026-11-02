//! Embed the release version and locate private Linux runtime libraries.

fn main() {
    println!("cargo:rerun-if-env-changed=ONDA_VERSION");
    let version = std::env::var("ONDA_VERSION")
        .unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").unwrap());
    println!("cargo:rustc-env=ONDA_VERSION={version}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bin=onda=-Wl,-rpath,$ORIGIN/../lib/onda");
    }
}
