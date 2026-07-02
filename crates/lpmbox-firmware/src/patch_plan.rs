use std::collections::HashMap;

use lpmbox_core::{
    PartitionPatchAction, PatchPlan, PatchPlanMode, PatchValidationResult,
    PatchedPartitionSnapshot, RomRegion, ScatterPartition, ScatterXmlInfo,
};

pub fn build_patch_plans(
    model: &str,
    image_region: RomRegion,
    scatter_info: &ScatterXmlInfo,
) -> Vec<PatchPlan> {
    vec![
        build_convert_wipe_plan(model, scatter_info),
        build_row_update_keep_data_plan(model, image_region, scatter_info),
        build_reinstall_wipe_plan(model, scatter_info),
        build_country_reset_plan(scatter_info),
    ]
}

pub fn apply_patch_plans(
    plans: &[PatchPlan],
    partitions: &[ScatterPartition],
) -> Vec<PatchedPartitionSnapshot> {
    plans
        .iter()
        .map(|plan| apply_patch_plan(plan, partitions))
        .collect()
}

pub fn validate_patch_results(
    model: &str,
    image_region: RomRegion,
    plans: &[PatchPlan],
    snapshots: &[PatchedPartitionSnapshot],
) -> Vec<PatchValidationResult> {
    snapshots
        .iter()
        .map(|snapshot| {
            let plan_warnings = plans
                .iter()
                .find(|plan| plan.mode == snapshot.mode)
                .map(|plan| plan.warnings.clone())
                .unwrap_or_default();

            validate_patch_snapshot(model, image_region, snapshot, plan_warnings)
        })
        .collect()
}

fn append_lk_dtbo_actions(model: &str, actions: &mut Vec<PartitionPatchAction>) {
    for partition in ["lk_a", "lk_b", "dtbo_a", "dtbo_b"] {
        let enabled = lk_dtbo_partition_enabled(model, partition);
        let target_value = if enabled { "true" } else { "false" };
        let reason = lk_dtbo_partition_reason(model, enabled);

        actions.push(action(
            partition,
            Some(partition),
            Some(target_value),
            Some(target_value),
            reason,
        ));
    }
}

fn lk_dtbo_partition_enabled(model: &str, partition: &str) -> bool {
    match model {
        "TB375FC" | "TB373FU" => matches!(partition, "lk_a" | "lk_b" | "dtbo_a" | "dtbo_b"),
        "TB365FC" | "TB361FU" | "TB335FC" | "TB336FU" => {
            matches!(partition, "lk_a" | "dtbo_a")
        }
        _ => false,
    }
}

fn lk_dtbo_partition_reason(model: &str, enabled: bool) -> &'static str {
    match model {
        "TB375FC" | "TB373FU" => {
            "LPMBox v2.1.5 기준: TB375FC/TB373FU는 lk/dtbo 4개 partition을 모두 활성화합니다."
        }
        "TB365FC" | "TB361FU" | "TB335FC" | "TB336FU" if enabled => {
            "LPMBox v2.1.5 기준: 이 모델은 lk_a/dtbo_a만 활성화합니다."
        }
        "TB365FC" | "TB361FU" | "TB335FC" | "TB336FU" => {
            "LPMBox v2.1.5 기준: 이 모델은 lk_b/dtbo_b를 플래싱하지 않도록 비활성화합니다."
        }
        _ => {
            "LPMBox v2.1.5 기준: 이 모델은 lk/dtbo 외부 파일 활성화 대상이 아니므로 비활성화합니다."
        }
    }
}

