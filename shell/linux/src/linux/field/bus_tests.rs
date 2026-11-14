//! The reader against a stand-in accessibility bus: this process serves the
//! registry and the applications on the private session bus of
//! `scripts/test-linux-gui.sh`, and says that bus is the accessibility bus.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use super::*;

const ROOT: &str = "/org/a11y/atspi/accessible/root";
const INTERFACES: &str = r#"<node>
  <interface name="org.a11y.Bus">
    <method name="GetAddress"><arg type="s" direction="out"/></method>
  </interface>
  <interface name="org.a11y.atspi.Accessible">
    <method name="GetChildren"><arg type="a(so)" direction="out"/></method>
    <method name="GetState"><arg type="au" direction="out"/></method>
    <method name="GetRole"><arg type="u" direction="out"/></method>
    <method name="GetInterfaces"><arg type="as" direction="out"/></method>
  </interface>
  <interface name="org.a11y.atspi.Text">
    <method name="GetText">
      <arg type="i" direction="in"/><arg type="i" direction="in"/>
      <arg type="s" direction="out"/>
    </method>
    <method name="GetNSelections"><arg type="i" direction="out"/></method>
    <method name="GetSelection">
      <arg type="i" direction="in"/>
      <arg type="i" direction="out"/><arg type="i" direction="out"/>
    </method>
    <property name="CaretOffset" type="i" access="read"/>
  </interface>
</node>"#;

const TEXT_ROLE: u32 = 61;
const BUTTON_ROLE: u32 = 43;
const FRAME_ROLE: u32 = 23;

/// One accessible of the stand-in bus. Its children are indices into the
/// case's tree, whose first node is the registry.
#[derive(Clone, Default)]
struct Node {
    children: &'static [usize],
    states: &'static [u32],
    role: u32,
    text: Option<Text>,
    /// an application that never answers
    hangs: bool,
}

#[derive(Clone)]
struct Text {
    value: &'static str,
    caret: i32,
    selection: Option<(i32, i32)>,
}

#[derive(Default)]
struct Stage {
    tree: Vec<Node>,
    text_reads: usize,
    unanswered: Vec<gio::DBusMethodInvocation>,
}

fn path(index: usize) -> String {
    match index {
        0 => ROOT.into(),
        index => format!("/edu/greendale/accessible/{index}"),
    }
}

fn index_of(path: &str) -> usize {
    path.rsplit('/')
        .next()
        .and_then(|last| last.parse().ok())
        .unwrap_or(0)
}

fn parent(children: &'static [usize]) -> Node {
    Node {
        children,
        role: FRAME_ROLE,
        ..Node::default()
    }
}

fn window(states: &'static [u32], children: &'static [usize]) -> Node {
    Node {
        states,
        ..parent(children)
    }
}

fn field(value: &'static str, caret: i32) -> Node {
    Node {
        states: &[FOCUSED],
        role: TEXT_ROLE,
        text: Some(Text {
            value,
            caret,
            selection: None,
        }),
        ..Node::default()
    }
}

fn hung() -> Node {
    Node {
        hangs: true,
        ..Node::default()
    }
}

/// registry, application, active window, and `focused` under it
fn one_application(focused: Node) -> Vec<Node> {
    vec![parent(&[1]), parent(&[2]), window(&[ACTIVE], &[3]), focused]
}

fn serve(connection: &gio::DBusConnection, stage: &Rc<RefCell<Stage>>, nodes: usize) {
    let interfaces = gio::DBusNodeInfo::for_xml(INTERFACES).expect("interface description");
    let interface = |name| interfaces.lookup_interface(name).expect("interface");
    connection
        .register_object("/org/a11y/bus", &interface("org.a11y.Bus"))
        .method_call(|_, _, _, _, _, _, invocation| {
            let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("a session bus");
            invocation.return_value(Some(&(address,).to_variant()));
        })
        .build()
        .expect("the stand-in bus locator");
    for index in 0..nodes {
        for name in [ACCESSIBLE, TEXT] {
            let asked = stage.clone();
            let read = stage.clone();
            connection
                .register_object(&path(index), &interface(name))
                .method_call(move |connection, _, path, _, method, _, invocation| {
                    let own = connection.unique_name().expect("a bus name");
                    answer(&asked, &own, index_of(path), method, invocation);
                })
                .property(move |_, _, path, _, _| {
                    let stage = read.borrow();
                    let text = stage
                        .tree
                        .get(index_of(path))
                        .and_then(|node| node.text.as_ref());
                    text.map_or(-1, |text| text.caret).to_variant()
                })
                .build()
                .expect("a stand-in accessible");
        }
    }
}

