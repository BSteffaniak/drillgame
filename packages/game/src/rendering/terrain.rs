use std::collections::{BTreeMap, BTreeSet};

use raylib::prelude::*;

use crate::{
    game_state::{GameState, TILE_SIZE},
    terrain::{ArtifactKind, MineralKind, TileKind, TilePosition},
};

pub(super) const TERRAIN_CHUNK_SIZE_TILES: i32 = 32;

const TILE_SIZE_PIXELS: i32 = TILE_SIZE as i32;
const SKY_CLEAR_MAX_TILE_Y: i32 = 4;
const UNEXPLORED_GRADIENT_START_TILE_Y: i32 = SKY_CLEAR_MAX_TILE_Y + 1;
const UNEXPLORED_FULL_DARK_TILE_Y: i32 = 34;
const UNEXPLORED_MIN_DARK_ALPHA: i32 = 48;
const UNEXPLORED_MAX_DARK_ALPHA: i32 = 255;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ChunkPosition {
    x: i32,
    y: i32,
}

pub(super) struct TerrainRenderer {
    chunks: BTreeMap<ChunkPosition, TerrainChunk>,
    dirty_chunks: BTreeSet<ChunkPosition>,
    terrain_width: i32,
    terrain_height: i32,
}

struct TerrainChunk {
    texture: RenderTexture2D,
    tile_min_x: i32,
    tile_min_y: i32,
    tile_max_x: i32,
    tile_max_y: i32,
    pixel_width: i32,
    pixel_height: i32,
}

impl TerrainRenderer {
    pub(super) fn new(raylib: &mut RaylibHandle, thread: &RaylibThread, game: &GameState) -> Self {
        let mut renderer = Self {
            chunks: BTreeMap::new(),
            dirty_chunks: BTreeSet::new(),
            terrain_width: game.terrain.width(),
            terrain_height: game.terrain.height(),
        };
        renderer.recreate_chunks(raylib, thread, game);
        renderer
    }

    pub(super) fn mark_all_dirty(&mut self) {
        self.dirty_chunks.extend(self.chunks.keys().copied());
    }

    pub(super) fn mark_tile_dirty(&mut self, position: TilePosition) {
        if position.x < 0
            || position.y < 0
            || position.x >= self.terrain_width
            || position.y >= self.terrain_height
        {
            return;
        }
        self.dirty_chunks.insert(chunk_position_for_tile(position));
    }

    pub(super) fn sync(
        &mut self,
        raylib: &mut RaylibHandle,
        thread: &RaylibThread,
        game: &GameState,
    ) {
        if self.terrain_width != game.terrain.width()
            || self.terrain_height != game.terrain.height()
        {
            self.recreate_chunks(raylib, thread, game);
        }

        let dirty_chunks = std::mem::take(&mut self.dirty_chunks);
        for position in dirty_chunks {
            if let Some(chunk) = self.chunks.get_mut(&position) {
                chunk.rebuild(raylib, thread, game);
            }
        }
    }

    pub(super) fn draw(&self, draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>, camera: Vector2) {
        for chunk in self.chunks.values() {
            if chunk.is_visible(camera) {
                chunk.draw(draw);
            }
        }
    }

    fn recreate_chunks(
        &mut self,
        raylib: &mut RaylibHandle,
        thread: &RaylibThread,
        game: &GameState,
    ) {
        self.chunks.clear();
        self.dirty_chunks.clear();
        self.terrain_width = game.terrain.width();
        self.terrain_height = game.terrain.height();

        let chunk_columns = chunk_count(self.terrain_width);
        let chunk_rows = chunk_count(self.terrain_height);
        for y in 0..chunk_rows {
            for x in 0..chunk_columns {
                let position = ChunkPosition { x, y };
                let chunk = TerrainChunk::new(raylib, thread, position, game);
                self.chunks.insert(position, chunk);
                self.dirty_chunks.insert(position);
            }
        }
    }
}

