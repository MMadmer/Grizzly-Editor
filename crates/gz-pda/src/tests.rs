use std::sync::Arc;

use gz_lua::LuaHost;

use super::*;

fn bundled_reader() -> Reader {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../gamedata/configs/pda/xms_pda_layout.script");
    Reader::new(std::fs::read_to_string(path).unwrap(), false)
}

fn read(text: &str) -> Result<Layout, ReadError> {
    evaluate(&LuaHost::default(), &bundled_reader(), text, "app", "rus")
}

const NOW: Clock = Clock { hour: 9, minute: 5, day: 3, month: 11 };

#[test]
fn the_bundled_reader_evaluates_the_template() {
    let l = read(&template("new_pda", "New PDA")).unwrap();
    assert!(l.ok, "{:?}", l.problems);
    assert_eq!(l.errors, 0);
    assert_eq!(l.id, "new_pda");
    assert_eq!(l.title, "New PDA");
    assert_eq!(l.start, "home");
    let pages: Vec<&str> = l.pages.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(pages, ["home", "map", "journal"]);
    assert_eq!(l.find_page("journal").map(|p| p.stock.as_str()), Some("eptTasks"));
    assert_eq!(l.key_map, "page:map");
    assert_eq!(l.key_tasks, "page:home");
    assert_eq!(l.key_contacts, "stock:eptContacts");
    assert_eq!((l.canvas_w, l.canvas_h), (960.0, 680.0));
    // the tabs are buttons with their page as the action, selected while it shows
    let tabs: Vec<&Item> = l.items.iter().filter(|it| it.kind == "button").collect();
    assert_eq!(tabs.len(), 3);
    assert_eq!(tabs.first().map(|t| t.action.clone()), Some(vec!["page:home".to_owned()]));
    assert_eq!(tabs.first().map(|t| t.selected_on.clone()), Some(vec!["home".to_owned()]));
    assert!(l.items.iter().any(|it| it.kind == "map" && it.page == "map"));
    // a group's children are relative to it and point at it
    let group = l.items.iter().position(|it| it.kind == "group").unwrap();
    assert!(l.items.iter().any(|it| it.parent == group + 1 && it.kind == "text"));
}

#[test]
fn a_text_that_does_not_parse_has_its_line() {
    let e = read("return {\n  pda = 1,\n  id = \n}").unwrap_err();
    assert_eq!(e.line, 4);
    let code = read("return { pda = 1, id = string.rep('x', 3) }").unwrap_err();
    assert!(code.message.contains("code") || code.line > 0, "{code:?}");
}

#[test]
fn coded_problems_come_from_the_reader() {
    let l = read("return { pda = 2, id = \"Bad Id\", pages = {} }").unwrap();
    assert!(!l.ok);
    let codes: Vec<&str> = l.problems.iter().map(|p| p.code.as_str()).collect();
    assert!(codes.contains(&"E301"), "{codes:?}");
    assert!(codes.contains(&"E302"), "{codes:?}");
    assert!(codes.contains(&"E303"), "{codes:?}");
    assert_eq!(l.errors as usize, l.problems.iter().filter(|p| p.is_error()).count());
}

#[test]
fn the_preview_block_is_read_off_the_asset() {
    let text = "return { pda = 1, id = \"a\", pages = { { id = \"p\", title = \"P\", widgets = {} } },\n\
                preview = { level = \"l01_escape\", actor = { 10, 0, 20 }, vars = { money = 5, who = \"me\" },\n\
                markers = { { x = 1, z = 2, hint = \"here\" } } } }";
    let l = read(text).unwrap();
    assert_eq!(l.preview_level, "l01_escape");
    assert_eq!(l.actor, Some((10.0, 20.0)));
    assert!(l.preview_vars.contains(&("money".to_owned(), "5".to_owned())));
    assert_eq!(l.markers.len(), 1);
    assert_eq!(l.markers.first().map(|m| m.icon.as_str()), Some("ui_xms_icon_pin"));
}

#[test]
fn the_template_names_safely() {
    let t = template("My App!", "Say \"hi\"\n");
    assert!(t.contains("id = \"my_app_\""));
    assert!(t.contains("title = \"Say \\\"hi\\\"\""));
    assert!(template("", "").contains("id = \"app\""));
}

#[test]
fn the_preview_plays_actions() {
    let text = r##"return { pda = 1, id = "a", start = "one", vars = { who = "nobody" },
      pages = {
        { id = "one", title = "One", widgets = {
          { kind = "button", id = "go", x = 0, y = 0, w = 100, h = 50, text = "{who}", page = "two" },
          { kind = "button", id = "sig", x = 0, y = 60, w = 100, h = 50, action = { "signal:ping", "var:who=Stalker {money}" } },
          { kind = "rect", id = "box", x = 200, y = 0, w = 10, h = 10, color = "#FF0000" },
          { kind = "button", x = 0, y = 120, w = 100, h = 50, action = "toggle:box" },
        } },
        { id = "two", title = "Two", widgets = {
          { kind = "button", x = 0, y = 0, w = 100, h = 50, action = "back" },
        } },
        { id = "tasks", title = "Tasks", stock = "eptTasks" },
      } }"##;
    let l = Arc::new(read(text).unwrap());
    let mut p = Preview::new(l.clone());
    assert_eq!(p.page, "one");
    let boxes = p.boxes();
    let go = p.target_by_id(&boxes, "go", 0).unwrap();
    assert_eq!(p.visible(&boxes, &NOW).iter().find(|v| v.id == "go").and_then(|v| v.text.clone()), Some("nobody".to_owned()));
    p.click(go, &NOW);
    assert_eq!((p.page.as_str(), p.prev_page.as_str()), ("two", "one"));
    // the page's own button, by a canvas point
    let back = p.target_at(&p.boxes(), 10.0, 10.0).unwrap();
    p.click(back, &NOW);
    assert_eq!(p.page, "one");

    let sig = p.target_by_id(&p.boxes(), "sig", 0).unwrap();
    p.click(sig, &NOW);
    assert_eq!(p.toast, "Quests receive signal \"ping\"");
    assert_eq!(p.var_of("who").as_deref(), Some("Stalker 12500"));
    assert!(p.log.iter().any(|l| l == "signal:ping"));

    let boxes = p.boxes();
    let box_i = l.find_id("box").unwrap();
    assert!(boxes.get(box_i).is_some_and(|b| b.shown));
    let toggle = p.target_at(&boxes, 10.0, 130.0).unwrap();
    p.click(toggle, &NOW);
    assert!(!p.item_visible(box_i));

    p.run_action("stock:eptTasks", &NOW);
    assert_eq!((p.page.as_str(), p.stock.as_str()), ("tasks", "eptTasks"));
    p.open_key("map", &NOW);
    assert_eq!(p.page, "one");
    assert_eq!(p.go_to_page("nope").unwrap_err(), "there is no page 'nope'");
    assert_eq!(p.format("{time} {date} {title} {page} {unknown}", &NOW), "09:05 03.11.2012 a One {unknown}");
    p.reset();
    assert!(p.vars.is_empty() && p.vis.is_empty());
}