fn answer(
    stage: &Rc<RefCell<Stage>>,
    own: &str,
    index: usize,
    method: &str,
    invocation: gio::DBusMethodInvocation,
) {
    let mut stage = stage.borrow_mut();
    let Some(node) = stage.tree.get(index).cloned() else {
        invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownObject", "no such node");
        return;
    };
    if node.hangs {
        stage.unanswered.push(invocation);
        return;
    }
    let reply = match (method, &node.text) {
        ("GetChildren", _) => {
            let children: Vec<(String, ObjectPath)> = node
                .children
                .iter()
                .map(|child| {
                    let path = ObjectPath::try_from(path(*child)).expect("an object path");
                    (own.to_owned(), path)
                })
                .collect();
            (children,).to_variant()
        }
        ("GetState", _) => {
            let mut words = vec![0u32; 2];
            for state in node.states {
                words[(state / 32) as usize] |= 1 << (state % 32);
            }
            (words,).to_variant()
        }
        ("GetRole", _) => (node.role,).to_variant(),
        ("GetInterfaces", text) => {
            let mut interfaces = vec![ACCESSIBLE];
            interfaces.extend(text.is_some().then_some(TEXT));
            (interfaces,).to_variant()
        }
        ("GetText", Some(text)) => {
            stage.text_reads += 1;
            (text.value,).to_variant()
        }
        ("GetNSelections", Some(text)) => (i32::from(text.selection.is_some()),).to_variant(),
        ("GetSelection", Some(text)) => match text.selection {
            Some(selection) => selection.to_variant(),
            None => {
                invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.InvalidArgs",
                    "Not a valid selection: 0",
                );
                return;
            }
        },
        _ => {
            invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                "not a text field",
            );
            return;
        }
    };
    invocation.return_value(Some(&reply));
}

/// Owns `name`, or fails where a real service already does: this test
/// stands in for the accessibility bus and must not run beside one.
fn own(connection: &gio::DBusConnection, name: &'static str) {
    let owned = Rc::new(Cell::new(None));
    gio::bus_own_name_on_connection(
        connection,
        name,
        gio::BusNameOwnerFlags::DO_NOT_QUEUE,
        {
            let owned = owned.clone();
            move |_, _| owned.set(Some(true))
        },
        {
            let owned = owned.clone();
            move |_, _| owned.set(Some(false))
        },
    );
    let context = glib::MainContext::default();
    while owned.get().is_none() {
        context.iteration(true);
    }
    assert_eq!(
        owned.get(),
        Some(true),
        "{name} is taken; run this test with scripts/test-linux-gui.sh"
    );
}

