use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GamescopeConfig {
    /// Output width/height (what you see). `None` = gamescope default.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Frame cap. Match your monitor refresh rate for smooth frame pacing.
    pub fps_limit: Option<u32>,
    pub fullscreen: bool,
}

/// One game = one TOML file = one private Wine prefix.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Game {
    /// Stable id (slug). Also the folder name of the prefix.
    pub id: String,
    pub name: String,
    pub exe: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    /// umu GAMEID. `umu-<steam appid>` makes umu apply that game's known
    /// fixes automatically. `umu-0` means "no known fixes".
    #[serde(default = "default_game_id")]
    pub game_id: String,
    /// Pinned Proton: a directory, or a name like `GE-Proton` / `UMU-Latest`.
    /// `None` lets umu use its default (UMU-Proton). Pin it once a game works.
    #[serde(default)]
    pub proton: Option<String>,
    #[serde(default)]
    pub gamemode: bool,
    #[serde(default)]
    pub mangohud: bool,
    #[serde(default)]
    pub gamescope: Option<GamescopeConfig>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

fn default_game_id() -> String {
    "umu-0".into()
}

impl Game {
    pub fn new(name: &str, exe: PathBuf) -> Self {
        Self {
            id: slugify(name),
            name: name.to_string(),
            exe,
            args: vec![],
            game_id: default_game_id(),
            proton: None,
            gamemode: true,
            mangohud: false,
            gamescope: None,
            env: BTreeMap::new(),
        }
    }

    pub fn prefix(&self, paths: &Paths) -> PathBuf {
        paths.prefixes_dir().join(&self.id)
    }
}

/// Turn "FIFA 23: Ultimate!" into "fifa-23-ultimate".
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "game".into()
    } else {
        out
    }
}

/// Derive a nice display name from an exe path: `/x/My_Game/setup.exe` -> "My Game".
pub fn name_from_exe(exe: &Path) -> String {
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("Game");
    let generic = ["setup", "install", "installer", "launcher", "game", "start", "run"];
    let pick = if generic.contains(&stem.to_ascii_lowercase().as_str()) {
        exe.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or(stem)
    } else {
        stem
    };
    pick.replace(['_', '-'], " ").trim().to_string()
}

#[derive(Clone, Debug)]
pub struct Library {
    pub paths: Paths,
}

impl Library {
    pub fn open() -> Result<Self> {
        Self::with_paths(Paths::discover())
    }

    pub fn with_paths(paths: Paths) -> Result<Self> {
        fs::create_dir_all(paths.games_dir())?;
        fs::create_dir_all(paths.prefixes_dir())?;
        fs::create_dir_all(paths.logs_dir())?;
        Ok(Self { paths })
    }

    fn file(&self, id: &str) -> PathBuf {
        self.paths.games_dir().join(format!("{id}.toml"))
    }

    pub fn list(&self) -> Result<Vec<Game>> {
        let mut games = Vec::new();
        for entry in fs::read_dir(self.paths.games_dir())? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "toml") {
                // A broken file must never hide the rest of the library.
                match fs::read_to_string(&path)
                    .map_err(anyhow::Error::from)
                    .and_then(|s| toml::from_str::<Game>(&s).map_err(Into::into))
                {
                    Ok(g) => games.push(g),
                    Err(e) => eprintln!("warning: skipping {}: {e}", path.display()),
                }
            }
        }
        games.sort_by_key(|g| g.name.to_lowercase());
        Ok(games)
    }

    pub fn get(&self, id: &str) -> Result<Game> {
        let text = fs::read_to_string(self.file(id))
            .with_context(|| format!("no game with id '{id}' (try `winbridge list`)"))?;
        Ok(toml::from_str(&text)?)
    }

    /// Add a new game. Picks a unique id if the slug is already taken.
    pub fn add(&self, mut game: Game) -> Result<Game> {
        if !game.exe.is_file() {
            bail!("'{}' is not a file", game.exe.display());
        }
        let base = game.id.clone();
        let mut n = 2;
        while self.file(&game.id).exists() {
            game.id = format!("{base}-{n}");
            n += 1;
        }
        self.save(&game)?;
        Ok(game)
    }

    /// Write atomically so a crash can't leave a half-written config.
    pub fn save(&self, game: &Game) -> Result<()> {
        let path = self.file(&game.id);
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, toml::to_string_pretty(game)?)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Remove the game entry. The prefix (saves!) is only deleted on request.
    pub fn remove(&self, id: &str, delete_prefix: bool) -> Result<()> {
        let game = self.get(id)?;
        fs::remove_file(self.file(id))?;
        if delete_prefix {
            let prefix = game.prefix(&self.paths);
            if prefix.starts_with(self.paths.prefixes_dir()) && prefix.exists() {
                fs::remove_dir_all(prefix)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths { config: dir.path().join("c"), data: dir.path().join("d") };
        (dir, Library::with_paths(paths).unwrap())
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("FIFA 23: Ultimate!"), "fifa-23-ultimate");
        assert_eq!(slugify("!!!"), "game");
    }

    #[test]
    fn names_from_exe() {
        assert_eq!(name_from_exe(Path::new("/g/Cool_Game/setup.exe")), "Cool Game");
        assert_eq!(name_from_exe(Path::new("/g/x/Hades.exe")), "Hades");
    }

    #[test]
    fn add_list_remove_roundtrip() {
        let (dir, lib) = lib();
        let exe = dir.path().join("a.exe");
        fs::write(&exe, b"MZ").unwrap();
        let a = lib.add(Game::new("Test Game", exe.clone())).unwrap();
        let b = lib.add(Game::new("Test Game", exe)).unwrap();
        assert_eq!(a.id, "test-game");
        assert_eq!(b.id, "test-game-2");
        assert_eq!(lib.list().unwrap().len(), 2);
        lib.remove("test-game", false).unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
    }

    #[test]
    fn broken_file_does_not_hide_library() {
        let (dir, lib) = lib();
        let exe = dir.path().join("a.exe");
        fs::write(&exe, b"MZ").unwrap();
        lib.add(Game::new("Ok", exe)).unwrap();
        fs::write(lib.paths.games_dir().join("bad.toml"), "not = [valid").unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
    }

    #[test]
    fn rejects_missing_exe() {
        let (_d, lib) = lib();
        assert!(lib.add(Game::new("Nope", "/does/not/exist.exe".into())).is_err());
    }
}
