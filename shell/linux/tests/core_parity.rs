//! Fails when the Linux shell leaves a core command uncalled without saying
//! so in `uncalled-core-commands.txt`, and when that list names a command the
//! shell calls. A gap between the shells can then neither open nor close
//! without a change to the list.
//!
//! The check reads source text. It counts `app.<command>(` and
//! `App::<command>(` outside test code as a call, which is how every call site
//! in `shell/linux/src` is written.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const LIST: &str = "shell/linux/tests/uncalled-core-commands.txt";
const CORE: &str = "crates/onda-app/src";
const SHELL: &str = "shell/linux/src";

/// One shell source file, reduced to the text that can hold a call.
struct Source {
    path: String,
    code: String,
}

/// The public methods of every `impl App` block in one core source file.
fn commands(core: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut in_app = false;
    for line in core.lines() {
        match line {
            "impl App {" => in_app = true,
            "}" => in_app = false,
            _ if in_app => found.extend(public_method(line)),
            _ => {}
        }
    }
    found
}

fn public_method(line: &str) -> Option<String> {
    let mut rest = line.strip_prefix("    pub ")?;
    for qualifier in ["const ", "async ", "unsafe "] {
        rest = rest.strip_prefix(qualifier).unwrap_or(rest);
    }
    let name: String = rest
        .strip_prefix("fn ")?
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    Some(name)
}

/// Drops comment lines and the trailing test module, then all whitespace, so
/// a receiver that rustfmt moved to the line above still sits next to its call.
fn code(source: &str) -> String {
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let tests = lines
        .windows(2)
        .position(|pair| {
            pair[0] == "#[cfg(test)]" && pair[1].starts_with("mod ") && pair[1].ends_with('{')
        })
        .unwrap_or(lines.len());
    lines[..tests]
        .iter()
        .filter(|line| !line.starts_with("//"))
        .flat_map(|line| line.split_whitespace())
        .collect()
}

