use crate::scan_progress;

#[test]
fn stage_update_serialises_only_the_four_permitted_fields() {
    let json = serde_json::to_value(scan_progress::StageUpdate::done("废纸篓", &[])).unwrap();
    let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["found_bytes", "item_count", "stage", "state"]);
    assert!(!json.to_string().contains("path"));
}

/// 截取 lib.rs 中某个 `#[tauri::command] async fn` 的完整签名 + 函数体。
fn command_source(name: &str) -> String {
    let source = include_str!("lib.rs");
    let start = source
        .find(&format!("async fn {name}("))
        .unwrap_or_else(|| panic!("lib.rs 里找不到 async fn {name}"));
    let rest = &source[start..];
    let open = rest
        .find('{')
        .unwrap_or_else(|| panic!("async fn {name} 的签名后面没有函数体"));
    let mut depth = 0usize;
    for (offset, byte) in rest.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return rest[..=open + offset].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("async fn {name} 的花括号不配对，源码被改坏了");
}

/// 事件名必须是写死的字符串字面量，不能和任何用户输入拼在一起。
/// 断言里把**闭合引号**也带上：`format!("cache-scan-progress-{x}")`
/// 展开后是 `"cache-scan-progress-{x}"`，不含 `"cache-scan-progress"`，
/// 因此这条断言能挡住「事件名里混入用户数据」。
#[test]
fn scan_commands_emit_hard_coded_progress_events() {
    for (command, event) in [
        ("scan_cache", "cache-scan-progress"),
        ("scan_app_residues_batch", "residue-scan-progress"),
    ] {
        let body = command_source(command);
        assert!(
            body.contains(&format!("\"{event}\"")),
            "{command} 必须 emit 写死的事件名 {event:?}",
        );
        assert!(
            body.contains("app: tauri::AppHandle"),
            "{command} 必须拿 AppHandle 才能发事件",
        );
        assert!(
            body.contains("Emitter"),
            "{command} 必须引入 tauri::Emitter",
        );
        // 一个 command 只能 emit 一次。多出来的那一次几乎必然是为了「顺带」
        // 把整个 AppResidue 推给前端，而 AppResidue 里带 bundle_id 和每条
        // ResidueItem.path，直接违反 spec §5「载荷不得携带路径 / PID / bundle id」。
        let emit_count = body.match_indices("app.emit(").count();
        assert_eq!(
            emit_count, 1,
            "{command} 里必须恰好出现 1 次 app.emit(，实际 {emit_count} 次",
        );
    }
}

/// 截出 `scan_progress::StageUpdate { ... }` 载荷体的顶层字段名，按书写顺序。
///
/// 不用「黑名单子串」那套：`path:` 拦不住 `app_path:`，`bundle_id` 拦不住
/// `bundle_identifier`，`pid` 拦不住 `user_pid`。这里改成解析字段名并与白名单
/// **逐一精确比对**，多一个、少一个、改名、调序都会失败。
fn stage_update_field_names(body: &str) -> Vec<String> {
    let anchor = body
        .find("scan_progress::StageUpdate {")
        .expect("命令体里没有 scan_progress::StageUpdate 载荷");
    let rest = &body[anchor..];
    let open = rest.find('{').expect("StageUpdate 后面没有载荷体");
    let mut depth = 0usize;
    let mut end = None;
    for (offset, byte) in rest.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(open + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.expect("StageUpdate 载荷体的花括号不配对");
    rest[open + 1..end]
        .split(',')
        .map(|field| {
            field
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        })
        .filter(|field| !field.is_empty())
        .collect()
}

#[test]
fn residue_progress_payload_only_carries_the_permitted_stage_fields() {
    let body = command_source("scan_app_residues_batch");

    let payload_count = body.match_indices("scan_progress::StageUpdate").count();
    assert_eq!(
        payload_count, 1,
        "命令体里必须恰好构造 1 个 StageUpdate 载荷，实际 {payload_count} 个",
    );

    assert_eq!(
        stage_update_field_names(&body),
        vec!["stage", "state", "item_count", "found_bytes"],
        "residue 进度载荷的字段必须恰好是 spec §5 允许的那四个（字段名逐一精确比对）",
    );
}
