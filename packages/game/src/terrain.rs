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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum TileKind {
    Air,
    Dirt,
    Clay,
    Stone,
    HardRock,
    Ore(MineralKind),
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

#[derive(Debug, Deserialize, Serialize)]
pub struct Terrain {
    width: i32,
    height: i32,
    tiles: Vec<Tile>,
}

impl Terrain {
    #[must_use]
    pub fn new(width: i32, height: i32) -> Self {
        let mut tiles = Vec::with_capacity((width * height) as usize);

        for y in 0..height {
            for x in 0..width {
                let kind = generated_tile_kind(x, y);
                tiles.push(Tile {
                    kind,
                    durability: tile_durability(kind),
                });
            }
        }

        Self {
            width,
            height,
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
    pub fn tile(&self, position: TilePosition) -> Option<Tile> {
        self.index(position).map(|index| self.tiles[index])
    }

    #[must_use]
    pub fn is_solid_at(&self, position: TilePosition) -> bool {
        self.tile(position)
            .is_some_and(|tile| tile.kind != TileKind::Air)
    }

    pub fn mine(&mut self, position: TilePosition, drill_strength: u8) -> MineResult {
        let Some(index) = self.index(position) else {
            return MineResult::Blocked;
        };
        let tile = &mut self.tiles[index];

        if tile.kind == TileKind::Air {
            return MineResult::Blocked;
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
    Chipped,
    Mined(TileKind),
}

const fn generated_tile_kind(x: i32, y: i32) -> TileKind {
    match y {
        0..=4 => TileKind::Air,
        5..=14 => ore_or_base_tile(x, y, TileKind::Dirt),
        15..=29 => ore_or_base_tile(x, y, TileKind::Clay),
        30..=54 => ore_or_base_tile(x, y, TileKind::Stone),
        _ => ore_or_base_tile(x, y, TileKind::HardRock),
    }
}

const fn ore_or_base_tile(x: i32, y: i32, base: TileKind) -> TileKind {
    if !patterned_ore(x, y) {
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

const fn patterned_ore(x: i32, y: i32) -> bool {
    let vein_a = (x * 17 + y * 31).rem_euclid(37);
    let vein_b = (x * 7 + y * 11).rem_euclid(53);
    let pocket = ((x / 3) * 19 + (y / 3) * 23).rem_euclid(29);
    vein_a <= 2 || vein_b <= 1 || pocket == 0
}

const fn tile_hardness(kind: TileKind) -> u8 {
    match kind {
        TileKind::Air => 0,
        TileKind::Dirt | TileKind::Ore(MineralKind::Copper | MineralKind::Iron) => 1,
        TileKind::Clay | TileKind::Ore(MineralKind::Silver | MineralKind::Gold) => 2,
        TileKind::Stone | TileKind::Ore(MineralKind::Emerald | MineralKind::Ruby) => 3,
        TileKind::HardRock | TileKind::Ore(MineralKind::Diamond) => 4,
    }
}

const fn tile_durability(kind: TileKind) -> u8 {
    match kind {
        TileKind::Air => 0,
        TileKind::Dirt => 2,
        TileKind::Clay => 4,
        TileKind::Stone => 7,
        TileKind::HardRock => 11,
        TileKind::Ore(mineral) => tile_hardness(TileKind::Ore(mineral)) + 2,
    }
}
