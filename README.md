# Drillgame

A Rust-native 2D mining game mechanically inspired by classic drill/mining games, with original names, placeholder programmer art, and a desktop-first initial target.

## Development

```bash
cargo fmt
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run --bin drillgame
```

## Controls

- `A` / `D` or left/right arrows: move horizontally
- `W` / up arrow / space: thrust upward
- `S` / down arrow: drill downward faster
- `Esc`: quit

Return to the surface base to automatically sell cargo and refuel.
