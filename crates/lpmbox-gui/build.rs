fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os == "windows" {
        embed_windows_resources();
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }
}

fn embed_windows_resources() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    let host = std::env::var("HOST").unwrap_or_default();
    let host_is_windows = host.contains("windows");

    if host_is_windows {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "LPMBox");
        res.set("FileDescription", "LPMBox - Lenovo MediaTek ROM Utility");
        res.set("InternalName", "LPMBox");
        res.set("OriginalFilename", "lpmbox.exe");
        res.compile().expect("Failed to compile Windows resources");
        return;
    }

    use std::io::Write;

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let rc_path = out_dir.join("lpmbox.rc");
    let res_path = out_dir.join("lpmbox.res");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let icon_abs = std::path::Path::new(&manifest_dir)
        .join("assets")
        .join("icon.ico");

    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let parts: Vec<u32> = pkg_version
        .split(['.', '-'])
        .filter_map(|part| part.parse().ok())
        .chain(std::iter::repeat(0))
        .take(4)
        .collect();
    let (v0, v1, v2, v3) = (parts[0], parts[1], parts[2], parts[3]);
    let icon_str = icon_abs.display().to_string().replace('\\', "/");

    let rc_src = format!(
        r#"#pragma code_page(65001)
MAINICON ICON "{icon_str}"

1 VERSIONINFO
FILEVERSION {v0},{v1},{v2},{v3}
PRODUCTVERSION {v0},{v1},{v2},{v3}
FILEOS 0x40004
FILETYPE 0x1
{{
  BLOCK "StringFileInfo"
  {{
    BLOCK "040904b0"
    {{
      VALUE "ProductName", "LPMBox\0"
      VALUE "FileDescription", "LPMBox - Lenovo MediaTek ROM Utility\0"
      VALUE "InternalName", "LPMBox\0"
      VALUE "OriginalFilename", "lpmbox.exe\0"
      VALUE "FileVersion", "{pkg_version}\0"
      VALUE "ProductVersion", "{pkg_version}\0"
    }}
  }}
  BLOCK "VarFileInfo"
  {{
    VALUE "Translation", 0x0409, 0x04B0
  }}
}}
"#,
    );

    let mut file = std::fs::File::create(&rc_path).expect("create lpmbox.rc");
    file.write_all(rc_src.as_bytes()).expect("write lpmbox.rc");
    drop(file);

    let llvm_rc = std::env::var("LLVM_RC").unwrap_or_else(|_| "llvm-rc".into());
    let status = std::process::Command::new(&llvm_rc)
        .arg(format!("/fo{}", res_path.display()))
        .arg(&rc_path)
        .status()
        .unwrap_or_else(|err| panic!("spawn {llvm_rc}: {err}"));

    assert!(status.success(), "{llvm_rc} failed on {}", rc_path.display());

    println!("cargo:rustc-link-arg-bin=lpmbox={}", res_path.display());
}
