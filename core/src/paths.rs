use std::path::PathBuf;

/// Root directories used by WinBridge. Overridable with `WINBRIDGE_HOME`
/// (used by tests and for portable installs).
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
}

impl Paths {
    pub fn discover() -> Self {
        if let Some(home) = std::env::var_os("WINBRIDGE_HOME") {
            let home = PathBuf::from(home);
            return Self { config: home.join("config"), data: home.join("data") };
        }
        let dirs = directories::ProjectDirs::from("", "", "winbridge").expect("could not determine home directory");
        Self { config: dirs.config_dir().to_path_buf(), data: dirs.data_dir().to_path_buf() }
    }

    pub fn games_dir(&self) -> PathBuf {
        self.config.join("games")
    }
    pub fn prefixes_dir(&self) -> PathBuf {
        self.data.join("prefixes")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.data.join("logs")
    }
}
