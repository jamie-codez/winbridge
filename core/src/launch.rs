use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write as _,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};

use crate::{config::Game, paths::Paths};

/// Find an executable on `PATH` (also checks /usr/games, where Debian/Ubuntu put gamemode).
pub fn which(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.push("/usr/games".into());
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// Everything needed to start a game, computed without side effects so it can be tested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    /// Ask the GameMode daemon for a performance boost while the game runs. Done from the
    /// host (`gamemoded -r`) because the `gamemoderun` wrapper can't work inside Proton's container.
    pub gamemode: bool,
    /// Things the user asked for that we had to skip (e.g. MangoHud not installed).
    pub warnings: Vec<String>,
}

/// Environment variable names that probably hold a credential. Their values never reach a log.
fn is_sensitive(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["TOKEN", "KEY", "SECRET", "PASS", "CRED", "AUTH", "COOKIE"].iter().any(|w| n.contains(w))
}

impl LaunchPlan {
    /// Human-readable command line, written to the log for troubleshooting.
    pub fn display(&self) -> String {
        let env: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| {
                if is_sensitive(k) {
                    format!("{k}=<hidden>")
                } else if k == "SDL_GAMECONTROLLERCONFIG" {
                    format!("{k}=<{} controller mapping(s)>", v.lines().count())
                } else {
                    format!("{k}={v:?}")
                }
            })
            .collect();
        format!("{} {} {}", env.join(" "), self.program, self.args.join(" "))
    }
}

/// Build the launch command:
/// `gamescope [..] -- mangohud umu-run game.exe` (+ a host-side GameMode request)
///
/// `have` answers "is this tool installed?" so tests don't depend on the machine.
pub fn plan(game: &Game, paths: &Paths, have: &dyn Fn(&str) -> bool) -> Result<LaunchPlan> {
    if !have("umu-run") {
        bail!(
            "umu-launcher is not installed, so games can't be started yet. \
             Open the Health Check for install steps."
        );
    }
    if !game.exe.is_file() {
        bail!("The game file '{}' no longer exists. Was the drive unplugged?", game.exe.display());
    }

    let mut warnings = Vec::new();
    let mut chain: Vec<String> = Vec::new();

    let mut gamemode = false;
    if game.gamemode {
        if have("gamemoded") {
            gamemode = true;
        } else {
            warnings.push("GameMode is turned on but not installed; running without it.".into());
        }
    }

    if let Some(gs) = &game.gamescope {
        if have("gamescope") {
            chain.push("gamescope".into());
            if gs.fullscreen {
                chain.push("-f".into());
            }
            if let Some(w) = gs.width {
                chain.extend(["-W".into(), w.to_string()]);
            }
            if let Some(h) = gs.height {
                chain.extend(["-H".into(), h.to_string()]);
            }
            if let Some(f) = gs.fps_limit {
                chain.extend(["-r".into(), f.to_string()]);
            }
            chain.push("--".into());
        } else {
            warnings.push("Gamescope is turned on but not installed; running without it.".into());
        }
    }

    if game.mangohud {
        if have("mangohud") {
            chain.push("mangohud".into());
        } else {
            warnings.push("MangoHud is turned on but not installed; running without it.".into());
        }
    }

    chain.push("umu-run".into());
    chain.push(game.exe.to_string_lossy().into_owned());
    chain.extend(game.args.iter().cloned());

    let mut env = BTreeMap::new();
    env.insert("WINEPREFIX".into(), game.prefix(paths).to_string_lossy().into_owned());
    env.insert("GAMEID".into(), game.game_id.clone());
    if let Some(p) = &game.proton {
        env.insert("PROTONPATH".into(), p.clone());
    }
    // Button mappings the user created for their controllers (see mapping.rs). Games read
    // controllers through SDL, which accepts extra mappings via this variable.
    let maps = crate::mapping::load_all(paths);
    if !maps.is_empty() {
        env.insert("SDL_GAMECONTROLLERCONFIG".into(), maps.join("\n"));
    }
    // User overrides win over our defaults.
    env.extend(game.env.clone());

    let cwd = game.exe.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let program = chain.remove(0);
    Ok(LaunchPlan { program, args: chain, env, cwd, gamemode, warnings })
}

