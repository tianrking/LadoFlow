use std::{env, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        add_swift_runtime_search_path();
    }
    tauri_build::build();
}

fn add_swift_runtime_search_path() {
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");

    let Some(developer_dir) = selected_developer_dir() else {
        return;
    };
    let candidates = [
        developer_dir.join("usr/lib/swift/macosx"),
        developer_dir.join("Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx"),
    ];
    if let Some(runtime_dir) = candidates.into_iter().find(|path| path.is_dir()) {
        println!("cargo:rustc-link-search=native={}", runtime_dir.display());
    }
}

fn selected_developer_dir() -> Option<PathBuf> {
    if let Some(configured) = env::var_os("DEVELOPER_DIR").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(configured));
    }

    let Ok(output) = Command::new("xcode-select").arg("-p").output() else {
        return None;
    };
    if !output.status.success() {
        return None;
    }

    let developer_dir = String::from_utf8_lossy(&output.stdout);
    Some(PathBuf::from(developer_dir.trim()))
}
