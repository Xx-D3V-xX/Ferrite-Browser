//! The operating system's own file picker, for `<input type=file>` (T-293).
//!
//! Each system already ships a program that shows its native picker and prints
//! the chosen paths: `osascript` on macOS, PowerShell's `OpenFileDialog` on
//! Windows, and `zenity` or `kdialog` on Linux desktops. Running that program
//! needs no new dependency. When none is there (a minimal Linux), the file card
//! keeps its typed-path field, which always works.
//!
//! Nothing the page sent goes into the command: not its `accept` list, not its
//! name. The commands are fixed text apart from the single/multiple switch, so a
//! page cannot inject into an AppleScript or a shell line. The chosen paths are
//! not given to the page here either: they fill the card, which checks them and
//! waits for the person to press Open.

use std::path::PathBuf;
use std::process::Command;

/// The program and arguments that show the picker on `os` (`std::env::consts::OS`),
/// with `linux_tool` the Linux picker found on `PATH`, if any.
pub(crate) fn picker_command(
    os: &str,
    multiple: bool,
    linux_tool: Option<&str>,
) -> Option<(String, Vec<String>)> {
    let args = |list: &[&str]| list.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
    match os {
        "macos" => {
            let script = if multiple {
                "set chosen to choose file with prompt \"Choose files for the page\" \
                 with multiple selections allowed\n\
                 set out to \"\"\n\
                 repeat with f in chosen\n\
                 set out to out & POSIX path of f & linefeed\n\
                 end repeat\n\
                 return out"
            } else {
                "return POSIX path of (choose file with prompt \"Choose a file for the page\")"
            };
            Some(("osascript".into(), args(&["-e", script])))
        }
        "windows" => {
            let script = format!(
                "Add-Type -AssemblyName System.Windows.Forms; \
                 $d = New-Object System.Windows.Forms.OpenFileDialog; \
                 $d.Multiselect = ${multiple}; \
                 $d.Title = 'Choose for the page'; \
                 if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{ $d.FileNames -join \"`n\" }}"
            );
            Some((
                "powershell".into(),
                vec![
                    "-NoProfile".into(),
                    "-STA".into(),
                    "-Command".into(),
                    script,
                ],
            ))
        }
        _ => match linux_tool? {
            "zenity" => {
                let mut list = args(&["--file-selection", "--title=Choose for the page"]);
                if multiple {
                    list.extend(args(&["--multiple", "--separator=\n"]));
                }
                Some(("zenity".into(), list))
            }
            "kdialog" => {
                let mut list = args(&["--title", "Choose for the page", "--getopenfilename", "."]);
                if multiple {
                    list.extend(args(&["--multiple", "--separate-output"]));
                }
                Some(("kdialog".into(), list))
            }
            _ => None,
        },
    }
}

/// The paths a picker printed: one per line, blank lines dropped.
pub(crate) fn parse_output(stdout: &str) -> Vec<PathBuf> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The Linux picker on `PATH`: zenity first (GNOME and most others), then kdialog.
fn linux_tool() -> Option<&'static str> {
    let path = std::env::var_os("PATH")?;
    ["zenity", "kdialog"]
        .into_iter()
        .find(|tool| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

/// Shows the system picker and returns what the person chose: an empty list
/// if they cancelled, an error if there is no picker to show.
pub(crate) async fn pick(multiple: bool) -> Result<Vec<PathBuf>, String> {
    tokio::task::spawn_blocking(move || {
        let os = std::env::consts::OS;
        let tool = if os == "macos" || os == "windows" {
            None
        } else {
            linux_tool()
        };
        let Some((program, args)) = picker_command(os, multiple, tool) else {
            return Err("No system file picker was found (zenity or kdialog). \
                        Type the path instead."
                .to_string());
        };
        let mut command = Command::new(&program);
        command.args(&args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window flashing up behind the picker.
            command.creation_flags(0x0800_0000);
        }
        let output = command
            .output()
            .map_err(|e| format!("Could not open the system file picker ({program}): {e}"))?;
        // Cancelling exits non-zero with nothing printed: that is a choice, not an error.
        Ok(parse_output(&String::from_utf8_lossy(&output.stdout)))
    })
    .await
    .map_err(|e| format!("The file picker stopped: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_system_gets_its_own_picker() {
        let (program, args) = picker_command("macos", false, None).unwrap();
        assert_eq!(program, "osascript");
        assert!(args[1].contains("POSIX path of (choose file"));
        let (_, args) = picker_command("macos", true, None).unwrap();
        assert!(args[1].contains("with multiple selections allowed"));

        let (program, args) = picker_command("windows", true, None).unwrap();
        assert_eq!(program, "powershell");
        assert!(args.last().unwrap().contains("$d.Multiselect = $true"));
        let (_, args) = picker_command("windows", false, None).unwrap();
        assert!(args.last().unwrap().contains("$d.Multiselect = $false"));

        let (program, args) = picker_command("linux", true, Some("zenity")).unwrap();
        assert_eq!(program, "zenity");
        assert!(args.contains(&"--multiple".to_string()));
        let (program, args) = picker_command("linux", false, Some("kdialog")).unwrap();
        assert_eq!(program, "kdialog");
        assert!(!args.contains(&"--multiple".to_string()));

        // A Linux without either picker: the typed field stays the way.
        assert!(picker_command("linux", false, None).is_none());
    }

    #[test]
    fn printed_paths_are_one_per_line() {
        assert_eq!(
            parse_output("/home/me/a.png\n\n/home/me/b c.pdf\r\n"),
            vec![
                PathBuf::from("/home/me/a.png"),
                PathBuf::from("/home/me/b c.pdf")
            ]
        );
        // A cancelled picker prints nothing.
        assert!(parse_output("").is_empty());
    }
}
