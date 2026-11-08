//! Health check. Every message is written for a non-technical reader and,
//! where something is wrong, includes the exact command that fixes it.

use std::{fs, path::Path, process::Command};

use crate::{controllers, launch::which, paths::Paths};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// Works, but something optional or recommended is missing.
    Warn,
    /// Games will not start until fixed.
    Fail,
    Info,
}

#[derive(Clone, Debug)]
pub struct Check {
    pub title: &'static str,
    pub status: Status,
    pub message: String,
    pub fix: Option<String>,
}

fn check(title: &'static str, status: Status, message: impl Into<String>, fix: Option<&str>) -> Check {
    Check { title, status, message: message.into(), fix: fix.map(str::to_string) }
}

pub fn run_checks(paths: &Paths) -> Vec<Check> {
    vec![
        check_umu(),
        check_gpu(),
        check_vulkan(),
        check_vulkan32(),
        check_map_count(),
        check_ntsync(),
        check_optional("GameMode", "gamemoded", "gamemode", "Boosts CPU speed while you play."),
        check_optional("Gamescope", "gamescope", "gamescope", "Helps with smooth frame pacing and scaling."),
        check_optional("MangoHud", "mangohud", "mangohud", "Shows an FPS counter on screen."),
        check_controllers(),
        check_disk(&paths.data),
    ]
}

/// True when nothing blocks games from starting.
pub fn is_ready(checks: &[Check]) -> bool {
    !checks.iter().any(|c| c.status == Status::Fail)
}

fn check_umu() -> Check {
    match which("umu-run") {
        Some(p) => check("Game engine (umu + Proton)", Status::Ok, format!("Found at {}.", p.display()), None),
        None => check(
            "Game engine (umu + Proton)",
            Status::Fail,
            "The game engine isn't installed, so games can't start yet. This is the one required piece.",
            Some("Press “Set up now” in WinBridge, or in a terminal run: winbridge setup   (developers: ./scripts/install-deps.sh)")),
    }
}

/// Reads GPU vendors from sysfs (no extra tools needed).
fn gpu_vendors() -> Vec<&'static str> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/drm") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("card") || name.contains('-') {
                continue;
            }
            if let Ok(id) = fs::read_to_string(e.path().join("device/vendor")) {
                match id.trim() {
                    "0x8086" => v.push("Intel"),
                    "0x1002" => v.push("AMD"),
                    "0x10de" => v.push("NVIDIA"),
                    _ => {}
                }
            }
        }
    }
    v
}

fn check_gpu() -> Check {
    let v = gpu_vendors();
    if v.is_empty() {
        return check("Graphics card", Status::Warn, "Couldn't identify your graphics card.", None);
    }
    let list = v.join(" + ");
    if v.iter().all(|x| *x == "Intel") {
        check(
            "Graphics card",
            Status::Info,
            format!("{list} graphics detected. Integrated graphics run lighter and older games well, but very demanding new games (Call of Duty, recent EA FC) will be slow or unplayable regardless of software."),
            None,
        )
    } else {
        check("Graphics card", Status::Ok, format!("{list} graphics detected."), None)
    }
}

fn first_existing(paths: &[&str]) -> Option<String> {
    paths.iter().find(|p| Path::new(p).exists()).map(|p| p.to_string())
}

/// The right fix for missing graphics support on any supported distribution: the setup knows the
/// package names for apt, dnf and pacman, so we point at it instead of hard-coding one manager.
const SETUP_FIX: &str = "Press “Install everything” in WinBridge, or run in a terminal: winbridge setup";

fn check_vulkan() -> Check {
    let lib = first_existing(&[
        "/usr/lib/x86_64-linux-gnu/libvulkan.so.1",
        "/usr/lib64/libvulkan.so.1",
        "/usr/lib/libvulkan.so.1",
    ]);
    let icd = ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d"].iter().any(|d| {
        fs::read_dir(d)
            .map(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "json")))
            .unwrap_or(false)
    });
    match (lib, icd) {
        (Some(_), true) => {
            check("Graphics driver (Vulkan)", Status::Ok, "Vulkan is installed. Most games need it.", None)
        }
        _ => check(
            "Graphics driver (Vulkan)",
            Status::Fail,
            "Vulkan graphics support is missing. Windows games are translated to Vulkan to run on Linux.",
            Some(SETUP_FIX),
        ),
    }
}

fn check_vulkan32() -> Check {
    let lib = first_existing(&["/usr/lib/i386-linux-gnu/libvulkan.so.1", "/usr/lib32/libvulkan.so.1"]);
    match lib {
        Some(_) => check("32-bit graphics support", Status::Ok, "Installed. Needed by older 32-bit games.", None),
        None => check(
            "32-bit graphics support",
            Status::Warn,
            "Missing. Newer 64-bit games are fine, but older 32-bit games may not start.",
            Some(SETUP_FIX),
        ),
    }
}

fn check_map_count() -> Check {
    let n: u64 = fs::read_to_string("/proc/sys/vm/max_map_count").ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    if n >= 1_048_576 {
        check("Memory limit", Status::Ok, "Set high enough for demanding games.", None)
    } else {
        check(
            "Memory limit",
            Status::Warn,
            format!("Your system limit ({n}) is low; some big games crash at startup because of it."),
            Some("echo 'vm.max_map_count=1048576' | sudo tee /etc/sysctl.d/99-winbridge.conf && sudo sysctl --system"),
        )
    }
}

