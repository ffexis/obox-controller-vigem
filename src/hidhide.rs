//! HidHide CLI integration: register the app + hide all HID interfaces of the physical OBOX controller.
//!
//! Follows a "conservative changes" principle:
//! - Check whether the app is already registered before registering it (idempotent)
//! - Check whether devices are already hidden before hiding them (idempotent)
//! - On enable, if the global cloak switch is OFF, turn it ON automatically
//! - On disable, only unhide this controller's devices; never touch the global cloak switch (to avoid affecting other apps)
//! - Never call commands such as --app-clean that would break other apps' configuration
//!
//! HidHideCLI output formats (important!):
//! - `--cloak-state` → a single line `--cloak-on` or `--cloak-off`
//! - `--app-list`   → multiple lines of `--app-reg "<path>"` (command-style list, not JSON)
//! - `--dev-list`   → multiple lines of `--dev-hide "<path>"` (command-style list, not JSON)
//! - `--dev-gaming` → JSON array (with a deviceInstancePath field; backslashes escaped as `\\`)
//!
//! Multiple arguments can be chained in a single process invocation, e.g. `--cloak-state --app-list --dev-list`.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

/// Default HidHideCLI.exe installation path
const DEFAULT_CLI_PATH: &str = r"C:\Program Files\Nefarius Software Solutions\HidHide\x64\HidHideCLI.exe";

/// deviceInstancePath match substrings (lowercase) for the OBOX device Col01 (main Gamepad interface).
///
/// In practice, hiding Col01 alone is enough for HidHide to mask all interfaces of the controller
/// (Col02/Col03 are hidden automatically along with it), so there is no need to hide each one.
///
/// Covers both VID encodings: Bluetooth HID ("VID&00020a5c") and standard HID ("VID&0A5C").
/// The PID is "PID&4502" under both encodings.
/// "col01" pins the match to the Gamepad interface, avoiding false matches on Col02/Col03.
const OBOX_PATTERNS: [&str; 3] = [
    "0a5c", // VID part of VENDOR_ID = 0x0A5C
    "pid&4502", // "PID&" + PRODUCT_ID = 0x4502
    "col01", // lock onto the main Gamepad interface only
];

/// Locate HidHideCLI.exe. Uses the default installation path.
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

/// Run HidHideCLI and return stdout.
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

/// Automatically check and enable HidHide hiding at startup:
/// - register the current exe (idempotent)
/// - hide all interfaces of the OBOX device (idempotent)
/// - cloak-on automatically if cloak is OFF
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

