use std::{error::Error, fmt, fs, io, path::Path};

use crate::game_state::GameState;

const SAVE_PATH: &str = "drillgame-save.json";

pub fn save_game(game: &GameState) -> Result<(), SaveError> {
    let json = serde_json::to_string_pretty(game).map_err(SaveError::Serialize)?;
    fs::write(SAVE_PATH, json).map_err(SaveError::Io)
}

pub fn load_game() -> Result<GameState, SaveError> {
    let json = fs::read_to_string(SAVE_PATH).map_err(SaveError::Io)?;
    serde_json::from_str(&json).map_err(SaveError::Serialize)
}

#[must_use]
pub fn save_exists() -> bool {
    Path::new(SAVE_PATH).exists()
}

#[derive(Debug)]
pub enum SaveError {
    Io(io::Error),
    Serialize(serde_json::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Serialize(error) => write!(formatter, "serialization error: {error}"),
        }
    }
}

impl Error for SaveError {}

#[cfg(test)]
mod tests {
    use crate::game_state::GameState;

    #[test]
    fn game_state_round_trips_through_json() {
        let game = GameState::new();
        let json = serde_json::to_string(&game).expect("serialize game");
        let loaded: GameState = serde_json::from_str(&json).expect("deserialize game");

        assert_eq!(loaded.player.cargo_capacity, game.player.cargo_capacity);
        assert_eq!(loaded.terrain.width(), game.terrain.width());
    }
}
