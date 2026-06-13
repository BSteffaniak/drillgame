use std::{error::Error, fmt, fs, io, path::Path};

use serde::{Deserialize, Serialize};

use crate::game_state::GameState;

const SAVE_PATH: &str = "drillgame-save.json";
const SAVE_VERSION: u32 = 2;

#[derive(Debug, Deserialize, Serialize)]
struct SaveFile {
    version: u32,
    game: GameState,
}

pub fn save_game(game: &GameState) -> Result<(), SaveError> {
    let save = SaveFile {
        version: SAVE_VERSION,
        game: game.clone_for_save(),
    };
    let json = serde_json::to_string_pretty(&save).map_err(SaveError::Serialize)?;
    fs::write(SAVE_PATH, json).map_err(SaveError::Io)
}

pub fn load_game() -> Result<GameState, SaveError> {
    let json = fs::read_to_string(SAVE_PATH).map_err(SaveError::Io)?;
    let save: SaveFile = serde_json::from_str(&json).map_err(SaveError::Serialize)?;
    if save.version != SAVE_VERSION {
        return Err(SaveError::UnsupportedVersion(save.version));
    }
    Ok(save.game)
}

#[must_use]
pub fn save_exists() -> bool {
    Path::new(SAVE_PATH).exists()
}

#[derive(Debug)]
pub enum SaveError {
    Io(io::Error),
    Serialize(serde_json::Error),
    UnsupportedVersion(u32),
}

impl fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Serialize(error) => write!(formatter, "serialization error: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported save version: {version}")
            }
        }
    }
}

impl Error for SaveError {}

#[cfg(test)]
mod tests {
    use crate::{game_state::GameState, save::SaveFile};

    #[test]
    fn game_state_round_trips_through_versioned_json() {
        let game = GameState::new();
        let save = SaveFile {
            version: 2,
            game: game.clone_for_save(),
        };
        let json = serde_json::to_string(&save).expect("serialize game");
        let loaded: SaveFile = serde_json::from_str(&json).expect("deserialize game");

        assert_eq!(loaded.version, 2);
        assert_eq!(
            loaded.game.player.cargo_capacity,
            game.player.cargo_capacity
        );
        assert_eq!(loaded.game.terrain.width(), game.terrain.width());
    }
}
