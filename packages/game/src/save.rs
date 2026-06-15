use std::{
    env,
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::{
    game_state::GameState,
    multiplayer::{LOCAL_PLAYER_ID, PlayerId, SimulationTick},
    session::WorldState,
};

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
    world: PersistentWorldSave,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SaveAuthority {
    #[default]
    LocalSinglePlayer,
    HostOwnedSession,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SaveSessionKind {
    #[default]
    LocalOnly,
    HostOwned,
    DedicatedServer,
    CloudSession,
}

impl From<SaveAuthority> for SaveSessionKind {
    fn from(authority: SaveAuthority) -> Self {
        match authority {
            SaveAuthority::LocalSinglePlayer => Self::LocalOnly,
            SaveAuthority::HostOwnedSession => Self::HostOwned,
        }
    }
}

fn default_player_roster() -> Vec<PlayerId> {
    vec![LOCAL_PLAYER_ID]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PersistentPlayerState {
    pub player_id: PlayerId,
    pub credits: u32,
    pub cargo_used: u32,
    pub cargo_capacity: u32,
    pub fuel: f32,
    pub hull: f32,
}

impl PersistentPlayerState {
    #[must_use]
    pub fn local_from_game(game: &GameState) -> Self {
        Self {
            player_id: LOCAL_PLAYER_ID,
            credits: game.player.credits,
            cargo_used: game.player.cargo_used(),
            cargo_capacity: game.player.cargo_capacity,
            fuel: game.player.fuel,
            hull: game.player.hull,
        }
    }

    #[must_use]
    pub fn from_world_player(player_id: PlayerId, world: &WorldState) -> Option<Self> {
        let player = world.player(player_id)?;
        Some(Self {
            player_id,
            credits: player.credits,
            cargo_used: player.cargo_used(),
            cargo_capacity: player.cargo_capacity,
            fuel: player.fuel,
            hull: player.hull,
        })
    }
}

const fn default_persistent_players() -> Vec<PersistentPlayerState> {
    Vec::new()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PersistentSessionState {
    pub simulation_tick: SimulationTick,
    pub players: Vec<PersistentPlayerState>,
}

impl PersistentSessionState {
    #[must_use]
    pub fn local_from_game(game: &GameState) -> Self {
        Self {
            simulation_tick: SimulationTick::default(),
            players: vec![PersistentPlayerState::local_from_game(game)],
        }
    }

    #[must_use]
    pub fn from_world(world: &WorldState) -> Self {
        Self {
            simulation_tick: world.simulation_tick(),
            players: world
                .player_ids()
                .filter_map(|player_id| PersistentPlayerState::from_world_player(player_id, world))
                .collect(),
        }
    }
}

impl Default for PersistentSessionState {
    fn default() -> Self {
        Self {
            simulation_tick: SimulationTick::default(),
            players: default_persistent_players(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PersistentWorldSave {
    #[serde(default)]
    pub save_authority: SaveAuthority,
    #[serde(default)]
    pub session_kind: SaveSessionKind,
    #[serde(default = "default_player_roster")]
    pub player_roster: Vec<PlayerId>,
    pub default_player_id: PlayerId,
    #[serde(default)]
    pub session: PersistentSessionState,
    pub game: GameState,
}

impl PersistentWorldSave {
    #[must_use]
    pub fn from_legacy_game(game: &GameState) -> Self {
        Self {
            save_authority: SaveAuthority::LocalSinglePlayer,
            session_kind: SaveSessionKind::LocalOnly,
            player_roster: default_player_roster(),
            default_player_id: LOCAL_PLAYER_ID,
            session: PersistentSessionState::local_from_game(game),
            game: game.clone_for_save(),
        }
    }

    #[must_use]
    pub fn from_world_and_legacy_game(world: &WorldState, game: &GameState) -> Self {
        let mut player_roster = world.player_ids().collect::<Vec<_>>();
        if player_roster.is_empty() {
            player_roster = default_player_roster();
        }
        Self {
            save_authority: SaveAuthority::HostOwnedSession,
            session_kind: SaveSessionKind::HostOwned,
            default_player_id: player_roster.first().copied().unwrap_or(LOCAL_PLAYER_ID),
            player_roster,
            session: PersistentSessionState::from_world(world),
            game: game.clone_for_save(),
        }
    }

    #[must_use]
    pub fn into_legacy_game(self) -> GameState {
        self.game
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct LegacySaveFile {
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
        world: PersistentWorldSave::from_legacy_game(game),
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

pub fn load_latest_game() -> Result<GameState, SaveError> {
    let Some(path) = latest_save_path() else {
        return Err(SaveError::NoSaveFound);
    };
    load_game_from_path(path)
}

fn load_game_from_path(path: impl AsRef<Path>) -> Result<GameState, SaveError> {
    let json = fs::read_to_string(path).map_err(SaveError::Io)?;
    if let Ok(save) = serde_json::from_str::<SaveFile>(&json) {
        validate_save_version(save.version)?;
        let mut game = save.world.into_legacy_game();
        game.migrate_after_load();
        return Ok(game);
    }

    let legacy_save: LegacySaveFile = serde_json::from_str(&json).map_err(SaveError::Serialize)?;
    validate_save_version(legacy_save.version)?;
    let mut game = PersistentWorldSave {
        save_authority: SaveAuthority::LocalSinglePlayer,
        session_kind: SaveSessionKind::LocalOnly,
        player_roster: default_player_roster(),
        default_player_id: LOCAL_PLAYER_ID,
        session: PersistentSessionState::local_from_game(&legacy_save.game),
        game: legacy_save.game,
    }
    .into_legacy_game();
    game.migrate_after_load();
    Ok(game)
}

const fn validate_save_version(version: u32) -> Result<(), SaveError> {
    if version != SAVE_VERSION && version != 1 {
        return Err(SaveError::UnsupportedVersion(version));
    }
    Ok(())
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

#[must_use]
pub fn save_slot_metadata(slot: usize) -> Option<SaveSlotMetadata> {
    save_metadata_from_path(slot_path(slot))
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "save slot depth is displayed as an integral tile depth"
)]
fn save_metadata_from_path(path: impl AsRef<Path>) -> Option<SaveSlotMetadata> {
    let path = path.as_ref();
    let modified_unix_seconds = fs::metadata(path)
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
pub fn saves_exist() -> bool {
    save_exists() || (0..SAVE_SLOTS).any(save_slot_exists)
}

#[must_use]
pub fn latest_save_summary() -> Option<SaveSlotMetadata> {
    latest_save_path().and_then(save_metadata_from_path)
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

fn latest_save_path() -> Option<PathBuf> {
    std::iter::once(save_path())
        .chain((0..SAVE_SLOTS).map(slot_path))
        .filter_map(|path| {
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
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
    NoSaveFound,
}

impl fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Serialize(error) => write!(formatter, "serialization error: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported save version: {version}")
            }
            Self::NoSaveFound => formatter.write_str("no save file found"),
        }
    }
}

impl Error for SaveError {}

#[cfg(test)]
mod tests {
    use crate::{
        game_state::GameState,
        multiplayer::{LOCAL_PLAYER_ID, SimulationTick},
        save::{LegacySaveFile, PersistentWorldSave, SaveAuthority, SaveFile, SaveSessionKind},
        session::WorldState,
    };

    #[test]
    fn game_state_round_trips_through_versioned_json() {
        let game = GameState::new();
        let save = SaveFile {
            version: 2,
            world: PersistentWorldSave::from_legacy_game(&game),
        };
        let json = serde_json::to_string(&save).expect("serialize game");
        let loaded: SaveFile = serde_json::from_str(&json).expect("deserialize game");

        assert_eq!(loaded.version, 2);
        assert_eq!(
            loaded.world.save_authority,
            SaveAuthority::LocalSinglePlayer
        );
        assert_eq!(loaded.world.session_kind, SaveSessionKind::LocalOnly);
        assert_eq!(loaded.world.default_player_id, LOCAL_PLAYER_ID);
        assert_eq!(loaded.world.player_roster, vec![LOCAL_PLAYER_ID]);
        assert_eq!(
            loaded.world.session.simulation_tick,
            SimulationTick::default()
        );
        assert_eq!(loaded.world.session.players[0].player_id, LOCAL_PLAYER_ID);
        assert_eq!(
            loaded.world.game.player.cargo_capacity,
            game.player.cargo_capacity
        );
        assert_eq!(loaded.world.game.terrain.width(), game.terrain.width());
    }

    #[test]
    fn legacy_save_shape_still_deserializes_for_migration() {
        let game = GameState::new();
        let legacy = LegacySaveFile {
            version: 2,
            game: game.clone_for_save(),
        };
        let json = serde_json::to_string(&legacy).expect("serialize legacy game");
        let loaded: LegacySaveFile = serde_json::from_str(&json).expect("deserialize legacy game");

        assert_eq!(loaded.version, 2);
        assert_eq!(
            loaded.game.player.cargo_capacity,
            game.player.cargo_capacity
        );
    }

    #[test]
    fn persistent_session_state_preserves_local_player_metadata() {
        let game = GameState::new();
        let session = super::PersistentSessionState::local_from_game(&game);

        assert_eq!(session.simulation_tick, SimulationTick::default());
        assert_eq!(session.players.len(), 1);
        assert_eq!(session.players[0].player_id, LOCAL_PLAYER_ID);
        assert_eq!(
            session.players[0].cargo_capacity,
            game.player.cargo_capacity
        );
    }

    #[test]
    fn persistent_world_save_can_be_built_from_world_state() {
        let mut game = GameState::new();
        game.player.credits = 321;
        let mut world = WorldState::from_legacy_game(&game);
        world.set_simulation_tick(SimulationTick::new(42));
        world
            .player_mut(LOCAL_PLAYER_ID)
            .expect("local player exists")
            .fuel = 12.0;

        let save = PersistentWorldSave::from_world_and_legacy_game(&world, &game);

        assert_eq!(save.save_authority, SaveAuthority::HostOwnedSession);
        assert_eq!(save.session_kind, SaveSessionKind::HostOwned);
        assert_eq!(save.player_roster, vec![LOCAL_PLAYER_ID]);
        assert_eq!(save.session.simulation_tick, SimulationTick::new(42));
        assert_eq!(save.session.players[0].credits, 321);
        assert!((save.session.players[0].fuel - 12.0).abs() < f32::EPSILON);
    }

    #[test]
    fn save_authority_maps_to_session_kind() {
        assert_eq!(
            SaveSessionKind::from(SaveAuthority::LocalSinglePlayer),
            SaveSessionKind::LocalOnly
        );
        assert_eq!(
            SaveSessionKind::from(SaveAuthority::HostOwnedSession),
            SaveSessionKind::HostOwned
        );
    }
}
