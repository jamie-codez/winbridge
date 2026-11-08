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
        check_optional("GameMode", "gamemoded", "Boosts CPU speed while you play.", "sudo apt install gamemode"),
        check_optional(
            "Gamescope",
            "gamescope",
            "Helps with smooth frame pacing and scaling.",
            "sudo apt install gamescope",
        ),
        check_optional("MangoHud", "mangohud", "Shows an FPS counter on screen.", "sudo apt install mangohud"),
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
            Some("sudo apt install mesa-vulkan-drivers libvulkan1 vulkan-tools"),
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
            Some("sudo dpkg --add-architecture i386 && sudo apt update && sudo apt install libvulkan1:i386 mesa-vulkan-drivers:i386"),
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

fn check_optional(title: &'static str, bin: &str, why: &str, fix: &str) -> Check {
    if which(bin).is_some() {
        check(title, Status::Ok, format!("Installed. {why}"), None)
    } else {
        check(title, Status::Info, format!("Optional, not installed. {why}"), Some(fix))
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
