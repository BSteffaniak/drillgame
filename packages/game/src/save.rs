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
    player::Player,
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
    pub x: f32,
    pub y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
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
            x: game.player.x,
            y: game.player.y,
            velocity_x: game.player.velocity_x,
            velocity_y: game.player.velocity_y,
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
        Some(Self::from_player(player_id, player))
    }

    #[must_use]
    pub fn from_player(player_id: PlayerId, player: &Player) -> Self {
        Self {
            player_id,
            x: player.x,
            y: player.y,
            velocity_x: player.velocity_x,
            velocity_y: player.velocity_y,
            credits: player.credits,
            cargo_used: player.cargo_used(),
            cargo_capacity: player.cargo_capacity,
            fuel: player.fuel,
            hull: player.hull,
        }
    }

    pub const fn apply_to_player(&self, player: &mut Player) {
        player.x = self.x;
        player.y = self.y;
        player.velocity_x = self.velocity_x;
        player.velocity_y = self.velocity_y;
        player.credits = self.credits;
        player.cargo_capacity = self.cargo_capacity;
        player.fuel = self.fuel;
        player.hull = self.hull;
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PersistentWorldRestoreSummary {
    pub simulation_tick: SimulationTick,
    pub roster_players: usize,
    pub persistent_players: usize,
    pub default_player_present: bool,
}

impl PersistentWorldRestoreSummary {
    #[must_use]
    pub fn from_save(save: &PersistentWorldSave) -> Self {
        Self {
            simulation_tick: save.session.simulation_tick,
            roster_players: save.player_roster.len(),
            persistent_players: save.session.players.len(),
            default_player_present: save.player_roster.contains(&save.default_player_id),
        }
    }

    #[must_use]
    pub const fn roster_matches_persistent_players(&self) -> bool {
        self.roster_players == self.persistent_players
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
        } else {
            player_roster.sort();
            player_roster.dedup();
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

    pub fn restore_into_world(&self, world: &mut WorldState) {
        world.set_simulation_tick(self.session.simulation_tick);
        for player_state in &self.session.players {
            if let Some(player) = world.player_mut(player_state.player_id) {
                player_state.apply_to_player(player);
            } else {
                let mut player = self.game.player.clone();
                player_state.apply_to_player(&mut player);
                world.insert_player(player_state.player_id, player);
            }
        }
    }

    #[must_use]
    pub fn restore_summary(&self) -> PersistentWorldRestoreSummary {
        PersistentWorldRestoreSummary::from_save(self)
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
        multiplayer::{LOCAL_PLAYER_ID, PlayerId, SimulationTick},
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
    fn persistent_world_save_restores_authoritative_world_player_state() {
        let mut game = GameState::new();
        game.player.credits = 111;
        let mut world = WorldState::from_legacy_game(&game);
        world.set_simulation_tick(SimulationTick::new(13));
        {
            let player = world.player_mut(LOCAL_PLAYER_ID).expect("player exists");
            player.credits = 222;
            player.x = 123.0;
            player.y = 234.0;
            player.velocity_x = 5.0;
            player.velocity_y = -6.0;
            player.fuel = 33.0;
            player.hull = 44.0;
            player.cargo_capacity = 55;
        }
        let save = PersistentWorldSave::from_world_and_legacy_game(&world, &game);
        let mut restored_world = WorldState::from_legacy_game(&GameState::new());

        save.restore_into_world(&mut restored_world);

        let restored_player = restored_world
            .player(LOCAL_PLAYER_ID)
            .expect("restored player exists");
        assert_eq!(restored_world.simulation_tick(), SimulationTick::new(13));
        assert_eq!(restored_player.credits, 222);
        assert!((restored_player.x - 123.0).abs() < f32::EPSILON);
        assert!((restored_player.y - 234.0).abs() < f32::EPSILON);
        assert!((restored_player.velocity_x - 5.0).abs() < f32::EPSILON);
        assert!((restored_player.velocity_y + 6.0).abs() < f32::EPSILON);
        assert!((restored_player.fuel - 33.0).abs() < f32::EPSILON);
        assert!((restored_player.hull - 44.0).abs() < f32::EPSILON);
        assert_eq!(restored_player.cargo_capacity, 55);
    }

    #[test]
    fn persistent_world_save_preserves_multi_player_roster_and_restore_order() {
        let game = GameState::new();
        let mut world = WorldState::from_legacy_game(&game);
        let second_player_id = PlayerId::new(2);
        let mut second_player = game.player.clone();
        second_player.x = 222.0;
        second_player.y = 333.0;
        second_player.credits = 444;
        world.insert_player(second_player_id, second_player);
        world.set_simulation_tick(SimulationTick::new(99));
        world
            .player_mut(LOCAL_PLAYER_ID)
            .expect("local player exists")
            .credits = 123;

        let save = PersistentWorldSave::from_world_and_legacy_game(&world, &game);
        let json = serde_json::to_string(&save).expect("serialize multi-player save");
        let loaded: PersistentWorldSave =
            serde_json::from_str(&json).expect("deserialize multi-player save");
        let mut restored_world = WorldState::from_legacy_game(&GameState::new());
        loaded.restore_into_world(&mut restored_world);

        assert_eq!(
            loaded.player_roster,
            vec![LOCAL_PLAYER_ID, second_player_id]
        );
        assert_eq!(loaded.default_player_id, LOCAL_PLAYER_ID);
        assert_eq!(restored_world.simulation_tick(), SimulationTick::new(99));
        assert_eq!(
            restored_world
                .player(LOCAL_PLAYER_ID)
                .expect("local player restored")
                .credits,
            123
        );
        let restored_second = restored_world
            .player(second_player_id)
            .expect("second player restored");
        assert_eq!(restored_second.credits, 444);
        assert!((restored_second.x - 222.0).abs() < f32::EPSILON);
        assert!((restored_second.y - 333.0).abs() < f32::EPSILON);
    }

    #[test]
    fn world_save_restore_summary_tracks_roster_and_session_state() {
        let mut game = GameState::new();
        game.player.credits = 654;
        let mut world = WorldState::from_legacy_game(&game);
        world.set_simulation_tick(SimulationTick::new(77));

        let save = PersistentWorldSave::from_world_and_legacy_game(&world, &game);
        let summary = save.restore_summary();

        assert_eq!(summary.simulation_tick, SimulationTick::new(77));
        assert_eq!(summary.roster_players, 1);
        assert_eq!(summary.persistent_players, 1);
        assert!(summary.default_player_present);
        assert!(summary.roster_matches_persistent_players());
    }

    #[test]
    fn save_file_regression_round_trips_versioned_world_envelope() {
        let mut game = GameState::new();
        game.player.credits = 777;
        game.player.fuel = 42.0;
        let save = SaveFile {
            version: 2,
            world: PersistentWorldSave::from_legacy_game(&game),
        };

        let json = serde_json::to_string(&save).expect("serialize save");
        let loaded: SaveFile = serde_json::from_str(&json).expect("deserialize save");

        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.world.session.players[0].credits, 777);
        assert!((loaded.world.game.player.fuel - 42.0).abs() < f32::EPSILON);
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
