//! The major tabs as data: the strip's layout and the session file, both pure.

use gz_ui::Icon;

/// The id of the Level tab, which is always first and never closes.
pub const LEVEL: &str = "level";

/// What a tab of the strip measures, in pt.
pub const W_MIN: f32 = 96.0;
pub const W_MAX: f32 = 220.0;
pub const PAD_L: f32 = 10.0;
pub const PAD_R: f32 = 6.0;
pub const ICON: f32 = 16.0;
pub const GAP: f32 = 6.0;
pub const SLOT: f32 = 16.0;
pub const SPACING: f32 = 1.0;
pub const CHEVRON: f32 = 24.0;

/// The width a tab wants for a label of `text_w`. A point more than the text: the tab is laid on
/// whole pixels, and a name that fits only to the fraction would lose its last letters to the rounding.
#[must_use]
pub fn want_width(text_w: f32) -> f32 {
    (PAD_L + ICON + GAP + text_w.ceil() + 1.0 + GAP + SLOT + PAD_R).clamp(W_MIN, W_MAX)
}

/// The room a tab's name has: the slot takes room only when it holds something.
#[must_use]
pub fn text_room(width: f32, cross: bool, dot: bool) -> f32 {
    let dot_room = 3.5 + 4.0;
    let end = if cross {
        width - PAD_R - SLOT - GAP
    } else if dot {
        width - PAD_R - SLOT * 0.5 - dot_room
    } else {
        width - PAD_R
    };
    (end - PAD_L - ICON - GAP).max(0.0)
}

/// A tab that shows its icon alone: as wide as the icon and its pads.
pub const ICON_ONLY_W: f32 = PAD_L + ICON + PAD_L;

/// A tool tab: a singleton of the editor (Characters, Items, World Rules, Preferences, a library),
/// whose icon says what it is - it gives way to its icon alone before any name is cut.
#[must_use]
pub fn is_tool(kind: &str) -> bool {
    matches!(
        kind,
        "characters"
            | "items"
            | "economy"
            | "mod_settings"
            | "world_rules"
            | "screens"
            | "releases"
            | "preferences"
            | "light_animations"
            | "texture_library"
            | "sound_library"
    )
}

/// When the tabs want more than `avail`, the tool tabs show their icon alone, the rightmost
/// first, until the rest fits; `want` takes their narrower width. Which tab is up plays no part:
/// a switch between tabs moves none of them (a folded tab that is up is told by its highlight and
/// names itself in its tip). Answers which tabs fold.
#[must_use]
pub fn fold_tools(want: &mut [f32], tool: &[bool], avail: f32) -> Vec<bool> {
    let mut folded = vec![false; want.len()];
    let total = |w: &[f32]| w.iter().sum::<f32>() + SPACING * w.len() as f32;
    for i in (0..want.len()).rev() {
        if total(want) <= avail {
            break;
        }
        if tool.get(i).copied().unwrap_or(false) {
            if let Some(w) = want.get_mut(i) {
                *w = ICON_ONLY_W;
            }
            if let Some(f) = folded.get_mut(i) {
                *f = true;
            }
        }
    }
    folded
}

/// How the strip lays its tabs out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StripPlan {
    pub widths: Vec<f32>,
    /// In the strip; the others are under the chevron.
    pub shown: Vec<bool>,
}

impl StripPlan {
    #[must_use]
    pub fn hidden(&self) -> usize {
        self.shown.iter().filter(|s| !**s).count()
    }
}