fn apply_patch_plan(plan: &PatchPlan, partitions: &[ScatterPartition]) -> PatchedPartitionSnapshot {
    if !plan.available {
        return PatchedPartitionSnapshot {
            mode: plan.mode,
            title: plan.title.clone(),
            available: false,
            changed_count: 0,
            partitions: partitions.to_vec(),
            summary: vec!["사용 불가 plan이므로 patch를 적용하지 않았습니다.".to_string()],
        };
    }

    let mut patched = partitions.to_vec();
    let mut changed_count = 0usize;
    let mut summary = Vec::new();

    for action in &plan.actions {
        if action.partition == "*" {
            let changed = apply_to_all_partitions(&mut patched, action);
            changed_count += changed;

            summary.push(format!(
                "{}: 전체 partition 대상 / 변경 {}개",
                action.reason, changed
            ));

            continue;
        }

        if action.partition == "file_name=NONE partitions" {
            let changed = apply_to_none_file_partitions(&mut patched, action);
            changed_count += changed;

            summary.push(format!("{}: 변경 {}개", action.reason, changed));

            continue;
        }

        let changed = apply_to_named_partition(&mut patched, action);
        changed_count += changed;

        summary.push(format!(
            "{}: {} / 변경 {}개",
            action.partition, action.reason, changed
        ));
    }

    PatchedPartitionSnapshot {
        mode: plan.mode,
        title: plan.title.clone(),
        available: true,
        changed_count,
        partitions: patched,
        summary,
    }
}

fn apply_to_named_partition(
    partitions: &mut [ScatterPartition],
    action: &PartitionPatchAction,
) -> usize {
    let Some(partition) = partitions
        .iter_mut()
        .find(|partition| partition.name.eq_ignore_ascii_case(&action.partition))
    else {
        return 0;
    };

    apply_action_to_partition(partition, action)
}

fn apply_to_all_partitions(
    partitions: &mut [ScatterPartition],
    action: &PartitionPatchAction,
) -> usize {
    let mut changed = 0usize;

    for partition in partitions {
        changed += apply_action_to_partition(partition, action);
    }

    changed
}

fn apply_to_none_file_partitions(
    partitions: &mut [ScatterPartition],
    action: &PartitionPatchAction,
) -> usize {
    let mut changed = 0usize;

    for partition in partitions {
        let is_none_file = partition
            .file_name
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case("NONE"))
            .unwrap_or(false);

        if is_none_file {
            changed += apply_action_to_partition(partition, action);
        }
    }

    changed
}

fn apply_action_to_partition(
    partition: &mut ScatterPartition,
    action: &PartitionPatchAction,
) -> usize {
    let mut changed = 0usize;

    if let Some(target) = &action.target_file_name {
        if set_option_value(&mut partition.file_name, target) {
            changed += 1;
        }
    }

    if let Some(target) = &action.target_is_download {
        if set_option_value(&mut partition.is_download, target) {
            changed += 1;
        }
    }

    if let Some(target) = &action.target_is_upgradable {
        if set_option_value(&mut partition.is_upgradable, target) {
            changed += 1;
        }
    }

    changed
}

fn set_option_value(field: &mut Option<String>, target: &str) -> bool {
    let current = field.as_deref().unwrap_or("");

    if current.eq_ignore_ascii_case(target) {
        return false;
    }

    *field = Some(target.to_string());
    true
}

fn validate_patch_snapshot(
    model: &str,
    image_region: RomRegion,
    snapshot: &PatchedPartitionSnapshot,
    mut warnings: Vec<String>,
) -> PatchValidationResult {
    let mut errors = Vec::new();

    match snapshot.mode {
        PatchPlanMode::ConvertWipe => {
            validate_available(snapshot, &mut errors);

            if snapshot.available {
                validate_wipe_like_snapshot(model, snapshot, &mut errors);
            }
        }

        PatchPlanMode::RowUpdateKeepData => match image_region {
            RomRegion::Row => {
                validate_available(snapshot, &mut errors);

                if snapshot.available {
                    validate_row_update_snapshot(model, snapshot, &mut errors);
                }
            }

            RomRegion::Prc | RomRegion::Unknown => {
                if snapshot.available {
                    errors.push(
                        "ROW 업데이트 [데이터 유지]는 ROW 이미지에서만 available=true가 되어야 합니다."
                            .to_string(),
                    );

                    validate_row_update_snapshot(model, snapshot, &mut errors);
                } else {
                    warnings.push(
                        "ROW 이미지가 아니므로 ROW 업데이트 [데이터 유지] 미적용 상태가 정상입니다."
                            .to_string(),
                    );
                }
            }
        },

        PatchPlanMode::ReinstallWipe => {
            validate_available(snapshot, &mut errors);

            if snapshot.available {
                validate_wipe_like_snapshot(model, snapshot, &mut errors);
            }

            if !warnings
                .iter()
                .any(|warning| warning.contains("current slot stage"))
            {
                warnings.push(
                    "재설치 모드는 current slot stage 없이 PreLoader/SPFlashToolV6 단계로 진행해야 합니다."
                        .to_string(),
                );
            }
        }

        PatchPlanMode::CountryReset => {
            validate_available(snapshot, &mut errors);

            if snapshot.available {
                validate_country_reset_snapshot(snapshot, &mut errors);
            }
        }
    }

    PatchValidationResult {
        mode: snapshot.mode,
        title: snapshot.title.clone(),
        passed: errors.is_empty(),
        errors,
        warnings,
    }
}

