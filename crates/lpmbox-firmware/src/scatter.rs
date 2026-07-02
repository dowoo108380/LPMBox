use lpmbox_core::{LpmError, RequiredPartitionCheck, Result, ScatterPartition, ScatterXmlInfo};
use std::fs;
use std::path::Path;

pub fn parse_scatter_xml(scatter_xml: &Path) -> Result<ScatterXmlInfo> {
    if !scatter_xml.is_file() {
        return Err(LpmError::FileNotFound(scatter_xml.display().to_string()));
    }

    let xml = fs::read_to_string(scatter_xml)?;
    parse_scatter_xml_text(&xml, &scatter_xml.display().to_string())
}

pub fn parse_scatter_xml_text(xml: &str, source_label: &str) -> Result<ScatterXmlInfo> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|err| LpmError::XmlParseFailed(format!("{source_label} / {err}")))?;

    let root = document.root_element();
    let root_name = root.tag_name().name().to_string();

    if root_name.trim().is_empty() {
        return Err(LpmError::XmlParseFailed(
            "XML root element 이름이 비어 있습니다.".to_string(),
        ));
    }

    let partitions = extract_partitions(&document);
    let partition_count = partitions.len();

    if partition_count == 0 {
        return Err(LpmError::XmlParseFailed(
            "scatter XML에서 partition_name 항목을 찾지 못했습니다.".to_string(),
        ));
    }

    let partition_names: Vec<String> = partitions
        .iter()
        .map(|partition| partition.name.clone())
        .collect();

    let required_check = check_required_partitions(&partition_names);

    Ok(ScatterXmlInfo {
        root_name,
        xml_size: xml.len(),
        partition_count,
        partition_names,
        partitions,
        required_check,
        patch_plans: Vec::new(),
        patched_snapshots: Vec::new(),
        patch_validations: Vec::new(),
    })
}

fn extract_partitions(document: &roxmltree::Document<'_>) -> Vec<ScatterPartition> {
    let mut partitions = Vec::new();

    for node in document.descendants() {
        if !node.is_element() {
            continue;
        }

        if !node
            .tag_name()
            .name()
            .eq_ignore_ascii_case("partition_name")
        {
            continue;
        }

        let Some(name_text) = node.text() else {
            continue;
        };

        let name = name_text.trim();

        if name.is_empty() {
            continue;
        }

        if partitions
            .iter()
            .any(|partition: &ScatterPartition| partition.name.eq_ignore_ascii_case(name))
        {
            continue;
        }

        let Some(parent) = node.parent() else {
            continue;
        };

        partitions.push(ScatterPartition {
            name: name.to_string(),
            partition_index: child_text(parent, "partition_index"),
            file_name: child_text(parent, "file_name"),
            is_download: child_text(parent, "is_download"),
            is_upgradable: child_text(parent, "is_upgradable"),
            partition_type: child_text(parent, "type"),
            storage: child_text(parent, "storage"),
            linear_start_addr: child_text(parent, "linear_start_addr"),
            physical_start_addr: child_text(parent, "physical_start_addr"),
            partition_size: child_text(parent, "partition_size"),
        });
    }

    partitions
}

fn child_text(parent: roxmltree::Node<'_, '_>, tag: &str) -> Option<String> {
    for child in parent.children() {
        if !child.is_element() {
            continue;
        }

        if !child.tag_name().name().eq_ignore_ascii_case(tag) {
            continue;
        }

        let value = child.text().unwrap_or("").trim();

        if value.is_empty() {
            return None;
        }

        return Some(value.to_string());
    }

    None
}

fn check_required_partitions(partition_names: &[String]) -> RequiredPartitionCheck {
    let has_proinfo = has_partition(partition_names, "proinfo");
    let has_userdata = has_partition(partition_names, "userdata");
    let has_super = has_partition(partition_names, "super");

    let has_boot_ab = has_partition_pair(partition_names, "boot");
    let has_vendor_boot_ab = has_partition_pair(partition_names, "vendor_boot");
    let has_init_boot_ab = has_partition_pair(partition_names, "init_boot");

    let has_vbmeta_ab = has_partition_pair(partition_names, "vbmeta");
    let has_vbmeta_system_ab = has_partition_pair(partition_names, "vbmeta_system");
    let has_vbmeta_vendor_ab = has_partition_pair(partition_names, "vbmeta_vendor");

    let has_lk_ab = has_partition_pair(partition_names, "lk");
    let has_dtbo_ab = has_partition_pair(partition_names, "dtbo");

    let mut missing_partitions = Vec::new();

    push_missing_single(&mut missing_partitions, partition_names, "proinfo");
    push_missing_single(&mut missing_partitions, partition_names, "userdata");
    push_missing_single(&mut missing_partitions, partition_names, "super");

    push_missing_pair(&mut missing_partitions, partition_names, "boot");
    push_missing_pair(&mut missing_partitions, partition_names, "vendor_boot");
    push_missing_pair(&mut missing_partitions, partition_names, "init_boot");

    push_missing_pair(&mut missing_partitions, partition_names, "vbmeta");
    push_missing_pair(&mut missing_partitions, partition_names, "vbmeta_system");
    push_missing_pair(&mut missing_partitions, partition_names, "vbmeta_vendor");

    push_missing_pair(&mut missing_partitions, partition_names, "lk");
    push_missing_pair(&mut missing_partitions, partition_names, "dtbo");

    let all_required_ok = missing_partitions.is_empty();

    RequiredPartitionCheck {
        has_proinfo,
        has_userdata,
        has_super,
        has_boot_ab,
        has_vendor_boot_ab,
        has_init_boot_ab,
        has_vbmeta_ab,
        has_vbmeta_system_ab,
        has_vbmeta_vendor_ab,
        has_lk_ab,
        has_dtbo_ab,
        all_required_ok,
        missing_partitions,
    }
}

fn has_partition(partition_names: &[String], target: &str) -> bool {
    partition_names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(target))
}

fn has_partition_pair(partition_names: &[String], base: &str) -> bool {
    has_partition(partition_names, &format!("{base}_a"))
        && has_partition(partition_names, &format!("{base}_b"))
}

fn push_missing_single(missing: &mut Vec<String>, partition_names: &[String], target: &str) {
    if !has_partition(partition_names, target) {
        missing.push(target.to_string());
    }
}

fn push_missing_pair(missing: &mut Vec<String>, partition_names: &[String], base: &str) {
    let a = format!("{base}_a");
    let b = format!("{base}_b");

    if !has_partition(partition_names, &a) {
        missing.push(a);
    }

    if !has_partition(partition_names, &b) {
        missing.push(b);
    }
}
