//! Stress tests of the frame clock and of saving as it is edited; the typed commands, their gates and `help`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use gz_base::{Origin, Rev};
use gz_store::{Apply, CmdError, State, Store, StoreOptions};
use gz_task::{ExecOptions, Runtime, Wake};

use crate::{AutoSave, FrameClock, FrameStamp};

fn runtime() -> Runtime {
    Runtime::start(ExecOptions { async_threads: 4, cpu_threads: 2, io_threads: 4 }).unwrap()
}

#[test]
fn frame_held_requests_are_answered_once_and_never_before_their_frame() {
    let rt = runtime();
    let wake = Wake::new();
    let asked = Arc::new(AtomicUsize::new(0));
    let a = asked.clone();
    wake.set(move || {
        a.fetch_add(1, Ordering::Relaxed);
    });
    let clock = Arc::new(FrameClock::new(wake));
    clock.publish(FrameStamp { frame: 1, ..FrameStamp::default() });
    let presented = Arc::new(AtomicU64::new(1));
    let stop = Arc::new(AtomicBool::new(false));
    // the window: presents a frame every few hundred microseconds while anybody waits
    let window = {
        let (clock, presented, stop) = (clock.clone(), presented.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_micros(300));
                let next = presented.fetch_add(1, Ordering::SeqCst) + 1;
                clock.publish(FrameStamp { frame: next, ..FrameStamp::default() });
            }
        })
    };
    let answered = Arc::new(AtomicUsize::new(0));
    // 32 connections, 20 requests each, for 1..4 frames
    let tasks: Vec<_> = (0..32u64)
        .map(|c| {
            let (clock, presented, answered) = (clock.clone(), presented.clone(), answered.clone());
            rt.exec().spawn(async move {
                for i in 0..20u64 {
                    let frames = 1 + (c + i) % 4;
                    let before = clock.current().frame;
                    let stamp = clock.after(frames).await.unwrap();
                    // the frame it waited for was presented after the request was made
                    assert!(stamp.frame >= before + frames, "{} < {before} + {frames}", stamp.frame);
                    assert!(stamp.frame <= presented.load(Ordering::SeqCst));
                    answered.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();
    rt.block_on(async {
        for t in tasks {
            t.await.unwrap();
        }
    });
    stop.store(true, Ordering::Relaxed);
    window.join().unwrap();
    assert_eq!(answered.load(Ordering::SeqCst), 32 * 20);
    // every request asked the window for its frames
    assert!(asked.load(Ordering::Relaxed) >= 32 * 20);
}

/// A document that is one number, rendered as its text.
#[derive(Clone, Default)]
struct Doc(u64);

impl State for Doc {
    type Cmd = u64;
    type Reply = ();
    fn apply(&mut self, v: u64, cx: &mut Apply) -> Result<(), CmdError> {
        self.0 = v;
        cx.content_changed();
        Ok(())
    }
}

#[test]
fn a_burst_of_edits_ends_on_disk_as_the_last_one_and_nothing_follows_the_flush() {
    let rt = runtime();
    let dir = tempfile::tempdir().unwrap();
    let file: PathBuf = dir.path().join("doc.txt");
    let store = Store::spawn("doc", Doc::default(), &rt.exec(), StoreOptions::default());
    let writes = Arc::new(AtomicUsize::new(0));
    let (f, w) = (file.clone(), writes.clone());
    let saver = AutoSave::spawn(
        &rt.exec(),
        &store,
        move |d: &Doc| {
            w.fetch_add(1, Ordering::SeqCst);
            Some((f.clone(), d.0.to_string().into_bytes()))
        },
        |e| panic!("{e}"),
    );
    // eight writers, 100 edits each; each flushes its own edit like a command that answers
    let tasks: Vec<_> = (0..8u64)
        .map(|t| {
            let (store, saver) = (store.clone(), saver.clone());
            rt.exec().spawn(async move {
                for i in 0..100u64 {
                    store.send(t * 1000 + i + 1, Origin::Mcp).await.unwrap();
                    let rev = store.rev();
                    saver.flush(rev).await.unwrap();
                }
            })
        })
        .collect();
    rt.block_on(async {
        for t in tasks {
            t.await.unwrap();
        }
    });
    let last = store.snapshot();
    assert_eq!(last.rev, Rev(800));
    rt.block_on(saver.flush(last.rev)).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), last.0.to_string());
    // overtaken states were skipped, and after the flush nothing is rendered or written
    let rendered = writes.load(Ordering::SeqCst);
    assert!(rendered <= 800);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(writes.load(Ordering::SeqCst), rendered);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), last.0.to_string());
}

