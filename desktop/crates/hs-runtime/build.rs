fn main() {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD:crates/hs-core"])
        .output();
    let revision = output
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "source-archive".into());
    println!("cargo:rustc-env=HS_CORE_REVISION={}", revision.trim());
    println!("cargo:rerun-if-changed=../../../crates/hs-core/src");
}
