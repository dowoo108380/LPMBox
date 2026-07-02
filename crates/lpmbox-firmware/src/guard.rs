use crate::patch_plan::{apply_patch_plans, build_patch_plans, validate_patch_results};
use crate::scatter::parse_scatter_xml_text;
use crate::xml_crypto::decrypt_scatter_x;
use lpmbox_core::{app_paths, BlockedFirmwareCheck, FirmwareInfo, LpmError, Result, RomRegion};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const SUPPORTED_MODELS: &[&str] = &[
    "TB375FC", "TB373FU", "TB365FC", "TB361FU", "TB335FC", "TB336FU",
];

const PRC_REGION_TOKENS: &[&str] = &[
    "PRC_OPEN",
    "RC_OPEN",
    "CN_OPEN",
    "N_OPEN",
    "PRC_OPEN_USER",
    "RC_OPEN_USER",
    "CN_OPEN_USER",
    "N_OPEN_USER",
];

const ROW_REGION_TOKENS: &[&str] = &["ROW_OPEN", "OW_OPEN", "ROW_OPEN_USER", "OW_OPEN_USER"];

const BLOCK_FIRMWARE_URLS: &[&str] = &[
    "https://github.com/dwas-KR/LPMBox/blob/7c849df378277555111383e5f9d52189736e494a/block_firmware.ini",
];

pub fn refresh_block_firmware_ini() -> Result<PathBuf> {
    let path = app_paths::block_firmware_ini_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    if path.exists() {
        fs::remove_file(&path).map_err(|err| {
            LpmError::FileNotFound(format!(
                "기존 block_firmware.ini 제거 실패: {} / {err}",
                path.display()
            ))
        })?;
    }

    let mut last_error = String::new();

    for raw_url in BLOCK_FIRMWARE_URLS {
        let Some(url) = normalize_block_firmware_url(raw_url) else {
            continue;
        };

        match download_text_file(&url) {
            Ok(text) => {
                let trimmed = text.trim_start();

                if trimmed.starts_with("<!DOCTYPE")
                    || trimmed.starts_with("<html")
                    || trimmed.contains("<html")
                {
                    last_error = format!("HTML 페이지가 내려왔습니다: {url}");
                    continue;
                }

                let mut file = fs::File::create(&path)?;
                file.write_all(text.as_bytes())?;

                let size = fs::metadata(&path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);

                if size == 0 {
                    last_error = format!("다운로드된 파일 크기가 0입니다: {}", path.display());
                    let _ = fs::remove_file(&path);
                    continue;
                }

                return Ok(path);
            }

            Err(err) => {
                last_error = format!("{url} / {err}");
            }
        }
    }

    Err(LpmError::FileNotFound(format!(
        "block_firmware.ini 다운로드 실패: {last_error}"
    )))
}

fn normalize_block_firmware_url(url: &str) -> Option<String> {
    if url.starts_with("blob:") {
        return None;
    }

    if let Some(rest) = url.strip_prefix("https://github.com/") {
        if rest.contains("/blob/") {
            let mut parts = rest.splitn(4, '/');

            let owner = parts.next()?;
            let repo = parts.next()?;
            let _blob = parts.next()?;
            let commit_and_path = parts.next()?;

            return Some(format!(
                "https://raw.githubusercontent.com/{owner}/{repo}/{commit_and_path}"
            ));
        }
    }

    Some(url.to_string())
}

fn download_text_file(url: &str) -> Result<String> {
    let response = ureq::get(url)
        .set("User-Agent", "Mozilla/5.0")
        .call()
        .map_err(|err| LpmError::FileNotFound(format!("다운로드 실패: {err}")))?;

    if !(200..300).contains(&response.status()) {
        return Err(LpmError::FileNotFound(format!(
            "HTTP 상태 코드 오류: {}",
            response.status()
        )));
    }

    response
        .into_string()
        .map_err(|err| LpmError::FileNotFound(format!("응답 읽기 실패: {err}")))
}

