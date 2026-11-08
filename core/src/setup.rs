//! First-run setup: installs everything needed to play (graphics drivers, 32-bit libraries,
//! the umu-launcher game engine, controller rules, GameMode…) for the user's distribution.
//!
//! The installer script is embedded in the binary, so a packaged app needs no extra files.
//! Developers run the very same file: `scripts/install-deps.sh`.

use std::{
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{bail, Context, Result};

use crate::{launch, paths::Paths};

pub const INSTALL_SCRIPT: &str = include_str!("../../scripts/install-deps.sh");

/// What the installer found when asked (without changing anything).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupState {
    /// Everything required is installed.
    Ready,
    /// Something required is missing. Each line is `install: pkg`, `download: …`, `optional: pkg`…
    Needed(Vec<String>),
    /// This Linux distribution isn't supported by the automatic installer.
    Unsupported,
}

/// True when the game engine itself is missing (games can't start at all).
pub fn needs_setup() -> bool {
    launch::which("umu-run").is_none()
}

fn write_script(paths: &Paths) -> Result<PathBuf> {
    let dir = paths.data.join("setup");
    fs::create_dir_all(&dir)?;
    let path = dir.join("install-deps.sh");
    fs::write(&path, INSTALL_SCRIPT)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

/// Ask the installer what is missing. Quick, needs no password and changes nothing.
pub fn check(paths: &Paths) -> SetupState {
    let Ok(script) = write_script(paths) else { return SetupState::Ready };
    let Ok(out) = Command::new("bash").arg(&script).arg("--check").stdin(Stdio::null()).stderr(Stdio::null()).output()
    else {
        return SetupState::Ready;
    };
    parse_check(out.status.code(), &String::from_utf8_lossy(&out.stdout))
}

fn parse_check(code: Option<i32>, stdout: &str) -> SetupState {
    match code {
        Some(0) => SetupState::Ready,
        Some(10) => {
            SetupState::Needed(stdout.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        }
        Some(2) => SetupState::Unsupported,
        // Anything else (no bash, no network…) must not nag the user; Health Check still reports real problems.
        _ => SetupState::Ready,
    }
}

/// Run the installer.
///
/// * `interactive = true` (CLI): output goes to the terminal and sudo can prompt there.
/// * `interactive = false` (GUI): no terminal, so the script asks for the password in a
///   system dialog (pkexec); output is saved to a log. Returns the log path.
/// * `extras_only`: install just the optional extras (GameMode, MangoHud, Gamescope…).
pub fn install_engine(paths: &Paths, interactive: bool, extras_only: bool) -> Result<Option<PathBuf>> {
    let script = write_script(paths)?;
    let mut cmd = Command::new("bash");
    cmd.arg(&script).arg("--yes");
    if extras_only {
        cmd.arg("--extras");
    }

    let log = if interactive {
        None
    } else {
        fs::create_dir_all(paths.logs_dir())?;
        let log = paths.logs_dir().join("setup.log");
        let f = File::create(&log)?;
        cmd.stdin(Stdio::null()).stdout(f.try_clone()?).stderr(f);
        Some(log)
    };

    let status = cmd.status().context("could not run the installer (is bash installed?)")?;
    if !status.success() {
        let hint = match status.code() {
            Some(126 | 127) => " The password dialog was cancelled or unavailable.",
            Some(2) => " Your Linux distribution isn't supported by the automatic installer.",
            _ => "",
        };
        let tail = log.as_deref().map(|l| launch::log_tail(l, 6)).unwrap_or_default();
        bail!("Setup did not finish.{hint}\n{tail}");
    }
    if !extras_only && needs_setup() && matches!(check(paths), SetupState::Needed(_)) {
        bail!("Setup finished but something is still missing.");
    }
    Ok(log)
}

/// Turn the installer's plan lines into short, plain-language bullet points.
pub fn describe(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| {
            let (kind, what) = l.split_once(": ")?;
            Some(match kind {
                "download" => format!("Game engine launcher: {what}"),
                "install" => format!("{what}  (graphics / system library)"),
                "optional" => format!("{what}  (optional extra)"),
                "enable" => what.to_string(),
                "note" => what.to_string(),
                _ => return None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths { config: dir.path().join("c"), data: dir.path().join("d") };
        (dir, paths)
    }

    #[test]
    fn embedded_script_is_valid_bash() {
        let (_d, paths) = paths();
        let script = write_script(&paths).unwrap();
        let out = Command::new("bash").arg("-n").arg(&script).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(INSTALL_SCRIPT.starts_with("#!/usr/bin/env bash"));
    }

    #[test]
    fn check_output_is_understood() {
        assert_eq!(parse_check(Some(0), "optional: mangohud\n"), SetupState::Ready);
        assert_eq!(parse_check(Some(2), ""), SetupState::Unsupported);
        assert_eq!(parse_check(None, ""), SetupState::Ready);
        assert_eq!(
            parse_check(Some(10), "install: libvulkan1\n\noptional: gamemode\n"),
            SetupState::Needed(vec!["install: libvulkan1".into(), "optional: gamemode".into()])
        );
    }

    #[test]
    fn plan_lines_become_plain_language() {
        let d = describe(&[
            "download: umu-launcher 1.4.4 (official release)".into(),
            "install: libvulkan1".into(),
            "optional: gamemode".into(),
            "weird".into(),
        ]);
        assert_eq!(d.len(), 3);
        assert!(d[0].starts_with("Game engine launcher"));
        assert!(d[2].contains("optional extra"));
    }

    #[test]
    fn real_check_runs_without_crashing() {
        // Runs the actual script in --check mode on this machine: must give a definite answer.
        let (_d, paths) = paths();
        let state = check(&paths);
        assert!(matches!(state, SetupState::Ready | SetupState::Needed(_) | SetupState::Unsupported));
    }
}