impl TerrainChunk {
    fn new(
        raylib: &mut RaylibHandle,
        thread: &RaylibThread,
        position: ChunkPosition,
        game: &GameState,
    ) -> Self {
        let tile_min_x = position.x * TERRAIN_CHUNK_SIZE_TILES;
        let tile_min_y = position.y * TERRAIN_CHUNK_SIZE_TILES;
        let tile_max_x = (tile_min_x + TERRAIN_CHUNK_SIZE_TILES).min(game.terrain.width());
        let tile_max_y = (tile_min_y + TERRAIN_CHUNK_SIZE_TILES).min(game.terrain.height());
        let pixel_width = (tile_max_x - tile_min_x) * TILE_SIZE_PIXELS;
        let pixel_height = (tile_max_y - tile_min_y) * TILE_SIZE_PIXELS;
        let texture = raylib
            .load_render_texture(thread, pixel_width as u32, pixel_height as u32)
            .expect("terrain chunk render texture");
        texture
            .texture()
            .set_texture_filter(thread, TextureFilter::TEXTURE_FILTER_POINT);

        Self {
            texture,
            tile_min_x,
            tile_min_y,
            tile_max_x,
            tile_max_y,
            pixel_width,
            pixel_height,
        }
    }

    fn rebuild(&mut self, raylib: &mut RaylibHandle, thread: &RaylibThread, game: &GameState) {
        let mut texture_draw = raylib.begin_texture_mode(thread, &mut self.texture);
        texture_draw.clear_background(Color::BLANK);
        for tile_y in self.tile_min_y..self.tile_max_y {
            for tile_x in self.tile_min_x..self.tile_max_x {
                let position = TilePosition {
                    x: tile_x,
                    y: tile_y,
                };
                let Some(tile) = game.terrain.tile(position) else {
                    continue;
                };

                let explored = game.is_explored(position);
                if (explored || tile_y <= SKY_CLEAR_MAX_TILE_Y) && tile.kind == TileKind::Air {
                    continue;
                }

                let local_x = (tile_x - self.tile_min_x) * TILE_SIZE_PIXELS;
                let local_y = (tile_y - self.tile_min_y) * TILE_SIZE_PIXELS;
                texture_draw.draw_rectangle(
                    local_x,
                    local_y,
                    TILE_SIZE_PIXELS,
                    TILE_SIZE_PIXELS,
                    layer_tile_color(tile.kind, tile_y),
                );

                if !explored {
                    let darkness_alpha = unexplored_darkness_alpha(tile_y);
                    if darkness_alpha > 0 {
                        texture_draw.draw_rectangle(
                            local_x,
                            local_y,
                            TILE_SIZE_PIXELS,
                            TILE_SIZE_PIXELS,
                            Color::new(0, 0, 0, darkness_alpha),
                        );
                    }
                    continue;
                }

                if tile.kind != TileKind::Air {
                    draw_tile_texture(
                        &mut texture_draw,
                        local_x,
                        local_y,
                        tile_x,
                        tile_y,
                        tile.kind,
                    );
                }

                if tile.durability > 0 {
                    draw_tile_durability_lines(
                        &mut texture_draw,
                        local_x,
                        local_y,
                        tile_x,
                        tile_y,
                        ChunkTileBounds {
                            min_x: self.tile_min_x,
                            min_y: self.tile_min_y,
                            max_x: self.tile_max_x,
                            max_y: self.tile_max_y,
                        },
                    );
                }
            }
        }
    }

    fn draw(&self, draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>) {
        // RenderTexture2D content is vertically flipped when sampled back by raylib,
        // so source height is negative.
        let world_x = self.tile_min_x as f32 * TILE_SIZE;
        let world_y = self.tile_min_y as f32 * TILE_SIZE;
        draw.draw_texture_pro(
            self.texture.texture(),
            Rectangle::new(0.0, 0.0, self.pixel_width as f32, -self.pixel_height as f32),
            Rectangle::new(
                world_x,
                world_y,
                self.pixel_width as f32,
                self.pixel_height as f32,
            ),
            Vector2::zero(),
            0.0,
            Color::WHITE,
        );
    }

