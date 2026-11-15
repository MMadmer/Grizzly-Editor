//! In the Hands: the first-person picture of the edited item in the Items tab's preview column
//! (docs/UI_DESIGN.md, section 6, "Items").
//!
//! What the picture shows is `gz_ui_previews::hands` (the game's placement, ported); this file
//! says what it is made of - the item's HUD section as it is edited (what the last check
//! resolved, with the form's own values over it, so an edit moves the picture at once), the
//! hands of the game's player HUD section, the HUD field of view - and draws the block: the
//! picture, a line in its place when it cannot be shown, play and stop, the time scrubbed by
//! dragging.

use egui::{Align2, CornerRadius, Rect, Sense, Ui, UiBuilder, Vec2, pos2};
use gz_editor::Value;
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Tx, icon_button, palette, tr};
use gz_ui_previews::hands::{self, HandsView, HudSpec, Keys, Missing, Status};

use crate::items::ItemsDoc;

/// The picture's height for its width: the game's 16:9 screen.
const ASPECT: f32 = 9.0 / 16.0;
/// The band at the picture's foot: play, the timeline, the time.
const BAR_H: f32 = 28.0;
/// Where the game's hands motion files are listed by the hands model's file stem.
const HAND_MOTIONS: &str = "da_hud_animations";
/// The prefix of a player HUD section's attach bones.
const ANCHOR: &str = "ancor_";
/// The engine class of a weapon starts so: every firearm, knife and binoculars of the game.
const WEAPON_CLASS: &str = "WP_";

fn clean(v: &str) -> String {
    v.trim().trim_matches('"').trim().to_owned()
}

impl ItemsDoc {
    /// What the picture shows for the item as it is edited, or `None` when it resolves no HUD
    /// section.
    pub(crate) fn hud_spec(&self) -> Option<HudSpec> {
        let res = self.res.as_ref()?;
        let hud = res.hud.as_ref()?;
        let mut keys: Keys = hud
            .rows
            .iter()
            .map(|r| (r.key.clone(), r.value.as_deref().map(clean).unwrap_or_default()))
            .collect();
        // the form's own values over what the last check resolved: an edit shows at once
        if let Some(h) = &self.edit.hud {
            for (k, v) in &h.changes.set {
                keys.insert(k.clone(), clean(&v.ltx()));
            }
            for k in &h.changes.unset {
                keys.remove(k);
            }
        }
        let ltx = self.loaded.as_ref().map(|l| &l.roster.composition.ltx);
        let player = ltx.and_then(|l| l.section(hands::DEFAULT_HANDS));
        let visual = player
            .and_then(|s| s.get("visual"))
            .and_then(|it| it.value.as_deref())
            .map(clean)
            .unwrap_or_default();
        let anchors: Vec<String> = player
            .map(|s| {
                s.items
                    .iter()
                    .filter(|it| it.key.starts_with(ANCHOR))
                    .filter_map(|it| it.value.as_deref().map(clean))
                    .collect()
            })
            .unwrap_or_default();
        let stem = visual.rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase();
        let stem = stem.strip_suffix(".ogf").unwrap_or(&stem).to_owned();
        let hand_motions: Vec<String> = ltx
            .and_then(|l| l.section(HAND_MOTIONS))
            .map(|s| {
                s.items
                    .iter()
                    .filter(|it| !stem.is_empty() && it.key.eq_ignore_ascii_case(&stem))
                    .filter_map(|it| it.value.as_deref().map(clean))
                    .collect()
            })
            .unwrap_or_default();
        let mut spec = HudSpec {
            section: hud.name.clone(),
            keys,
            hands: visual,
            anchors,
            hand_motions,
            fov: 0.0,
            row: self.hud_row.clone(),
            hidden: Vec::new(),
        };
        let weapon = res.class.as_deref().is_some_and(|c| c.trim().starts_with(WEAPON_CLASS));
        // the item's section as it is edited: its own values over what the check resolved
        let own = |k: &str| self.edit.changes.get(k).map(|v| clean(&v.ltx()));
        let item = |k: &str| own(k).or_else(|| res.main.get(k).and_then(|r| r.value.as_deref()).map(clean));
        let on_hud = |k: &str| spec.get(k).map(str::to_owned);
        spec.fov = hands::hud_fov(weapon, spec.aim() > 0, &item, &on_hud);
        spec.hidden = hands::hidden_addons(weapon, &item);
        Some(spec)
    }

    /// The picture follows the item and the row the author is on: a frame and a command call it.
    pub(crate) fn refresh_hands(&mut self) {
        let key = (self.edit_gen, self.res_gen, self.hud_row.clone(), self.loaded.is_some());
        if self.hands_key.as_ref() == Some(&key) {
            return;
        }
        let spec = self.hud_spec();
        // the same item read again is being checked: the picture stays, its clock goes on
        if spec.is_none() && self.res.is_none() && self.hands_file == self.sel {
            return;
        }
        self.hands_key = Some(key);
        self.hands_file.clone_from(&self.sel);
        if spec.is_none() && self.hands.is_none() {
            return;
        }
        let view = self.hands.get_or_insert_with(|| HandsView::new(&self.editor));
        view.set_spec(spec);
    }

    /// What `items.view` answers of the picture.
    #[must_use]
    pub(crate) fn hands_json(&self) -> Value {
        self.hands.as_ref().map_or(Value::Null, HandsView::state_json)
    }

