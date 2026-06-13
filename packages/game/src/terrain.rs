#![allow(
    clippy::cast_sign_loss,
    reason = "terrain bounds are validated before indexing"
)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum MineralKind {
    Copper,
    Iron,
    Silver,
    Gold,
    Emerald,
    Ruby,
    Diamond,
}

impl MineralKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Copper => "Copper",
            Self::Iron => "Iron",
            Self::Silver => "Silver",
            Self::Gold => "Gold",
            Self::Emerald => "Emerald",
            Self::Ruby => "Ruby",
            Self::Diamond => "Diamond",
        }
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        match self {
            Self::Copper => 8,
            Self::Iron => 14,
            Self::Silver => 26,
            Self::Gold => 44,
            Self::Emerald => 78,
            Self::Ruby => 120,
            Self::Diamond => 190,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum ArtifactKind {
    Fossil,
    OldCircuit,
    BuriedIdol,
    StarCore,
}

impl ArtifactKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fossil => "Fossil",
            Self::OldCircuit => "Old Circuit",
            Self::BuriedIdol => "Buried Idol",
            Self::StarCore => "Star Core",
        }
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        match self {
            Self::Fossil => 130,
            Self::OldCircuit => 240,
            Self::BuriedIdol => 420,
            Self::StarCore => 760,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum TileKind {
    Air,
    Dirt,
    Clay,
    Stone,
    HardRock,
    Lava,
    Gas,
    Ore(MineralKind),
    Artifact(ArtifactKind),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Tile {
    pub kind: TileKind,
    pub durability: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TilePosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Terrain {
    width: i32,
    height: i32,
    seed: u64,
    tiles: Vec<Tile>,
}

impl Terrain {
    #[cfg(test)]
    #[must_use]
    pub fn new(width: i32, height: i32) -> Self {
        Self::new_seeded(width, height, 0xD1_11_6A_4E)
    }

    #[must_use]
    pub fn new_seeded(width: i32, height: i32, seed: u64) -> Self {
        let mut tiles = Vec::with_capacity((width * height) as usize);

        for y in 0..height {
            for x in 0..width {
                let kind = generated_tile_kind(x, y, seed);
                tiles.push(Tile {
                    kind,
                    durability: tile_durability(kind),
                });
            }
        }

        Self {
            width,
            height,
            seed,
            tiles,
        }
    }

    #[must_use]
    pub const fn width(&self) -> i32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> i32 {
        self.height
    }

    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub fn tile(&self, position: TilePosition) -> Option<Tile> {
        self.index(position).map(|index| self.tiles[index])
    }

    #[must_use]
    pub fn is_solid_at(&self, position: TilePosition) -> bool {
        self.tile(position)
            .is_some_and(|tile| tile.kind != TileKind::Air)
    }

    #[must_use]
    pub fn is_lava_at(&self, position: TilePosition) -> bool {
        self.tile(position)
            .is_some_and(|tile| tile.kind == TileKind::Lava)
    }

    pub fn mine(&mut self, position: TilePosition, drill_strength: u8) -> MineResult {
        let Some(index) = self.index(position) else {
            return MineResult::Blocked;
        };
        let tile = &mut self.tiles[index];

        if tile.kind == TileKind::Air {
            return MineResult::Blocked;
        }

        if tile.kind == TileKind::Lava {
            return MineResult::TooDangerous;
        }

        if tile.kind == TileKind::Gas {
            tile.kind = TileKind::Air;
            tile.durability = 0;
            return MineResult::Exploded;
        }

        if tile_hardness(tile.kind) > drill_strength {
            return MineResult::TooHard;
        }

        tile.durability = tile.durability.saturating_sub(drill_strength);
        if tile.durability > 0 {
            return MineResult::Chipped;
        }

        let mined = tile.kind;
        tile.kind = TileKind::Air;
        MineResult::Mined(mined)
    }

    const fn index(&self, position: TilePosition) -> Option<usize> {
        if position.x < 0 || position.y < 0 || position.x >= self.width || position.y >= self.height
        {
            return None;
        }

        Some((position.y * self.width + position.x) as usize)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MineResult {
    Blocked,
    TooHard,
    TooDangerous,
    Exploded,
    Chipped,
    Mined(TileKind),
}

const fn generated_tile_kind(x: i32, y: i32, seed: u64) -> TileKind {
    if y <= 4 {
        return TileKind::Air;
    }

    if cave_air(x, y, seed) {
        return TileKind::Air;
    }

    if lava_pocket(x, y, seed) {
        return TileKind::Lava;
    }

    if gas_pocket(x, y, seed) {
        return TileKind::Gas;
    }

    if artifact_spot(x, y, seed) {
        return TileKind::Artifact(artifact_at_depth(x, y));
    }

    let base = match y {
        5..=14 => TileKind::Dirt,
        15..=29 => TileKind::Clay,
        30..=54 => TileKind::Stone,
        _ => TileKind::HardRock,
    };

    ore_or_base_tile(x, y, base, seed)
}

const fn cave_air(x: i32, y: i32, seed: u64) -> bool {
    if y < 12 {
        return false;
    }

    let cavern = seeded_hash(x / 5, y / 4, seed) % 47;
    let tunnel = seeded_hash(x, y, seed ^ 0xA5A5) % 97;
    cavern == 0 || tunnel == 0
}

const fn lava_pocket(x: i32, y: i32, seed: u64) -> bool {
    if y < 48 {
        return false;
    }

    let pocket = seeded_hash(x / 4, y / 3, seed ^ 0x1A5A) % 61;
    pocket == 0 || (pocket == 1 && y > 68)
}

const fn gas_pocket(x: i32, y: i32, seed: u64) -> bool {
    if y < 24 || y > 74 {
        return false;
    }

    seeded_hash(x / 3, y / 3, seed ^ 0x6A5).is_multiple_of(83)
}

const fn artifact_spot(x: i32, y: i32, seed: u64) -> bool {
    y > 20 && seeded_hash(x, y, seed ^ 0x5EED).is_multiple_of(211)
}

const fn artifact_at_depth(x: i32, y: i32) -> ArtifactKind {
    match y {
        0..=30 => ArtifactKind::Fossil,
        31..=50 => ArtifactKind::OldCircuit,
        51..=70 => {
            if (x + y).rem_euclid(2) == 0 {
                ArtifactKind::BuriedIdol
            } else {
                ArtifactKind::OldCircuit
            }
        }
        _ => ArtifactKind::StarCore,
    }
}

const fn ore_or_base_tile(x: i32, y: i32, base: TileKind, seed: u64) -> TileKind {
    if !patterned_ore(x, y, seed) {
        return base;
    }

    match y {
        0..=16 => TileKind::Ore(MineralKind::Copper),
        17..=26 => TileKind::Ore(if (x + y).rem_euclid(3) == 0 {
            MineralKind::Silver
        } else {
            MineralKind::Iron
        }),
        27..=42 => TileKind::Ore(if (x * 5 + y).rem_euclid(5) == 0 {
            MineralKind::Gold
        } else {
            MineralKind::Silver
        }),
        43..=62 => TileKind::Ore(match (x * 13 + y * 7).rem_euclid(7) {
            0 => MineralKind::Ruby,
            1 | 2 => MineralKind::Emerald,
            _ => MineralKind::Gold,
        }),
        _ => TileKind::Ore(match (x * 19 + y * 23).rem_euclid(9) {
            0 => MineralKind::Diamond,
            1 | 2 => MineralKind::Ruby,
            _ => MineralKind::Emerald,
        }),
    }
}

const fn patterned_ore(x: i32, y: i32, seed: u64) -> bool {
    let vein_a = seeded_hash(x, y, seed ^ 0xC0DE) % 37;
    let vein_b = seeded_hash(x, y, seed ^ 0xBEEF) % 53;
    let pocket = seeded_hash(x / 3, y / 3, seed ^ 0xFACE) % 29;
    vein_a <= 2 || vein_b <= 1 || pocket == 0
}

const fn seeded_hash(x: i32, y: i32, seed: u64) -> u64 {
    let mut value =
        seed ^ ((x as u64).wrapping_mul(0x9E37_79B1)) ^ ((y as u64).wrapping_mul(0x85EB_CA77));
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value
}

const fn tile_hardness(kind: TileKind) -> u8 {
    match kind {
        TileKind::Air | TileKind::Lava | TileKind::Gas => 0,
        TileKind::Dirt | TileKind::Ore(MineralKind::Copper | MineralKind::Iron) => 1,
        TileKind::Clay | TileKind::Ore(MineralKind::Silver | MineralKind::Gold) => 2,
        TileKind::Stone
        | TileKind::Ore(MineralKind::Emerald | MineralKind::Ruby)
        | TileKind::Artifact(ArtifactKind::Fossil | ArtifactKind::OldCircuit) => 3,
        TileKind::HardRock
        | TileKind::Ore(MineralKind::Diamond)
        | TileKind::Artifact(ArtifactKind::BuriedIdol | ArtifactKind::StarCore) => 4,
    }
}

const fn tile_durability(kind: TileKind) -> u8 {
    match kind {
        TileKind::Air => 0,
        TileKind::Dirt => 2,
        TileKind::Clay => 4,
        TileKind::Stone => 7,
        TileKind::HardRock => 11,
        TileKind::Lava | TileKind::Gas => 1,
        TileKind::Ore(mineral) => tile_hardness(TileKind::Ore(mineral)) + 2,
        TileKind::Artifact(artifact) => tile_hardness(TileKind::Artifact(artifact)) + 5,
    }
}

#[cfg(test)]
mod tests {
    use super::{Terrain, TileKind, TilePosition};

    #[test]
    fn generated_world_contains_caves_and_hazards() {
        let terrain = Terrain::new(120, 90);
        let mut air_below_surface = 0;
        let mut lava = 0;
        let mut gas = 0;

        for y in 8..terrain.height() {
            for x in 0..terrain.width() {
                match terrain
                    .tile(TilePosition { x, y })
                    .expect("tile exists")
                    .kind
                {
                    TileKind::Air => air_below_surface += 1,
                    TileKind::Lava => lava += 1,
                    TileKind::Gas => gas += 1,
                    _ => {}
                }
            }
        }

        assert!(air_below_surface > 0);
        assert!(lava > 0);
        assert!(gas > 0);
    }
}
