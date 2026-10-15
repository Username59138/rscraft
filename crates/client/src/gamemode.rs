//! Game modes, like vanilla's.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameMode {
    #[default]
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl GameMode {
    /// The order of the F3+F4 switcher.
    pub const SWITCHER: [GameMode; 4] = [GameMode::Creative, GameMode::Survival, GameMode::Adventure, GameMode::Spectator];

    pub fn name(self) -> &'static str {
        match self {
            GameMode::Survival => "Survival Mode",
            GameMode::Creative => "Creative Mode",
            GameMode::Adventure => "Adventure Mode",
            GameMode::Spectator => "Spectator Mode",
        }
    }

    /// The next mode in the switcher.
    pub fn next(self) -> GameMode {
        let i = Self::SWITCHER.iter().position(|&m| m == self).unwrap_or(0);
        Self::SWITCHER[(i + 1) % Self::SWITCHER.len()]
    }

    /// Double-tapping jump toggles flying.
    pub fn may_fly(self) -> bool {
        matches!(self, GameMode::Creative | GameMode::Spectator)
    }

    pub fn may_build(self) -> bool {
        matches!(self, GameMode::Survival | GameMode::Creative)
    }

    /// Blocks break at once, drop nothing, and placing uses no items.
    pub fn is_creative(self) -> bool {
        self == GameMode::Creative
    }

    pub fn is_spectator(self) -> bool {
        self == GameMode::Spectator
    }
}