    /// The In the Hands block: its title and the picture, as wide as the column and 16:9; what
    /// keeps the picture from being shown is a line in its place. The block keeps its height
    /// whatever it shows, so the world model under it never moves.
    pub(crate) fn draw_hands(&mut self, ui: &mut Ui, cx: &gz_app::DocCx<'_>) {
        self.refresh_hands();
        let Some(view) = self.hands.as_mut() else { return };
        if view.status() == Status::Off {
            return;
        }
        let p = palette(ui.ctx());
        let _ = text::label(ui, "In the Hands", Font::Strong, p.text, Clip::End);
        ui.add_space(6.0);
        let w = ui.available_width().max(1.0);
        let (block, resp) =
            ui.allocate_exact_size(Vec2::new(w, (w * ASPECT).round() + BAR_H), Sense::click_and_drag());
        let rect = Rect::from_min_max(block.min, pos2(block.max.x, block.max.y - BAR_H));
        let status = view.status();
        if status != Status::Ready {
            ui.painter().rect_filled(rect, CornerRadius::same(4), p.inset);
            let line = match &status {
                Status::NoDevice(why) => {
                    tr!("The picture cannot be drawn: {why}.", why = Tx::ui(why.clone()))
                }
                Status::Missing(m) => missing_line(m),
                _ => Tx::from("Loading the hands..."),
            };
            // the line in the middle of the picture's place, its rows centred
            let room = (rect.width() - 32.0).max(1.0);
            let rows = (text::width(ui.ctx(), &Font::Body.id(), line.borrow()) / room).ceil().max(1.0);
            let high = rows * Font::Body.line();
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-world.items.hands.line")
                    .max_rect(Rect::from_center_size(rect.center(), Vec2::new(room, high + 4.0)))
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            if status == Status::Loading {
                gz_ui::state::busy_line(&mut child, line);
            } else {
                let _ = text::wrapped(&mut child, line, Font::Body, p.text_muted);
            }
            ui.add_space(8.0);
            return;
        }
        view.draw(ui, cx.gpu, rect, p.canvas);
        if !view.has_picture() {
            gz_ui::state::spinner_at(
                ui.ctx(),
                ui.painter(),
                Rect::from_center_size(rect.center(), Vec2::splat(20.0)),
            );
        }
        let length = view.timing().map_or(0.0, |t| t.length);
        // the band under the picture: play / stop, the timeline, the time
        let bar = Rect::from_min_max(pos2(block.min.x, rect.max.y), block.max);
        let font = Font::Small.id();
        let time = view.time();
        let (now, all) = (format!("{time:.2}"), format!("{length:.2}"));
        let shown = tr!("{t} / {length} s", t = now.as_str(), length = all.as_str());
        let time_w = text::width(ui.ctx(), &font, shown.borrow()) + 8.0;
        let track = Rect::from_min_max(
            pos2(bar.min.x + BAR_H + 6.0, bar.center().y - 1.5),
            pos2((bar.max.x - time_w - 8.0).max(bar.min.x + BAR_H + 7.0), bar.center().y + 1.5),
        );
        ui.painter().rect_filled(track, CornerRadius::same(1), p.border);
        if length > 0.0 {
            let x = track.min.x + track.width() * (time / length).clamp(0.0, 1.0);
            ui.painter().rect_filled(
                Rect::from_min_max(track.min, pos2(x, track.max.y)),
                CornerRadius::same(1),
                p.accent_bright,
            );
            ui.painter().circle_filled(pos2(x, track.center().y), 4.5, p.accent_bright);
        }
        let _ = text::paint(
            ui.painter(),
            &font,
            pos2(bar.max.x - 2.0, bar.center().y),
            Align2::RIGHT_CENTER,
            p.text_secondary,
            shown,
            Some(time_w),
            Clip::End,
        );
        // a drag over the picture or its timeline scrubs the time and holds it there
        if length > 0.0
            && (resp.dragged() || resp.clicked())
            && let Some(at) = resp.interact_pointer_pos()
        {
            let f = ((at.x - track.min.x) / track.width().max(1.0)).clamp(0.0, 1.0);
            view.set_playing(false);
            view.set_time(f * length);
        }
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.items.hands.play")
                .max_rect(Rect::from_min_size(pos2(bar.min.x, bar.min.y + 2.0), Vec2::splat(BAR_H - 4.0))),
        );
        let playing = view.playing();
        let (icon, tip) =
            if playing { (Icon::Stop, "Stop the motion here") } else { (Icon::Play, "Play the motion") };
        if icon_button(&mut child, icon, tip, false, length > 0.0).clicked() {
            view.set_playing(!playing);
        }
        ui.add_space(8.0);
    }
}

/// The sentence the picture's place shows when it cannot be shown.
fn missing_line(m: &Missing) -> Tx<'static> {
    match m {
        Missing::NoItemModel => Tx::from("The HUD section names no model."),
        Missing::ItemModel(name, why) if why == "not found" => {
            tr!("The HUD model {name} is neither in the game nor in the project.", name = name.as_str())
        }
        Missing::ItemModel(name, why) => {
            tr!(
                "The HUD model {name} cannot be shown: {why}.",
                name = name.as_str(),
                why = Tx::ui(why.clone())
            )
        }
        Missing::NoHands => {
            tr!("No hands hold it: the game has no {section}.", section = hands::DEFAULT_HANDS)
        }
        Missing::HandsModel(name, why) if why == "not found" => {
            tr!("The hands' model {name} is neither in the game nor in the project.", name = name.as_str())
        }
        Missing::HandsModel(name, why) => {
            tr!(
                "The hands' model {name} cannot be shown: {why}.",
                name = name.as_str(),
                why = Tx::ui(why.clone())
            )
        }
        Missing::NoMotion => Tx::from("The HUD section has no motion line."),
        Missing::Motion(line, motion) => tr!(
            "Neither the hands nor the item hold the motion {motion} of {line}.",
            motion = motion.as_str(),
            line = line.as_str()
        ),
        Missing::Bone(bone) => tr!("The hands have no bone {bone} to hold the item.", bone = bone.as_str()),
    }
}