fn check_ntsync() -> Check {
    if Path::new("/dev/ntsync").exists() {
        check("Fast synchronisation (NTSYNC)", Status::Ok, "Your kernel supports it, which reduces stutter.", None)
    } else {
        check(
            "Fast synchronisation (NTSYNC)",
            Status::Info,
            "Not available on this kernel. Games still run (they fall back to FSYNC).",
            Some("sudo modprobe ntsync"),
        )
    }
}

/// The system's package manager, if we know how to talk to it.
fn package_manager() -> Option<&'static str> {
    ["apt-get", "dnf", "pacman"].into_iter().find(|c| which(c).is_some())
}

/// Does `apt-cache policy` output show an installable candidate version?
fn candidate_available(policy: &str) -> bool {
    policy
        .lines()
        .find_map(|l| l.trim().strip_prefix("Candidate:"))
        .map(|c| {
            let c = c.trim();
            !c.is_empty() && c != "(none)"
        })
        .unwrap_or(false)
}

/// Can `pkg` really be installed from this distribution's software sources? Only apt can tell us
/// cheaply; for the others we assume yes.
fn installable(pm: &str, pkg: &str) -> bool {
    match pm {
        "apt-get" => Command::new("apt-cache")
            .args(["policy", pkg])
            .output()
            .map(|o| candidate_available(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or(true),
        _ => true,
    }
}

fn install_command(pm: &str, pkg: &str) -> String {
    match pm {
        "apt-get" => format!("sudo apt install {pkg}"),
        "dnf" => format!("sudo dnf install {pkg}"),
        _ => format!("sudo pacman -S {pkg}"),
    }
}

/// An optional extra. We only suggest an install command when the distribution actually offers
/// the package, so people are never told to run something that can't work.
fn check_optional(title: &'static str, bin: &str, pkg: &str, why: &str) -> Check {
    if which(bin).is_some() {
        return check(title, Status::Ok, format!("Installed. {why}"), None);
    }
    match package_manager() {
        Some(pm) if installable(pm, pkg) => {
            check(title, Status::Info, format!("Optional, not installed. {why}"), Some(&install_command(pm, pkg)))
        }
        _ => check(
            title,
            Status::Info,
            format!("Optional. Not available from your system's software sources, so it can't be installed from here. {why} Games run fine without it."),
            None,
        ),
    }
}

fn check_controllers() -> Check {
    let pads = controllers::detect();
    if pads.is_empty() {
        return check(
            "Controllers",
            Status::Info,
            "No controller detected. Plug one in, or pair it in Bluetooth settings, and it will appear here.",
            None,
        );
    }
    let list: Vec<String> = pads
        .iter()
        .map(|p| {
            let battery = p.battery.as_ref().map(|b| format!(", battery {}%", b.percent)).unwrap_or_default();
            format!("{} {} ({}{})", p.kind.label(), p.name, p.connection.label(), battery)
        })
        .collect();
    let problems: Vec<&str> = pads.iter().flat_map(|p| p.hints.iter().map(String::as_str)).collect();
    if problems.is_empty() {
        check("Controllers", Status::Ok, list.join("; "), None)
    } else {
        check("Controllers", Status::Warn, format!("{}. {}", list.join("; "), problems.join(" ")), None)
    }
}

fn check_disk(dir: &Path) -> Check {
    // `df` avoids needing a libc dependency just for statvfs.
    let probe = if dir.exists() {
        dir.to_path_buf()
    } else {
        dir.ancestors().find(|p| p.exists()).unwrap_or(Path::new("/")).to_path_buf()
    };
    let avail_gb = Command::new("df")
        .args(["-Pk"])
        .arg(&probe)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().nth(1).and_then(|l| l.split_whitespace().nth(3).and_then(|k| k.parse::<u64>().ok())))
        .map(|kb| kb / 1024 / 1024);
    match avail_gb {
        Some(g) if g < 20 => {
            check("Disk space", Status::Warn, format!("Only {g} GB free. Modern games need 50 GB or more."), None)
        }
        Some(g) => check("Disk space", Status::Ok, format!("{g} GB free."), None),
        None => check("Disk space", Status::Info, "Couldn't measure free space.", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_fix_is_not_tied_to_one_package_manager() {
        for word in ["apt", "dpkg", "dnf", "pacman"] {
            assert!(!SETUP_FIX.contains(word), "the fix would be wrong on other distributions: {SETUP_FIX}");
        }
        assert!(SETUP_FIX.contains("winbridge setup"));
    }

    #[test]
    fn apt_candidate_parsing() {
        assert!(!candidate_available("gamescope:\n  Installed: (none)\n  Candidate: (none)\n"));
        assert!(candidate_available("mangohud:\n  Installed: (none)\n  Candidate: 0.6.9.1-2build1\n"));
        assert!(!candidate_available(""));
    }

    #[test]
    fn explanations_never_contain_commands() {
        // Regression: arguments to check_optional were once mixed up, putting a command into the message.
        let c = check_optional(
            "X",
            "definitely-not-a-real-binary-xyz",
            "definitely-not-a-real-package-xyz",
            "Does things.",
        );
        assert!(!c.message.contains("sudo"), "{}", c.message);
        if let Some(fix) = &c.fix {
            assert!(fix.starts_with("sudo ") && fix.ends_with("definitely-not-a-real-package-xyz"), "{fix}");
        } else {
            assert!(c.message.contains("can't be installed from here"), "{}", c.message);
        }
    }

    #[test]
    fn install_commands_match_the_package_manager() {
        assert_eq!(install_command("apt-get", "x"), "sudo apt install x");
        assert_eq!(install_command("dnf", "x"), "sudo dnf install x");
        assert_eq!(install_command("pacman", "x"), "sudo pacman -S x");
    }
}
