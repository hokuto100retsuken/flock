// サイドバーのプラグインを WASM にビルドし、バイナリに埋め込めるよう OUT_DIR に置く
use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    // 外側のビルドと target を分けないと、同じ target のロックを待って止まる
    let target_dir = out.join("plugin-target");
    let status = Command::new(env::var("CARGO").unwrap())
        .args(["build", "--release", "--target", "wasm32-wasip1", "--manifest-path"])
        .arg(root.join("plugin/Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target_dir)
        // 外側のビルド向けのフラグを WASM のビルドに持ち込まない
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .status()
        .expect("cannot run cargo");
    assert!(status.success(), "failed to build the plugin");
    fs::copy(
        target_dir.join("wasm32-wasip1/release/flock-plugin.wasm"),
        out.join("flock.wasm"),
    )
    .unwrap();
    println!("cargo:rerun-if-changed=../plugin/src");
    println!("cargo:rerun-if-changed=../plugin/Cargo.toml");
}