fn validate_available(snapshot: &PatchedPartitionSnapshot, errors: &mut Vec<String>) {
    if !snapshot.available {
        errors.push(format!(
            "{} plan이 사용 가능해야 하지만 미적용 상태입니다.",
            snapshot.title
        ));
    }
}

fn validate_wipe_like_snapshot(
    model: &str,
    snapshot: &PatchedPartitionSnapshot,
    errors: &mut Vec<String>,
) {
    validate_partition_values(
        &snapshot.partitions,
        "proinfo",
        Some("NONE"),
        Some("false"),
        Some("false"),
        errors,
    );

    validate_partition_values(
        &snapshot.partitions,
        "userdata",
        Some("userdata.img"),
        Some("true"),
        Some("true"),
        errors,
    );

    validate_lk_dtbo_policy(model, snapshot, errors);
    validate_none_file_partition_rule(snapshot, errors);
}

fn validate_row_update_snapshot(
    model: &str,
    snapshot: &PatchedPartitionSnapshot,
    errors: &mut Vec<String>,
) {
    validate_partition_values(
        &snapshot.partitions,
        "proinfo",
        Some("NONE"),
        Some("false"),
        Some("false"),
        errors,
    );

    validate_partition_values(
        &snapshot.partitions,
        "userdata",
        Some("userdata.img"),
        Some("false"),
        Some("false"),
        errors,
    );

    validate_lk_dtbo_policy(model, snapshot, errors);
    validate_none_file_partition_rule(snapshot, errors);
}

fn validate_lk_dtbo_policy(
    model: &str,
    snapshot: &PatchedPartitionSnapshot,
    errors: &mut Vec<String>,
) {
    for partition in ["lk_a", "lk_b", "dtbo_a", "dtbo_b"] {
        let enabled = lk_dtbo_partition_enabled(model, partition);
        let target_value = if enabled { "true" } else { "false" };

        validate_partition_values(
            &snapshot.partitions,
            partition,
            Some(partition),
            Some(target_value),
            Some(target_value),
            errors,
        );
    }
}

fn validate_none_file_partition_rule(
    snapshot: &PatchedPartitionSnapshot,
    errors: &mut Vec<String>,
) {
    for partition in snapshot.partitions.iter().filter(|partition| {
        partition
            .file_name
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case("NONE"))
            .unwrap_or(false)
    }) {
        if !option_eq(partition.is_download.as_deref(), "false") {
            errors.push(format!(
                "{}: file_name=NONE partition은 is_download=false여야 합니다. 현재값={}",
                partition.name,
                option_label(&partition.is_download)
            ));
        }

        if !option_eq(partition.is_upgradable.as_deref(), "false") {
            errors.push(format!(
                "{}: file_name=NONE partition은 is_upgradable=false여야 합니다. 현재값={}",
                partition.name,
                option_label(&partition.is_upgradable)
            ));
        }
    }
}