/// Lays the strip out: what each tab wants, shrunk evenly toward 96 pt, the rest under the
/// chevron. The active tab keeps its whole name and always stays in the strip; the first tab
/// (Level) is always shown. `count_extra` is the room the "+N" label takes beyond the plain
/// chevron.
#[must_use]
pub fn plan_strip(want: &[f32], active: Option<usize>, avail: f32, count_extra: f32) -> StripPlan {
    let n = want.len();
    let mut avail = avail.max(0.0);
    let mut width = want.to_vec();
    let total: f32 = want.iter().sum::<f32>() + SPACING * n as f32;
    let at_min: f32 = SPACING * n as f32
        + want
            .iter()
            .enumerate()
            .map(|(i, w)| if Some(i) == active { *w } else { w.min(W_MIN) })
            .sum::<f32>();
    // a tab folded to its icon is narrower than any name: it keeps that width
    let floor = |w: f32, cap: f32| if w <= ICON_ONLY_W { w } else { w.min(cap) };
    if at_min > avail {
        avail = (avail - count_extra).max(0.0);
    }
    if total > avail && n > 0 {
        if let Some(a) = active
            && let Some(w) = want.get(a).copied()
        {
            let others = if a > 0 { W_MIN + SPACING } else { 0.0 };
            if let Some(slot) = width.get_mut(a) {
                *slot = w.min(W_MIN.max(avail - others - SPACING));
            }
        }
        let rest = avail - active.and_then(|a| width.get(a)).map_or(0.0, |w| w + SPACING);
        let (mut lo, mut hi) = (W_MIN, W_MAX);
        for _ in 0..24 {
            let cap = (lo + hi) * 0.5;
            let sum: f32 = want
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != active)
                .map(|(_, w)| floor(*w, cap) + SPACING)
                .sum();
            if sum > rest {
                hi = cap;
            } else {
                lo = cap;
            }
        }
        for (i, w) in width.iter_mut().enumerate() {
            if Some(i) != active {
                *w = floor(want.get(i).copied().unwrap_or(W_MIN), lo);
            }
        }
    }
    let mut shown = vec![false; n];
    let mut used = 0.0;
    for i in 0..n {
        let w = width.get(i).copied().unwrap_or(0.0);
        let fits = used + w + SPACING <= avail || i == 0;
        if let Some(s) = shown.get_mut(i) {
            *s = fits;
        }
        if fits {
            used += w + SPACING;
        }
    }
    if let Some(a) = active
        && !shown.get(a).copied().unwrap_or(true)
    {
        let wa = width.get(a).copied().unwrap_or(0.0);
        for j in (1..n).rev() {
            if shown.get(j).copied().unwrap_or(false) {
                if let Some(s) = shown.get_mut(j) {
                    *s = false;
                }
                used -= width.get(j).copied().unwrap_or(0.0) + SPACING;
                if used + wa + SPACING <= avail {
                    break;
                }
            }
        }
        if let Some(s) = shown.get_mut(a) {
            *s = true;
        }
    }
    StripPlan { widths: width, shown }
}

/// A tab as the session file keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTab {
    pub id: String,
    pub kind: String,
    pub icon: Icon,
    pub label: String,
    /// What the factory opens: a project-relative path, or nothing for a tool.
    pub arg: String,
}

/// `[session]` then `tabN = id|kind|icon|label|arg`, one per tab but Level.
#[must_use]
pub fn encode_session(tabs: &[SessionTab]) -> String {
    let mut out = String::from("[session]\n");
    for (n, t) in tabs.iter().filter(|t| t.id != LEVEL).enumerate() {
        out.push_str(&format!("tab{n} = {}|{}|{}|{}|{}\n", t.id, t.kind, t.icon.name(), t.label, t.arg));
    }
    out
}

/// Reads a session file; lines it does not understand are skipped.
#[must_use]
pub fn decode_session(text: &str) -> Vec<SessionTab> {
    let mut out: Vec<SessionTab> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches([' ', '\r']);
        if !line.starts_with("tab") {
            continue;
        }
        let Some((_, rest)) = line.split_once('=') else { continue };
        let mut fields = rest.trim_start_matches(' ').splitn(5, '|');
        let (Some(id), Some(kind), Some(icon), Some(label)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let arg = fields.next().unwrap_or("");
        if id.is_empty() || id == LEVEL || out.iter().any(|t| t.id == id) {
            continue;
        }
        out.push(SessionTab {
            id: id.to_owned(),
            kind: kind.to_owned(),
            icon: Icon::from_name(icon).unwrap_or(Icon::File),
            label: label.to_owned(),
            arg: arg.to_owned(),
        });
    }
    out
}