/// A started game. The GUI/CLI waits on `child` and reads `log_path` if it fails.
pub struct RunningGame {
    pub child: Child,
    pub log_path: PathBuf,
    pub started: Instant,
    /// `gamemoded -r` companion; stopped when the game ends.
    _gamemode: Option<GameModeHelper>,
}

/// Asks the host's GameMode daemon for a performance boost for as long as it lives. Dropping it
/// stops the request, including when `spawn` fails after the helper has already started.
struct GameModeHelper(Child);

impl Drop for GameModeHelper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Clone, Debug)]
pub struct ExitInfo {
    pub success: bool,
    pub code: Option<i32>,
    pub seconds: u64,
}

impl RunningGame {
    pub fn wait(mut self) -> Result<(ExitInfo, PathBuf)> {
        let status: ExitStatus = self.child.wait()?;
        let info =
            ExitInfo { success: status.success(), code: status.code(), seconds: self.started.elapsed().as_secs() };
        Ok((info, self.log_path.clone()))
    }
}

pub fn spawn(game: &Game, paths: &Paths, plan: &LaunchPlan) -> Result<RunningGame> {
    fs::create_dir_all(game.prefix(paths))?;
    let logs = paths.logs_dir();
    fs::create_dir_all(&logs)?;
    // Logs can contain paths and game output: keep them to the owner.
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700))?;

    let (log_path, mut log) = create_log(&logs, &game.id)?;
    writeln!(log, "# WinBridge launch of '{}'\n# {}\n", game.name, plan.display())?;
    for w in &plan.warnings {
        writeln!(log, "# warning: {w}")?;
    }
    prune_logs(&logs, &game.id, 10, &log_path);

    // Everything that can fail is done before the GameMode helper starts, and the helper sits in a
    // guard, so a failed launch can never leave the performance request running.
    let stdout = Stdio::from(log.try_clone()?);
    let gamemode = if plan.gamemode {
        match Command::new("gamemoded")
            .arg("-r")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(h) => Some(GameModeHelper(h)),
            Err(e) => {
                writeln!(log, "# warning: could not start GameMode: {e}")?;
                None
            }
        }
    } else {
        None
    };

    let child = Command::new(&plan.program)
        .args(&plan.args)
        .envs(&plan.env)
        .current_dir(&plan.cwd)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::from(log))
        .spawn()
        .with_context(|| format!("could not start '{}'", plan.program))?; // `gamemode` drops (and stops) here on error
    Ok(RunningGame { child, log_path, started: Instant::now(), _gamemode: gamemode })
}

/// Create a brand-new log file `<id>-<unix millis>.log`, owner-readable only. Never truncates an
/// existing log: on a name clash (two launches in the same millisecond) it uses `<id>-<millis>_<n>.log`.
fn create_log(dir: &Path, id: &str) -> Result<(PathBuf, File)> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    for n in 0..1000u32 {
        let name = if n == 0 { format!("{id}-{millis}.log") } else { format!("{id}-{millis}_{n}.log") };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    bail!("could not create a unique log file in {}", dir.display())
}

/// Sort key of a log file name for `id`, or None if it isn't one of this game's logs. The part after
/// `<id>-` must be `<digits>` or `<digits>_<digits>`, so `game-2-123.log` is NOT a log of `game`.
fn log_key(file_name: &str, id: &str) -> Option<(u128, u32)> {
    let rest = file_name.strip_prefix(id)?.strip_prefix('-')?.strip_suffix(".log")?;
    let (stamp, n) = match rest.split_once('_') {
        Some((a, b)) => (a, b),
        None => (rest, "0"),
    };
    let all_digits = |t: &str| !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit());
    if all_digits(stamp) && all_digits(n) {
        Some((stamp.parse().ok()?, n.parse().ok()?))
    } else {
        None
    }
}

