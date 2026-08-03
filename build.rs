use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=TAPAS_BUILD_LABEL");
    let label = env::var("TAPAS_BUILD_LABEL").unwrap_or_else(|_| {
        env::var("CARGO_PKG_VERSION").expect("Cargo always provides CARGO_PKG_VERSION")
    });
    assert!(
        !label.contains(['\n', '\r']),
        "TAPAS_BUILD_LABEL must be one line"
    );
    println!("cargo:rustc-env=TAPAS_BUILD_LABEL={label}");
}