fn validate_country_reset_snapshot(snapshot: &PatchedPartitionSnapshot, errors: &mut Vec<String>) {
    validate_partition_values(
        &snapshot.partitions,
        "proinfo",
        Some("proinfo"),
        Some("true"),
        Some("true"),
        errors,
    );

    let download_true_partitions = true_partition_names(&snapshot.partitions, FieldKind::Download);
    let upgradable_true_partitions =
        true_partition_names(&snapshot.partitions, FieldKind::Upgradable);

    if !is_only_proinfo(&download_true_partitions) {
        errors.push(format!(
            "국가 코드 재설정: is_download=true partition은 proinfo 하나뿐이어야 합니다. 현재={}",
            list_or_dash(&download_true_partitions)
        ));
    }

    if !is_only_proinfo(&upgradable_true_partitions) {
        errors.push(format!(
            "국가 코드 재설정: is_upgradable=true partition은 proinfo 하나뿐이어야 합니다. 현재={}",
            list_or_dash(&upgradable_true_partitions)
        ));
    }

    for partition in snapshot
        .partitions
        .iter()
        .filter(|partition| !partition.name.eq_ignore_ascii_case("proinfo"))
    {
        if !option_eq(partition.is_download.as_deref(), "false") {
            errors.push(format!(
                "{}: 국가 코드 재설정에서는 proinfo 외 partition의 is_download=false여야 합니다. 현재값={}",
                partition.name,
                option_label(&partition.is_download)
            ));
        }

        if !option_eq(partition.is_upgradable.as_deref(), "false") {
            errors.push(format!(
                "{}: 국가 코드 재설정에서는 proinfo 외 partition의 is_upgradable=false여야 합니다. 현재값={}",
                partition.name,
                option_label(&partition.is_upgradable)
            ));
        }
    }
}

