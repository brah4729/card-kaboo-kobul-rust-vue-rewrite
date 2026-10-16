//! All rejection messages are deliberately generic and identical in tone.
//!
//! A rejection reason is itself information. "That card doesn't match" would
//! tell the actor what an opponent's face-down card is; nothing that depends on
//! a hidden card may ever be distinguished. When in doubt, return `illegal()`.

use crate::types::{EngineError, EngineErrorCode};

pub fn message(code: EngineErrorCode) -> &'static str {
    match code {
        EngineErrorCode::Illegal => "That isn't allowed right now.",
        EngineErrorCode::NotYourTurn => "It isn't your turn.",
        EngineErrorCode::Stale => "The table moved on. Refresh to catch up.",
        EngineErrorCode::TooLate => "Too late.",
        EngineErrorCode::UnknownPlayer => "Unknown player.",
        EngineErrorCode::RoomFull => "That room is full.",
        EngineErrorCode::GameOver => "That game has finished.",
    }
}

pub fn err(code: EngineErrorCode) -> EngineError {
    EngineError { ok: false, code, message: message(code).to_string() }
}

/// The catch-all: never say why, when why could leak.
pub fn illegal() -> EngineError {
    err(EngineErrorCode::Illegal)
}

/// What a handler returns: `Ok(())` when the action applied.
pub type Handled = Result<(), EngineError>;