pub fn inspect_firmware(image_dir: &Path) -> Result<FirmwareInfo> {
    if !image_dir.exists() || !image_dir.is_dir() {
        return Err(LpmError::InvalidFirmwareFolder(
            image_dir.display().to_string(),
        ));
    }

    let vendor_boot = image_dir.join("vendor_boot-debug.img");

    if !vendor_boot.exists() {
        return Err(LpmError::FileNotFound(vendor_boot.display().to_string()));
    }

    let data = fs::read(&vendor_boot)?;
    let text = String::from_utf8_lossy(&data);
    let upper = text.to_uppercase();

    let model = extract_model(&upper).ok_or_else(|| {
        LpmError::InvalidFirmwareFolder(
            "vendor_boot-debug.img에서 지원 모델명을 찾지 못했습니다.".to_string(),
        )
    })?;

    let version = extract_version(&upper);
    let region = extract_region(&upper, version.as_deref());
    let blocked_firmware_check =
    check_blocked_firmware(&model, version.as_deref(), &app_paths::block_firmware_ini_path());

    let flash_xml = find_flash_xml(image_dir);

    let platform = flash_xml
        .as_ref()
        .and_then(|path| extract_platform_from_flash_xml(path))
        .or_else(|| infer_platform_from_scatter_file(image_dir));

    let scatter_payload = if let Some(platform) = platform.as_deref() {
        Some(load_scatter_xml_text(image_dir, platform)?)
    } else {
        None
    };

    let (scatter_xml, scatter_xml_info) =
        if let Some((scatter_source, scatter_xml_text)) = scatter_payload {
            let mut info =
                parse_scatter_xml_text(&scatter_xml_text, &scatter_source.display().to_string())?;

            info.patch_plans = build_patch_plans(&model, region, &info);
            info.patched_snapshots = apply_patch_plans(&info.patch_plans, &info.partitions);
            info.patch_validations =
                validate_patch_results(&model, region, &info.patch_plans, &info.patched_snapshots);

            (Some(scatter_source), Some(info))
        } else {
            (None, None)
        };

    Ok(FirmwareInfo {
        image_dir: image_dir.to_path_buf(),
        model,
        version,
        region,
        platform,
        blocked_firmware_check,
        flash_xml,
        scatter_xml,
        scatter_xml_info,
    })
}

fn check_blocked_firmware(
    model: &str,
    version: Option<&str>,
    ini_path: &Path,
) -> BlockedFirmwareCheck {
    let blocked_versions = blocked_versions_for_model(model, ini_path);
    let normalized_version = version.map(normalize_version);

    let matched_version = normalized_version.as_deref().and_then(|current| {
        blocked_versions
            .iter()
            .find(|blocked| normalize_version(blocked) == current)
            .cloned()
    });

    let blocked = matched_version.is_some();

    let source = ini_path.display().to_string();

    let message = if let Some(matched) = &matched_version {
        format!("{model} {matched} 버전은 설치 금지 목록에 포함되어 있습니다.")
    } else if blocked_versions.is_empty() {
        format!("{model} 모델의 설치 금지 버전이 등록되어 있지 않습니다.")
    } else if let Some(version) = version {
        format!("{model} {version} 버전은 설치 금지 목록에 포함되어 있지 않습니다.")
    } else {
        format!(
            "{model} 설치 금지 버전 {}개가 있지만 현재 펌웨어 버전을 감지하지 못했습니다.",
            blocked_versions.len()
        )
    };

    BlockedFirmwareCheck {
        checked: ini_path.is_file(),
        blocked,
        source,
        model: model.to_string(),
        version: version.map(|value| value.to_string()),
        blocked_versions,
        matched_version,
        message,
    }
}

fn blocked_versions_for_model(model: &str, ini_path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(ini_path) else {
        return Vec::new();
    };

    for raw_line in text.lines() {
        let line = strip_inline_comment(raw_line).trim();

        if line.is_empty() {
            continue;
        }

        let pair = line
            .split_once(':')
            .or_else(|| line.split_once('='));

        let Some((left, right)) = pair else {
            continue;
        };

        if !left.trim().eq_ignore_ascii_case(model) {
            continue;
        }

        return right
            .split(',')
            .map(|value| value.trim().trim_matches('"').trim_matches('\''))
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .collect();
    }

    Vec::new()
}

fn strip_inline_comment(line: &str) -> &str {
    let slash_comment = line.find("//");
    let hash_comment = line.find('#');

    let cut_at = match (slash_comment, hash_comment) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };

    if let Some(index) = cut_at {
        &line[..index]
    } else {
        line
    }
}

fn normalize_version(version: &str) -> String {
    version.trim().to_ascii_uppercase()
}

fn extract_model(upper_text: &str) -> Option<String> {
    for model in SUPPORTED_MODELS {
        if upper_text.contains(model) {
            return Some((*model).to_string());
        }
    }

    None
}

fn extract_version(upper_text: &str) -> Option<String> {
    for marker in ["ZUXOS_", "ZUI_"] {
        let mut search_start = 0usize;

        while let Some(relative_start) = upper_text[search_start..].find(marker) {
            let start = search_start + relative_start;
            let rest = &upper_text[start + marker.len()..];

            let Some((number_end, number_part)) = take_version_number_part(rest) else {
                search_start = start + marker.len();
                continue;
            };

            let after_number = &rest[number_end..];

            if !after_number.starts_with('_') {
                search_start = start + marker.len();
                continue;
            }

            let suffix_rest = &after_number[1..];
            let suffix: String = suffix_rest
                .chars()
                .take_while(|ch| ch.is_ascii_uppercase())
                .collect();

            if !suffix.is_empty() {
                return Some(format!("{marker}{number_part}_{suffix}"));
            }

            search_start = start + marker.len();
        }
    }

    None
}

