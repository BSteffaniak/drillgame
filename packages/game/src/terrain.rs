#![allow(
    clippy::cast_sign_loss,
    reason = "terrain bounds are validated before indexing"
)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TileKind {
    Air,
    Dirt,
    Stone,
    Ore,
}

#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub kind: TileKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TilePosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug)]
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
                tiles.push(Tile {
                    kind: generated_tile_kind(x, y),
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

    pub fn mine(&mut self, position: TilePosition, drill_strength: u8) -> Option<TileKind> {
        let index = self.index(position)?;
        let kind = self.tiles[index].kind;

        if kind == TileKind::Air || tile_hardness(kind) > drill_strength {
            return None;
        }

        self.tiles[index].kind = TileKind::Air;
        Some(kind)
    }

    const fn index(&self, position: TilePosition) -> Option<usize> {
        if position.x < 0 || position.y < 0 || position.x >= self.width || position.y >= self.height
        {
            return None;
        }

        Some((position.y * self.width + position.x) as usize)
    }
}

const fn generated_tile_kind(x: i32, y: i32) -> TileKind {
    match y {
        0..=4 => TileKind::Air,
        5 => TileKind::Dirt,
        _ if y > 30 && patterned_ore(x, y) => TileKind::Ore,
        _ if y > 18 => TileKind::Stone,
        _ => TileKind::Dirt,
    }
}

const fn patterned_ore(x: i32, y: i32) -> bool {
    (x * 17 + y * 31).rem_euclid(29) == 0 || (x * 7 + y * 11).rem_euclid(43) == 0
}

const fn tile_hardness(kind: TileKind) -> u8 {
    match kind {
        TileKind::Air => 0,
        TileKind::Dirt | TileKind::Ore => 1,
        TileKind::Stone => 2,
    }
}
