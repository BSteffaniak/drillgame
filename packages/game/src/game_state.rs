#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "world coordinates intentionally cross integer tile and floating render spaces"
)]

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::{
    contract::ContractLog,
    economy::{
        PurchaseError, SurfaceZone, buy_upgrade, refuel_amount, repair_amount, sell_cargo,
        upgrade_offers, upgrade_tier_name,
    },
    input::PlayerInput,
    player::Player,
    save::{load_game, save_exists, save_game},
    terrain::{MineResult, Terrain, TileKind, TilePosition},
};

pub const TILE_SIZE: f32 = 32.0;
const WORLD_WIDTH: i32 = 120;
const WORLD_HEIGHT: i32 = 90;
const GRAVITY: f32 = 780.0;
const HORIZONTAL_ACCELERATION: f32 = 900.0;
const THRUST_ACCELERATION: f32 = 1_250.0;
const MAX_HORIZONTAL_SPEED: f32 = 260.0;
const MAX_FALL_SPEED: f32 = 560.0;
const DRAG: f32 = 0.86;
const FUEL_BURN_PER_SECOND: f32 = 5.0;
const DRILL_FUEL_COST: f32 = 0.45;
const PLAYER_RADIUS: f32 = 10.5;
const SAFE_LANDING_SPEED: f32 = 330.0;
const CRASH_DAMAGE_SCALE: f32 = 0.11;
const HEAT_START_DEPTH: f32 = 18.0 * TILE_SIZE;
const HEAT_DAMAGE_PER_SECOND: f32 = 3.5;
const CAMERA_SMOOTHING: f32 = 8.0;
const WORLD_SEED: u64 = 0xD1_11_6A_4E;

const fn default_master_volume() -> f32 {
    0.8
}

