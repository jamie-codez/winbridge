use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use winbridge_core::{
    config::name_from_exe,
    controllers,
    doctor::{self, Status},
    launch::{self, which},
    Game, Library,
};

#[derive(Parser)]
#[command(name = "winbridge", version, about = "Run Windows games on Linux, the easy way")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check that your system is ready to run games
    Doctor,
    /// Install everything needed to play (graphics, 32-bit libraries, game engine, extras)
    Setup {
        /// Only install the optional extras (GameMode, MangoHud, Gamescope…)
        #[arg(long)]
        extras: bool,
    },
    /// Show connected controllers (--watch prints live button presses for 15 seconds)
    Controllers {
        #[arg(long)]
        watch: bool,
        /// How long --watch listens
        #[arg(long, default_value_t = 15)]
        seconds: u64,
    },
    /// List games in your library
    List,
    /// Add a game (or installer) .exe to your library
    Add {
        exe: PathBuf,
        /// Display name (default: guessed from the file)
        #[arg(long)]
        name: Option<String>,
        /// Steam app id, so known per-game fixes are applied (e.g. 1245620)
        #[arg(long)]
        steam_id: Option<u32>,
    },
    /// Start a game
    Run { id: String },
    /// Show the command that would run, without starting anything
    Plan { id: String },
    /// Find .exe files an installer created in a game's prefix
    Scan { id: String },
    /// Remove a game from the library
    Remove {
        id: String,
        /// Also delete its private Windows folder (this deletes save files!)
        #[arg(long)]
        delete_data: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let lib = Library::open()?;
    let have = |t: &str| which(t).is_some();

    match cli.cmd {
        Cmd::Doctor => {
            let checks = doctor::run_checks(&lib.paths);
            for c in &checks {
                let icon = match c.status {
                    Status::Ok => "✔",
                    Status::Warn => "⚠",
                    Status::Fail => "✖",
                    Status::Info => "ℹ",
                };
                println!("{icon} {}: {}", c.title, c.message);
                if let Some(fix) = &c.fix {
                    if c.status != Status::Ok {
                        println!("    fix: {fix}");
                    }
                }
            }
            if !doctor::is_ready(&checks) {
                std::process::exit(1);
            }
        }
        Cmd::Setup { extras } => {
            winbridge_core::setup::install_engine(&lib.paths, true, extras)?;
            println!("Setup complete.");
        }
        Cmd::Controllers { watch, seconds } => {
            let pads = controllers::detect();
            if pads.is_empty() {
                println!("No controllers detected.");
            }
            for p in &pads {
                let battery = p
                    .battery
                    .as_ref()
                    .map(|b| format!(", battery {}%{}", b.percent, if b.charging { " (charging)" } else { "" }))
                    .unwrap_or_default();
                println!("{} — {} — {}{}", p.name, p.kind.label(), p.connection.label(), battery);
                for h in &p.hints {
                    println!("    ⚠ {h}");
                }
            }
            if watch {
                let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let (tx, rx) = std::sync::mpsc::channel();
                // Listen on every controller (and every port of multi-port adapters).
                let mut listening = Vec::new();
                for (i, pad) in pads.iter().enumerate() {
                    let Some(path) = pad.event_path.clone() else { continue };
                    let (ptx, prx) = std::sync::mpsc::channel();
                    controllers::watch(path, stop.clone(), ptx, || {})?;
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        while let Ok(ev) = prx.recv() {
                            if tx.send((i, ev)).is_err() {
                                break;
                            }
                        }
                    });
                    listening.push(i);
                }
                println!("\nGO! Press buttons and move sticks now ({seconds} seconds)…");
                let end = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
                let mut count = 0;
                while let Some(left) = end.checked_duration_since(std::time::Instant::now()) {
                    if let Ok((i, (t, c, v))) = rx.recv_timeout(left) {
                        let pad = &pads[i];
                        count += 1;
                        match t {
                            controllers::EV_KEY => {
                                let label = controllers::button_name(pad.kind, c);
                                println!("[pad {}] {label}: {}", i + 1, if v != 0 { "pressed" } else { "released" });
                            }
                            controllers::EV_ABS => println!("[pad {}] axis {c:#x} = {v}", i + 1),
                            _ => {}
                        }
                    }
                }
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                println!("{count} input events received.");
            }
        }
        Cmd::List => {
            let games = lib.list()?;
            if games.is_empty() {
                println!("No games yet. Add one with: winbridge add /path/to/game.exe");
            }
            for g in games {
                println!("{:<24} {}", g.id, g.name);
            }
        }
        Cmd::Add { exe, name, steam_id } => {
            let exe = exe.canonicalize()?;
            let name = name.unwrap_or_else(|| name_from_exe(&exe));
            let mut game = Game::new(&name, exe);
            if let Some(id) = steam_id {
                game.game_id = format!("umu-{id}");
            }
            let game = lib.add(game)?;
            println!("Added '{}' (id: {}). Start it with: winbridge run {}", game.name, game.id, game.id);
        }
        Cmd::Plan { id } => {
            let game = lib.get(&id)?;
            let plan = launch::plan(&game, &lib.paths, &have)?;
            println!("{}", plan.display());
            for w in plan.warnings {
                println!("warning: {w}");
            }
        }
        Cmd::Run { id } => {
            let game = lib.get(&id)?;
            let plan = launch::plan(&game, &lib.paths, &have)?;
            for w in &plan.warnings {
                eprintln!("note: {w}");
            }
            println!("Starting {}…", game.name);
            let running = launch::spawn(&game, &lib.paths, &plan)?;
            let (info, log) = running.wait()?;
            if info.success {
                println!("Closed normally after {}s.", info.seconds);
            } else {
                eprintln!(
                    "{} closed unexpectedly (exit {:?}) after {}s. Log: {}",
                    game.name,
                    info.code,
                    info.seconds,
                    log.display()
                );
                eprintln!("--- last lines ---\n{}", launch::log_tail(&log, 15));
                std::process::exit(1);
            }
        }
        Cmd::Scan { id } => {
            let game = lib.get(&id)?;
            for p in launch::scan_installed_exes(&game.prefix(&lib.paths)) {
                println!("{}", p.display());
            }
        }
        Cmd::Remove { id, delete_data } => {
            lib.remove(&id, delete_data)?;
            println!("Removed {id}.");
        }
    }
    Ok(())
}
