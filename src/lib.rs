pub mod board;
pub mod bot;
pub mod game;
pub mod live;
pub mod movegen;
pub mod net;
pub mod piece;
#[cfg(not(target_arch = "wasm32"))]
pub mod ppt;
pub mod setups;
mod setups_data;
pub mod versus;
pub mod wasm;