#[test]
fn an_edit_made_before_the_saver_runs_is_saved_and_flushed() {
    let rt = runtime();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("doc.txt");
    for round in 0..50u64 {
        let store = Store::spawn("doc", Doc::default(), &rt.exec(), StoreOptions::default());
        let f = file.clone();
        let saver = AutoSave::spawn(
            &rt.exec(),
            &store,
            move |d: &Doc| Some((f.clone(), d.0.to_string().into_bytes())),
            |_| {},
        );
        // sent at once: the saver's task may not have run yet
        rt.block_on(async {
            store.send(round + 1, Origin::Ui).await.unwrap();
            tokio::time::timeout(Duration::from_secs(2), saver.flush(store.rev())).await.unwrap().unwrap();
        });
        assert_eq!(std::fs::read_to_string(&file).unwrap(), (round + 1).to_string());
    }
}

// ---- commands with a card ---------------------------------------------------------------------

mod cards {
    use std::sync::Arc;

    use gz_base::Origin;
    use gz_task::{ExecOptions, Runtime};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::{Map, Value, json};

    use crate::{Editor, EditorOptions, Gate, Log, Registry, Shape, Spec};

    /// What to place.
    #[derive(Debug, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    enum What {
        Object,
        Light,
    }

    #[derive(Debug, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Place {
        /// What to place.
        kind: What,
        /// Where, in metres.
        position: [f32; 3],
        /// How many.
        #[serde(default)]
        count: Option<u32>,
        /// Names to give.
        #[serde(default)]
        names: Vec<String>,
        /// Whether to snap to the ground.
        #[serde(default)]
        snap: bool,
    }

    #[derive(Debug, Default, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Close {
        /// Drop unsaved work.
        #[serde(default)]
        discard: bool,
    }

    fn editor() -> (Runtime, Arc<Editor>) {
        let rt = Runtime::start(ExecOptions { async_threads: 2, cpu_threads: 1, io_threads: 1 }).unwrap();
        let (log, _guard) = Log::start(None, false).unwrap();
        let mut reg = Registry::new();
        reg.area("scene", "The open level");
        reg.cmd(
            Spec::write("scene.place", "Places things in the scene.").example("kind=object position=1,2,3"),
            |_ed, p: Place, _| async move {
                Ok(json!({ "kind": format!("{:?}", p.kind), "position": p.position, "count": p.count,
                           "names": p.names, "snap": p.snap }))
            },
        );
        reg.cmd_sync(
            Spec::write("project.close", "Closes the project.").destroys_with(&["discard"]),
            |_ed, p: &Close, _| Ok(json!({ "discarded": p.discard })),
        );
        reg.cmd_sync(
            Spec::destroy("file.delete", "Deletes a file for good."),
            |_ed, _: &crate::NoParams, _| Ok(Value::Null),
        );
        reg.install_help();
        let editor = Editor::new(EditorOptions::default(), rt.exec(), log, reg);
        (rt, editor)
    }

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn a_request_is_read_into_the_commands_type_and_text_is_taken_for_what_it_says() {
        let (rt, ed) = editor();
        // as a command line gives them: every value a string
        let reply = rt.block_on(ed.call(
            "scene.place",
            args(json!({ "kind": "light", "position": "1, 2.5,3", "count": "4", "names": "a, b", "snap": "yes" })),
            Origin::Mcp,
        ));
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["kind"], "Light");
        assert_eq!(reply["position"], json!([1.0, 2.5, 3.0]));
        assert_eq!(reply["count"], 4);
        assert_eq!(reply["names"], json!(["a", "b"]));
        assert_eq!(reply["snap"], true);
        // and as JSON gives them, a null meaning "not given"
        let reply = rt.block_on(ed.call(
            "scene.place",
            args(json!({ "kind": "object", "position": [0, 0, 0], "count": null })),
            Origin::Mcp,
        ));
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["count"], Value::Null);
    }

    #[test]
    fn a_wrong_parameter_is_refused_with_the_usage() {
        let (rt, ed) = editor();
        let reply = rt.block_on(ed.call(
            "scene.place",
            args(json!({ "kind": "object", "postion": [0, 0, 0] })),
            Origin::Mcp,
        ));
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["code"], "invalid");
        assert_eq!(reply["error"], "scene.place has no parameter 'postion' - did you mean 'position'?");
        let usage = reply["usage"].as_str().unwrap();
        assert!(usage.contains("  kind: object|light (required) - What to place."), "{usage}");
        assert!(usage.contains("  position: [x,y,z] (required) - Where, in metres."), "{usage}");
        assert!(usage.contains("  count: integer - How many."), "{usage}");
        assert!(usage.contains("  names: [string] - Names to give."), "{usage}");
        // a missing one names itself
        let reply = rt.block_on(ed.call("scene.place", args(json!({ "kind": "object" })), Origin::Mcp));
        assert_eq!(reply["error"], "scene.place: missing field `position`", "{reply}");
        // an unknown command names its neighbours
        let reply = rt.block_on(ed.call("scene.plase", Map::new(), Origin::Mcp));
        assert_eq!(reply["code"], "not_found");
        assert!(reply["error"].as_str().unwrap().contains("did you mean: scene.place?"), "{reply}");
    }

    #[test]
    fn a_door_lets_through_only_its_own_effect() {
        let (rt, ed) = editor();
        let call =
            |name: &str, a: Value, gate: Gate| rt.block_on(ed.call_gated(name, args(a), Origin::Mcp, gate));
        assert_eq!(call("project.close", json!({}), Gate::Run)["ok"], true);
        // the same command loses data with `discard`: then it belongs to the other door
        let refused = call("project.close", json!({ "discard": true }), Gate::Run);
        assert_eq!(refused["code"], "gate");
        assert_eq!(refused["error"], "project.close loses data for good here: run it with grizzly_destroy");
        assert_eq!(call("project.close", json!({ "discard": true }), Gate::Destroy)["discarded"], true);
        assert_eq!(call("project.close", json!({ "discard": "false" }), Gate::Run)["ok"], true);
        assert_eq!(call("file.delete", json!({}), Gate::Run)["code"], "gate");
        assert_eq!(call("file.delete", json!({}), Gate::Destroy)["ok"], true);
        assert_eq!(
            call("scene.place", json!({}), Gate::Destroy)["error"],
            "scene.place is an ordinary command: run it with grizzly_run"
        );
        // no door: the author
        assert_eq!(call("file.delete", json!({}), Gate::Any)["ok"], true);
    }

    #[test]
    fn help_reads_the_registry() {
        let (rt, ed) = editor();
        let text = |about: Value| {
            let a = if about.is_null() { Map::new() } else { args(json!({ "about": about })) };
            rt.block_on(ed.call("help", a, Origin::Mcp))["text"].as_str().unwrap_or_default().to_owned()
        };
        let top = text(Value::Null);
        assert!(top.contains("  scene    The open level (1)"), "{top}");
        assert!(top.contains("  project   (1)"), "{top}");
        assert!(!top.contains("Without a card"), "{top}");
        assert!(top.contains("  help  read"), "{top}");
        let area = text(json!("scene"));
        assert!(
            area.starts_with("scene - The open level\n  scene.place  write    Places things in the scene."),
            "{area}"
        );
        let one = text(json!("scene.place"));
        assert!(
            one.starts_with(
                "scene.place (write) - Places things in the scene.\n  kind: object|light (required)"
            ),
            "{one}"
        );
        assert!(one.ends_with("Example: scene.place kind=object position=1,2,3\n"), "{one}");
        assert!(text(json!("project.close")).contains("Loses data with: discard."));
        let none = rt.block_on(ed.call("help", args(json!({ "about": "plac" })), Origin::Mcp));
        assert!(none["error"].as_str().unwrap().contains("Commands with that word: scene.place."), "{none}");
        // the shape of a type is what its fields say
        let shape = Shape::of::<Place>();
        assert_eq!(
            shape.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["kind", "position", "count", "names", "snap"]
        );
    }

    /// A choice some of whose words say what they are is still one list of words.
    #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
    #[serde(rename_all = "snake_case")]
    #[allow(dead_code)]
    enum Mixed {
        Back,
        Forward,
        /// Shows where the browser stands.
        Get,
    }

    #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
    #[allow(dead_code)]
    struct MixedParams {
        /// What to do.
        action: Mixed,
        #[serde(default)]
        light: Option<Light>,
    }

    /// How it is lit.
    #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
    #[serde(rename_all = "snake_case")]
    #[allow(dead_code)]
    enum Light {
        /// A key light from above.
        Studio,
        /// The textures as they are.
        Unlit,
    }

    #[test]
    fn a_choice_with_some_words_documented_is_one_list() {
        let shape = Shape::of::<MixedParams>();
        assert_eq!(shape.params[0].form, "back|forward|get");
        // an optional choice with no doc of its own says what its type and its words say
        assert_eq!(shape.params[1].form, "studio|unlit");
        assert!(!shape.params[1].doc.is_empty(), "{:?}", shape.params[1]);
    }
}