fn take_version_number_part(text: &str) -> Option<(usize, &str)> {
    let mut end = 0usize;
    let mut has_digit = false;
    let mut has_dot = false;

    for (index, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            has_digit = true;
            end = index + ch.len_utf8();
            continue;
        }

        if ch == '.' {
            has_dot = true;
            end = index + ch.len_utf8();
            continue;
        }

        break;
    }

    if !has_digit || !has_dot || end == 0 {
        return None;
    }

    Some((end, &text[..end]))
}

fn extract_region(upper_text: &str, version: Option<&str>) -> RomRegion {
    let has_prc = PRC_REGION_TOKENS
        .iter()
        .any(|token| upper_text.contains(token));

    let has_row = ROW_REGION_TOKENS
        .iter()
        .any(|token| upper_text.contains(token));

    match (has_prc, has_row) {
        (true, false) => RomRegion::Prc,
        (false, true) => RomRegion::Row,
        _ => {
            if let Some(version) = version {
                if version.to_uppercase().starts_with("ZUXOS_") {
                    return RomRegion::Prc;
                }
            }

            RomRegion::Unknown
        }
    }
}

fn find_flash_xml(image_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        image_dir.join("download_agent").join("flash.xml"),
        image_dir.join("flash.xml"),
    ];

    candidates.into_iter().find(|path| path.is_file())
}

fn extract_platform_from_flash_xml(flash_xml: &Path) -> Option<String> {
    let xml = fs::read_to_string(flash_xml).ok()?;
    let upper = xml.to_uppercase();

    extract_platform_from_marker(&upper, "_ANDROID_SCATTER.XML")
        .or_else(|| extract_platform_from_marker(&upper, "_ANDROID_SCATTER.X"))
}

fn extract_platform_from_marker(upper_text: &str, marker: &str) -> Option<String> {
    let marker_pos = upper_text.find(marker)?;
    let before = &upper_text[..marker_pos];
    let mt_pos = before.rfind("MT")?;
    let platform = &before[mt_pos..];

    if platform.starts_with("MT") && platform.len() >= 4 {
        Some(platform.to_string())
    } else {
        None
    }
}

fn infer_platform_from_scatter_file(image_dir: &Path) -> Option<String> {
    let entries = fs::read_dir(image_dir).ok()?;

    let mut candidates: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();

            if !path.is_file() {
                return None;
            }

            let name = path.file_name()?.to_string_lossy().to_uppercase();

            if !name.ends_with("_ANDROID_SCATTER.X") && !name.ends_with("_ANDROID_SCATTER.XML") {
                return None;
            }

            let platform = name
                .split("_ANDROID_SCATTER")
                .next()
                .unwrap_or("")
                .to_string();

            if platform.starts_with("MT") && platform.len() >= 4 {
                Some(platform)
            } else {
                None
            }
        })
        .collect();

    candidates.sort();
    candidates.dedup();

    candidates.into_iter().next()
}

fn load_scatter_xml_text(image_dir: &Path, platform: &str) -> Result<(PathBuf, String)> {
    let xml_path = image_dir.join(format!("{platform}_Android_scatter.xml"));

    if xml_path.is_file() {
        let xml_text = fs::read_to_string(&xml_path)?;
        return Ok((xml_path, xml_text));
    }

    let x_path = image_dir.join(format!("{platform}_Android_scatter.x"));

    if x_path.is_file() {
        let xml_text = decrypt_scatter_x_to_string(&x_path)?;
        return Ok((x_path, xml_text));
    }

    let source = find_any_scatter_source(image_dir).ok_or_else(|| {
        LpmError::FileNotFound(format!(
            "{}_Android_scatter.xml 또는 {}_Android_scatter.x",
            platform, platform
        ))
    })?;

    match source.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("xml") => {
            let xml_text = fs::read_to_string(&source)?;
            Ok((source, xml_text))
        }

        Some(ext) if ext.eq_ignore_ascii_case("x") => {
            let xml_text = decrypt_scatter_x_to_string(&source)?;
            Ok((source, xml_text))
        }

        _ => Err(LpmError::InvalidFirmwareFolder(format!(
            "지원하지 않는 scatter 파일입니다: {}",
            source.display()
        ))),
    }
}

fn decrypt_scatter_x_to_string(x_path: &Path) -> Result<String> {
    let decrypted = decrypt_scatter_x(x_path)?;
    Ok(String::from_utf8_lossy(&decrypted).to_string())
}

fn find_any_scatter_source(image_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(image_dir).ok()?;

    let mut xml_candidates = Vec::new();
    let mut x_candidates = Vec::new();

    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let Some(name) = path.file_name() else {
            continue;
        };

        let name = name.to_string_lossy().to_uppercase();

        if name.ends_with("_ANDROID_SCATTER.XML") {
            xml_candidates.push(path);
        } else if name.ends_with("_ANDROID_SCATTER.X") {
            x_candidates.push(path);
        }
    }

    xml_candidates.sort();
    x_candidates.sort();

    xml_candidates
        .into_iter()
        .next()
        .or_else(|| x_candidates.into_iter().next())
}