fn calls(code: &str, command: &str) -> bool {
    if code.contains(&format!("App::{command}(")) {
        return true;
    }
    let call = format!("app.{command}(");
    code.match_indices(&call).any(|(at, _)| {
        !code[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// The list as `command -> line number`, or what is wrong with it.
fn listed(list: &str) -> Result<BTreeMap<&str, usize>, Vec<String>> {
    let mut entries = BTreeMap::new();
    let mut problems = Vec::new();
    for (index, line) in list.lines().enumerate() {
        let number = index + 1;
        let mut words = line.split_whitespace();
        let Some(command) = words.next().filter(|word| !word.starts_with('#')) else {
            continue;
        };
        let explained = match words.next() {
            Some("never") => words.next().is_some(),
            Some(issue) => issue
                .strip_prefix('#')
                .is_some_and(|digits| digits.parse::<u32>().is_ok()),
            None => false,
        };
        if !explained {
            problems.push(format!(
                "{LIST}:{number}: write `{command} #<issue that will call it>` or \
                 `{command} never <why Linux never needs it>`."
            ));
        }
        if entries.insert(command, number).is_some() {
            problems.push(format!(
                "{LIST}:{number}: delete this line. `{command}` is listed twice."
            ));
        }
    }
    if problems.is_empty() {
        Ok(entries)
    } else {
        Err(problems)
    }
}

fn problems(commands: &BTreeSet<String>, shell: &[Source], list: &str) -> Vec<String> {
    let listed = match listed(list) {
        Ok(listed) => listed,
        Err(problems) => return problems,
    };
    let mut problems = Vec::new();
    for command in commands {
        let caller = shell.iter().find(|source| calls(&source.code, command));
        match (caller, listed.get(command.as_str())) {
            (Some(_), None) => {}
            (None, Some(_)) => {}
            (Some(source), Some(number)) => problems.push(format!(
                "{LIST}:{number}: delete this line. {} now calls `{command}`.",
                source.path
            )),
            (None, None) => problems.push(format!(
                "`{command}` is a public command of `onda_app::App` that the Linux shell never \
                 calls. Call it as `app.{command}(..)` under {SHELL}, or add a line to {LIST}: \
                 `{command} #<issue that will call it>`."
            )),
        }
    }
    for (command, number) in &listed {
        if !commands.contains(*command) {
            problems.push(format!(
                "{LIST}:{number}: delete this line. `onda_app::App` has no public command \
                 `{command}`."
            ));
        }
    }
    problems
}

/// Every `.rs` file under `directory` that `keep` accepts, as `(path, text)`
/// with the path relative to `root`.
fn rust_files(root: &Path, directory: &str, keep: fn(&str) -> bool) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut pending = vec![root.join(directory)];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("a readable source directory") {
            let path = entry.expect("a readable directory entry").path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if path.is_dir() {
                pending.push(path);
            } else if name.ends_with(".rs") && keep(name) {
                let text = fs::read_to_string(&path).expect("a readable source file");
                let relative = path.strip_prefix(root).expect("a path under the root");
                files.push((relative.display().to_string(), text));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn every_core_command_is_called_or_listed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let commands: BTreeSet<String> = rust_files(&root, CORE, |_| true)
        .iter()
        .flat_map(|(_, text)| commands(text))
        .collect();
    assert!(
        !commands.is_empty(),
        "found no `impl App {{` block with public methods under {CORE}"
    );
    let shell: Vec<Source> = rust_files(&root, SHELL, |name| {
        name != "tests.rs" && !name.ends_with("_tests.rs")
    })
    .into_iter()
    .map(|(path, text)| Source {
        path,
        code: code(&text),
    })
    .collect();
    let list = fs::read_to_string(root.join(LIST)).expect("the list of uncalled commands");

    let problems = problems(&commands, &shell, &list);
    assert!(
        problems.is_empty(),
        "the Linux shell and {LIST} disagree:\n\n{}\n",
        problems.join("\n\n")
    );
}

#[test]
fn reports_what_to_change() {
    const CORE: &str = "\
impl App {
    /// Starts the core.
    pub fn start_on(runtime: &Handle) -> Self {
    pub fn ui_ready(&self) {
    pub async fn shutdown_async(&self) {
    pub(crate) fn internal(&self) {
    fn send(&self, msg: Msg) {
}
impl Other {
    pub fn elsewhere(&self) {
}
";
    const CALLS_ALL: &str = "
        let app = App::start_on(runtime.handle());
        owner
            .core
            .app
            .ui_ready();
        block_on(window.core.app.shutdown_async());
    ";
    struct Case {
        shell: &'static str,
        list: &'static str,
        want: &'static [&'static str],
    }
    let cases = BTreeMap::from([
        (
            "every command called, nothing listed",
            Case {
                shell: CALLS_ALL,
                list: "# nothing left\n",
                want: &[],
            },
        ),
        (
            "an uncalled command that is listed",
            Case {
                shell: "App::start_on(handle); app.shutdown_async();",
                list: "ui_ready #15 onboarding\n",
                want: &[],
            },
        ),
        (
            "an uncalled command that is not listed",
            Case {
                shell: "App::start_on(handle); app.shutdown_async();",
                list: "",
                want: &["`ui_ready` is a public command"],
            },
        ),
        (
            "a listed command that is now called",
            Case {
                shell: CALLS_ALL,
                list: "# header\nui_ready #15\n",
                want: &[":2: delete this line. ui.rs now calls `ui_ready`."],
            },
        ),
        (
            "a listed name that is not a command",
            Case {
                shell: CALLS_ALL,
                list: "elsewhere #3\ninternal never private\n",
                want: &[
                    ":1: delete this line. `onda_app::App` has no public command `elsewhere`.",
                    ":2: delete this line. `onda_app::App` has no public command `internal`.",
                ],
            },
        ),
        (
            "a same-named method on another receiver is not a call",
            Case {
                shell: "App::start_on(handle); settings.ui_ready(); wrapped.ui_ready(); \
                        mapp.ui_ready(); app.shutdown_async();",
                list: "",
                want: &["`ui_ready` is a public command"],
            },
        ),
        (
            "a call in a comment or a test module is not a call",
            Case {
                shell: "App::start_on(handle); app.shutdown_async();
                    // app.ui_ready();
                    #[cfg(test)]
                    mod tests {
                        fn ready() { app.ui_ready(); }
                    }",
                list: "",
                want: &["`ui_ready` is a public command"],
            },
        ),
        (
            "an entry without an issue or a reason",
            Case {
                shell: "App::start_on(handle); app.shutdown_async();",
                list: "ui_ready\nui_ready never\nui_ready later\n",
                want: &[
                    ":1: write `ui_ready #<issue",
                    ":2: write `ui_ready #<issue",
                    ":2: delete this line. `ui_ready` is listed twice.",
                    ":3: write `ui_ready #<issue",
                    ":3: delete this line. `ui_ready` is listed twice.",
                ],
            },
        ),
    ]);
    let commands = commands(CORE);
    assert_eq!(
        commands.iter().map(String::as_str).collect::<Vec<_>>(),
        ["shutdown_async", "start_on", "ui_ready"]
    );
    for (name, tc) in cases {
        let shell = [Source {
            path: "ui.rs".into(),
            code: code(tc.shell),
        }];
        let got = problems(&commands, &shell, tc.list);
        assert_eq!(got.len(), tc.want.len(), "{name}: {got:#?}");
        for (got, want) in got.iter().zip(tc.want) {
            assert!(got.contains(want), "{name}: {got:?} lacks {want:?}");
        }
    }
}
