// 开发者工具专属残留规则：为特定开发工具定义额外扫描路径

/// 开发者工具残留规则
pub struct DevToolRule {
    /// Bundle ID 匹配模式（精确或前缀）
    pub bundle_id_pattern: String,
    /// 额外扫描路径（支持 ~ 展开）
    pub extra_paths: Vec<String>,
    /// 规则标签，如 "Xcode 专属数据"
    pub label: String,
}

/// 获取匹配的开发者工具规则
pub fn get_dev_tool_rules(bundle_id: &str) -> Vec<DevToolRule> {
    if bundle_id.is_empty() {
        return Vec::new();
    }
    build_all_rules()
        .into_iter()
        .filter(|rule| matches_pattern(bundle_id, &rule.bundle_id_pattern))
        .collect()
}

/// 判断 Bundle ID 是否匹配规则模式
/// 支持精确匹配和通配符前缀匹配（如 com.jetbrains.*）
fn matches_pattern(bundle_id: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return bundle_id.starts_with(prefix);
    }
    bundle_id == pattern
}

/// 构建所有内置开发者工具规则
fn build_all_rules() -> Vec<DevToolRule> {
    vec![
        build_xcode_rule(),
        build_vscode_rule(),
        build_docker_rule(),
        build_jetbrains_rule(),
        build_android_studio_rule(),
    ]
}

/// Xcode 专属规则
fn build_xcode_rule() -> DevToolRule {
    DevToolRule {
        bundle_id_pattern: "com.apple.dt.Xcode".to_string(),
        extra_paths: vec![
            "~/Library/Developer/Xcode/DerivedData".to_string(),
            "~/Library/Developer/Xcode/Archives".to_string(),
            "~/Library/Developer/Xcode/iOS DeviceSupport".to_string(),
            "~/Library/Developer/Xcode/watchOS DeviceSupport".to_string(),
            "~/Library/Developer/CoreSimulator".to_string(),
        ],
        label: "Xcode 专属数据".to_string(),
    }
}

/// VS Code 专属规则
fn build_vscode_rule() -> DevToolRule {
    DevToolRule {
        bundle_id_pattern: "com.microsoft.VSCode".to_string(),
        extra_paths: vec![
            "~/.vscode/extensions".to_string(),
            "~/.vscode/argv.json".to_string(),
            "~/Library/Application Support/Code".to_string(),
        ],
        label: "VS Code 专属数据".to_string(),
    }
}

/// Docker Desktop 专属规则
fn build_docker_rule() -> DevToolRule {
    DevToolRule {
        bundle_id_pattern: "com.docker.docker".to_string(),
        extra_paths: vec![
            "~/Library/Containers/com.docker.docker".to_string(),
            "~/Library/Group Containers/group.com.docker".to_string(),
            "~/.docker".to_string(),
        ],
        label: "Docker Desktop 专属数据".to_string(),
    }
}

/// JetBrains 系列 IDE 专属规则（前缀匹配 com.jetbrains.*）
fn build_jetbrains_rule() -> DevToolRule {
    DevToolRule {
        bundle_id_pattern: "com.jetbrains.*".to_string(),
        extra_paths: vec!["~/Library/Application Support/JetBrains".to_string()],
        label: "JetBrains 专属数据".to_string(),
    }
}

/// Android Studio 专属规则
fn build_android_studio_rule() -> DevToolRule {
    DevToolRule {
        bundle_id_pattern: "com.google.android.studio".to_string(),
        extra_paths: vec!["~/.android".to_string(), "~/Library/Android".to_string()],
        label: "Android Studio 专属数据".to_string(),
    }
}