/// Reads on another thread, as the core does, while this one answers.
fn read_while_serving(process_id: Option<u32>) -> (Option<FieldText>, Duration) {
    let started = Instant::now();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || sender.send(read(process_id)));
    let context = glib::MainContext::default();
    loop {
        while context.iteration(false) {}
        if let Ok(field) = receiver.try_recv() {
            return (field, started.elapsed());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn found(text: &str, caret_utf16: usize) -> Option<FieldText> {
    Some(FieldText {
        text: text.into(),
        caret_utf16,
    })
}

struct Case {
    name: &'static str,
    tree: Vec<Node>,
    /// the process to look in; `None` searches every application
    process_id: Option<u32>,
    want: Option<FieldText>,
    /// how often an application was asked for its text
    text_reads: usize,
    /// the least the read takes, and what it stays under
    takes: (Duration, Duration),
}

#[test]
#[ignore = "needs the private session bus of scripts/test-linux-gui.sh"]
fn the_focused_field_is_read_from_the_accessibility_bus() {
    let ours = std::process::id();
    // an answer, as opposed to a read that was given up
    let prompt = (Duration::ZERO, LIMIT);
    let cases = [
        Case {
            name: "the field of the target application",
            tree: one_application(field("Troy Barnes joined the", 22)),
            process_id: Some(ours),
            want: found("Troy Barnes joined the", 22),
            text_reads: 1,
            takes: prompt,
        },
        Case {
            name: "the field of whichever application is active",
            tree: vec![
                parent(&[1, 3]),
                parent(&[2]),
                window(&[], &[]),
                parent(&[4]),
                window(&[ACTIVE], &[5, 6]),
                parent(&[]),
                field("Greendale is open.", 18),
            ],
            process_id: None,
            want: found("Greendale is open.", 18),
            text_reads: 1,
            takes: prompt,
        },
        Case {
            name: "a caret after characters outside the BMP",
            tree: one_application(field("🎓🎓 Troy", 3)),
            process_id: Some(ours),
            want: found("🎓🎓 Troy", 5),
            text_reads: 1,
            takes: prompt,
        },
        Case {
            name: "a selection counts from its start, wherever the caret is",
            tree: one_application(Node {
                text: Some(Text {
                    value: "Troy Barnes joined",
                    caret: 11,
                    selection: Some((5, 11)),
                }),
                ..field("", 0)
            }),
            process_id: Some(ours),
            want: found("Troy Barnes joined", 5),
            text_reads: 1,
            takes: prompt,
        },
        Case {
            name: "a password field is not asked for its text",
            tree: one_application(Node {
                role: PASSWORD_TEXT,
                ..field("••••••••••", 10)
            }),
            process_id: Some(ours),
            want: None,
            text_reads: 0,
            takes: prompt,
        },
        Case {
            name: "hidden text in an ordinary field",
            tree: one_application(field("••••••••••", 10)),
            process_id: Some(ours),
            want: None,
            text_reads: 1,
            takes: prompt,
        },
        Case {
            name: "a field without a caret",
            tree: one_application(field("Troy Barnes", -1)),
            process_id: Some(ours),
            want: None,
            text_reads: 1,
            takes: prompt,
        },
        Case {
            name: "the focus on something without text",
            tree: one_application(Node {
                states: &[FOCUSED],
                role: BUTTON_ROLE,
                ..Node::default()
            }),
            process_id: Some(ours),
            want: None,
            text_reads: 0,
            takes: prompt,
        },
        Case {
            name: "a focused field in a window that is not active",
            tree: vec![
                parent(&[1]),
                parent(&[2]),
                window(&[], &[3]),
                field("Troy Barnes", 4),
            ],
            process_id: Some(ours),
            want: None,
            text_reads: 0,
            takes: prompt,
        },
        Case {
            name: "another process than the target",
            tree: one_application(field("Troy Barnes", 4)),
            // the session's first process is never this test
            process_id: Some(1),
            want: None,
            text_reads: 0,
            takes: prompt,
        },
        Case {
            name: "no application at all",
            tree: vec![parent(&[])],
            process_id: None,
            want: None,
            text_reads: 0,
            takes: prompt,
        },
        Case {
            name: "an application that hangs is passed over",
            tree: vec![
                parent(&[1, 2]),
                hung(),
                parent(&[3]),
                window(&[ACTIVE], &[4]),
                field("Troy Barnes joined the", 22),
            ],
            process_id: None,
            want: found("Troy Barnes joined the", 22),
            text_reads: 1,
            takes: (CALL_TIMEOUT, LIMIT),
        },
        Case {
            name: "the read is given up at the limit",
            tree: vec![
                parent(&[1, 2, 3, 4, 5, 6, 7, 8]),
                hung(),
                hung(),
                hung(),
                hung(),
                hung(),
                hung(),
                hung(),
                hung(),
            ],
            process_id: None,
            want: None,
            text_reads: 0,
            // well before the eight applications have each timed out
            takes: (LIMIT, LIMIT * 2),
        },
    ];

    let connection =
        gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).expect("a session bus");
    let stage = Rc::new(RefCell::new(Stage::default()));
    let nodes = cases.iter().map(|case| case.tree.len()).max().unwrap_or(0);
    serve(&connection, &stage, nodes);
    own(&connection, "org.a11y.Bus");
    own(&connection, "org.a11y.atspi.Registry");

    for case in cases {
        *stage.borrow_mut() = Stage {
            tree: case.tree,
            ..Stage::default()
        };
        let (got, took) = read_while_serving(case.process_id);
        assert_eq!(got, case.want, "{}", case.name);
        assert_eq!(stage.borrow().text_reads, case.text_reads, "{}", case.name);
        let (least, under) = case.takes;
        assert!(
            took >= least && took < under,
            "{}: took {took:?}, expected {least:?} to {under:?}",
            case.name
        );
    }
}