/// Show the current HidHide status: cloak, app registration, OBOX device hiding state.
pub fn print_status() -> Result<()> {
    let cli = find_cli()?;
    let exe = std::env::current_exe().context("Failed to get current executable path")?;
    let exe_str = exe.to_string_lossy().into_owned();

    // Fetch all state in a single call (HidHideCLI supports chained arguments)
    let combined = run(&cli, &["--cloak-state", "--app-list", "--dev-list", "--dev-gaming"])?;

    // Split each output segment by its "--" prefix. When HidHideCLI runs chained arguments each
    // segment is output line by line, but we use the more robust approach: scan each line and dispatch by line prefix.
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
            // The JSON array ends with ` ] ` (HidHideCLI output style)
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

    // 1. Cloak state
    let cloak_str = cloak_state.as_str();
    println!("Cloak: {}", cloak_str);
    if cloak_str.contains("--cloak-off") {
        println!("  (INACTIVE — hidden devices are NOT actually hidden)");
    } else if cloak_str.contains("--cloak-on") {
        println!("  (ACTIVE — hidden devices are hidden from other apps)");
    }

    // 2. App registration
    let exe_lower = exe_str.to_lowercase();
    let app_registered = registered_apps.iter().any(|p| p.to_lowercase() == exe_lower);
    println!("This app registered: {}", if app_registered { "YES" } else { "NO" });
    println!("  (path: {})", exe_str);
    println!("Total apps registered: {}", registered_apps.len());

    // 3. Device hiding state (take Col01 from the --dev-gaming JSON and compare against hidden_devs)
    // In practice, hiding Col01 alone is enough for HidHide to mask all interfaces of the controller
    // (Col02/Col03 are hidden automatically along with it), so only Col01 is checked.
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

/// Disable HidHide's hiding of the OBOX controller.
/// Only unhide this controller's devices; never touch the global cloak switch (to avoid affecting other apps).
/// To fully turn off HidHide, the user should run `HidHideCLI.exe --cloak-off` manually.
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

/// Check whether the current exe is already registered; run --app-reg if not (idempotent).
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

/// Parse the JSON output of --dev-gaming, extract the deviceInstancePath of every OBOX device,
/// and call --dev-hide on interfaces that are not hidden yet (idempotent).
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

/// Turn cloak-on automatically if cloak is OFF (called at startup only).
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

/// Parse the set of registered app paths from `--app-list` output.
///
/// Input format (one command per line):
/// ```text
/// --app-reg "C:\path\to\app1.exe"
/// --app-reg "C:\path\to\app2.exe"
/// ```
///
/// All returned paths are lowercased for case-insensitive comparison.
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

/// Parse the set of hidden device paths from `--dev-list` output.
///
/// Input format (one command per line):
/// ```text
/// --dev-hide "HID\...\Col01\..."
/// --dev-hide "HID\...\Col02\..."
/// ```
///
/// All returned paths are lowercased for case-insensitive comparison.
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

/// Extract the quoted value (lowercased) from a set of `"--cmd "value""` lines.
///
/// Generic parser: strip the prefix, then take everything from the first `"` to the trailing `"`.
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

/// Extract every `deviceInstancePath` field value from the --dev-gaming JSON text
/// that contains all of the given patterns.
///
/// JSON format (HidHideCLI output; spaces between field name and value, still valid):
/// `"deviceInstancePath" : "HID\\...\\Col01\\..."`
///
/// **Important**: backslashes are escaped in JSON strings (`\\`), so after extraction they must
/// be unescaped to single backslashes before they can match paths in `--dev-list` output.
///
/// Uses a simple text scan to avoid pulling in a serde_json dependency. Matching is case-insensitive.
fn extract_instance_paths(json: &str, patterns: &[&str]) -> Vec<String> {
    let key = "\"deviceInstancePath\"";
    let mut results = Vec::new();
    let bytes = json.as_bytes();
    let mut i = 0usize;
    while i + key.len() <= bytes.len() {
        if &bytes[i..i + key.len()] == key.as_bytes() {
            // Found the key; scan forward to the first '"', then to the next '"'
            let mut j = i + key.len();
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            if j >= bytes.len() {
                break;
            }
            let start = j + 1; // skip the opening quote
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'"' {
                end += 1;
            }
            if end > start && end <= bytes.len() {
                let val_raw = &json[start..end];
                // JSON unescape: \\  →  \
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
        // Bluetooth HID: VID encoding is "VID&00020a5c" (with the 0002 Bluetooth prefix)
        // Note that backslashes are escaped in JSON (as \\)
        let json = r#"
        { "deviceInstancePath" : "HID\\{00001124-0000-1000-8000-00805f9b34fb}_VID&00020a5c_PID&4502&Col01\\c&c7f3abb&4&0000" , "foo" : "bar" }
        "#;
        let patterns = ["0a5c", "pid&4502", "col01"];
        let paths = extract_instance_paths(json, &patterns);
        assert_eq!(paths.len(), 1);
        // After unescaping there should be a single backslash
        assert!(paths[0].contains(r"Col01\"));
        assert!(!paths[0].contains(r"Col01\\"));
    }

    #[test]
    fn test_extract_instance_paths_standard_encoding() {
        // Standard HID: VID encoding is "VID&0A5C"
        let json = r#"
        { "deviceInstancePath" : "HID\\VID&0A5C&PID&4502&Col01\\c&1234&4&0000" }
        "#;
        let patterns = ["0a5c", "pid&4502", "col01"];
        let paths = extract_instance_paths(json, &patterns);
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn test_extract_instance_paths_only_col01_matched() {
        // When Col01/Col02/Col03 all exist, only Col01 should match
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
        // PID matches but VID does not, so nothing should be returned
        let json = r#"{ "deviceInstancePath" : "HID\\VID&045E&PID&4502\\fake" }"#;
        let patterns = ["0a5c", "pid&4502", "col01"];
        let paths = extract_instance_paths(json, &patterns);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_extract_instance_paths_col02_not_matched() {
        // Col02 should not match (avoid hiding non-Gamepad interfaces by mistake)
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
        // Simulate a real scenario: the Col01 path extracted from --dev-gaming JSON (after unescaping)
        // should be found as an exact match in the --dev-list output
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
        // The cloak-state output is just a single command-style line
        let on = "--cloak-on";
        let off = "--cloak-off";
        assert!(on.contains("--cloak-on"));
        assert!(!on.contains("--cloak-off"));
        assert!(off.contains("--cloak-off"));
    }
}
