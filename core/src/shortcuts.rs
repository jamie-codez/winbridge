//! Desktop shortcuts: put a game in the normal Linux applications menu (and optionally on the
//! desktop) so it can be started like any other program, without opening WinBridge first.
//!
//! A shortcut is a freedesktop `.desktop` file that runs `winbridge run <game id>`.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result};

use crate::{config::Game, paths::Paths};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The applications menu / launcher (`~/.local/share/applications`).
    Menu,
    /// A file on the desktop (`~/Desktop`).
    Desktop,
}

/// Where shortcut files go.
#[derive(Clone, Debug)]
pub struct Dirs {
    pub menu: PathBuf,
    pub desktop: Option<PathBuf>,
}

impl Dirs {
    /// The real folders when WinBridge uses its real data folders; otherwise a private sandbox inside
    /// the data folder. So tests and `WINBRIDGE_HOME` experiments never create, change or delete the
    /// user's real menu entries.
    pub fn for_paths(paths: &Paths) -> Self {
        let real = directories::ProjectDirs::from("", "", "winbridge").is_some_and(|d| d.config_dir() == paths.config);
        if real {
            Self {
                menu: directories::BaseDirs::new()
                    .map(|b| b.data_dir().join("applications"))
                    .unwrap_or_else(|| PathBuf::from(".local/share/applications")),
                desktop: directories::UserDirs::new().and_then(|u| u.desktop_dir().map(Path::to_path_buf)),
            }
        } else {
            Self { menu: paths.data.join("shortcuts/menu"), desktop: Some(paths.data.join("shortcuts/desktop")) }
        }
    }

    fn dir(&self, kind: Kind) -> Option<&Path> {
        match kind {
            Kind::Menu => Some(&self.menu),
            Kind::Desktop => self.desktop.as_deref(),
        }
    }

    pub fn path(&self, id: &str, kind: Kind) -> Option<PathBuf> {
        self.dir(kind).map(|d| d.join(format!("winbridge-{id}.desktop")))
    }
}

/// The command a shortcut runs: the `winbridge` program installed next to the running program,
/// else whatever `winbridge` is on the PATH.
pub fn run_command() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("winbridge")))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "winbridge".to_string())
}

