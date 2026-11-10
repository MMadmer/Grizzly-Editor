use crate::bitmap::{GlyphTable, font_section, font_texture, layout_text, parse_glyphs};
use crate::gamefile::{Device, FRect, LtxSection, atof, comma_floats, ltx_value, parse_level_map, strip_comments, tag_attr, xml_node};
use crate::screens::{default_show, surface_mask, take_files};

#[test]
fn numbers_read_like_c() {
    assert_eq!(atof("  12.5px"), 12.5);
    assert_eq!(atof("-3"), -3.0);
    assert_eq!(atof("abc"), 0.0);
    assert_eq!(atof("1e2"), 100.0);
    assert_eq!(comma_floats("1 , 2,3", 3), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(comma_floats("1 2 3", 3), None);
    assert_eq!(comma_floats("4,5", 3), None);
}

#[test]
fn ltx_values_follow_the_c_scan() {
    let text = "; head\n[base]\nheight = 16 ; comment\n[child]:base\nx = 1\n  [other]\n x = 2\n[child2] x = 3\n";
    assert_eq!(ltx_value(text, "child", "x").as_deref(), Some("1"));
    // a key the section lacks comes from its parent
    assert_eq!(ltx_value(text, "child", "height").as_deref(), Some("16"));
    // a header after white space counts
    assert_eq!(ltx_value(text, "other", "x").as_deref(), Some("2"));
    assert_eq!(ltx_value(text, "missing", "x"), None);
    // the first header that stands on its own is the section; a later one is never looked at
    let twice = "[a]\nk = 1\n[a]\nk = 2\nj = 3\n";
    assert_eq!(ltx_value(twice, "a", "k").as_deref(), Some("1"));
    assert_eq!(ltx_value(twice, "a", "j"), None);
    // a name glued to something before it is not a header
    assert_eq!(ltx_value("x[a]\nk = 1\n", "a", "k"), None);
    let sec = LtxSection::read(text, "child").unwrap();
    assert_eq!(sec.get("x"), Some("1"));
    assert_eq!(sec.get("height"), Some("16"));
    assert_eq!(sec.get("nope"), None);
}

#[test]
fn xml_nodes_and_attributes() {
    assert_eq!(tag_attr("<t xid=\"q\" id=\"z\"", "id"), "z");
    assert_eq!(tag_attr("<t id=\"z\"", "x"), "");
    assert_eq!(strip_comments("a<!-- b -->c<!-- d"), "ac");
    let xml = "<w><noice_static_x x=\"9\"/><noice_static x=\"10\" y=\"20\" width=\"100\" height=\"50\">\n<texture a=\"90\">ui_noise</texture></noice_static>\
               <close_button x=\"1\" y=\"2\" width=\"3\" height=\"4\"><texture_e>ui_close_e</texture_e></close_button></w>";
    let n = xml_node(xml, "noice_static", None).unwrap();
    assert_eq!(n.rect, FRect::new(10.0, 20.0, 110.0, 70.0));
    assert_eq!(n.texture, "ui_noise");
    assert_eq!(n.alpha, 90);
    let c = xml_node(xml, "close_button", Some("texture_e")).unwrap();
    assert_eq!(c.texture, "ui_close_e");
    assert!(xml_node(xml, "background_static", None).is_none());
    // a texture tag after the node's end belongs to another node
    let after = "<a x=\"1\"></a><texture>t</texture>";
    assert_eq!(xml_node(after, "a", None).unwrap().texture, "");
}

#[test]
fn the_device_has_dead_airs_defaults_and_takes_the_games_files() {
    let mut d = Device::default();
    assert_eq!(d.side(false).noise, FRect::new(25.0, 33.0, 998.0, 716.0));
    assert_eq!(d.side(true).bg, FRect::new(102.0, 0.0, 921.0, 768.0));
    assert_eq!(d.rect(false), FRect::new(0.0, 0.0, 1024.0, 768.0));
    // on 16:9 the art covers only the middle of the screen; the close button sits on it
    assert_eq!(d.rect(true), FRect::new(102.0, 0.0, 921.0, 768.0));
    d.apply_xml(true, "<!-- old --><background_static x=\"100\" y=\"0\" width=\"824\" height=\"768\"><texture>ui_pda_16</texture></background_static>\
                       <noice_static x=\"120\" y=\"30\" width=\"780\" height=\"690\"/>");
    let w = d.side(true);
    assert_eq!(w.bg, FRect::new(100.0, 0.0, 924.0, 768.0));
    assert_eq!(w.bg_tex, "ui_pda_16");
    // a node without a texture tag keeps the default texture and gets the full alpha
    assert_eq!(w.noise_tex, "ui_pda2_noice");
    assert_eq!(w.noise_alpha, 255);
    assert_eq!(d.side(false).noise_alpha, 70);
}

#[test]
fn level_maps() {
    let ltx = "[level_map]\ntexture = UI\\Map_Escape\nbound_rect = -500.0f, -600.0f, 500.0f, 400.0f\n";
    let m = parse_level_map(ltx).unwrap();
    assert_eq!(m.tex, "ui\\map_escape");
    assert_eq!(m.bound, [-500.0, -600.0, 500.0, 400.0]);
    assert!(parse_level_map("[level_map]\ntexture = a\nbound_rect = 5, 5, 1, 1\n").is_none());
    assert!(parse_level_map("[level_map]\ntexture = a\n").is_none());
}

fn table(widths: &[(u8, f32)]) -> GlyphTable {
    let mut t = GlyphTable { height: 10.0, x: [0.0; 256], y: [0.0; 256], w: [0.0; 256] };
    for &(c, w) in widths {
        t.w[usize::from(c)] = w;
    }
    t
}

#[test]
fn fonts_and_glyphs() {
    assert_eq!(font_section("letterica25"), "ui_font_letter_25");
    assert_eq!(font_section(""), "ui_font_letterica18_russian");
    let fonts = "[ui_font_letter_25]\ntexture = UI\\ui_font_letter_25\n";
    assert_eq!(font_texture(fonts, "letterica25").as_deref(), Some("ui\\ui_font_letter_25"));
    let ini = "[symbol_coords]\nheight = 18\n065 = 10, 20, 22\n066 = 1 , 2 , 3\n067 = bad\n";
    let g = parse_glyphs(ini).unwrap();
    assert_eq!(g.height, 18.0);
    assert_eq!((g.x[65], g.y[65], g.w[65]), (10.0, 20.0, 12.0));
    assert_eq!(g.w[66], 2.0);
    assert_eq!(g.w[67], 0.0);
    assert!(parse_glyphs("[symbol_coords]\n065 = 1,2,3\n").is_none());
}

#[test]
fn labels_cut_or_wrap_like_the_game() {
    let t = table(&[(b'a', 10.0), (b'b', 10.0), (b' ', 5.0), (b'.', 2.0)]);
    // one line cut with "..."
    let cut = layout_text(&t, b"aaaa", 30.0, false);
    assert_eq!(cut.len(), 1);
    assert_eq!(cut[0].cp, b"aa...");
    assert_eq!(cut[0].w, 26.0);
    // a line that fits stays whole; every paragraph is a line of its own
    let two = layout_text(&t, b"ab\nb", 100.0, false);
    assert_eq!(two.iter().map(|l| l.cp.clone()).collect::<Vec<_>>(), vec![b"ab".to_vec(), b"b".to_vec()]);
    // words wrap
    let wrapped = layout_text(&t, b"aa bb aa", 50.0, true);
    assert_eq!(wrapped.iter().map(|l| l.cp.clone()).collect::<Vec<_>>(), vec![b"aa bb".to_vec(), b"aa".to_vec()]);
    // a word wider than the line breaks where it must
    let long = layout_text(&t, b"aaaaa", 20.0, true);
    assert_eq!(long.iter().map(|l| l.cp.clone()).collect::<Vec<_>>(), vec![b"aa".to_vec(), b"aa".to_vec(), b"a".to_vec()]);
    // an empty text is one empty line
    assert_eq!(layout_text(&t, b"", 20.0, true).len(), 1);
}

#[test]
fn files_land_where_they_are_dropped() {
    let mut s = gz_screens::Screens::default();
    let mut err = String::new();
    // anything but music on the tab: a screen of its own each, on the tab's surface
    let sel = take_files(&mut s, &["screens\\night.dds".to_owned(), "screens\\fog.ps".to_owned()], "", -1, false, default_show(1), &mut err);
    assert_eq!(s.screens.len(), 2);
    assert_eq!(s.screens[0].id, "night");
    assert_eq!(s.screens[0].show, surface_mask(1));
    assert_eq!(sel.screen.as_deref(), Some("fog"));
    // music is a track wherever it lands
    let sel = take_files(&mut s, &["screens\\rain theme.ogg".to_owned()], "night", 0, false, default_show(0), &mut err);
    assert_eq!(s.tracks.len(), 1);
    assert_eq!(s.tracks[0].id, "rain_theme");
    assert_eq!(sel.track.as_deref(), Some("rain_theme"));
    // on a layer it replaces the layer's file; on Layers it adds one
    take_files(&mut s, &["screens\\day.dds".to_owned()], "night", 0, false, default_show(0), &mut err);
    assert_eq!(s.screens[0].layers[0].file, "screens\\day.dds");
    let sel = take_files(&mut s, &["screens\\top.gif".to_owned()], "night", -1, false, default_show(0), &mut err);
    assert_eq!(s.screens[0].layers.len(), 2);
    assert_eq!(sel.layer, Some(1));
    // the music list takes .ogg files only
    take_files(&mut s, &["screens\\x.dds".to_owned()], "", -1, true, default_show(0), &mut err);
    assert_eq!(err, "Only .ogg files go into the music.");
    assert_eq!(s.screens.len(), 2);
}
