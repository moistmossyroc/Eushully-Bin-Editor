//! Per-game / per-engine-build profiles, keyed by the AGE build version stamped
//! in the BIN signature (e.g. `SYS4443` → 443). Within one `sys_version` the
//! build number is the real discriminator for the engine's one-offs — speaker and
//! box-transition mechanisms differ between Eushully titles — so this module is
//! the home for per-build handling decisions (used by speaker/bridge detection).
//!
//! Known builds (observed): 415 = Ikusa Megami Zero, 443 = Madou Koukaku,
//! 448 = Sankai no Yubiwa. Unknown builds fall back to the generic heuristics.

use serde::Serialize;

/// A known Eushully game/build, identified by `(sys_version, engine_version)`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct GameProfile {
    /// Human-readable game name for the editor's engine indicator.
    pub name: &'static str,
}

/// Look up the profile for a BIN's `(sys_version, engine_version)`. `None` for
/// unknown builds (caller shows a generic `SYS<n> (v###)` label and uses the
/// version-agnostic heuristics).
pub fn profile_for(sys_version: u8, engine_version: u16) -> Option<GameProfile> {
    let name = match (sys_version, engine_version) {
        (4, 415) => "Ikusa Megami Zero",
        (4, 443) => "Madou Koukaku",
        (4, 448) => "Sankai no Yubiwa",
        _ => return None,
    };
    Some(GameProfile { name })
}

/// Engine indicator label for the UI: `"Madou Koukaku · SYS4 (v443)"` when the
/// build is known, else `"SYS4 (v443)"`.
pub fn engine_label(sys_version: u8, engine_version: u16) -> String {
    match profile_for(sys_version, engine_version) {
        Some(p) => format!("{} · SYS{} (v{})", p.name, sys_version, engine_version),
        None => format!("SYS{} (v{})", sys_version, engine_version),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_builds_resolve() {
        assert_eq!(profile_for(4, 443).unwrap().name, "Madou Koukaku");
        assert_eq!(profile_for(4, 415).unwrap().name, "Ikusa Megami Zero");
        assert_eq!(engine_label(4, 448), "Sankai no Yubiwa · SYS4 (v448)");
        assert!(profile_for(4, 999).is_none());
        assert_eq!(engine_label(4, 999), "SYS4 (v999)");
    }
}