    fn is_visible(&self, camera: Vector2) -> bool {
        let world_x = self.tile_min_x as f32 * TILE_SIZE;
        let world_y = self.tile_min_y as f32 * TILE_SIZE;
        let width = self.pixel_width as f32;
        let height = self.pixel_height as f32;
        world_x + width >= camera.x
            && world_x <= camera.x + super::SCREEN_WIDTH as f32
            && world_y + height >= camera.y
            && world_y <= camera.y + super::SCREEN_HEIGHT as f32
    }
}

fn draw_tile_texture<D: RaylibDraw>(
    draw: &mut D,
    base_x: i32,
    base_y: i32,
    tile_x: i32,
    tile_y: i32,
    kind: TileKind,
) {
    let seed = texture_hash(tile_x, tile_y);
    let color = match kind {
        TileKind::Dirt
        | TileKind::Clay
        | TileKind::Stone
        | TileKind::HardRock
        | TileKind::Foundation => Color::new(255, 255, 255, 28),
        TileKind::Ore(_) => Color::new(255, 245, 180, 80),
        TileKind::Artifact(_) => Color::new(255, 120, 255, 95),
        TileKind::Gas => Color::new(120, 255, 120, 70),
        TileKind::Lava
        | TileKind::MagmaVent
        | TileKind::ExplosivePocket
        | TileKind::PressurePocket => Color::new(255, 120, 60, 85),
        TileKind::Air => return,
    };
    for index in 0..3 {
        let px = base_x as f32 + 4.0 + ((seed >> (index * 5)) & 15) as f32;
        let py = base_y as f32 + 4.0 + ((seed >> (index * 7 + 3)) & 15) as f32;
        draw.draw_circle_v(
            Vector2::new(px, py),
            if index == 0 { 1.8 } else { 1.2 },
            color,
        );
    }
}

#[derive(Clone, Copy)]
struct ChunkTileBounds {
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
}

fn draw_tile_durability_lines<D: RaylibDraw>(
    draw: &mut D,
    local_x: i32,
    local_y: i32,
    tile_x: i32,
    tile_y: i32,
    bounds: ChunkTileBounds,
) {
    let color = Color::new(0, 0, 0, 30);
    if tile_x >= bounds.min_x {
        draw.draw_line(local_x, local_y, local_x, local_y + TILE_SIZE_PIXELS, color);
    }
    if tile_y >= bounds.min_y {
        draw.draw_line(local_x, local_y, local_x + TILE_SIZE_PIXELS, local_y, color);
    }
    if tile_x + 1 < bounds.max_x {
        draw.draw_line(
            local_x + TILE_SIZE_PIXELS,
            local_y,
            local_x + TILE_SIZE_PIXELS,
            local_y + TILE_SIZE_PIXELS,
            color,
        );
    }
    if tile_y + 1 < bounds.max_y {
        draw.draw_line(
            local_x,
            local_y + TILE_SIZE_PIXELS,
            local_x + TILE_SIZE_PIXELS,
            local_y + TILE_SIZE_PIXELS,
            color,
        );
    }
}

pub(super) const fn layer_tile_color(kind: TileKind, y: i32) -> Color {
    let base = tile_color(kind);
    if matches!(
        kind,
        TileKind::Dirt
            | TileKind::Clay
            | TileKind::Stone
            | TileKind::HardRock
            | TileKind::Foundation
    ) {
        let depth = if y / 12 < 6 { y / 12 } else { 6 } as u8;
        return Color::new(
            base.r.saturating_sub(depth * 8),
            base.g.saturating_sub(depth * 6),
            base.b.saturating_sub(depth * 4),
            base.a,
        );
    }
    base
}