/// Keep only the newest `keep` logs of this game, and never delete `current`.
fn prune_logs(dir: &Path, id: &str, keep: usize, current: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut logs: Vec<((u128, u32), PathBuf)> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter_map(|p| Some((log_key(p.file_name()?.to_str()?, id)?, p)))
        .collect();
    logs.sort();
    if logs.len() > keep {
        for (_, old) in &logs[..logs.len() - keep] {
            if old != current {
                let _ = fs::remove_file(old);
            }
        }
    }
}

/// Last `n` lines of a log, for showing the user what went wrong.
pub fn log_tail(path: &Path, n: usize) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// After running an installer, find the .exe files it created inside the game's prefix
/// so the user can pick the real game instead of hunting through hidden folders.
pub fn scan_installed_exes(prefix: &Path) -> Vec<PathBuf> {
    const SKIP: [&str; 9] =
        ["unins", "uninstall", "vc_redist", "vcredist", "dxsetup", "crashhandler", "crashreport", "dotnet", "setup"];
    let mut found = Vec::new();
    for root in ["drive_c/Program Files", "drive_c/Program Files (x86)", "drive_c/Games"] {
        walk(&prefix.join(root), 0, &mut found);
    }
    found.retain(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        !SKIP.iter().any(|s| name.contains(s))
            && !p.components().any(|c| {
                matches!(
                    c.as_os_str().to_str(),
                    Some("Common Files" | "Windows NT" | "Internet Explorer" | "Windows Media Player")
                )
            })
    });
    found.sort();
    found
}

