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
        PurchaseError, SurfaceZone, buy_upgrade, refuel, repair, sell_cargo, upgrade_offers,
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
const PLAYER_RADIUS: f32 = 12.0;
const SAFE_LANDING_SPEED: f32 = 330.0;
const CRASH_DAMAGE_SCALE: f32 = 0.11;
const HEAT_START_DEPTH: f32 = 18.0 * TILE_SIZE;
const HEAT_DAMAGE_PER_SECOND: f32 = 3.5;
const CAMERA_SMOOTHING: f32 = 8.0;
const WORLD_SEED: u64 = 0xD1_11_6A_4E;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RunMode {
    Title,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ModalScreen {
    Fuel,
    Repair,
    Depot,
    Shop,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct DustParticle {
    pub x: f32,
    pub y: f32,
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
    pub deepest_tile_reached: i32,
    pub next_milestone_tile: i32,
    pub game_over: bool,
    pub camera_x: f32,
    pub camera_y: f32,
    pub drill_flash_seconds: f32,
    pub dust_particles: Vec<DustParticle>,
    pub sound_cues: Vec<SoundCue>,
}

impl GameState {
    #[must_use]
    pub fn clone_for_save(&self) -> Self {
        let mut saved = self.clone();
        saved.run_mode = RunMode::Playing;
        saved.modal = None;
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
            deepest_tile_reached: 0,
            next_milestone_tile: 20,
            game_over: false,
            camera_x: 0.0,
            camera_y: 0.0,
            drill_flash_seconds: 0.0,
            dust_particles: Vec::new(),
            sound_cues: Vec::new(),
        }
    }

    pub fn update(&mut self, input: PlayerInput, delta_seconds: f32) {
        self.sound_cues.clear();
        self.handle_save_load(input);
        self.update_particles(delta_seconds);
        self.drill_flash_seconds = (self.drill_flash_seconds - delta_seconds).max(0.0);

        match self.run_mode {
            RunMode::Title => {
                if input.confirm {
                    self.run_mode = RunMode::Playing;
                    "Welcome to the dig site. Visit the depot for contracts."
                        .clone_into(&mut self.message);
                }
                return;
            }
            RunMode::Paused => {
                if input.pause || input.cancel || input.confirm {
                    self.run_mode = RunMode::Playing;
                }
                return;
            }
            RunMode::Playing => {}
        }

        if input.pause {
            self.run_mode = RunMode::Paused;
            return;
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

        self.handle_interaction(input);
        self.apply_movement(input, delta_seconds);
        self.try_mine(input);
        self.apply_depth_pressure(delta_seconds);
        self.apply_lava_damage(delta_seconds);
        self.update_depth_milestones();
        self.update_status_messages();
        self.check_failure();
        self.update_camera(delta_seconds);
    }

    fn handle_save_load(&mut self, input: PlayerInput) {
        if input.save {
            match save_game(self) {
                Ok(()) => "Game saved to drillgame-save.json.".clone_into(&mut self.message),
                Err(error) => self.message = format!("Save failed: {error}"),
            }
        }

        if input.load {
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
            let max_item = if modal == ModalScreen::Shop {
                upgrade_offers(&self.player).len() - 1
            } else {
                0
            };
            self.selected_menu_item = (self.selected_menu_item + 1).min(max_item);
        }

        if let Some(index) = input.selected_upgrade {
            self.selected_menu_item = index.min(upgrade_offers(&self.player).len() - 1);
        }

        if input.confirm {
            match modal {
                ModalScreen::Fuel => self.confirm_refuel(),
                ModalScreen::Repair => self.confirm_repair(),
                ModalScreen::Depot => self.confirm_depot(),
                ModalScreen::Shop => self.try_buy_upgrade(self.selected_menu_item),
            }
        }

        true
    }

    fn confirm_refuel(&mut self) {
        let cost = refuel(&mut self.player);
        self.message = if cost == 0 {
            "Fuel already full or no credits available.".to_owned()
        } else {
            format!("Fuel topped up for {cost} credits.")
        };
    }

    fn confirm_repair(&mut self) {
        let cost = repair(&mut self.player);
        self.message = if cost == 0 {
            "Hull already repaired or no credits available.".to_owned()
        } else {
            format!("Hull repaired for {cost} credits.")
        };
    }

    fn confirm_depot(&mut self) {
        if let Some(reward) = self.contracts.try_complete(&mut self.player) {
            self.sound_cues.push(SoundCue::Sell);
            self.message = format!("Contract complete! Bonus paid: {reward} credits.");
            return;
        }

        let payout = sell_cargo(&mut self.player);
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
                self.message = format!("Bought {} upgrade.", offer.name);
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
    }

    fn apply_movement(&mut self, input: PlayerInput, delta_seconds: f32) {
        let can_burn_fuel = self.player.fuel > 0.0;
        let engine_multiplier = 1.0 + f32::from(self.player.engine_level.saturating_sub(1)) * 0.18;

        self.player.velocity_x +=
            input.horizontal * HORIZONTAL_ACCELERATION * engine_multiplier * delta_seconds;

        if input.thrust && can_burn_fuel {
            self.player.velocity_y -= THRUST_ACCELERATION * engine_multiplier * delta_seconds;
            self.player.fuel = (self.player.fuel - FUEL_BURN_PER_SECOND * delta_seconds).max(0.0);
        }

        self.player.velocity_y += GRAVITY * delta_seconds;
        self.player.velocity_x *= DRAG.powf(delta_seconds * 60.0);
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
        self.message = format!("Hard landing! Hull took {damage:.0} damage.");
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

    fn try_mine(&mut self, input: PlayerInput) {
        let Some(target) = mine_target(&self.player, input) else {
            return;
        };

        if !input.drill_down && !self.is_grounded() {
            "Side drilling requires ground contact.".clone_into(&mut self.message);
            return;
        }

        if self.player.fuel < DRILL_FUEL_COST {
            "Out of fuel. Reach a fuel station or await rescue.".clone_into(&mut self.message);
            return;
        }

        match self.terrain.mine(target, self.player.drill_strength) {
            MineResult::Blocked => {}
            MineResult::TooHard => {
                "That layer is too hard. Upgrade your drill.".clone_into(&mut self.message);
            }
            MineResult::TooDangerous => {
                self.player.hull = (self.player.hull - 8.0).max(0.0);
                self.sound_cues.push(SoundCue::Damage);
                "Lava pocket! Hull scorched.".clone_into(&mut self.message);
            }
            MineResult::Chipped => {
                self.player.fuel -= DRILL_FUEL_COST;
                self.sound_cues.push(SoundCue::Drill);
                self.spawn_dust();
                self.drill_flash_seconds = 0.08;
                "Drilling...".clone_into(&mut self.message);
            }
            MineResult::Mined(mined) => self.collect_mined_tile(mined),
        }
    }

    fn collect_mined_tile(&mut self, mined: TileKind) {
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
    }

    fn update_particles(&mut self, delta_seconds: f32) {
        for particle in &mut self.dust_particles {
            particle.life -= delta_seconds;
            particle.y -= 18.0 * delta_seconds;
        }
        self.dust_particles.retain(|particle| particle.life > 0.0);
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

        self.sound_cues.push(SoundCue::Milestone);
        self.message = format!(
            "Depth milestone reached: {}m. Richer ore lies below.",
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
        if !collision_points(self.player.x, self.player.y)
            .iter()
            .any(|position| self.terrain.is_lava_at(*position))
        {
            return;
        }

        let damage = 24.0 * delta_seconds;
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

    fn update_status_messages(&mut self) {
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
        let lost_items = drop_half_cargo(&mut self.player);
        self.player.x = 12.0 * TILE_SIZE;
        self.player.y = 4.0 * TILE_SIZE;
        self.player.velocity_x = 0.0;
        self.player.velocity_y = 0.0;
        self.player.fuel = self.player.fuel_capacity * 0.5;
        self.player.hull = self.player.max_hull() * 0.5;
        self.game_over = false;
        self.message =
            format!("Emergency rescue completed. Fee: {fee} credits. Cargo lost: {lost_items}.");
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

fn mine_target(player: &Player, input: PlayerInput) -> Option<TilePosition> {
    if !input.drill_down && input.horizontal == 0.0 {
        return None;
    }

    let current_tile = player.tile_position(TILE_SIZE);
    Some(if input.drill_down {
        TilePosition {
            x: current_tile.x,
            y: current_tile.y + 1,
        }
    } else {
        TilePosition {
            x: current_tile.x + facing_direction(input.horizontal),
            y: current_tile.y,
        }
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