/// Quote one argument of an `Exec=` line as the Desktop Entry specification requires.
fn exec_quote(arg: &str) -> String {
    let special = |c: char| c.is_whitespace() || "\"'\\><~|&;$*?#()`%".contains(c);
    if arg.is_empty() || arg.chars().any(special) {
        let mut out = String::from("\"");
        for c in arg.chars() {
            match c {
                '"' | '`' | '$' | '\\' => {
                    out.push('\\');
                    out.push(c);
                }
                '%' => out.push_str("%%"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    } else {
        arg.to_string()
    }
}

/// Text value for a `.desktop` file: no control characters, backslashes escaped.
fn text_value(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().replace('\\', "\\\\")
}

/// The contents of the shortcut file for `game`.
pub fn render(game: &Game, run_cmd: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name={name}\n\
         Comment=Windows game, started with WinBridge\n\
         Exec={cmd} run {id}\n\
         Icon=winbridge\n\
         Terminal=false\n\
         Categories=Game;\n\
         Keywords=windows;game;winbridge;\n\
         X-WinBridge-Id={id}\n",
        name = text_value(&game.name),
        cmd = exec_quote(run_cmd),
        id = exec_quote(&game.id),
    )
}

/// Create (or refresh) the shortcut. Returns the file that was written.
pub fn install(game: &Game, dirs: &Dirs, kind: Kind, run_cmd: &str) -> Result<PathBuf> {
    let path = dirs.path(&game.id, kind).context("this system has no desktop folder")?;
    let dir = path.parent().context("bad shortcut path")?;
    fs::create_dir_all(dir)?;
    // Write atomically so a menu never sees half a file.
    let tmp = path.with_extension("desktop.tmp");
    fs::write(&tmp, render(game, run_cmd))?;
    // Menu entries are plain data; desktop files must be executable to be launchable by double click.
    fs::set_permissions(&tmp, fs::Permissions::from_mode(if kind == Kind::Desktop { 0o755 } else { 0o644 }))?;
    fs::rename(&tmp, &path)?;
    if kind == Kind::Desktop {
        // GNOME only runs desktop files it has been told to trust. Best effort; harmless elsewhere.
        let _ = Command::new("gio")
            .args(["set"])
            .arg(&path)
            .arg("metadata::trusted")
            .arg("true")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    Ok(path)
}

/// Delete the shortcut. Returns whether a file was actually removed.
pub fn remove(id: &str, dirs: &Dirs, kind: Kind) -> Result<bool> {
    match dirs.path(id, kind) {
        Some(p) if p.exists() => {
            fs::remove_file(&p).with_context(|| format!("could not remove {}", p.display()))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub fn exists(id: &str, dirs: &Dirs, kind: Kind) -> bool {
    dirs.path(id, kind).is_some_and(|p| p.exists())
}

/// Make the files match the game's settings: create or refresh what is switched on, remove what is off.
pub fn sync(game: &Game, dirs: &Dirs, run_cmd: &str) -> Result<()> {
    for (kind, wanted) in [(Kind::Menu, game.menu_shortcut), (Kind::Desktop, game.desktop_shortcut)] {
        if wanted {
            install(game, dirs, kind, run_cmd)?;
        } else {
            remove(&game.id, dirs, kind)?;
        }
    }
    Ok(())
}

/// Remove every shortcut for a game (used when the game is removed from the library).
pub fn remove_all(id: &str, dirs: &Dirs) {
    for kind in [Kind::Menu, Kind::Desktop] {
        let _ = remove(id, dirs, kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path) -> Dirs {
        Dirs { menu: root.join("apps"), desktop: Some(root.join("Desktop")) }
    }

    fn game() -> Game {
        let mut g = Game::new("FIFA 14", PathBuf::from("/games/fifa14.exe"));
        g.menu_shortcut = true;
        g
    }

    #[test]
    fn rendered_entry_has_the_fields_launchers_need() {
        let text = render(&game(), "/usr/bin/winbridge");
        for line in [
            "[Desktop Entry]",
            "Type=Application",
            "Name=FIFA 14",
            "Exec=/usr/bin/winbridge run fifa-14",
            "Terminal=false",
            "Categories=Game;",
            "X-WinBridge-Id=fifa-14",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line:?} in:\n{text}");
        }
    }

    #[test]
    fn exec_paths_with_spaces_and_specials_are_quoted() {
        assert_eq!(exec_quote("/usr/bin/winbridge"), "/usr/bin/winbridge");
        assert_eq!(exec_quote("/my apps/winbridge"), "\"/my apps/winbridge\"");
        assert_eq!(exec_quote("a$b"), "\"a\\$b\"");
        assert_eq!(exec_quote("100%"), "\"100%%\"");
    }

    #[test]
    fn odd_names_cannot_break_the_file() {
        let mut g = game();
        g.name = "Line one\nExec=evil\\x".into();
        let text = render(&g, "winbridge");
        assert_eq!(text.lines().filter(|l| l.starts_with("Exec=")).count(), 1, "{text}");
        assert!(text.contains("Name=Line oneExec=evil\\\\x"));
    }

    #[test]
    fn install_and_remove_roundtrip_with_right_permissions() {
        let d = tempfile::tempdir().unwrap();
        let dirs = dirs(d.path());
        let g = game();
        let menu = install(&g, &dirs, Kind::Menu, "winbridge").unwrap();
        let desk = install(&g, &dirs, Kind::Desktop, "winbridge").unwrap();
        assert_eq!(fs::metadata(&menu).unwrap().permissions().mode() & 0o777, 0o644);
        assert_eq!(fs::metadata(&desk).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(exists("fifa-14", &dirs, Kind::Menu) && exists("fifa-14", &dirs, Kind::Desktop));
        assert!(remove("fifa-14", &dirs, Kind::Menu).unwrap());
        assert!(!remove("fifa-14", &dirs, Kind::Menu).unwrap(), "second removal finds nothing");
        remove_all("fifa-14", &dirs);
        assert!(!exists("fifa-14", &dirs, Kind::Desktop));
    }

    #[test]
    fn sync_follows_the_settings_and_refreshes_names() {
        let d = tempfile::tempdir().unwrap();
        let dirs = dirs(d.path());
        let mut g = game();
        sync(&g, &dirs, "winbridge").unwrap();
        assert!(exists(&g.id, &dirs, Kind::Menu) && !exists(&g.id, &dirs, Kind::Desktop));
        g.desktop_shortcut = true;
        g.menu_shortcut = false;
        g.name = "FIFA 14 (renamed)".into();
        sync(&g, &dirs, "winbridge").unwrap();
        assert!(!exists(&g.id, &dirs, Kind::Menu) && exists(&g.id, &dirs, Kind::Desktop));
        let text = fs::read_to_string(dirs.path(&g.id, Kind::Desktop).unwrap()).unwrap();
        assert!(text.contains("Name=FIFA 14 (renamed)"));
    }

    #[test]
    fn no_desktop_folder_is_a_clear_error_not_a_panic() {
        let d = tempfile::tempdir().unwrap();
        let dirs = Dirs { menu: d.path().join("apps"), desktop: None };
        assert!(install(&game(), &dirs, Kind::Desktop, "winbridge").is_err());
    }

    #[test]
    fn sandboxed_unless_using_the_real_data_folders() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths { config: d.path().join("c"), data: d.path().join("d") };
        let dirs = Dirs::for_paths(&paths);
        assert!(dirs.menu.starts_with(d.path()), "test paths must never reach the real applications menu");
        assert!(dirs.desktop.unwrap().starts_with(d.path()));
    }

    /// If the desktop-file-validate tool is installed, the generated file must pass it.
    #[test]
    fn entry_passes_desktop_file_validate_when_available() {
        let d = tempfile::tempdir().unwrap();
        let dirs = dirs(d.path());
        let path = install(&game(), &dirs, Kind::Menu, "/bin/true").unwrap();
        match Command::new("desktop-file-validate").arg(&path).output() {
            Ok(out) => {
                let msg = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
                // Warnings about the icon theme (winbridge may not be installed here) are fine; errors are not.
                assert!(!msg.contains("error:"), "{msg}");
            }
            Err(_) => eprintln!("desktop-file-validate not installed; skipping"),
        }
    }
}
