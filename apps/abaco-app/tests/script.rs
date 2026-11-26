// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The script workspace end to end, without a window: the worker thread, the session reducer,
//! the command window's history, stopping a run, errors, variables and figures.

use std::time::{Duration, Instant};

use abaco_app::script::engine::{Job, ScriptEngine};
use abaco_app::script::session::{Kind, Session};

struct Rig {
    engine: ScriptEngine,
    session: Session,
}

impl Rig {
    fn new() -> Rig {
        Rig {
            engine: ScriptEngine::spawn(|| {}).expect("spawn"),
            session: Session::new(),
        }
    }

    fn send(&mut self, job: Option<Job>) {
        if let Some(job) = job {
            self.engine.send(job).expect("worker is alive");
        }
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            for reply in self.engine.poll() {
                self.session.apply(reply);
            }
            if self.engine.in_flight() == 0 {
                return;
            }
            assert!(Instant::now() < deadline, "the worker did not answer");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn command(&mut self, line: &str) {
        self.session.input = line.to_owned();
        let job = self.session.submit();
        self.send(job);
        self.settle();
    }

    fn texts(&self, kind: Kind) -> Vec<&str> {
        self.session
            .lines
            .iter()
            .filter(|l| l.kind == kind)
            .map(|l| l.text.as_str())
            .collect()
    }
}

#[test]
fn a_command_prints_and_defines_variables_that_later_commands_use() {
    let mut r = Rig::new();
    r.command("x = [1 2 3]*2");
    assert!(r.texts(Kind::Input).contains(&">> x = [1 2 3]*2"));
    let out = r.texts(Kind::Output).join("\n");
    assert!(out.contains("x ="), "{out}");
    assert!(out.contains('6'), "{out}");
    assert_eq!(r.session.vars.len(), 1);
    assert_eq!(r.session.vars[0].name, "x");
    assert_eq!(r.session.vars[0].size, "1x3");
    assert_eq!(r.session.vars[0].class, "double");
    r.command("s = sum(x)");
    assert!(r
        .texts(Kind::Output)
        .iter()
        .any(|t| t.contains("s =") && t.contains("12")));
    assert!(!r.session.running);
    assert!(r.session.last_millis.is_some());
}

#[test]
fn a_failing_command_shows_the_error_and_keeps_the_session_alive() {
    let mut r = Rig::new();
    r.command("y = [1 2 3](7)");
    assert_eq!(r.texts(Kind::Error).len(), 1, "{:?}", r.session.lines);
    r.command("z = 5;");
    assert_eq!(r.session.vars.len(), 1);
    assert!(!r.session.running);
}

#[test]
fn the_editor_runs_as_a_script_and_marks_the_failing_line() {
    let mut r = Rig::new();
    r.session.editor = "a = 1;\nb = 2;\nc = a + undefined_name;\n".to_owned();
    let job = r.session.run_editor();
    r.send(job);
    r.settle();
    assert_eq!(r.session.error_line, Some(3), "{:?}", r.session.lines);
    assert_eq!(r.texts(Kind::Error).len(), 1);
    // The variables set before the failure are kept.
    let names: Vec<&str> = r.session.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    r.session.editor = "c = a + b".to_owned();
    let job = r.session.run_editor();
    r.send(job);
    r.settle();
    assert_eq!(r.session.error_line, None);
}

#[test]
fn history_walks_up_and_down_and_returns_to_the_draft() {
    let mut r = Rig::new();
    r.command("a = 1;");
    r.command("b = 2;");
    r.command("b = 2;");
    assert_eq!(r.session.history().len(), 2, "a repeat is not stored twice");
    r.session.input = "dra".to_owned();
    r.session.history_up();
    assert_eq!(r.session.input, "b = 2;");
    r.session.history_up();
    assert_eq!(r.session.input, "a = 1;");
    r.session.history_up();
    assert_eq!(r.session.input, "a = 1;", "stops at the oldest");
    r.session.history_down();
    assert_eq!(r.session.input, "b = 2;");
    r.session.history_down();
    assert_eq!(r.session.input, "dra", "and back to what was being typed");
}

#[test]
fn a_run_that_never_ends_can_be_stopped() {
    let mut r = Rig::new();
    r.session.editor = "x = 0;\nwhile true\n  x = x + 1;\nend\n".to_owned();
    let job = r.session.run_editor();
    r.send(job);
    assert!(r.session.running);
    std::thread::sleep(Duration::from_millis(100));
    r.engine.stop();
    r.settle();
    assert!(!r.session.running);
    assert!(r.texts(Kind::Error).is_empty(), "stopping is not an error");
    assert!(r.texts(Kind::Info).contains(&"■"));
    // The interpreter is usable afterwards.
    r.command("ok = 1;");
    assert!(r.session.vars.iter().any(|v| v.name == "ok"));
}

#[test]
fn nothing_starts_while_a_run_is_going_or_for_blank_input() {
    let mut r = Rig::new();
    r.session.input = "   ".to_owned();
    assert!(r.session.submit().is_none());
    r.session.editor = "pause(0);".to_owned();
    let first = r.session.run_editor();
    assert!(first.is_some());
    assert!(r.session.run_editor().is_none(), "one run at a time");
    r.session.input = "q = 1".to_owned();
    assert!(r.session.submit().is_none());
    assert_eq!(r.session.input, "q = 1", "what was typed is kept");
    r.send(first);
    r.settle();
}

#[test]
fn variables_can_be_inspected_removed_and_cleared() {
    let mut r = Rig::new();
    r.command("m = magic(3); k = 4;");
    let job = r.session.select_var("m");
    r.send(Some(job));
    r.settle();
    assert!(r.session.inspected.contains('8'), "{}", r.session.inspected);
    r.send(Some(Job::Remove("m".to_owned())));
    r.settle();
    assert_eq!(
        r.session.selected_var, None,
        "the selection followed the variable"
    );
    assert!(r.session.inspected.is_empty());
    assert_eq!(r.session.vars.len(), 1);
    r.send(Some(Job::ClearVariables));
    r.settle();
    assert!(r.session.vars.is_empty());
}

#[test]
fn plotting_fills_the_figures_and_close_all_empties_them() {
    let mut r = Rig::new();
    r.command("t = 0:0.1:1; plot(t, t.^2, 'r--'); title('squares'); xlabel('t')");
    assert_eq!(r.session.figures.list.len(), 1);
    assert!(r.session.figure_appeared);
    let (n, fig) = &r.session.figures.list[0];
    assert_eq!(*n, r.session.figure);
    assert_eq!(fig.axes[0].title, "squares");
    assert_eq!(fig.axes[0].series[0].y.len(), 11);
    r.command("figure(2); plot([1 2 3])");
    assert_eq!(r.session.figures.list.len(), 2);
    assert_eq!(r.session.figure, 2, "the new figure is the one on show");
    r.send(Some(Job::CloseFigures));
    r.settle();
    assert!(r.session.figures.list.is_empty());
}

#[test]
fn clc_clears_the_window_and_warnings_are_kept() {
    let mut r = Rig::new();
    r.command("disp('hello')");
    assert!(!r.session.lines.is_empty());
    r.command("clc");
    assert!(r.session.lines.is_empty() || r.texts(Kind::Output).is_empty());
    r.command("warning('careful')");
    assert!(r.texts(Kind::Warning).iter().any(|t| t.contains("careful")));
}

#[test]
fn the_window_keeps_at_most_a_few_thousand_lines() {
    let mut s = Session::new();
    for i in 0..(abaco_app::script::session::MAX_LINES + 50) {
        s.push(Kind::Output, format!("line {i}"));
    }
    assert_eq!(s.lines.len(), abaco_app::script::session::MAX_LINES);
    assert_eq!(s.lines[0].text, "line 50", "the oldest went first");
}

#[test]
fn files_name_and_dirty_state_follow_open_and_save() {
    let mut s = Session::new();
    assert_eq!(s.file_name(), None);
    s.open("/tmp/demo.m".into(), "x = 1;".to_owned());
    assert_eq!(s.file_name().as_deref(), Some("demo.m"));
    assert!(!s.dirty);
    s.dirty = true;
    s.saved("/tmp/other.m".into());
    assert!(!s.dirty);
    assert_eq!(s.file_name().as_deref(), Some("other.m"));
    s.new_script();
    assert_eq!(s.file_name(), None);
    assert!(s.editor.is_empty());
}