fn walk(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, depth + 1, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GamescopeConfig;

    fn game(dir: &Path) -> Game {
        let exe = dir.join("g.exe");
        fs::write(&exe, b"MZ").unwrap();
        Game::new("My Game", exe)
    }
    fn paths(dir: &Path) -> Paths {
        Paths { config: dir.join("c"), data: dir.join("d") }
    }

    #[test]
    fn full_wrapper_order() {
        let d = tempfile::tempdir().unwrap();
        let mut g = game(d.path());
        g.gamemode = true;
        g.mangohud = true;
        g.gamescope =
            Some(GamescopeConfig { width: Some(1920), height: Some(1080), fps_limit: Some(60), fullscreen: true });
        g.args = vec!["-windowed".into()];
        let p = plan(&g, &paths(d.path()), &|_| true).unwrap();
        assert!(p.gamemode);
        assert_eq!(p.program, "gamescope");
        assert_eq!(
            p.args,
            ["-f", "-W", "1920", "-H", "1080", "-r", "60", "--", "mangohud", "umu-run"]
                .iter()
                .map(|s| s.to_string())
                .chain([g.exe.to_string_lossy().into_owned(), "-windowed".into()])
                .collect::<Vec<_>>()
        );
        assert_eq!(p.env["GAMEID"], "umu-0");
        assert!(p.env["WINEPREFIX"].ends_with("prefixes/my-game"));
    }

    #[test]
    fn missing_optional_tools_warn_but_run() {
        let d = tempfile::tempdir().unwrap();
        let mut g = game(d.path());
        g.mangohud = true;
        let p = plan(&g, &paths(d.path()), &|t| t == "umu-run").unwrap();
        assert_eq!(p.program, "umu-run");
        assert_eq!(p.warnings.len(), 2); // gamemode (default on) + mangohud
    }

    #[test]
    fn missing_umu_is_a_clear_error() {
        let d = tempfile::tempdir().unwrap();
        let g = game(d.path());
        let e = plan(&g, &paths(d.path()), &|_| false).unwrap_err().to_string();
        assert!(e.contains("umu-launcher"));
    }

    #[test]
    fn saved_controller_mappings_reach_the_game() {
        let d = tempfile::tempdir().unwrap();
        let g = game(d.path());
        let p = paths(d.path());
        assert!(!plan(&g, &p, &|_| true).unwrap().env.contains_key("SDL_GAMECONTROLLERCONFIG"));
        crate::mapping::save(&p, 1, 2, "guid1,Pad One,a:b0,platform:Linux").unwrap();
        crate::mapping::save(&p, 3, 4, "guid2,Pad Two,a:b1,platform:Linux").unwrap();
        let env = plan(&g, &p, &|_| true).unwrap().env;
        assert_eq!(
            env["SDL_GAMECONTROLLERCONFIG"],
            "guid1,Pad One,a:b0,platform:Linux\nguid2,Pad Two,a:b1,platform:Linux"
        );
    }

    #[test]
    fn pinned_proton_and_env_override() {
        let d = tempfile::tempdir().unwrap();
        let mut g = game(d.path());
        g.proton = Some("GE-Proton".into());
        g.env.insert("GAMEID".into(), "umu-1234".into());
        let p = plan(&g, &paths(d.path()), &|_| true).unwrap();
        assert_eq!(p.env["PROTONPATH"], "GE-Proton");
        assert_eq!(p.env["GAMEID"], "umu-1234");
    }

    #[test]
    fn scan_finds_game_not_uninstaller() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().join("drive_c/Program Files/Cool Game");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("cool.exe"), b"MZ").unwrap();
        fs::write(base.join("unins000.exe"), b"MZ").unwrap();
        let found = scan_installed_exes(d.path());
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("cool.exe"));
    }
    #[test]
    fn pruning_never_touches_another_games_logs() {
        let d = tempfile::tempdir().unwrap();
        // Games "game" and "game-2" (the id a second game named "Game" gets) share a prefix.
        for i in 0..12 {
            fs::write(d.path().join(format!("game-{}.log", 1000 + i)), "a").unwrap();
            fs::write(d.path().join(format!("game-2-{}.log", 1000 + i)), "b").unwrap();
        }
        let current = d.path().join("game-1011.log");
        prune_logs(d.path(), "game", 10, &current);
        let count = |prefix: &str| {
            fs::read_dir(d.path())
                .unwrap()
                .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with(prefix))
                .count()
        };
        assert_eq!(count("game-2-"), 12, "the other game lost logs");
        assert_eq!(count("game-1"), 10);
        assert!(current.exists(), "the newest log was deleted");
    }

    #[test]
    fn log_keys() {
        assert_eq!(log_key("game-1700.log", "game"), Some((1700, 0)));
        assert_eq!(log_key("game-1700_3.log", "game"), Some((1700, 3)));
        assert_eq!(log_key("game-2-1700.log", "game"), None);
        assert_eq!(log_key("game-2-1700.log", "game-2"), Some((1700, 0)));
        assert_eq!(log_key("other-1700.log", "game"), None);
    }

    #[test]
    fn rapid_launches_get_separate_logs_that_are_private() {
        let d = tempfile::tempdir().unwrap();
        let (p1, mut f1) = create_log(d.path(), "hades").unwrap();
        writeln!(f1, "first run failed").unwrap();
        // Force a collision by creating many logs at once; none may overwrite another.
        let mut paths = vec![p1.clone()];
        for _ in 0..50 {
            paths.push(create_log(d.path(), "hades").unwrap().0);
        }
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), 51, "two launches shared a log file");
        assert_eq!(fs::read_to_string(&p1).unwrap().trim(), "first run failed");
        assert_eq!(fs::metadata(&p1).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn credentials_do_not_reach_the_log() {
        let d = tempfile::tempdir().unwrap();
        let mut g = game(d.path());
        g.env.insert("EPIC_API_TOKEN".into(), "hunter2-secret".into());
        g.env.insert("DXVK_HUD".into(), "fps".into());
        let shown = plan(&g, &paths(d.path()), &|_| true).unwrap().display();
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("EPIC_API_TOKEN=<hidden>"));
        assert!(shown.contains("DXVK_HUD=\"fps\""));
    }

    #[test]
    fn gamemode_helper_stops_when_dropped() {
        // Models a failed game launch: the helper must not outlive its guard.
        let child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id();
        assert!(Path::new(&format!("/proc/{pid}")).exists());
        drop(GameModeHelper(child));
        assert!(!Path::new(&format!("/proc/{pid}")).exists(), "helper process was left running");
    }
}