fn validate_partition_values(
    partitions: &[ScatterPartition],
    partition_name: &str,
    expected_file_name: Option<&str>,
    expected_is_download: Option<&str>,
    expected_is_upgradable: Option<&str>,
    errors: &mut Vec<String>,
) {
    let Some(partition) = find_partition(partitions, partition_name) else {
        errors.push(format!("{partition_name}: partition을 찾지 못했습니다."));
        return;
    };

    if let Some(expected) = expected_file_name {
        if !option_eq(partition.file_name.as_deref(), expected) {
            errors.push(format!(
                "{}: file_name={}이어야 합니다. 현재값={}",
                partition.name,
                expected,
                option_label(&partition.file_name)
            ));
        }
    }

    if let Some(expected) = expected_is_download {
        if !option_eq(partition.is_download.as_deref(), expected) {
            errors.push(format!(
                "{}: is_download={}이어야 합니다. 현재값={}",
                partition.name,
                expected,
                option_label(&partition.is_download)
            ));
        }
    }

    if let Some(expected) = expected_is_upgradable {
        if !option_eq(partition.is_upgradable.as_deref(), expected) {
            errors.push(format!(
                "{}: is_upgradable={}이어야 합니다. 현재값={}",
                partition.name,
                expected,
                option_label(&partition.is_upgradable)
            ));
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum FieldKind {
    Download,
    Upgradable,
}

fn true_partition_names(partitions: &[ScatterPartition], field_kind: FieldKind) -> Vec<String> {
    partitions
        .iter()
        .filter(|partition| {
            let value = match field_kind {
                FieldKind::Download => partition.is_download.as_deref(),
                FieldKind::Upgradable => partition.is_upgradable.as_deref(),
            };

            option_eq(value, "true")
        })
        .map(|partition| partition.name.clone())
        .collect()
}

fn option_eq(value: Option<&str>, expected: &str) -> bool {
    value
        .map(|value| value.trim().eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn is_only_proinfo(partition_names: &[String]) -> bool {
    partition_names.len() == 1 && partition_names[0].eq_ignore_ascii_case("proinfo")
}

fn list_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

fn option_label(value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| "<없음>".to_string())
}

fn find_partition<'a>(
    partitions: &'a [ScatterPartition],
    partition_name: &str,
) -> Option<&'a ScatterPartition> {
    partitions
        .iter()
        .find(|partition| partition.name.eq_ignore_ascii_case(partition_name))
}

fn build_convert_wipe_plan(model: &str, scatter_info: &ScatterXmlInfo) -> PatchPlan {
    let mut warnings = Vec::new();
    let mut actions = Vec::new();

    if !scatter_info.required_check.all_required_ok {
        warnings.push(format!(
            "필수 partition 누락: {}",
            scatter_info.required_check.missing_partitions.join(", ")
        ));
    }

    actions.push(action(
        "proinfo",
        Some("NONE"),
        Some("false"),
        Some("false"),
        "일반 설치 기본값: 국가 코드 변경을 선택하지 않으면 proinfo는 비활성화합니다.",
    ));

    actions.push(action(
        "userdata",
        Some("userdata.img"),
        Some("true"),
        Some("true"),
        "일반 설치 [데이터 초기화]: userdata를 다운로드 대상으로 유지합니다.",
    ));

    append_ab_slot_mirror_actions(scatter_info, &mut actions);
    append_lk_dtbo_actions(model, &mut actions);
    append_none_file_partition_rule(scatter_info, &mut actions);

    PatchPlan {
        mode: PatchPlanMode::ConvertWipe,
        title: "일반 설치 [데이터 초기화]".to_string(),
        available: scatter_info.required_check.all_required_ok,
        warnings,
        actions,
    }
}

fn build_row_update_keep_data_plan(
    model: &str,
    image_region: RomRegion,
    scatter_info: &ScatterXmlInfo,
) -> PatchPlan {
    let mut warnings = Vec::new();
    let mut actions = Vec::new();

    if !scatter_info.required_check.all_required_ok {
        warnings.push(format!(
            "필수 partition 누락: {}",
            scatter_info.required_check.missing_partitions.join(", ")
        ));
    }

    if image_region != RomRegion::Row {
        warnings.push(
            "ROW 업데이트 [데이터 유지]는 ROW(글로벌롬) 이미지에서만 허용됩니다.".to_string(),
        );
    }

    actions.push(action(
        "proinfo",
        Some("NONE"),
        Some("false"),
        Some("false"),
        "ROW 업데이트 기본값: 국가 코드 변경을 선택하지 않으면 proinfo는 비활성화합니다.",
    ));

    actions.push(action(
        "userdata",
        Some("userdata.img"),
        Some("false"),
        Some("false"),
        "ROW 업데이트 [데이터 유지]: userdata를 반드시 비활성화하여 데이터를 유지합니다.",
    ));

    append_ab_slot_mirror_actions(scatter_info, &mut actions);
    append_lk_dtbo_actions(model, &mut actions);
    append_none_file_partition_rule(scatter_info, &mut actions);

    PatchPlan {
        mode: PatchPlanMode::RowUpdateKeepData,
        title: "ROW 업데이트 [데이터 유지]".to_string(),
        available: scatter_info.required_check.all_required_ok && image_region == RomRegion::Row,
        warnings,
        actions,
    }
}

fn build_reinstall_wipe_plan(model: &str, scatter_info: &ScatterXmlInfo) -> PatchPlan {
    let mut warnings = Vec::new();
    let mut actions = Vec::new();

    if !scatter_info.required_check.all_required_ok {
        warnings.push(format!(
            "필수 partition 누락: {}",
            scatter_info.required_check.missing_partitions.join(", ")
        ));
    }

    actions.push(action(
        "proinfo",
        Some("NONE"),
        Some("false"),
        Some("false"),
        "재설치 기본값: 국가 코드 변경을 선택하지 않으면 proinfo는 비활성화합니다.",
    ));

    actions.push(action(
        "userdata",
        Some("userdata.img"),
        Some("true"),
        Some("true"),
        "재설치 [데이터 초기화]: userdata를 다운로드 대상으로 유지합니다.",
    ));

    append_ab_slot_mirror_actions(scatter_info, &mut actions);
    append_lk_dtbo_actions(model, &mut actions);
    append_none_file_partition_rule(scatter_info, &mut actions);

    warnings.push(
        "재설치 모드는 current slot stage 없이 PreLoader/SPFlashToolV6 단계로 진행합니다."
            .to_string(),
    );

    PatchPlan {
        mode: PatchPlanMode::ReinstallWipe,
        title: "재설치 [데이터 초기화]".to_string(),
        available: scatter_info.required_check.all_required_ok,
        warnings,
        actions,
    }
}

fn build_country_reset_plan(scatter_info: &ScatterXmlInfo) -> PatchPlan {
    let mut warnings = Vec::new();
    let mut actions = Vec::new();

    if !scatter_info.required_check.has_proinfo {
        warnings.push("국가 코드 재설정에는 proinfo partition이 필요합니다.".to_string());
    }

    actions.push(action(
        "*",
        None,
        Some("false"),
        Some("false"),
        "국가 코드 재설정: 모든 partition을 비활성화합니다.",
    ));

    actions.push(action(
        "proinfo",
        Some("proinfo"),
        Some("true"),
        Some("true"),
        "국가 코드 재설정: proinfo만 다운로드 대상으로 활성화합니다.",
    ));

    PatchPlan {
        mode: PatchPlanMode::CountryReset,
        title: "국가 코드 재설정 [proinfo only]".to_string(),
        available: scatter_info.required_check.has_proinfo,
        warnings,
        actions,
    }
}

fn append_ab_slot_mirror_actions(
    scatter_info: &ScatterXmlInfo,
    actions: &mut Vec<PartitionPatchAction>,
) {
    let mut groups: HashMap<String, Vec<&ScatterPartition>> = HashMap::new();

    for partition in &scatter_info.partitions {
        let name = partition.name.to_ascii_lowercase();

        let Some(base) = slot_base_name(&name) else {
            continue;
        };

        groups.entry(base).or_default().push(partition);
    }

    for (base, partitions) in groups {
        let has_a = partitions
            .iter()
            .any(|partition| partition.name.eq_ignore_ascii_case(&format!("{base}_a")));

        let has_b = partitions
            .iter()
            .any(|partition| partition.name.eq_ignore_ascii_case(&format!("{base}_b")));

        if !has_a || !has_b {
            continue;
        }

        let Some(file_name) = partitions.iter().find_map(|partition| {
            partition
                .file_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .filter(|value| !value.eq_ignore_ascii_case("NONE"))
                .map(|value| value.to_string())
        }) else {
            continue;
        };

        for partition in partitions {
            actions.push(action(
                &partition.name,
                Some(&file_name),
                Some("true"),
                Some("true"),
                "A/B 슬롯 플래싱: _a/_b 파티션을 모두 다운로드 대상으로 활성화합니다.",
            ));
        }
    }
}

fn slot_base_name(name: &str) -> Option<String> {
    if let Some(base) = name.strip_suffix("_a") {
        return Some(base.to_string());
    }

    if let Some(base) = name.strip_suffix("_b") {
        return Some(base.to_string());
    }

    None
}

fn append_none_file_partition_rule(
    scatter_info: &ScatterXmlInfo,
    actions: &mut Vec<PartitionPatchAction>,
) {
    let none_count = count_none_file_partitions(&scatter_info.partitions);

    actions.push(action(
        "file_name=NONE partitions",
        None,
        Some("false"),
        Some("false"),
        &format!(
            "LPMBox v2.1.5 기준: file_name이 NONE인 partition {}개는 is_download=false / is_upgradable=false로 유지합니다.",
            none_count
        ),
    ));
}

fn count_none_file_partitions(partitions: &[ScatterPartition]) -> usize {
    partitions
        .iter()
        .filter(|partition| {
            partition
                .file_name
                .as_deref()
                .map(|value| value.eq_ignore_ascii_case("NONE"))
                .unwrap_or(false)
        })
        .count()
}

fn action(
    partition: &str,
    file_name: Option<&str>,
    is_download: Option<&str>,
    is_upgradable: Option<&str>,
    reason: &str,
) -> PartitionPatchAction {
    PartitionPatchAction {
        partition: partition.to_string(),
        target_file_name: file_name.map(|value| value.to_string()),
        target_is_download: is_download.map(|value| value.to_string()),
        target_is_upgradable: is_upgradable.map(|value| value.to_string()),
        reason: reason.to_string(),
    }
}