const fn default_fullscreen() -> bool {
    false
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DrillDirection {
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct DrillState {
    pub target: TilePosition,
    pub direction: DrillDirection,
    pub progress: f32,
    pub initial_durability: u8,
    pub seconds_per_chip: f32,
    pub sound_timer: f32,
    pub dust_timer: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RunMode {
    Title,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ModalScreen {
    Fuel,
    FuelConfirm,
    Repair,
    RepairConfirm,
    Depot,
    DepotReceiptHistory,
    Shop,
    ShopConfirm,
    ExitConfirm,
    Map,
    Help,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PauseOption {
    Resume,
    Save,
    Load,
    ExitToDesktop,
}

impl PauseOption {
    pub const ALL: [Self; 4] = [Self::Resume, Self::Save, Self::Load, Self::ExitToDesktop];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Resume => "Resume",
            Self::Save => "Save Game",
            Self::Load => "Load Game",
            Self::ExitToDesktop => "Exit to Desktop",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct DustParticle {
    pub x: f32,
    pub y: f32,
    pub life: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct HazardCloud {
    pub x: f32,
    pub y: f32,
    pub life: f32,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct FallingBoulder {
    pub x: f32,
    pub y: f32,
    pub velocity_y: f32,
    pub warning_seconds: f32,
    pub life: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct SparkParticle {
    pub x: f32,
    pub y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub life: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum SoundCue {
    Drill,
    Sell,
    Upgrade,
    Damage,
    Milestone,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "game state tracks several orthogonal UI/progression flags"
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GameState {
    pub terrain: Terrain,
    pub player: Player,
    pub message: String,
    pub current_zone: Option<SurfaceZone>,
    pub contracts: ContractLog,
    pub run_mode: RunMode,
    pub modal: Option<ModalScreen>,
    pub selected_menu_item: usize,
    #[serde(default)]
    pub selected_pause_item: usize,
    pub show_details: bool,
    #[serde(default)]
    pub request_exit: bool,
    #[serde(default)]
    pub won_game: bool,
    #[serde(default)]
    pub explored_tiles: Vec<bool>,
    #[serde(default)]
    pub last_depot_receipt: String,
    #[serde(default)]
    pub depot_receipts: Vec<String>,
    pub deepest_tile_reached: i32,
    #[serde(default)]
    pub total_earnings: u32,
    #[serde(default)]
    pub rescue_count: u32,
    #[serde(default)]
    pub artifacts_found: u32,
    pub next_milestone_tile: i32,
    #[serde(default)]
    pub current_layer_band: i32,
    pub game_over: bool,
    #[serde(default = "default_master_volume")]
    pub master_volume: f32,
    #[serde(default = "default_fullscreen")]
    pub fullscreen: bool,
    #[serde(default)]
    pub last_rescue_x: Option<f32>,
    #[serde(default)]
    pub last_rescue_y: Option<f32>,
    #[serde(default)]
    pub last_rescue_summary: String,
    pub camera_x: f32,
    pub camera_y: f32,
    pub drill_flash_seconds: f32,
    #[serde(default)]
    pub active_drill: Option<DrillState>,
    pub dust_particles: Vec<DustParticle>,
    #[serde(default)]
    pub hazard_clouds: Vec<HazardCloud>,
    #[serde(default)]
    pub falling_boulders: Vec<FallingBoulder>,
    #[serde(default)]
    pub spark_particles: Vec<SparkParticle>,
    #[serde(default)]
    pub camera_shake_seconds: f32,
    #[serde(default)]
    pub camera_shake_strength: f32,
    pub sound_cues: Vec<SoundCue>,
}

impl GameState {
    #[must_use]
    pub fn clone_for_save(&self) -> Self {
        let mut saved = self.clone();
        saved.run_mode = RunMode::Playing;
        saved.modal = None;
        saved.request_exit = false;
        saved.show_details = false;
        saved.active_drill = None;
        saved.dust_particles.clear();
        saved.hazard_clouds.clear();
        saved.falling_boulders.clear();
        saved.spark_particles.clear();
        saved.camera_shake_seconds = 0.0;
        saved.sound_cues.clear();
        saved
    }

    #[must_use]
    pub fn new() -> Self {
        Self {
            terrain: Terrain::new_seeded(WORLD_WIDTH, WORLD_HEIGHT, WORLD_SEED),
            player: Player::new(12.0 * TILE_SIZE, 4.0 * TILE_SIZE),
            message: "Mine ore, sell cargo, and buy upgrades. Press E at surface buildings."
                .to_owned(),
            current_zone: None,
            contracts: ContractLog::new(),
            run_mode: RunMode::Title,
            modal: None,
            selected_menu_item: 0,
            selected_pause_item: 0,
            show_details: false,
            request_exit: false,
            won_game: false,
            explored_tiles: vec![false; (WORLD_WIDTH * WORLD_HEIGHT) as usize],
            last_depot_receipt: String::new(),
            depot_receipts: Vec::new(),
            deepest_tile_reached: 0,
            total_earnings: 0,
            rescue_count: 0,
            artifacts_found: 0,
            next_milestone_tile: 20,
            current_layer_band: 0,
            game_over: false,
            master_volume: default_master_volume(),
            fullscreen: default_fullscreen(),
            last_rescue_x: None,
            last_rescue_y: None,
            last_rescue_summary: String::new(),
            camera_x: 0.0,
            camera_y: 0.0,
            drill_flash_seconds: 0.0,
            active_drill: None,
            dust_particles: Vec::new(),
            hazard_clouds: Vec::new(),
            falling_boulders: Vec::new(),
            spark_particles: Vec::new(),
            camera_shake_seconds: 0.0,
            camera_shake_strength: 0.0,
            sound_cues: Vec::new(),
        }
    }

    pub fn migrate_after_load(&mut self) {
        let expected_tiles = (self.terrain.width() * self.terrain.height()) as usize;
        if self.explored_tiles.len() != expected_tiles {
            self.explored_tiles = vec![false; expected_tiles];
        }
        self.request_exit = false;
        self.contracts.migrate_after_load();
    }

    pub fn update(&mut self, input: PlayerInput, delta_seconds: f32) {
        self.sound_cues.clear();
        self.show_details = input.details;
        self.handle_save_load(input);
        if input.map {
            self.modal = if self.modal == Some(ModalScreen::Map) {
                None
            } else {
                Some(ModalScreen::Map)
            };
        }
        if input.help {
            self.modal = if self.modal == Some(ModalScreen::Help) {
                None
            } else {
                Some(ModalScreen::Help)
            };
        }
        if input.volume_up {
            self.master_volume = (self.master_volume + 0.1).min(1.0);
            self.message = format!("Volume: {:.0}%", self.master_volume * 100.0);
        }
        if input.volume_down {
            self.master_volume = (self.master_volume - 0.1).max(0.0);
            self.message = format!("Volume: {:.0}%", self.master_volume * 100.0);
        }
        if input.fullscreen {
            self.fullscreen = !self.fullscreen;
            self.message = if self.fullscreen {
                "Fullscreen preference saved. Restart/toggle window integration pending.".to_owned()
            } else {
                "Windowed preference saved.".to_owned()
            };
        }
        self.update_particles(delta_seconds);
        self.update_boulders(delta_seconds);
        self.camera_shake_seconds = (self.camera_shake_seconds - delta_seconds).max(0.0);
        self.update_hazards(delta_seconds);
        self.reveal_near_player();
        self.drill_flash_seconds = (self.drill_flash_seconds - delta_seconds).max(0.0);

        match self.run_mode {
            RunMode::Title => {
                if input.confirm {
                    self.run_mode = RunMode::Playing;
                    self.sound_cues.push(SoundCue::Milestone);
                    "Welcome to the dig site. Visit the depot for contracts."
                        .clone_into(&mut self.message);
                }
                return;
            }
            RunMode::Paused => {
                self.handle_pause_menu(input);
                return;
            }
            RunMode::Playing => {}
        }

        if self.game_over {
            self.handle_rescue(input);
            self.update_camera(delta_seconds);
            return;
        }

        self.current_zone = surface_zone_at(self.player.x, self.player.y);
        if self.handle_modal(input) {
            self.update_camera(delta_seconds);
            return;
        }

        if input.pause || input.cancel {
            self.run_mode = RunMode::Paused;
            return;
        }

        self.handle_interaction(input);
        self.apply_movement(input, delta_seconds);
        self.update_drilling(input, delta_seconds);
        self.apply_depth_pressure(delta_seconds);
        self.apply_lava_damage(delta_seconds);
        self.update_depth_milestones();
        self.update_layer_band();
        self.update_warning_messages();
        self.update_status_messages();
        self.check_failure();
        self.update_camera(delta_seconds);
    }

    fn reveal_near_player(&mut self) {
        let center_x = (self.player.x / TILE_SIZE).floor() as i32;
        let center_y = (self.player.y / TILE_SIZE).floor() as i32;
        for y in center_y - 3..=center_y + 3 {
            for x in center_x - 4..=center_x + 4 {
                if let Some(index) = self.tile_index(TilePosition { x, y }) {
                    self.explored_tiles[index] = true;
                }
            }
        }
    }

    #[must_use]
    pub fn is_explored(&self, position: TilePosition) -> bool {
        self.tile_index(position)
            .and_then(|index| self.explored_tiles.get(index))
            .copied()
            .unwrap_or(false)
    }

    const fn tile_index(&self, position: TilePosition) -> Option<usize> {
        if position.x < 0
            || position.y < 0
            || position.x >= self.terrain.width()
            || position.y >= self.terrain.height()
        {
            return None;
        }
        Some((position.y * self.terrain.width() + position.x) as usize)
    }

    fn handle_pause_menu(&mut self, input: PlayerInput) {
        if self.modal == Some(ModalScreen::ExitConfirm) {
            if input.cancel {
                self.modal = None;
                return;
            }
            if input.confirm {
                self.request_exit = true;
            }
            return;
        }

        if input.menu_up {
            self.selected_pause_item = self.selected_pause_item.saturating_sub(1);
        }
        if input.menu_down {
            self.selected_pause_item =
                (self.selected_pause_item + 1).min(PauseOption::ALL.len() - 1);
        }

        if input.pause || input.cancel {
            self.run_mode = RunMode::Playing;
            return;
        }

        if !input.confirm {
            return;
        }

        match PauseOption::ALL[self.selected_pause_item] {
            PauseOption::Resume => self.run_mode = RunMode::Playing,
            PauseOption::Save => match save_game(self) {
                Ok(()) => "Game saved.".clone_into(&mut self.message),
                Err(error) => self.message = format!("Save failed: {error}"),
            },
            PauseOption::Load => self.load_into_self(),
            PauseOption::ExitToDesktop => self.modal = Some(ModalScreen::ExitConfirm),
        }
    }

    fn handle_save_load(&mut self, input: PlayerInput) {
        if input.save {
            match save_game(self) {
                Ok(()) => "Game saved to drillgame-save.json.".clone_into(&mut self.message),
                Err(error) => self.message = format!("Save failed: {error}"),
            }
        }

        if input.load {
            self.load_into_self();
        }
    }

    fn load_into_self(&mut self) {
        if !save_exists() {
            "No save file found.".clone_into(&mut self.message);
            return;
        }

        match load_game() {
            Ok(mut loaded) => {
                "Game loaded.".clone_into(&mut loaded.message);
                *self = loaded;
            }
            Err(error) => self.message = format!("Load failed: {error}"),
        }
    }

    fn handle_interaction(&mut self, input: PlayerInput) {
        if !input.interact {
            return;
        }

        match self.current_zone {
            Some(SurfaceZone::Fuel) => {
                self.modal = Some(ModalScreen::Fuel);
                self.selected_menu_item = 0;
            }
            Some(SurfaceZone::Repair) => {
                self.modal = Some(ModalScreen::Repair);
                self.selected_menu_item = 0;
            }
            Some(SurfaceZone::Depot) => {
                self.modal = Some(ModalScreen::Depot);
                self.selected_menu_item = 0;
            }
            Some(SurfaceZone::Shop) => {
                self.modal = Some(ModalScreen::Shop);
                self.selected_menu_item = 0;
            }
            None => "No surface service here.".clone_into(&mut self.message),
        }
    }

    fn handle_modal(&mut self, input: PlayerInput) -> bool {
        let Some(modal) = self.modal else {
            return false;
        };

        if input.cancel {
            self.modal = None;
            return true;
        }

        if input.menu_up {
            self.selected_menu_item = self.selected_menu_item.saturating_sub(1);
        }
        if input.menu_down {
            let max_item = match modal {
                ModalScreen::Depot | ModalScreen::Fuel | ModalScreen::Repair => 2,
                _ => 0,
            };
            self.selected_menu_item = (self.selected_menu_item + 1).min(max_item);
        }

        if matches!(modal, ModalScreen::Shop) {
            if input.menu_left {
                self.selected_menu_item = self.selected_menu_item.saturating_sub(1);
            }
            if input.menu_right {
                self.selected_menu_item =
                    (self.selected_menu_item + 1).min(upgrade_offers(&self.player).len() - 1);
            }
        }

        if let Some(index) = input.selected_upgrade {
            self.selected_menu_item = index.min(upgrade_offers(&self.player).len() - 1);
        }

        if input.confirm {
            match modal {
                ModalScreen::Fuel => self.modal = Some(ModalScreen::FuelConfirm),
                ModalScreen::FuelConfirm => self.confirm_refuel(),
                ModalScreen::Repair => self.modal = Some(ModalScreen::RepairConfirm),
                ModalScreen::RepairConfirm => self.confirm_repair(),
                ModalScreen::Depot => self.confirm_depot(),
                ModalScreen::DepotReceiptHistory => self.modal = Some(ModalScreen::Depot),
                ModalScreen::Shop => self.modal = Some(ModalScreen::ShopConfirm),
                ModalScreen::ShopConfirm => self.try_buy_upgrade(self.selected_menu_item),
                ModalScreen::ExitConfirm | ModalScreen::Map | ModalScreen::Help => {}
            }
        }

        true
    }

    const fn selected_service_fraction(&self) -> f32 {
        match self.selected_menu_item {
            0 => 0.25,
            1 => 0.5,
            _ => 1.0,
        }
    }

    fn confirm_refuel(&mut self) {
        let fraction = self.selected_service_fraction();
        let cost = refuel_amount(&mut self.player, fraction);
        self.message = if cost == 0 {
            "Fuel already full or no credits available.".to_owned()
        } else {
            format!("Fuel topped up for {cost} credits.")
        };
        if cost > 0 {
            self.sound_cues.push(SoundCue::Upgrade);
        }
        self.modal = Some(ModalScreen::Fuel);
    }

    fn confirm_repair(&mut self) {
        let fraction = self.selected_service_fraction();
        let cost = repair_amount(&mut self.player, fraction);
        self.message = if cost == 0 {
            "Hull already repaired or no credits available.".to_owned()
        } else {
            format!("Hull repaired for {cost} credits.")
        };
        if cost > 0 {
            self.sound_cues.push(SoundCue::Upgrade);
        }
        self.modal = Some(ModalScreen::Repair);
    }

    fn confirm_depot(&mut self) {
        match self.selected_menu_item {
            0 => self.confirm_contract(),
            1 => self.confirm_sell_cargo(),
            _ => self.modal = Some(ModalScreen::DepotReceiptHistory),
        }
    }

    fn confirm_contract(&mut self) {
        if let Some(completion) = self.contracts.try_complete(&mut self.player) {
            self.sound_cues.push(SoundCue::Sell);
            self.total_earnings += completion.reward;
            if completion.finished_story {
                self.won_game = true;
                self.message = format!(
                    "{} complete! Star Core secured. You win! Bonus: {} credits.",
                    completion.completed_title, completion.reward
                );
            } else {
                let story = ContractLog::story_for_completed(self.contracts.completed);
                self.message = format!(
                    "{} complete! Bonus paid: {} credits. {story}",
                    completion.completed_title, completion.reward
                );
            }
        } else {
            "Contract target not ready.".clone_into(&mut self.message);
        }
    }

    fn confirm_sell_cargo(&mut self) {
        self.last_depot_receipt.clear();
        for (mineral, count) in &self.player.cargo {
            let _ = writeln!(
                &mut self.last_depot_receipt,
                "{} x{} = {} cr",
                mineral.name(),
                count,
                mineral.value() * count
            );
        }
        for (artifact, count) in &self.player.artifacts {
            let _ = writeln!(
                &mut self.last_depot_receipt,
                "{} x{} = {} cr",
                artifact.name(),
                count,
                artifact.value() * count
            );
        }

        let payout = sell_cargo(&mut self.player);
        if payout > 0 {
            self.total_earnings += payout;
            let _ = writeln!(&mut self.last_depot_receipt, "TOTAL = {payout} cr");
            self.depot_receipts.push(self.last_depot_receipt.clone());
            if self.depot_receipts.len() > 5 {
                self.depot_receipts.remove(0);
            }
        }
        if payout == 0 {
            "No cargo to sell.".clone_into(&mut self.message);
        } else {
            self.sound_cues.push(SoundCue::Sell);
            self.message = format!("Sold cargo for {payout} credits.");
        }
    }

    fn try_buy_upgrade(&mut self, index: usize) {
        if self.current_zone != Some(SurfaceZone::Shop) {
            return;
        }

        match buy_upgrade(&mut self.player, index) {
            Ok(offer) => {
                self.sound_cues.push(SoundCue::Upgrade);
                self.message = format!(
                    "Bought {}.",
                    upgrade_tier_name(offer.kind, offer.level.saturating_sub(1))
                );
            }
            Err(PurchaseError::InvalidSelection) => {
                "Unknown upgrade selection.".clone_into(&mut self.message);
            }
            Err(PurchaseError::MaxLevel) => {
                "That upgrade is already maxed.".clone_into(&mut self.message);
            }
            Err(PurchaseError::NotEnoughCredits) => {
                "Not enough credits for that upgrade.".clone_into(&mut self.message);
            }
        }
        self.modal = Some(ModalScreen::Shop);
    }

    fn apply_movement(&mut self, input: PlayerInput, delta_seconds: f32) {
        let can_burn_fuel = self.player.fuel > 0.0;
        let grounded = self.is_grounded();
        let engine_multiplier = 1.0 + f32::from(self.player.engine_level.saturating_sub(1)) * 0.18;
        let horizontal_acceleration = if grounded {
            HORIZONTAL_ACCELERATION * 1.35
        } else {
            HORIZONTAL_ACCELERATION * 0.65
        };

        self.player.velocity_x +=
            input.horizontal * horizontal_acceleration * engine_multiplier * delta_seconds;

        if input.thrust && can_burn_fuel {
            self.player.velocity_y -= THRUST_ACCELERATION * engine_multiplier * delta_seconds;
            self.player.fuel = (self.player.fuel - FUEL_BURN_PER_SECOND * delta_seconds).max(0.0);
        }

        self.player.velocity_y += GRAVITY * delta_seconds;
        let drag = if grounded { 0.78 } else { DRAG };
        self.player.velocity_x *= drag.powf(delta_seconds * 60.0);
        self.player.velocity_x = self.player.velocity_x.clamp(
            -MAX_HORIZONTAL_SPEED * engine_multiplier,
            MAX_HORIZONTAL_SPEED * engine_multiplier,
        );
        self.player.velocity_y = self
            .player
            .velocity_y
            .clamp(-MAX_FALL_SPEED, MAX_FALL_SPEED);

        self.move_axis(self.player.velocity_x * delta_seconds, 0.0);
        self.move_axis(0.0, self.player.velocity_y * delta_seconds);
    }

    fn move_axis(&mut self, delta_x: f32, delta_y: f32) {
        let next_x = self.player.x + delta_x;
        let next_y = self.player.y + delta_y;

        if self.collides(next_x, next_y) {
            if delta_x != 0.0 {
                if self.player.velocity_x.abs() > SAFE_LANDING_SPEED * 0.75 {
                    self.apply_bump_damage(self.player.velocity_x.abs());
                }
                self.player.velocity_x = 0.0;
            }
            if delta_y > 0.0 {
                self.apply_landing_damage();
            }
            if delta_y != 0.0 {
                self.player.velocity_y = 0.0;
            }
            return;
        }

        self.player.x = next_x.clamp(0.0, (self.terrain.width() as f32 - 1.0) * TILE_SIZE);
        self.player.y = next_y.clamp(0.0, (self.terrain.height() as f32 - 1.0) * TILE_SIZE);
    }

    fn apply_landing_damage(&mut self) {
        if self.player.velocity_y <= SAFE_LANDING_SPEED {
            return;
        }

        let damage = (self.player.velocity_y - SAFE_LANDING_SPEED) * CRASH_DAMAGE_SCALE;
        self.player.hull = (self.player.hull - damage).max(0.0);
        self.sound_cues.push(SoundCue::Damage);
        self.shake_camera(0.28, 7.0);
        self.spawn_sparks();
        self.message = format!("Hard landing! Hull took {damage:.0} damage.");
    }

    fn apply_bump_damage(&mut self, speed: f32) {
        let damage = (speed - SAFE_LANDING_SPEED * 0.75) * CRASH_DAMAGE_SCALE * 0.5;
        self.player.hull = (self.player.hull - damage).max(0.0);
        self.sound_cues.push(SoundCue::Damage);
        self.shake_camera(0.2, 5.0);
        self.spawn_sparks();
        self.message = format!("Hull scraped the wall for {damage:.0} damage.");
    }

    fn collides(&self, x: f32, y: f32) -> bool {
        collision_points(x, y)
            .iter()
            .any(|position| self.terrain.is_solid_at(*position))
    }

    fn is_grounded(&self) -> bool {
        collision_points(self.player.x, self.player.y + 2.0)
            .iter()
            .any(|position| self.terrain.is_solid_at(*position))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "drilling update coordinates input, physics, terrain, feedback, and collection in one frame step"
    )]
    fn update_drilling(&mut self, input: PlayerInput, delta_seconds: f32) {
        let Some((target, direction)) = mine_target(&self.player, input) else {
            self.active_drill = None;
            return;
        };

        if direction != DrillDirection::Down
            && (!self.is_grounded() || self.player.velocity_y.abs() > 80.0)
        {
            self.active_drill = None;
            "Side drilling requires stable ground contact.".clone_into(&mut self.message);
            return;
        }

        if self.player.fuel <= 0.0 {
            self.active_drill = None;
            "Out of fuel. Reach a fuel station or await rescue.".clone_into(&mut self.message);
            return;
        }

        let Some(tile) = self.terrain.tile(target) else {
            self.active_drill = None;
            return;
        };
        if tile.kind == TileKind::Air {
            self.active_drill = None;
            return;
        }
        if self
            .terrain
            .hardness_at(target)
            .is_some_and(|hardness| hardness > self.player.drill_strength)
        {
            self.active_drill = None;
            "That layer is too hard. Upgrade your drill.".clone_into(&mut self.message);
            return;
        }

        let seconds_per_chip =
            drill_seconds_per_chip(tile.kind, self.player.drill_strength, direction);
        let reset = self
            .active_drill
            .is_none_or(|state| state.target != target || state.direction != direction);
        if reset {
            self.active_drill = Some(DrillState {
                target,
                direction,
                progress: 0.0,
                initial_durability: tile.durability.max(1),
                seconds_per_chip,
                sound_timer: 0.0,
                dust_timer: 0.0,
            });
        }

        self.player.fuel = (self.player.fuel - DRILL_FUEL_COST * 1.25 * delta_seconds).max(0.0);
        self.creep_into_drill(direction, delta_seconds);

        let mut should_chip = false;
        let mut should_spawn_dust = false;
        if let Some(state) = &mut self.active_drill {
            state.seconds_per_chip = seconds_per_chip;
            state.progress += delta_seconds / seconds_per_chip;
            state.sound_timer -= delta_seconds;
            state.dust_timer -= delta_seconds;
            if state.sound_timer <= 0.0 {
                self.sound_cues.push(SoundCue::Drill);
                state.sound_timer = 0.13;
            }
            if state.dust_timer <= 0.0 {
                should_spawn_dust = true;
                state.dust_timer = 0.09;
            }
            should_chip = state.progress >= 1.0;
            self.drill_flash_seconds = 0.09;
            let chipped = state.initial_durability.saturating_sub(tile.durability);
            let total_progress = ((f32::from(chipped) + state.progress.min(1.0))
                / f32::from(state.initial_durability.max(1)))
            .clamp(0.0, 1.0);
            self.message = format!(
                "Drilling {}... {:.0}%",
                tile.kind.name(),
                total_progress * 100.0
            );
        }
        if should_spawn_dust {
            self.spawn_dust();
        }

        if should_chip {
            if let Some(state) = &mut self.active_drill {
                state.progress -= 1.0;
            }
            match self.terrain.chip(target) {
                MineResult::Blocked => self.active_drill = None,
                MineResult::TooDangerous => {
                    self.active_drill = None;
                    self.player.hull = (self.player.hull - 8.0).max(0.0);
                    self.sound_cues.push(SoundCue::Damage);
                    "Lava pocket! Hull scorched.".clone_into(&mut self.message);
                }
                MineResult::Exploded => {
                    self.active_drill = None;
                    self.trigger_gas_explosion();
                }
                MineResult::Chipped => {}
                MineResult::Mined(mined) => {
                    self.active_drill = None;
                    self.collect_mined_tile(mined, target);
                }
            }
        }
    }

    fn creep_into_drill(&mut self, direction: DrillDirection, delta_seconds: f32) {
        let creep = 32.0 * delta_seconds;
        match direction {
            DrillDirection::Down => self.move_axis(0.0, creep),
            DrillDirection::Left => self.move_axis(-creep * 0.65, 0.0),
            DrillDirection::Right => self.move_axis(creep * 0.65, 0.0),
        }
    }

    fn trigger_gas_explosion(&mut self) {
        self.player.fuel = (self.player.fuel - DRILL_FUEL_COST).max(0.0);
        self.player.velocity_x *= -0.25;
        self.player.velocity_y = -90.0;
        self.sound_cues.push(SoundCue::Damage);
        self.drill_flash_seconds = 0.2;
        self.hazard_clouds.push(HazardCloud {
            x: self.player.x,
            y: self.player.y + TILE_SIZE,
            life: 8.0,
            radius: 10.0,
        });
        for _ in 0..5 {
            self.spawn_dust();
        }
        "Gas pocket venting! Clear the green leak before it turns corrosive."
            .clone_into(&mut self.message);
    }

    fn collect_mined_tile(&mut self, mined: TileKind, target: TilePosition) {
        self.player.fuel -= DRILL_FUEL_COST;
        self.sound_cues.push(SoundCue::Drill);
        self.spawn_dust();
        self.drill_flash_seconds = 0.12;

        if let TileKind::Ore(mineral) = mined {
            if self.player.add_cargo(mineral) {
                self.message = format!("Loaded {} ore worth {}.", mineral.name(), mineral.value());
            } else {
                "Cargo full. Return to depot to sell.".clone_into(&mut self.message);
            }
        } else if let TileKind::Artifact(artifact) = mined {
            if self.player.add_artifact(artifact) {
                self.artifacts_found += 1;
                self.message = format!(
                    "Recovered {} artifact worth {}.",
                    artifact.name(),
                    artifact.value()
                );
            } else {
                "Cargo full. Return to depot to sell.".clone_into(&mut self.message);
            }
        } else {
            "Tunnel opened.".clone_into(&mut self.message);
        }
        if matches!(mined, TileKind::Stone | TileKind::HardRock)
            && falling_rock_roll(target, self.terrain.seed())
        {
            self.falling_boulders.push(FallingBoulder {
                x: target.x as f32 * TILE_SIZE + TILE_SIZE * 0.5,
                y: (target.y as f32 - 1.0) * TILE_SIZE,
                velocity_y: 0.0,
                warning_seconds: 0.55,
                life: 3.6,
            });
            self.sound_cues.push(SoundCue::Damage);
            self.shake_camera(0.18, 4.0);
            self.message.push_str(" Unstable rock falling!");
        }
    }

    fn update_particles(&mut self, delta_seconds: f32) {
        for particle in &mut self.dust_particles {
            particle.life -= delta_seconds;
            particle.y -= 18.0 * delta_seconds;
        }
        self.dust_particles.retain(|particle| particle.life > 0.0);
        for spark in &mut self.spark_particles {
            spark.life -= delta_seconds;
            spark.x += spark.velocity_x * delta_seconds;
            spark.y += spark.velocity_y * delta_seconds;
            spark.velocity_y += 180.0 * delta_seconds;
        }
        self.spark_particles.retain(|particle| particle.life > 0.0);
    }

    fn update_boulders(&mut self, delta_seconds: f32) {
        for boulder in &mut self.falling_boulders {
            boulder.life -= delta_seconds;
            if boulder.warning_seconds > 0.0 {
                boulder.warning_seconds -= delta_seconds;
                continue;
            }
            boulder.velocity_y = (boulder.velocity_y + GRAVITY * 0.8 * delta_seconds).min(520.0);
            boulder.y += boulder.velocity_y * delta_seconds;
        }

        let hit_player = self.falling_boulders.iter().any(|boulder| {
            if boulder.warning_seconds > 0.0 {
                return false;
            }
            let dx = self.player.x - boulder.x;
            let dy = self.player.y - boulder.y;
            dx.hypot(dy) <= PLAYER_RADIUS + 8.0
        });
        if hit_player {
            self.player.hull = (self.player.hull - 12.0).max(0.0);
            self.sound_cues.push(SoundCue::Damage);
            self.shake_camera(0.35, 9.0);
            self.spawn_sparks();
            "Falling boulder slammed the rig!".clone_into(&mut self.message);
        }

        self.falling_boulders.retain(|boulder| {
            boulder.life > 0.0
                && boulder.y < (self.terrain.height() as f32 - 1.0) * TILE_SIZE
                && !self.terrain.is_solid_at(TilePosition {
                    x: (boulder.x / TILE_SIZE).floor() as i32,
                    y: (boulder.y / TILE_SIZE).floor() as i32,
                })
        });
    }

    fn update_hazards(&mut self, delta_seconds: f32) {
        for cloud in &mut self.hazard_clouds {
            cloud.life -= delta_seconds;
            cloud.radius += 8.0 * delta_seconds;
        }
        self.hazard_clouds.retain(|cloud| cloud.life > 0.0);

        let in_gas = self.hazard_clouds.iter().any(|cloud| {
            if cloud.life >= 6.0 {
                return false;
            }
            let dx = self.player.x - cloud.x;
            let dy = self.player.y - cloud.y;
            dx.hypot(dy) <= cloud.radius
        });
        if in_gas {
            self.player.hull = (self.player.hull - 4.0 * delta_seconds).max(0.0);
            "Corrosive gas cloud eating hull plating!".clone_into(&mut self.message);
        }
    }

    #[allow(
        clippy::missing_const_for_fn,
        reason = "uses f32 max for camera shake state"
    )]
    fn shake_camera(&mut self, seconds: f32, strength: f32) {
        self.camera_shake_seconds = self.camera_shake_seconds.max(seconds);
        self.camera_shake_strength = self.camera_shake_strength.max(strength);
    }

    fn spawn_sparks(&mut self) {
        for index in 0..8 {
            let side = if index % 2 == 0 { -1.0 } else { 1.0 };
            self.spark_particles.push(SparkParticle {
                x: self.player.x + side * 8.0,
                y: self.player.y,
                velocity_x: side * (45.0 + index as f32 * 8.0),
                velocity_y: -80.0 + index as f32 * 12.0,
                life: 0.45,
            });
        }
    }

    fn spawn_dust(&mut self) {
        let base_x = self.player.x;
        let base_y = self.player.y + 18.0;
        self.dust_particles.push(DustParticle {
            x: base_x - 7.0,
            y: base_y,
            life: 0.35,
        });
        self.dust_particles.push(DustParticle {
            x: base_x + 7.0,
            y: base_y + 2.0,
            life: 0.28,
        });
    }

    fn update_depth_milestones(&mut self) {
        let current_tile = (self.player.y / TILE_SIZE).floor() as i32;
        self.deepest_tile_reached = self.deepest_tile_reached.max(current_tile);
        if self.deepest_tile_reached < self.next_milestone_tile {
            return;
        }

        let reward = u32::try_from(self.next_milestone_tile).unwrap_or(0) * 2;
        self.player.credits += reward;
        self.total_earnings += reward;
        self.sound_cues.push(SoundCue::Milestone);
        let unlock = match self.next_milestone_tile {
            20 => "Silver seams now appear in useful quantities.",
            40 => "Gold and relic pockets are becoming common.",
            60 => "Emerald, ruby, and heat hazards intensify below.",
            _ => "Diamond traces and Star Core readings strengthen below.",
        };
        self.message = format!(
            "Depth milestone reached: {}m. Survey bonus: {reward} credits. {unlock}",
            self.next_milestone_tile - 5
        );
        self.next_milestone_tile += 20;
    }

    fn apply_depth_pressure(&mut self, delta_seconds: f32) {
        let safe_depth = HEAT_START_DEPTH
            + f32::from(self.player.radiator_level.saturating_sub(1)) * 12.0 * TILE_SIZE;
        if self.player.y <= safe_depth {
            return;
        }

        let depth_factor = ((self.player.y - safe_depth) / (12.0 * TILE_SIZE)).max(1.0);
        let damage = HEAT_DAMAGE_PER_SECOND * depth_factor * delta_seconds;
        self.player.hull = (self.player.hull - damage).max(0.0);
        "Depth pressure overheating hull. Upgrade radiator.".clone_into(&mut self.message);
    }

    fn apply_lava_damage(&mut self, delta_seconds: f32) {
        let near_lava = (-2..=2).any(|dy| {
            (-2..=2).any(|dx| {
                let position = TilePosition {
                    x: (self.player.x / TILE_SIZE) as i32 + dx,
                    y: (self.player.y / TILE_SIZE) as i32 + dy,
                };
                self.terrain.is_lava_at(position)
            })
        });
        if !near_lava {
            return;
        }

        let damage = 9.0 * delta_seconds;
        self.player.hull = (self.player.hull - damage).max(0.0);
        self.sound_cues.push(SoundCue::Damage);
        "Lava heat is burning the hull!".clone_into(&mut self.message);
    }

    fn update_camera(&mut self, delta_seconds: f32) {
        let (target_x, target_y) = target_camera_offset(self);
        let blend = (delta_seconds * CAMERA_SMOOTHING).clamp(0.0, 1.0);
        self.camera_x += (target_x - self.camera_x) * blend;
        self.camera_y += (target_y - self.camera_y) * blend;
    }

    fn update_warning_messages(&mut self) {
        let low_fuel = self.player.fuel <= self.player.fuel_capacity * 0.18;
        let low_hull = self.player.hull <= self.player.max_hull() * 0.25;
        match (low_fuel, low_hull) {
            (true, true) => "CRITICAL: low fuel and damaged hull. Return to surface!"
                .clone_into(&mut self.message),
            (true, false) => "Warning: fuel reserves low.".clone_into(&mut self.message),
            (false, true) => "Warning: hull integrity low.".clone_into(&mut self.message),
            (false, false) => {}
        }
    }

    fn update_layer_band(&mut self) {
        let band = self.deepest_tile_reached / 20;
        if band <= self.current_layer_band {
            return;
        }
        self.current_layer_band = band;
        let layer = match band {
            1 => "Clay Belt",
            2 => "Silver Caverns",
            3 => "Thermal Strata",
            _ => "Core Fracture Zone",
        };
        self.message = format!("Entering {layer}. Hazards and ore density increased.");
    }

    fn update_status_messages(&mut self) {
        if self.message.starts_with("Warning:") || self.message.starts_with("CRITICAL:") {
            return;
        }
        if self.player.fuel <= self.player.fuel_capacity * 0.15 && self.player.y > 6.0 * TILE_SIZE {
            "CRITICAL: fuel reserve low. Return to the fuel station now."
                .clone_into(&mut self.message);
            return;
        }
        if self.player.cargo_used() >= self.player.cargo_capacity {
            "Warning: cargo hold full. Return to the depot or leave valuables behind."
                .clone_into(&mut self.message);
            return;
        }
        if let Some(zone) = self.current_zone {
            self.message = match zone {
                SurfaceZone::Fuel => {
                    "Fuel Station: press E to buy fuel (1 credit/unit).".to_owned()
                }
                SurfaceZone::Repair => {
                    "Repair Garage: press E to repair hull (2 credits/unit).".to_owned()
                }
                SurfaceZone::Depot => depot_prompt(self),
                SurfaceZone::Shop => shop_prompt(&self.player),
            };
        }
    }

    fn check_failure(&mut self) {
        if self.player.hull <= 0.0 {
            self.game_over = true;
            "Hull destroyed! Press E for emergency rescue.".clone_into(&mut self.message);
        } else if self.player.fuel <= 0.0 && self.player.y > 6.0 * TILE_SIZE {
            self.game_over = true;
            "Out of fuel underground! Press E for emergency rescue.".clone_into(&mut self.message);
        }
    }

    fn handle_rescue(&mut self, input: PlayerInput) {
        if !input.interact {
            return;
        }

        let fee = rescue_fee(self.player.y).min(self.player.credits);
        self.player.credits -= fee;
        self.rescue_count += 1;
        let lost_items = drop_half_cargo(&mut self.player);
        self.last_rescue_x = Some(self.player.x);
        self.last_rescue_y = Some(self.player.y);
        self.last_rescue_summary = format!("Fee: {fee} credits. Cargo lost: {lost_items}.");
        self.player.x = 12.0 * TILE_SIZE;
        self.player.y = 4.0 * TILE_SIZE;
        self.player.velocity_x = 0.0;
        self.player.velocity_y = 0.0;
        self.player.fuel = self.player.fuel_capacity * 0.5;
        self.player.hull = self.player.max_hull() * 0.5;
        self.game_over = false;
        self.message = format!("Emergency rescue completed. {}", self.last_rescue_summary);
    }
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

fn rescue_fee(player_y: f32) -> u32 {
    50 + ((player_y / TILE_SIZE).max(0.0) as u32 * 3)
}

fn drop_half_cargo(player: &mut Player) -> u32 {
    let mut lost = 0;
    for count in player.cargo.values_mut() {
        let dropped = (*count).div_ceil(2);
        *count -= dropped;
        lost += dropped;
    }
    player.cargo.retain(|_, count| *count > 0);

    for count in player.artifacts.values_mut() {
        let dropped = (*count).div_ceil(2);
        *count -= dropped;
        lost += dropped;
    }
    player.artifacts.retain(|_, count| *count > 0);
    lost
}

fn drill_seconds_per_chip(kind: TileKind, drill_strength: u8, direction: DrillDirection) -> f32 {
    let base = match kind {
        TileKind::Air => 0.0,
        TileKind::Dirt => 0.09,
        TileKind::Clay => 0.12,
        TileKind::Stone => 0.15,
        TileKind::HardRock => 0.19,
        TileKind::Lava | TileKind::Gas => 0.08,
        TileKind::Ore(_) => 0.16,
        TileKind::Artifact(_) => 0.21,
    };
    let drill_bonus = 1.0 + f32::from(drill_strength.saturating_sub(1)) * 0.28;
    let direction_penalty = if direction == DrillDirection::Down {
        1.0
    } else {
        1.45
    };
    (base * direction_penalty / drill_bonus).max(0.045)
}

fn mine_target(player: &Player, input: PlayerInput) -> Option<(TilePosition, DrillDirection)> {
    if !input.drill_down && input.horizontal == 0.0 {
        return None;
    }

    let current_tile = player.tile_position(TILE_SIZE);
    Some(if input.drill_down {
        (
            TilePosition {
                x: current_tile.x,
                y: current_tile.y + 1,
            },
            DrillDirection::Down,
        )
    } else {
        let facing = facing_direction(input.horizontal);
        (
            TilePosition {
                x: current_tile.x + facing,
                y: current_tile.y,
            },
            if facing < 0 {
                DrillDirection::Left
            } else {
                DrillDirection::Right
            },
        )
    })
}

fn surface_zone_at(x: f32, y: f32) -> Option<SurfaceZone> {
    if y > 5.5 * TILE_SIZE {
        return None;
    }

    match (x / TILE_SIZE).floor() as i32 {
        0..=7 => Some(SurfaceZone::Fuel),
        8..=15 => Some(SurfaceZone::Repair),
        16..=23 => Some(SurfaceZone::Depot),
        24..=35 => Some(SurfaceZone::Shop),
        _ => None,
    }
}

fn depot_prompt(game: &GameState) -> String {
    let contract = &game.contracts.active;
    format!(
        "Depot: E completes contract ({}/{}) {} for {} cr, otherwise sells cargo.",
        contract.progress(&game.player),
        contract.required,
        contract.target.name(),
        contract.reward
    )
}

fn shop_prompt(player: &Player) -> String {
    let offers = upgrade_offers(player);
    let mut prompt = String::from("Upgrade Shop: ");
    for (index, offer) in offers.iter().enumerate() {
        let label = if offer.level >= crate::economy::MAX_UPGRADE_LEVEL {
            "MAX".to_owned()
        } else {
            offer.cost.to_string()
        };
        let _ = write!(prompt, "{}:{}({label}) ", index + 1, offer.name);
    }
    prompt
}

const fn falling_rock_roll(position: TilePosition, seed: u64) -> bool {
    let value = seed
        ^ ((position.x as u64).wrapping_mul(0x9E37))
        ^ ((position.y as u64).wrapping_mul(0x85EB));
    value.is_multiple_of(9)
}

fn target_camera_offset(game: &GameState) -> (f32, f32) {
    let screen_width = 1280.0;
    let screen_height = 720.0;
    let max_x = game.terrain.width() as f32 * TILE_SIZE - screen_width;
    let max_y = game.terrain.height() as f32 * TILE_SIZE - screen_height;

    (
        (game.player.x - screen_width / 2.0).clamp(0.0, max_x),
        (game.player.y - screen_height / 2.0).clamp(0.0, max_y),
    )
}

fn collision_points(x: f32, y: f32) -> [TilePosition; 4] {
    [
        point_to_tile(x - PLAYER_RADIUS, y - PLAYER_RADIUS),
        point_to_tile(x + PLAYER_RADIUS, y - PLAYER_RADIUS),
        point_to_tile(x - PLAYER_RADIUS, y + PLAYER_RADIUS),
        point_to_tile(x + PLAYER_RADIUS, y + PLAYER_RADIUS),
    ]
}

fn point_to_tile(x: f32, y: f32) -> TilePosition {
    TilePosition {
        x: (x / TILE_SIZE).floor() as i32,
        y: (y / TILE_SIZE).floor() as i32,
    }
}

fn facing_direction(horizontal: f32) -> i32 {
    if horizontal < 0.0 { -1 } else { 1 }
}
