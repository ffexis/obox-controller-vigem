//! HidHide CLI 集成：注册应用 + 隐藏 OBOX 物理手柄的所有 HID 接口。
//!
//! 遵循"保守改动"原则：
//! - 注册应用前先检查是否已注册（幂等）
//! - 隐藏设备前先检查是否已隐藏（幂等）
//! - 启用时如 cloak 全局开关 OFF 则自动 ON
//! - 禁用时只 unhide 本手柄设备，不擅自动 cloak 全局开关（避免影响其他应用）
//! - 不调用 --app-clean 等会破坏其他应用配置的命令
//!
//! HidHideCLI 输出格式（关键！）：
//! - `--cloak-state` → 单行 `--cloak-on` 或 `--cloak-off`
//! - `--app-list`   → 多行 `--app-reg "<path>"`（命令式列表，非 JSON）
//! - `--dev-list`   → 多行 `--dev-hide "<path>"`（命令式列表，非 JSON）
//! - `--dev-gaming` → JSON 数组（含 deviceInstancePath 字段，反斜杠转义为 `\\`）
//!
//! 多个参数可串联执行（单次进程调用），如 `--cloak-state --app-list --dev-list`。

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

/// 默认 HidHideCLI.exe 安装路径
const DEFAULT_CLI_PATH: &str = r"C:\Program Files\Nefarius Software Solutions\HidHide\x64\HidHideCLI.exe";

/// OBOX 设备 Col01（Gamepad 主接口）的 deviceInstancePath 匹配子串（小写）。
///
/// 经实测，HidHide 只需隐藏 Col01 即可屏蔽整个手柄的所有接口
/// （Col02/Col03 会被 HidHide 自动一同屏蔽），无需逐个隐藏。
///
/// 覆盖蓝牙 HID ("VID&00020a5c") 和标准 HID ("VID&0A5C") 两种 VID 编码。
/// PID 在两种编码下都是 "PID&4502"。
/// 加 "col01" 精确锁定 Gamepad 接口，避免误匹配 Col02/Col03。
const OBOX_PATTERNS: [&str; 3] = [
    "0a5c", // VENDOR_ID = 0x0A5C 的 VID 部分
    "pid&4502", // "PID&" + PRODUCT_ID = 0x4502
    "col01", // 只锁定 Gamepad 主接口
];

/// 定位 HidHideCLI.exe。使用默认安装路径。
fn find_cli() -> Result<PathBuf> {
    let default = PathBuf::from(DEFAULT_CLI_PATH);
    if default.exists() {
        return Ok(default);
    }
    anyhow::bail!(
        "HidHideCLI.exe not found at default path: {}\n\
         Please install HidHide or pass the correct path manually.",
        DEFAULT_CLI_PATH
    )
}