/// The name a file's tab shows: the file name without its extension.
#[must_use]
pub fn label_of_path(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.get(..dot).unwrap_or(name).to_owned(),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ICON_ONLY_W, SessionTab, W_MIN, decode_session, encode_session, fold_tools, label_of_path,
        plan_strip, want_width,
    };
    use gz_ui::Icon;

    #[test]
    fn sessions_round_trip() {
        let tabs = vec![
            SessionTab {
                id: "quest:quests\\a.nqasset".into(),
                kind: "quest".into(),
                icon: Icon::Quest,
                label: "a".into(),
                arg: "quests\\a.nqasset".into(),
            },
            SessionTab {
                id: "characters".into(),
                kind: "characters".into(),
                icon: Icon::Character,
                label: "Characters".into(),
                arg: String::new(),
            },
        ];
        let text = encode_session(&tabs);
        assert!(text.starts_with("[session]\ntab0 = quest:"));
        assert_eq!(decode_session(&text), tabs);
    }

    #[test]
    fn level_and_doubles_are_skipped() {
        let text = "[session]\r\ntab0 = level|level|Level|Level|\r\ntab1 = pda|pda|PdaInterface|PDA|ui\\pda.pdaasset\r\ntab2 = pda|pda|x|y|z\r\nnoise\r\n";
        let tabs = decode_session(text);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.first().map(|t| t.arg.as_str()), Some("ui\\pda.pdaasset"));
    }

    #[test]
    fn the_strip_gives_way_and_keeps_the_active_tab() {
        let want = vec![120.0, 200.0, 200.0, 200.0, 200.0];
        let roomy = plan_strip(&want, Some(2), 2000.0, 0.0);
        assert_eq!(roomy.widths, want);
        assert_eq!(roomy.hidden(), 0);
        let tight = plan_strip(&want, Some(4), 500.0, 10.0);
        assert!(tight.shown.first().copied().unwrap_or(false));
        assert!(tight.shown.get(4).copied().unwrap_or(false));
        assert!(tight.widths.iter().all(|w| *w >= W_MIN));
        assert!(tight.hidden() > 0);
    }

    #[test]
    fn tool_tabs_fold_to_their_icon_before_a_name_is_cut() {
        // Level, a quest, three tool tabs: the rightmost fold first, whichever tab is up
        let tool = [false, false, true, true, true];
        let mut want = vec![120.0, 200.0, 140.0, 140.0, 140.0];
        let folded = fold_tools(&mut want, &tool, 600.0);
        assert_eq!(folded, vec![false, false, false, true, true]);
        assert_eq!(want[4], ICON_ONLY_W);
        assert_eq!(want[2], 140.0, "the first tool tab keeps its name: the rest fits");
        for up in [Some(2), Some(3), None] {
            let plan = plan_strip(&want, up, 600.0, 0.0);
            assert_eq!(plan.hidden(), 0);
            assert_eq!(plan.widths, want, "nothing else is cut, and a switch moves no tab");
        }
        // room enough: nothing folds
        let mut roomy = vec![120.0, 200.0, 140.0];
        assert_eq!(fold_tools(&mut roomy, &[false, false, true], 2000.0), vec![false; 3]);
    }

    #[test]
    fn widths_and_labels() {
        assert_eq!(want_width(0.0), W_MIN);
        assert_eq!(want_width(1000.0), 220.0);
        // a name keeps every letter on the active tab, its cross shown, whatever the rounding
        for text_w in [66.4_f32, 80.0, 101.7] {
            let w = want_width(text_w);
            assert!(super::text_room(w - 1.0, true, false) >= text_w, "{text_w} in {w}");
        }
        assert_eq!(label_of_path("quests\\Night Run.nqasset"), "Night Run");
        assert_eq!(label_of_path(".gitignore"), ".gitignore");
    }
}
