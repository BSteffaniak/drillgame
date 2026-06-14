use std::{
    env,
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::game_state::GameState;

const SETTINGS_FILE_NAME: &str = "settings.json";

const SAVE_FILE_NAME: &str = "save.json";
const SAVE_SLOTS: usize = 3;
const SAVE_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct SettingsFile {
    pub master_volume: f32,
    pub fullscreen: bool,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            master_volume: 0.8,
            fullscreen: false,
        }
    }
}

pub fn save_settings(settings: SettingsFile) -> Result<(), SaveError> {
    let json = serde_json::to_string_pretty(&settings).map_err(SaveError::Serialize)?;
    write_state_file(settings_path(), json)
}

#[must_use]
pub fn load_settings() -> SettingsFile {
    let Ok(json) = fs::read_to_string(settings_path()) else {
        return SettingsFile::default();
    };
    serde_json::from_str(&json).unwrap_or_default()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SaveSlotMetadata {
    pub depth: i32,
    pub credits: u32,
    pub cargo_used: u32,
    pub cargo_capacity: u32,
    pub contracts_completed: u32,
    pub play_seconds: f32,
    pub total_earnings: u32,
    pub mode: String,
    pub deep_claim_unlocked: bool,
    pub modified_unix_seconds: Option<u64>,
    pub won_game: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct SaveFile {
    version: u32,
    game: GameState,
}

pub fn save_game(game: &GameState) -> Result<(), SaveError> {
    save_game_to_path(game, save_path())
}

pub fn save_game_slot(game: &GameState, slot: usize) -> Result<(), SaveError> {
    save_game_to_path(game, slot_path(slot))
}

fn save_game_to_path(game: &GameState, path: impl AsRef<Path>) -> Result<(), SaveError> {
    let save = SaveFile {
        version: SAVE_VERSION,
        game: game.clone_for_save(),
    };
    let json = serde_json::to_string_pretty(&save).map_err(SaveError::Serialize)?;
    write_state_file(path, json)
}

pub fn load_game_slot(slot: usize) -> Result<GameState, SaveError> {
    load_game_from_path(slot_path(slot))
}

pub fn load_game() -> Result<GameState, SaveError> {
    load_game_from_path(save_path())
}

fn load_game_from_path(path: impl AsRef<Path>) -> Result<GameState, SaveError> {
    let json = fs::read_to_string(path).map_err(SaveError::Io)?;
    let mut save: SaveFile = serde_json::from_str(&json).map_err(SaveError::Serialize)?;
    if save.version != SAVE_VERSION && save.version != 1 {
        return Err(SaveError::UnsupportedVersion(save.version));
    }
    save.game.migrate_after_load();
    Ok(save.game)
}

const fn save_mode_label(game: &GameState) -> &'static str {
    if game.game_over {
        "rescue"
    } else if game.won_game {
        "deep claim"
    } else {
        match game.run_mode {
            crate::game_state::RunMode::Title => "title",
            crate::game_state::RunMode::Playing => "playing",
            crate::game_state::RunMode::Interior => "interior",
            crate::game_state::RunMode::Paused => "paused",
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "save slot depth is displayed as an integral tile depth"
)]
pub fn save_slot_metadata(slot: usize) -> Option<SaveSlotMetadata> {
    let path = slot_path(slot);
    let modified_unix_seconds = fs::metadata(&path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    let game = load_game_from_path(path).ok()?;
    Some(SaveSlotMetadata {
        depth: (game.player.y / crate::game_state::TILE_SIZE).floor() as i32,
        credits: game.player.credits,
        cargo_used: game.player.cargo_used(),
        cargo_capacity: game.player.cargo_capacity,
        contracts_completed: game.contracts.completed,
        play_seconds: game.play_seconds,
        total_earnings: game.total_earnings,
        mode: save_mode_label(&game).to_owned(),
        deep_claim_unlocked: game.deep_claim_status == crate::economy::DeepClaimStatus::Unlocked,
        modified_unix_seconds,
        won_game: game.won_game,
    })
}

#[must_use]
pub fn save_exists() -> bool {
    save_path().exists()
}

#[must_use]
pub fn save_slot_exists(slot: usize) -> bool {
    slot_path(slot).exists()
}

#[must_use]
pub const fn save_slot_count() -> usize {
    SAVE_SLOTS
}

fn write_state_file(path: impl AsRef<Path>, contents: String) -> Result<(), SaveError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(SaveError::Io)?;
    }
    fs::write(path, contents).map_err(SaveError::Io)
}

fn settings_path() -> PathBuf {
    default_state_dir().join(SETTINGS_FILE_NAME)
}

fn save_path() -> PathBuf {
    default_state_dir().join(SAVE_FILE_NAME)
}

fn slot_path(slot: usize) -> PathBuf {
    default_state_dir().join(format!("save-slot-{}.json", slot + 1))
}

fn default_state_dir() -> PathBuf {
    if let Ok(path) = env::var("DRILLGAME_STATE_DIR") {
        return PathBuf::from(path);
    }
    if let Ok(state_home) = env::var("XDG_STATE_HOME") {
        return PathBuf::from(state_home).join("drillgame");
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("drillgame");
    }
    env::temp_dir().join("drillgame")
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