const fn unexplored_darkness_alpha(tile_y: i32) -> u8 {
    if tile_y <= SKY_CLEAR_MAX_TILE_Y {
        return 0;
    }

    let gradient_span = UNEXPLORED_FULL_DARK_TILE_Y - UNEXPLORED_GRADIENT_START_TILE_Y;
    if gradient_span <= 0 || tile_y >= UNEXPLORED_FULL_DARK_TILE_Y {
        return UNEXPLORED_MAX_DARK_ALPHA as u8;
    }

    let distance = tile_y - UNEXPLORED_GRADIENT_START_TILE_Y;
    let alpha_range = UNEXPLORED_MAX_DARK_ALPHA - UNEXPLORED_MIN_DARK_ALPHA;
    (UNEXPLORED_MIN_DARK_ALPHA + (alpha_range * distance) / gradient_span) as u8
}

const fn tile_color(kind: TileKind) -> Color {
    match kind {
        TileKind::Air => Color::BLANK,
        TileKind::Dirt => Color::new(118, 75, 42, 255),
        TileKind::Clay => Color::new(145, 80, 68, 255),
        TileKind::Stone => Color::new(93, 87, 82, 255),
        TileKind::HardRock => Color::new(54, 50, 58, 255),
        TileKind::Foundation => Color::new(82, 78, 72, 255),
        TileKind::Lava => Color::ORANGE,
        TileKind::Gas => Color::LIME,
        TileKind::ExplosivePocket => Color::MAROON,
        TileKind::PressurePocket => Color::SKYBLUE,
        TileKind::MagmaVent => Color::new(255, 36, 12, 255),
        TileKind::Ore(mineral) => mineral_color(mineral),
        TileKind::Artifact(artifact) => artifact_color(artifact),
    }
}

pub(super) const fn mineral_color(mineral: MineralKind) -> Color {
    match mineral {
        MineralKind::Copper => Color::new(184, 102, 42, 255),
        MineralKind::Iron => Color::new(168, 150, 132, 255),
        MineralKind::Silver => Color::new(205, 220, 225, 255),
        MineralKind::Gold => Color::GOLD,
        MineralKind::Emerald => Color::GREEN,
        MineralKind::Ruby => Color::RED,
        MineralKind::Diamond => Color::SKYBLUE,
        MineralKind::Platinum => Color::new(210, 230, 245, 255),
        MineralKind::Uranium => Color::LIME,
        MineralKind::Mythril => Color::VIOLET,
    }
}

pub(super) const fn artifact_color(artifact: ArtifactKind) -> Color {
    match artifact {
        ArtifactKind::Fossil => Color::BEIGE,
        ArtifactKind::OldCircuit => Color::VIOLET,
        ArtifactKind::BuriedIdol => Color::PINK,
        ArtifactKind::StarCore => Color::new(120, 220, 255, 255),
    }
}

const fn texture_hash(x: i32, y: i32) -> u32 {
    let ux = x as u32;
    let uy = y as u32;
    ux.wrapping_mul(73_856_093) ^ uy.wrapping_mul(19_349_663) ^ 0x9E37_79B9
}

const fn chunk_count(tile_count: i32) -> i32 {
    (tile_count + TERRAIN_CHUNK_SIZE_TILES - 1) / TERRAIN_CHUNK_SIZE_TILES
}

const fn chunk_position_for_tile(position: TilePosition) -> ChunkPosition {
    ChunkPosition {
        x: position.x / TERRAIN_CHUNK_SIZE_TILES,
        y: position.y / TERRAIN_CHUNK_SIZE_TILES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unexplored_darkness_keeps_sky_clear() {
        assert_eq!(unexplored_darkness_alpha(SKY_CLEAR_MAX_TILE_Y), 0);
    }

    #[test]
    fn unexplored_darkness_increases_with_depth() {
        let shallow = unexplored_darkness_alpha(UNEXPLORED_GRADIENT_START_TILE_Y);
        let middle = unexplored_darkness_alpha(UNEXPLORED_GRADIENT_START_TILE_Y + 10);
        let deep = unexplored_darkness_alpha(UNEXPLORED_FULL_DARK_TILE_Y);

        assert!(shallow < middle);
        assert!(middle < deep);
        assert_eq!(deep, 255);
    }
}
