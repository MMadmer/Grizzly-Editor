//! Inheritance: `base` names another character to start from - every field this one does not
//! set comes from there, and the kits are MERGED (the base's rows plus these, a row of the same
//! section replacing the inherited count in its place, `count = 0` taking one away). A character
//! marked `abstract` exists only to be inherited from (`EditorCharacters::Resolve`).

use crate::model::{Character, Supply};

/// `src` wins wherever it actually says something; everything else stays (`ApplyOver`).
pub fn apply_over(dst: &mut Character, src: &Character) {
    if !src.name.is_nil() {
        dst.name = src.name.clone();
    }
    if !src.bio.is_nil() {
        dst.bio = src.bio.clone();
    }
    for (to, from) in [
        (&mut dst.community, &src.community),
        (&mut dst.visual, &src.visual),
        (&mut dst.icon, &src.icon),
        (&mut dst.start_dialog, &src.start_dialog),
        (&mut dst.spec_rank, &src.spec_rank),
    ] {
        if !from.is_empty() {
            from.clone_into(to);
        }
    }
    if src.has_rank {
        dst.rank = src.rank;
        dst.has_rank = true;
    }
    if src.has_reputation {
        dst.reputation = src.reputation;
        dst.has_reputation = true;
    }
    if src.has_money {
        dst.has_money = true;
        dst.money_min = src.money_min;
        dst.money_max = src.money_max;
        dst.money_infinite = src.money_infinite;
    }
    if src.has_squad {
        dst.squad = src.squad;
        dst.has_squad = true;
    }
    // a shop is one thing: a character that says anything about trade says all of it
    if src.trade.declared {
        dst.trade = src.trade.clone();
    }
    // the kit: the inherited rows keep their place, a repeat of a section takes over its count,
    // a new section is appended, and zero means "not this one"
    for row in &src.supplies {
        match dst.supplies.iter_mut().find(|d| d.section.eq_ignore_ascii_case(&row.section)) {
            Some(d) => d.count = row.count,
            None => dst.supplies.push(row.clone()),
        }
    }
}

/// Rows of no item left after the merge (`DropEmptyRows`).
pub fn drop_empty_rows(rows: &mut Vec<Supply>) {
    rows.retain(|r| r.count >= 1);
}

/// Where a problem of a character is: its file, or its id when it has none.
fn place(c: &Character) -> &str {
    if c.file.is_empty() { &c.id } else { &c.file }
}

/// The longest chain walked before it counts as a loop.
const MAX_CHAIN: usize = 32;

/// Flattens every `base` chain in place: each character comes back with the fields and the kit
/// it really has, and the defaults (`stalker`, `novice`) where nothing set them. A missing base
/// is `E111`, a cycle `E112` - reported, never looped over; such a character keeps its own
/// fields. Abstract characters stay in the list: the editor still shows them, the build is
/// what leaves them out.
pub fn resolve(all: &mut [Character], problems: &mut Vec<String>) {
    // the chain is walked over the originals
    let raw: Vec<Character> = all.to_vec();
    let by_id = |id: &str| raw.iter().position(|c| c.id == id);

    for c in all.iter_mut() {
        let place = place(c).to_owned();
        if !c.base.is_empty() {
            let mut chain: Vec<usize> = Vec::new();
            let mut next = c.base.clone();
            let mut broken = false;
            while !next.is_empty() {
                if next == c.id {
                    problems.push(format!("E112 {place}: '{}' inherits from itself", c.id));
                    broken = true;
                    break;
                }
                let Some(b) = by_id(&next) else {
                    problems.push(format!("E111 {place}: there is no character '{next}' to inherit from"));
                    broken = true;
                    break;
                };
                if chain.contains(&b) || chain.len() > MAX_CHAIN {
                    problems.push(format!("E112 {place}: the base chain of '{}' comes back to where it started", c.id));
                    broken = true;
                    break;
                }
                chain.push(b);
                next = raw.get(b).map(|x| x.base.clone()).unwrap_or_default();
            }
            if broken {
                drop_empty_rows(&mut c.supplies);
                continue;
            }

            let mut merged = Character::default();
            // deepest base first, then each one closer, then the character itself
            for &k in chain.iter().rev() {
                if let Some(b) = raw.get(k) {
                    apply_over(&mut merged, b);
                }
            }
            apply_over(&mut merged, c);
            merged.file = std::mem::take(&mut c.file);
            merged.id = std::mem::take(&mut c.id);
            merged.base = std::mem::take(&mut c.base);
            merged.is_abstract = c.is_abstract;
            *c = merged;
        }

        drop_empty_rows(&mut c.supplies);
        // the defaults, once, where they cannot override a base
        if c.community.is_empty() {
            "stalker".clone_into(&mut c.community);
        }
        if c.spec_rank.is_empty() {
            "novice".clone_into(&mut c.spec_rank);
        }
    }
}