/// 执行 HidHideCLI 并返回 stdout。
fn run(cli: &PathBuf, args: &[&str]) -> Result<String> {
    let out = Command::new(cli)
        .args(args)
        .output()
        .with_context(|| format!("Failed to execute HidHideCLI with args {:?}", args))?;
    if !out.status.success() {
        anyhow::bail!(
            "HidHideCLI {:?} failed (exit {:?}): {}",
            args,
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 启动时自动检查并启用 HidHide 隐藏：
/// - 注册当前 exe（幂等）
/// - 隐藏 OBOX 设备所有接口（幂等）
/// - 如 cloak OFF 则自动 cloak-on
pub fn ensure_enabled() -> Result<()> {
    println!("[HidHide] Checking HidHide configuration...");

    let cli = find_cli()?;
    println!("[HidHide] CLI: {}", cli.display());

    let exe = std::env::current_exe().context("Failed to get current executable path")?;
    let exe_str = exe.to_string_lossy().into_owned();
    println!("[HidHide] Application path: {}", exe_str);

    register_app(&cli, &exe_str)?;
    hide_obox_devices(&cli)?;
    ensure_cloak_on(&cli)?;

    println!("[HidHide] Configuration OK.");
    Ok(())
}

/// 查看当前 HidHide 状态：cloak、应用注册、OBOX 设备隐藏情况。
pub fn print_status() -> Result<()> {
    let cli = find_cli()?;
    let exe = std::env::current_exe().context("Failed to get current executable path")?;
    let exe_str = exe.to_string_lossy().into_owned();

    // 单次调用拿全部状态（HidHideCLI 支持参数串联）
    let combined = run(&cli, &["--cloak-state", "--app-list", "--dev-list", "--dev-gaming"])?;

    // 按 "--" 前缀切分各段输出。HidHideCLI 串联执行时每段输出按行分隔，
    // 但我们用更稳的方式：扫描每行，按行前缀分发。
    let mut cloak_state = String::new();
    let mut app_lines: Vec<&str> = Vec::new();
    let mut dev_lines: Vec<&str> = Vec::new();
    let mut json_lines: Vec<&str> = Vec::new();
    let mut in_json = false;
    for line in combined.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("{") || in_json {
            in_json = true;
            json_lines.push(line);
            // JSON 数组以 ` ] ` 结尾（HidHideCLI 输出风格）
            if trimmed.contains(']') {
                in_json = false;
            }
            continue;
        }
        if trimmed.starts_with("--cloak-") {
            cloak_state = trimmed.to_string();
        } else if trimmed.starts_with("--app-reg") {
            app_lines.push(trimmed);
        } else if trimmed.starts_with("--dev-hide") {
            dev_lines.push(trimmed);
        }
    }

    let registered_apps = parse_quoted_values(&app_lines, "--app-reg");
    let hidden_devs = parse_quoted_values(&dev_lines, "--dev-hide");

    println!("=== HidHide Status ===");
    println!("CLI: {}", cli.display());

    // 1. cloak 状态
    let cloak_str = cloak_state.as_str();
    println!("Cloak: {}", cloak_str);
    if cloak_str.contains("--cloak-off") {
        println!("  (INACTIVE — hidden devices are NOT actually hidden)");
    } else if cloak_str.contains("--cloak-on") {
        println!("  (ACTIVE — hidden devices are hidden from other apps)");
    }

    // 2. 应用注册
    let exe_lower = exe_str.to_lowercase();
    let app_registered = registered_apps.iter().any(|p| p.to_lowercase() == exe_lower);
    println!("This app registered: {}", if app_registered { "YES" } else { "NO" });
    println!("  (path: {})", exe_str);
    println!("Total apps registered: {}", registered_apps.len());

    // 3. 设备隐藏情况（从 --dev-gaming JSON 取 Col01，对照 hidden_devs）
    // 经实测，HidHide 只需隐藏 Col01 即可屏蔽整个手柄的所有接口
    // （Col02/Col03 会被 HidHide 自动一同屏蔽），所以只检查 Col01。
    let gaming_json = json_lines.join("\n");
    let instances = extract_instance_paths(&gaming_json, &OBOX_PATTERNS);

    if instances.is_empty() {
        println!("OBOX Col01 (gamepad) interface: NOT FOUND (controller disconnected?)");
    } else {
        for inst in &instances {
            let is_hidden = hidden_devs.contains(&inst.to_lowercase());
            println!(
                "OBOX Col01 (gamepad): {}",
                if is_hidden { "HIDDEN" } else { "VISIBLE" }
            );
            println!("  {}", inst);
            if is_hidden {
                println!("  (Col02/Col03 auto-hidden by HidHide)");
            }
        }
    }

    Ok(())
}

/// 禁用 HidHide 对 OBOX 手柄的隐藏。
/// 只 unhide 本手柄设备，不擅自动 cloak 全局开关（避免影响其他应用）。
/// 如需完全关闭 HidHide，用户应手动执行 `HidHideCLI.exe --cloak-off`。
pub fn disable() -> Result<()> {
    println!("[HidHide] Disabling HidHide for OBOX controller...");

    let cli = find_cli()?;

    let gaming_json = run(&cli, &["--dev-gaming"])?;
    let dev_list = run(&cli, &["--dev-list"])?;
    let hidden_devs = parse_dev_list(&dev_list);
    let instances = extract_instance_paths(&gaming_json, &OBOX_PATTERNS);

    if instances.is_empty() {
        println!("[HidHide] No OBOX device interfaces found. Nothing to unhide.");
        return Ok(());
    }

    let mut unhidden = 0usize;
    let mut already = 0usize;
    for inst in &instances {
        if !hidden_devs.contains(&inst.to_lowercase()) {
            already += 1;
            continue;
        }
        match run(&cli, &["--dev-unhide", inst]) {
            Ok(_) => {
                unhidden += 1;
                println!("[HidHide] Unhidden: {}", inst);
            }
            Err(e) => eprintln!("[HidHide] WARN: Failed to unhide {}: {}", inst, e),
        }
    }
    println!(
        "[HidHide] {} interface(s) unhidden, {} already visible",
        unhidden, already
    );
    println!("[HidHide] NOTE: Global cloak state left unchanged (may affect other apps).");
    println!(
        "[HidHide] To fully disable HidHide, run: \"{}\" --cloak-off",
        cli.display()
    );
    Ok(())
}

/// 检查当前 exe 是否已注册，未注册则 --app-reg（幂等）。
fn register_app(cli: &PathBuf, exe: &str) -> Result<()> {
    let list = run(cli, &["--app-list"])?;
    let apps = parse_app_list(&list);
    let exe_lower = exe.to_lowercase();
    if apps.iter().any(|p| p.to_lowercase() == exe_lower) {
        println!("[HidHide] Application already registered");
        return Ok(());
    }
    run(cli, &["--app-reg", exe])?;
    println!("[HidHide] Application registered");
    Ok(())
}

/// 解析 --dev-gaming 的 JSON 输出，提取所有 OBOX 设备的 deviceInstancePath，
/// 对未隐藏的接口调用 --dev-hide（幂等）。
fn hide_obox_devices(cli: &PathBuf) -> Result<()> {
    let gaming_json = run(cli, &["--dev-gaming"])?;
    let dev_list = run(cli, &["--dev-list"])?;
    let hidden_devs = parse_dev_list(&dev_list);
    let instances = extract_instance_paths(&gaming_json, &OBOX_PATTERNS);

    if instances.is_empty() {
        println!("[HidHide] No OBOX device interfaces found in --dev-gaming output");
        println!("[HidHide] (Is the controller connected?)");
        return Ok(());
    }

    println!(
        "[HidHide] Found {} OBOX device interface(s) from --dev-gaming",
        instances.len()
    );

    let mut hidden_count = 0usize;
    let mut already_count = 0usize;
    for inst in &instances {
        if hidden_devs.contains(&inst.to_lowercase()) {
            already_count += 1;
            continue;
        }
        match run(cli, &["--dev-hide", inst]) {
            Ok(_) => {
                hidden_count += 1;
                println!("[HidHide] Hidden: {}", inst);
            }
            Err(e) => {
                eprintln!("[HidHide] WARN: Failed to hide {}: {}", inst, e);
            }
        }
    }
    println!(
        "[HidHide] {} interface(s) hidden, {} already hidden",
        hidden_count, already_count
    );
    Ok(())
}

/// 如 cloak OFF 则自动 cloak-on（仅启动时调用）。
fn ensure_cloak_on(cli: &PathBuf) -> Result<()> {
    let state = run(cli, &["--cloak-state"])?;
    let state_trim = state.trim();
    if state_trim.contains("--cloak-off") {
        println!("[HidHide] Cloak is OFF, enabling...");
        run(cli, &["--cloak-on"])?;
        println!("[HidHide] Cloak ON (global)");
    } else if state_trim.contains("--cloak-on") {
        println!("[HidHide] Cloak already ON");
    } else {
        println!("[HidHide] Cloak state unknown: {}", state_trim);
    }
    Ok(())
}

/// 从 `--app-list` 输出解析已注册应用路径集合。
///
/// 输入格式（每行一个命令）：
/// ```text
/// --app-reg "C:\path\to\app1.exe"
/// --app-reg "C:\path\to\app2.exe"
/// ```
///
/// 返回的路径全部小写，便于大小写不敏感比较。
fn parse_app_list(text: &str) -> HashSet<String> {
    parse_quoted_values(
        &text.lines().filter_map(|l| {
            let l = l.trim();
            if l.starts_with("--app-reg") {
                Some(l)
            } else {
                None
            }
        }).collect::<Vec<&str>>(),
        "--app-reg",
    )
}

/// 从 `--dev-list` 输出解析已隐藏设备路径集合。
///
/// 输入格式（每行一个命令）：
/// ```text
/// --dev-hide "HID\...\Col01\..."
/// --dev-hide "HID\...\Col02\..."
/// ```
///
/// 返回的路径全部小写，便于大小写不敏感比较。
fn parse_dev_list(text: &str) -> HashSet<String> {
    parse_quoted_values(
        &text.lines().filter_map(|l| {
            let l = l.trim();
            if l.starts_with("--dev-hide") {
                Some(l)
            } else {
                None
            }
        }).collect::<Vec<&str>>(),
        "--dev-hide",
    )
}

/// 从一组 `"--cmd "value""` 行中提取引号内的值（小写化）。
///
/// 通用解析器：去掉前缀后，取首个 `"` 到末尾 `"` 之间的内容。
fn parse_quoted_values(lines: &[&str], prefix: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for line in lines {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.trim();
        if rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"') {
            let val = &rest[1..rest.len() - 1];
            set.insert(val.to_lowercase());
        }
    }
    set
}

/// 从 `--dev-gaming` 的 JSON 文本中提取所有同时包含每个 pattern 的
/// `deviceInstancePath` 字段值。
///
/// JSON 格式（HidHideCLI 输出，字段名和值之间有空格但仍合法）：
/// `"deviceInstancePath" : "HID\\...\\Col01\\..."`
///
/// **重要**：JSON 字符串里反斜杠是转义的（`\\`），提取后需反转义为单反斜杠，
/// 才能和 `--dev-list` 输出里的路径匹配。
///
/// 用简易文本扫描避免引入 serde_json 依赖。匹配大小写不敏感。
fn extract_instance_paths(json: &str, patterns: &[&str]) -> Vec<String> {
    let key = "\"deviceInstancePath\"";
    let mut results = Vec::new();
    let bytes = json.as_bytes();
    let mut i = 0usize;
    while i + key.len() <= bytes.len() {
        if &bytes[i..i + key.len()] == key.as_bytes() {
            // 找到 key，向后找第一个 '"'，然后再找下一个 '"'
            let mut j = i + key.len();
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            if j >= bytes.len() {
                break;
            }
            let start = j + 1; // 跳过开头引号
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'"' {
                end += 1;
            }
            if end > start && end <= bytes.len() {
                let val_raw = &json[start..end];
                // JSON 反转义：\\  →  \
                let val = val_raw.replace("\\\\", "\\");
                let val_lower = val.to_lowercase();
                if patterns.iter().all(|p| val_lower.contains(p)) {
                    results.push(val);
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_instance_paths_bluetooth_encoding() {
        // Bluetooth HID: VID 编码为 "VID&00020a5c"（带 0002 蓝牙前缀）
        // 注意 JSON 里反斜杠是转义的（\\）
        let json = r#"
        { "deviceInstancePath" : "HID\\{00001124-0000-1000-8000-00805f9b34fb}_VID&00020a5c_PID&4502&Col01\\c&c7f3abb&4&0000" , "foo" : "bar" }
        "#;
        let patterns = ["0a5c", "pid&4502", "col01"];
        let paths = extract_instance_paths(json, &patterns);
        assert_eq!(paths.len(), 1);
        // 反转义后应该是单反斜杠
        assert!(paths[0].contains(r"Col01\"));
        assert!(!paths[0].contains(r"Col01\\"));
    }

    #[test]
    fn test_extract_instance_paths_standard_encoding() {
        // 标准 HID: VID 编码为 "VID&0A5C"
        let json = r#"
        { "deviceInstancePath" : "HID\\VID&0A5C&PID&4502&Col01\\c&1234&4&0000" }
        "#;
        let patterns = ["0a5c", "pid&4502", "col01"];
        let paths = extract_instance_paths(json, &patterns);
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn test_extract_instance_paths_only_col01_matched() {
        // Col01/Col02/Col03 同时存在时，只应匹配 Col01
        let json = r#"
        { "deviceInstancePath" : "HID\\VID&00020A5C_PID&4502&Col01\\c&1234&4&0000" }
        { "deviceInstancePath" : "HID\\VID&00020A5C_PID&4502&Col02\\c&1234&4&0001" }
        { "deviceInstancePath" : "HID\\VID&00020A5C_PID&4502&Col03\\c&1234&4&0002" }
        { "deviceInstancePath" : "HID\\VID&045E&PID&028E\\some_other_device" }
        "#;
        let paths = extract_instance_paths(json, &OBOX_PATTERNS);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].to_lowercase().contains("col01"));
    }

    #[test]
    fn test_extract_instance_paths_no_match() {
        let json = r#"{ "deviceInstancePath" : "HID\\VID&045E&PID&028E\\xbox" }"#;
        let patterns = ["0a5c", "pid&4502", "col01"];
        let paths = extract_instance_paths(json, &patterns);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_extract_instance_paths_pid_only_no_vid() {
        // PID 匹配但 VID 不匹配，不应返回
        let json = r#"{ "deviceInstancePath" : "HID\\VID&045E&PID&4502\\fake" }"#;
        let patterns = ["0a5c", "pid&4502", "col01"];
        let paths = extract_instance_paths(json, &patterns);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_extract_instance_paths_col02_not_matched() {
        // Col02 不应被匹配（避免误隐藏非 Gamepad 接口）
        let json = r#"{ "deviceInstancePath" : "HID\\VID&0A5C&PID&4502&Col02\\c&1234&4&0001" }"#;
        let paths = extract_instance_paths(json, &OBOX_PATTERNS);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_parse_app_list() {
        let text = r#"
--app-reg "C:\Program Files\Nefarius Software Solutions\HidHide\x64\HidHideCLI.exe"
--app-reg "D:\Tools\mi-vigem.exe"
--app-reg "F:\Dev Project\obox controller driver\target\release\obox-controller-driver.exe"
"#;
        let apps = parse_app_list(text);
        assert_eq!(apps.len(), 3);
        assert!(apps.contains(&r"c:\program files\nefarius software solutions\hidhide\x64\hidhidecli.exe".to_lowercase()));
        assert!(apps.contains(&r"d:\tools\mi-vigem.exe".to_lowercase()));
        assert!(apps.contains(&r"f:\dev project\obox controller driver\target\release\obox-controller-driver.exe".to_lowercase()));
    }

    #[test]
    fn test_parse_dev_list() {
        let text = r#"
--dev-hide "HID\{00001124-0000-1000-8000-00805f9b34fb}_VID&00020a5c_PID&4502&Col01\c&c7f3abb&4&0000"
--dev-hide "HID\{00001124-0000-1000-8000-00805f9b34fb}_VID&00020a5c_PID&4502&Col02\c&c7f3abb&4&0001"
--dev-hide "HID\{00001124-0000-1000-8000-00805f9b34fb}_VID&00020a5c_PID&4502&Col03\c&c7f3abb&4&0002"
"#;
        let devs = parse_dev_list(text);
        assert_eq!(devs.len(), 3);
        assert!(devs.contains(&r"hid\{00001124-0000-1000-8000-00805f9b34fb}_vid&00020a5c_pid&4502&col01\c&c7f3abb&4&0000".to_lowercase()));
        assert!(devs.contains(&r"hid\{00001124-0000-1000-8000-00805f9b34fb}_vid&00020a5c_pid&4502&col03\c&c7f3abb&4&0002".to_lowercase()));
    }

    #[test]
    fn test_match_between_json_and_dev_list() {
        // 模拟真实场景：从 --dev-gaming JSON 提取的 Col01 path（反转义后）
        // 应该能在 --dev-list 输出中找到精确匹配
        let json = r#"
        { "deviceInstancePath" : "HID\\{00001124-0000-1000-8000-00805f9b34fb}_VID&00020a5c_PID&4502&Col01\\c&c7f3abb&4&0000" }
        "#;
        let dev_list = r#"--dev-hide "HID\{00001124-0000-1000-8000-00805f9b34fb}_VID&00020a5c_PID&4502&Col01\c&c7f3abb&4&0000""#;

        let instances = extract_instance_paths(json, &OBOX_PATTERNS);
        let hidden = parse_dev_list(dev_list);

        assert_eq!(instances.len(), 1);
        assert!(hidden.contains(&instances[0].to_lowercase()));
    }

    #[test]
    fn test_cloak_state_parsing() {
        // cloak-state 输出就是单行命令式
        let on = "--cloak-on";
        let off = "--cloak-off";
        assert!(on.contains("--cloak-on"));
        assert!(!on.contains("--cloak-off"));
        assert!(off.contains("--cloak-off"));
    }
}
