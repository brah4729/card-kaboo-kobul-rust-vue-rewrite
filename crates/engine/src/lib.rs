//! Kaboo's game engine. PURE: no IO, no clock, no randomness of its own — both
//! arrive through [`Ctx`]. Never depends on the room, the Worker, or the web.
//!
//! The public surface, kept stable:
//! [`apply_action`] · [`tick`] · [`start_game`] · [`project`] · [`next_deadline`].

pub mod cards;
pub mod effects;
pub mod errors;
pub mod events;
pub mod project;
pub mod reduce;
pub mod rules;
pub mod scoring;
pub mod state;
pub mod tap;
pub mod turn;
pub mod types;

pub use events::{EngineEvent, EventType};
pub use project::{Projection, legal_actions, peekable_slots, project};
pub use reduce::{ApplyResult, apply_action, next_deadline, start_game, tick};
pub use state::{MAX_PLAYERS, MIN_PLAYERS, PEEK_SLOTS, create_game};
pub use types::*;

#[cfg(feature = "testkit")]
pub mod testkit;
