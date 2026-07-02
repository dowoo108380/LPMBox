use std::path::PathBuf;

pub fn runtime_root() -> PathBuf {
    if let Ok(current_dir) = std::env::current_dir() {
        if current_dir.join("Cargo.toml").is_file() && current_dir.join("crates").is_dir() {
            return current_dir;
        }
    }

    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(std::env::temp_dir)
}

pub fn config_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(runtime_root)
        .join("LPMBox")
}

pub fn stable_adb_key_dir() -> PathBuf {
    config_root().join("adb")
}

pub fn stable_adb_key_path() -> PathBuf {
    stable_adb_key_dir().join("adbkey")
}

pub fn runtime_adb_key_path() -> PathBuf {
    runtime_root().join("adb").join("adbkey")
}

pub fn ensure_runtime_directories() -> std::io::Result<()> {
    std::fs::create_dir_all(tool_dir())?;
    std::fs::create_dir_all(tool_download_dir())?;
    std::fs::create_dir_all(backup_root())?;
    std::fs::create_dir_all(log_dir())?;
    std::fs::create_dir_all(stable_adb_key_dir())?;

    Ok(())
}

pub fn tool_dir() -> PathBuf {
    runtime_root().join("tools")
}

pub fn tool_download_dir() -> PathBuf {
    tool_dir().join("download")
}

pub fn block_firmware_ini_path() -> PathBuf {
    tool_dir().join("block_firmware.ini")
}

pub fn spflashtoolv6_dir() -> PathBuf {
    tool_dir().join("SPFlashToolV6")
}

pub fn spflashtoolv6_zip_path() -> PathBuf {
    tool_download_dir().join("SP_Flash_Tool_V6.2404_Win.zip")
}

pub fn mtk_driver_dir() -> PathBuf {
    tool_dir().join("MTK-Driver")
}

pub fn mtk_driver_zip_path() -> PathBuf {
    tool_download_dir().join("MTK-Driver-v5.2307.zip")
}

pub fn log_dir() -> PathBuf {
    runtime_root().join("logs")
}

pub fn work_root() -> PathBuf {
    runtime_root().join("work")
}

pub fn convert_wipe_work_dir() -> PathBuf {
    work_root().join("ConvertWipe")
}

pub fn backup_root() -> PathBuf {
    runtime_root().join("backup")
}

pub fn spft_log_dir() -> PathBuf {
    backup_root().join("spft_log")
}

pub fn proinfo_backup_path() -> PathBuf {
    backup_root().join("proinfo")
}

pub fn readback_config_xml_path() -> PathBuf {
    runtime_root().join("readback_config.xml")
}

pub fn adb_key_path() -> PathBuf {
    let stable_path = stable_adb_key_path();

    if stable_path.is_file() {
        return stable_path;
    }

    let legacy_path = runtime_adb_key_path();

    if legacy_path.is_file() {
        if let Some(parent) = stable_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if std::fs::copy(&legacy_path, &stable_path).is_ok() {
            let legacy_public_key = legacy_path.with_extension("pub");
            let stable_public_key = stable_path.with_extension("pub");

            if legacy_public_key.is_file() {
                let _ = std::fs::copy(legacy_public_key, stable_public_key);
            }

            return stable_path;
        }

        return legacy_path;
    }

    stable_path
}
