//! Port contracts: trait definitions and the DTOs they need.
//!
//! Ports belong to the core side of the hexagon: they may reference domain
//! types but never adapter types. Adapters implement them behind the
//! one-way dependency direction `adapters -> ports -> domain`.

pub mod driven;
pub mod driving;
