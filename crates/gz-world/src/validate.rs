//! Coded problems of the mode's world, the way quests and characters report them: `E2nn` stops a
//! build, `W2nn` is a warning.
//!
//! `E201` more than one Player Start, `E202` more than three player choices, `W203` a level the
//! linked game lacks, `W204` a transition the linked game lacks, `W205` the player starts outside
//! the area, `W206` an area without a Player Start, `E207` a start kit row with no item or a
//! count below 1, or an item listed twice, `W208` a start kit item neither the linked game nor the module has.

use gz_mod::{Issue, Spot};

use crate::game::GameWorld;
use crate::model::{MAX_PLAYER_CHOICES, StartKit, World};
use crate::starts::Start;

/// The problems of a world as lines, each starting with its code (see [`issues`]).
#[must_use]
pub fn validate(
    w: &World,
    k: &StartKit,
    starts: &[Start],
    game: Option<&GameWorld>,
    known_item: Option<&dyn Fn(&str) -> bool>,
) -> Vec<String> {
    issues(w, k, starts, game, known_item).iter().map(Issue::line).collect()
}

/// The problems of a world, each with the section and the key of `mod.ltx` it is about (a
/// Player Start is no line of it: `[world]`'s header stands for it). `game`: the linked game's
/// world when it was read; `known_item`: whether the linked game has an item section, when the
/// pickers are backed by a game.
#[must_use]
pub fn issues(
    w: &World,
    k: &StartKit,
    starts: &[Start],
    game: Option<&GameWorld>,
    known_item: Option<&dyn Fn(&str) -> bool>,
) -> Vec<Issue> {
    let mut problems = Vec::new();
    let world = Spot::section("world");
    let key = |name: &str| Spot::key("world", name);
    if starts.len() > 1 {
        let list: Vec<String> =
            starts.iter().map(|s| format!("{}:{} ({})", s.level, s.name, s.source)).collect();
        problems.push(Issue::new(
            "E201",
            format!("world: {} Player Starts - a module has one: {}", starts.len(), list.join(", ")),
            world.clone(),
        ));
    }
    if w.choice_count() > MAX_PLAYER_CHOICES {
        problems.push(Issue::new("E202", format!("world: {} rules offered to the player - the new-game screen has room for {MAX_PLAYER_CHOICES}", w.choice_count()), key("player_choice")));
    }

    if let Some(g) = game.filter(|g| g.ok) {
        for (name, list) in [("levels", &w.levels), ("area", &w.area), ("locked_levels", &w.locked_levels)] {
            for l in list.iter().filter(|l| !g.level_exists(l)) {
                problems.push(Issue::new(
                    "W203",
                    format!("world: {name} names '{l}', a level the linked game does not have"),
                    key(name),
                ));
            }
        }
        for t in w.locked.iter().filter(|t| g.transition(t).is_none()) {
            problems.push(Issue::new(
                "W204",
                format!("world: locked names '{t}', a transition the linked game does not have"),
                key("locked"),
            ));
        }
        for s in starts.iter().filter(|s| !g.level_exists(&s.level)) {
            problems.push(Issue::new(
                "W203",
                format!(
                    "world: the Player Start {} stands on '{}', a level the linked game does not have",
                    s.name, s.level
                ),
                world.clone(),
            ));
        }
    }
    if !w.area.is_empty() {
        if starts.is_empty() {
            problems.push(Issue::new(
                "W206",
                "world: the playable area is limited, but there is no Player Start - the player begins at the faction's start, which may lie outside it"
                    .to_owned(),
                key("area"),
            ));
        }
        for s in starts.iter().filter(|s| !w.area.contains(&s.level)) {
            problems.push(Issue::new(
                "W205",
                format!("world: the Player Start stands on {}, outside the playable area", s.level),
                key("area"),
            ));
        }
    }

    // a row the game cannot use is the author's slip; an item the linked game lacks may be one
    // the module brings, so that one only warns (the characters' W117)
    if k.declared {
        let mut seen: Vec<&str> = Vec::new();
        for it in &k.items {
            if it.section.is_empty() {
                problems.push(Issue::new(
                    "E207",
                    "world: a start kit row names no item".to_owned(),
                    Spot::section("start_kit"),
                ));
                continue;
            }
            let row = Spot::key("start_kit", &it.section);
            if it.count < 1 {
                problems.push(Issue::new(
                    "E207",
                    format!(
                        "world: start kit item '{}' has a count of {} - at least 1",
                        it.section, it.count
                    ),
                    row.clone(),
                ));
            }
            if seen.contains(&it.section.as_str()) {
                problems.push(Issue::new(
                    "E207",
                    format!("world: start kit item '{}' is listed twice - one row says how many", it.section),
                    // the second row is the one that is too many
                    Spot::nth("start_kit", &it.section, 1),
                ));
            }
            seen.push(&it.section);
            if let Some(known) = known_item
                && !known(&it.section)
            {
                problems.push(Issue::new(
                    "W208",
                    format!(
                        "world: start kit item '{}' is an item of neither the linked game nor the module - a new game leaves it out",
                        it.section
                    ),
                    row,
                ));
            }
        }
    }
    problems
}
