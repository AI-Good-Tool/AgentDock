//! System task payloads are pure builders, so tests never register real jobs.
use std::path::Path;

pub const MARKER: &str = "AgentDock shared check-in schedule v2";

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn launchd_plist(exe: &Path, data: &Path) -> String {
    let exe = xml(&exe.to_string_lossy());
    let log = xml(&data.join("auto-claim.log").to_string_lossy());
    let data = xml(&data.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <!-- {MARKER} -->
  <key>Label</key><string>com.agentdock.claim</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>--claim-due</string><string>--data-dir</string><string>{data}</string></array>
  <key>StartInterval</key><integer>60</integer>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#
    )
}

fn quote_windows(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        quoted.push_str(&"\\".repeat(if ch == '"' { 2 * slashes + 1 } else { slashes }));
        quoted.push(ch);
        slashes = 0;
    }
    quoted.push_str(&"\\".repeat(2 * slashes));
    quoted.push('"');
    quoted
}

pub fn windows_command(exe: &Path, data: &Path) -> String {
    format!(
        "{} --claim-due --data-dir {}",
        quote_windows(&exe.to_string_lossy()),
        quote_windows(&data.to_string_lossy())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_payload_uses_shared_due_check_and_escapes_paths() {
        let text = launchd_plist(
            Path::new("/Applications/A&B.app/a"),
            Path::new("/Users/a/<data>"),
        );
        assert!(text.contains(MARKER));
        assert!(text.contains("A&amp;B.app"));
        assert!(text.contains("&lt;data&gt;"));
        assert!(text.contains("<string>--claim-due</string>"));
        assert!(text.contains("<key>StartInterval</key><integer>60</integer>"));
        assert!(!text.contains("StartCalendarInterval"));
    }

    #[test]
    fn windows_paths_preserve_spaces_and_trailing_backslashes() {
        let command = windows_command(
            Path::new(r"C:\Program Files\AgentDock\agentdock.exe"),
            Path::new(r"C:\"),
        );
        assert!(command
            .starts_with(r#""C:\Program Files\AgentDock\agentdock.exe" --claim-due --data-dir "#));
        assert!(command.ends_with(r#""C:\\""#));
    }
}
