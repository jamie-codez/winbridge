//! WinBridge core: everything the CLI and GUI share.
//!
//! Design rules (see README):
//! * One Wine prefix per game, never shared.
//! * Games run through `umu-run` (Proton inside Valve's Steam Linux Runtime
//!   container) so distro updates can't break them.
//! * Nothing auto-updates; runner versions are pinned per game.

pub mod config;
pub mod controllers;
pub mod doctor;
pub mod launch;
pub mod layout;
pub mod mapping;
pub mod paths;
pub mod sdl;
pub mod setup;

pub use config::{Game, GamescopeConfig, Library};
pub use doctor::{run_checks, Check, Status};
pub use launch::{LaunchPlan, RunningGame};
