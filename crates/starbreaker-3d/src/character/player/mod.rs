//! Player packages: authored ports, Mannequin queries and locomotion assets.
mod blend_space;
mod clips;
mod diagnostics;
mod motion;
mod ports;
mod xml;

#[cfg(test)]
mod tests;

mod export;
mod loadouts;
mod sources;
pub use export::export_player;
pub use loadouts::{PlayerLoadout, PlayerRequest, player_loadouts};
