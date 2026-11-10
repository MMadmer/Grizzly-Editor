//! Coded problems of the mode's world, the way quests and characters report them: `E2nn` stops a
//! build, `W2nn` is a warning (`EditorWorld::Validate`).
//!
//! `E201` more than one Player Start, `E202` more than three player choices, `W203` a level the
//! linked game lacks, `W204` a transition the linked game lacks, `W205` the player starts outside
//! the area, `W206` an area without a Player Start, `E207` a start kit row with no item or a
//! count below 1, or an item listed twice, `W208` a start kit item the linked game lacks.

use crate::game::GameWorld;
use crate::model::{MAX_PLAYER_CHOICES, StartKit, World};
use crate::starts::Start;

/// The problems of a world. `game`: the linked game's world when it was read; `known_item`:
/// whether the linked game has an item section, when the pickers are backed by a game.
#[must_use]
pub fn validate(w: &World, k: &StartKit, starts: &[Start], game: Option<&GameWorld>, known_item: Option<&dyn Fn(&str) -> bool>) -> Vec<String> {
    let mut problems = Vec::new();
    if starts.len() > 1 {
        let list: Vec<String> = starts.iter().map(|s| format!("{}:{} ({})", s.level, s.name, s.source)).collect();
        problems.push(format!("E201 world: {} Player Starts - a module has one: {}", starts.len(), list.join(", ")));
    }
    if w.choice_count() > MAX_PLAYER_CHOICES {
        problems.push(format!("E202 world: {} rules offered to the player - the new-game screen has room for {MAX_PLAYER_CHOICES}", w.choice_count()));
    }

    if let Some(g) = game.filter(|g| g.ok) {
        for (key, list) in [("levels", &w.levels), ("area", &w.area), ("locked_levels", &w.locked_levels)] {
            for l in list.iter().filter(|l| !g.level_exists(l)) {
                problems.push(format!("W203 world: {key} names '{l}', a level the linked game does not have"));
            }
        }
        for t in w.locked.iter().filter(|t| g.transition(t).is_none()) {
            problems.push(format!("W204 world: locked names '{t}', a transition the linked game does not have"));
        }
        for s in starts.iter().filter(|s| !g.level_exists(&s.level)) {
            problems.push(format!("W203 world: the Player Start {} stands on '{}', a level the linked game does not have", s.name, s.level));
        }
    }
    if !w.area.is_empty() {
        if starts.is_empty() {
            problems.push(
                "W206 world: the playable area is limited, but there is no Player Start - the player begins at the faction's start, which may lie outside it"
                    .to_owned(),
            );
        }
        for s in starts.iter().filter(|s| !w.area.contains(&s.level)) {
            problems.push(format!("W205 world: the Player Start stands on {}, outside the playable area", s.level));
        }
    }

    // a row the game cannot use is the author's slip; an item the linked game lacks may be one
    // the module brings, so that one only warns (the characters' W117)
    if k.declared {
        let mut seen: Vec<&str> = Vec::new();
        for it in &k.items {
            if it.section.is_empty() {
                problems.push("E207 world: a start kit row names no item".to_owned());
                continue;
            }
            if it.count < 1 {
                problems.push(format!("E207 world: start kit item '{}' has a count of {} - at least 1", it.section, it.count));
            }
            if seen.contains(&it.section.as_str()) {
                problems.push(format!("E207 world: start kit item '{}' is listed twice - one row says how many", it.section));
            }
            seen.push(&it.section);
            if let Some(known) = known_item
                && !known(&it.section)
            {
                problems.push(format!(
                    "W208 world: start kit item '{}' is not an item of the linked game - a new game leaves it out unless the module brings it",
                    it.section
                ));
            }
        }
    }
    problems
}
