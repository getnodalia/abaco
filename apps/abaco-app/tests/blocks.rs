// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The block workspace end to end, without a window: editing the diagram, the worker thread, the
//! figure made from a run, stopping, errors and the file text.

use std::time::{Duration, Instant};

use abaco_app::blocks::engine::BlocksEngine;
use abaco_app::blocks::session::{auto_layout, Session, Status, EXAMPLE};
use abaco_blocks::{palette, BlockKind, PaletteEntry};

fn entry(name: &str) -> PaletteEntry {
    palette().into_iter().find(|e| e.name == name).unwrap()
}

fn run_to_end(engine: &mut BlocksEngine, s: &mut Session) {
    let job = s.run().expect("a run starts");
    engine.send(job).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while s.running {
        for r in engine.poll() {
            s.apply(r);
        }
        assert!(Instant::now() < deadline, "the worker did not answer");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn blocks_get_unique_names_and_snap_to_the_grid() {
    let mut s = Session::new();
    let a = s.add_block(&entry("Gain"), (13.0, 27.0)).unwrap();
    let b = s.add_block(&entry("Gain"), (0.0, 0.0)).unwrap();
    let c = s.add_block(&entry("Gain"), (0.0, 0.0)).unwrap();
    assert_eq!(
        (a.as_str(), b.as_str(), c.as_str()),
        ("Gain", "Gain1", "Gain2")
    );
    assert_eq!(s.model.block("Gain").unwrap().position, Some((10.0, 30.0)));
    assert!(s.dirty);
    // A hostile position is tamed, not stored.
    s.move_block("Gain", (f64::NAN, f64::INFINITY));
    assert_eq!(s.model.block("Gain").unwrap().position, Some((0.0, 0.0)));
}

#[test]
fn wiring_checks_ports_and_replaces_the_line_into_an_input() {
    let mut s = Session::new();
    s.add_block(&entry("Constant"), (0.0, 0.0)).unwrap();
    s.add_block(&entry("Step"), (0.0, 100.0)).unwrap();
    s.add_block(&entry("Gain"), (200.0, 0.0)).unwrap();
    s.connect(("Constant", 0), ("Gain", 0)).unwrap();
    s.connect(("Step", 0), ("Gain", 0)).unwrap();
    assert_eq!(s.model.lines.len(), 1, "the new line replaces the old one");
    assert_eq!(s.model.lines[0].from.0, "Step");
    assert!(s
        .connect(("Constant", 1), ("Gain", 0))
        .unwrap_err()
        .contains("no output 2"));
    assert!(s
        .connect(("Constant", 0), ("Gain", 1))
        .unwrap_err()
        .contains("no input 2"));
    assert!(s.connect(("Nope", 0), ("Gain", 0)).is_err());
    // Gain has no output... it has one; a Scope has none to start a line from.
    s.add_block(&entry("Scope"), (400.0, 0.0)).unwrap();
    assert!(s.connect(("Scope", 0), ("Gain", 0)).is_err());
}

#[test]
fn deleting_a_block_removes_its_lines_and_renaming_keeps_them() {
    let mut s = Session::new();
    s.add_block(&entry("Constant"), (0.0, 0.0)).unwrap();
    s.add_block(&entry("Gain"), (100.0, 0.0)).unwrap();
    s.add_block(&entry("Scope"), (200.0, 0.0)).unwrap();
    s.connect(("Constant", 0), ("Gain", 0)).unwrap();
    s.connect(("Gain", 0), ("Scope", 0)).unwrap();
    s.rename_block("Gain", "Amp").unwrap();
    assert_eq!(s.model.lines[0].to.0, "Amp");
    assert_eq!(s.model.lines[1].from.0, "Amp");
    assert!(s.rename_block("Amp", "Scope").is_err());
    assert!(s.rename_block("Amp", "a/b").is_err());
    assert!(s.rename_block("Amp", " ").is_err());
    s.delete_block("Amp");
    assert!(s.model.lines.is_empty());
    assert_eq!(s.model.blocks.len(), 2);
}

#[test]
fn parameters_are_edited_through_json_and_bad_ones_are_refused() {
    let mut s = Session::new();
    s.add_block(&entry("Constant"), (0.0, 0.0)).unwrap();
    s.add_block(&entry("Sum"), (100.0, 0.0)).unwrap();
    s.add_block(&entry("Scope"), (200.0, 0.0)).unwrap();
    s.connect(("Constant", 0), ("Sum", 1)).unwrap();
    s.connect(("Sum", 0), ("Scope", 0)).unwrap();
    let mut p = s.params("Sum").unwrap();
    assert_eq!(p["signs"], "+-");
    // Three inputs now: the lines stay.
    p.insert("signs".into(), "+-+".into());
    assert_eq!(s.set_params("Sum", p.clone()).unwrap(), 0);
    // One input: the line into input 2 goes with it.
    p.insert("signs".into(), "+".into());
    assert_eq!(s.set_params("Sum", p).unwrap(), 1);
    // Wrong types and impossible parameters are errors and change nothing.
    let mut bad = s.params("Constant").unwrap();
    bad.insert("value".into(), "text".into());
    assert!(s.set_params("Constant", bad).is_err());
    let mut bad = s.params("Sum").unwrap();
    bad.insert("signs".into(), "+x".into());
    assert!(s.set_params("Sum", bad).is_err());
    assert!(matches!(
        s.model.block("Constant").unwrap().kind,
        BlockKind::Constant { .. }
    ));
}

#[test]
fn the_example_opens_runs_and_plots_one_axes_per_scope() {
    let mut s = Session::new();
    s.open_example();
    assert!(s.model.blocks.iter().all(|b| b.position.is_some()));
    let mut engine = BlocksEngine::spawn(|| {}).unwrap();
    run_to_end(&mut engine, &mut s);
    let Status::Done { samples, steps, .. } = s.status else {
        panic!("{:?}", s.status)
    };
    assert!(samples > 10 && steps > 10);
    assert_eq!(s.figure.axes.len(), 2);
    assert_eq!(s.figure.axes[0].title, "Position");
    let last = *s.figure.axes[0].series[0].y.last().unwrap();
    assert!(
        (last - 1.0).abs() < 0.02,
        "position settles at 1, got {last}"
    );
}

#[test]
fn a_broken_diagram_reports_the_block_and_a_stopped_run_says_so() {
    let mut s = Session::new();
    s.add_block(&entry("Gain"), (0.0, 0.0)).unwrap();
    let mut engine = BlocksEngine::spawn(|| {}).unwrap();
    run_to_end(&mut engine, &mut s);
    let Status::Failed(m) = &s.status else {
        panic!("{:?}", s.status)
    };
    assert!(m.contains("Gain") && m.contains("not connected"), "{m}");
    // A long run, stopped.
    let mut s = Session::new();
    s.open(None, EXAMPLE).unwrap();
    s.model.solver.stop_time = 1.0e9;
    s.model.solver.max_step = Some(1e-4);
    let job = s.run().unwrap();
    engine.send(job).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    engine.stop();
    let deadline = Instant::now() + Duration::from_secs(30);
    while s.running {
        for r in engine.poll() {
            s.apply(r);
        }
        assert!(Instant::now() < deadline, "the stop was not honoured");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(s.status, Status::Stopped);
    // A second run is refused while one is going, and works after.
    assert!(s.run().is_some());
    assert!(s.run().is_none());
}

#[test]
fn files_round_trip_and_hostile_text_is_an_error() {
    let mut s = Session::new();
    s.open_example();
    let text = s.text().unwrap();
    let mut t = Session::new();
    t.open(None, &text).unwrap();
    assert_eq!(s.model, t.model);
    for bad in [
        "",
        "[]",
        "{",
        "null",
        "{\"blocks\": 3}",
        "{\"blocks\": [{\"name\": \"a\"}]}",
    ] {
        assert!(Session::new().open(None, bad).is_err(), "{bad}");
    }
    // Opening failed, so the diagram that was there is still there.
    let mut u = Session::new();
    u.open_example();
    let n = u.model.blocks.len();
    assert!(u.open(None, "[]").is_err());
    assert_eq!(u.model.blocks.len(), n);
}

#[test]
fn auto_layout_puts_sources_left_of_what_they_feed_and_keeps_given_positions() {
    let mut s = Session::new();
    s.open_example();
    let x = |n: &str| s.model.block(n).unwrap().position.unwrap().0;
    assert!(x("ref") < x("error") && x("error") < x("kp") && x("kp") < x("command"));
    let mut m = s.model.clone();
    for b in &mut m.blocks {
        b.position = Some((5.0, 5.0));
    }
    auto_layout(&mut m);
    assert!(m.blocks.iter().all(|b| b.position == Some((5.0, 5.0))));
}

#[test]
fn a_feedback_loop_does_not_stretch_the_layout() {
    let mut s = Session::new();
    s.open_example();
    let mut m = s.model.clone();
    for b in &mut m.blocks {
        b.position = None;
    }
    auto_layout(&mut m);
    // Ten blocks with one loop: seven columns at most, never one per block.
    let widest = m
        .blocks
        .iter()
        .filter_map(|b| b.position)
        .map(|p| p.0)
        .fold(0.0_f64, f64::max);
    assert!(widest <= 40.0 + 7.0 * 150.0, "layout is {widest} wide");
    let x = |n: &str| m.block(n).unwrap().position.unwrap().0;
    assert!(x("ref") < x("error") && x("error") < x("kp") && x("kp") < x("command"));
}

#[test]
fn simulink_files_open_and_the_example_exports_to_them() {
    use abaco_app::blocks::session::{
        read_diagram, simulink_bytes, with_simulink_extension, write_diagram,
    };
    let dir = std::env::temp_dir().join(format!("abaco-app-simulink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    for name in ["servo.slx", "servo.mdl"] {
        let path = dir.join(name);
        let bytes = simulink_bytes(EXAMPLE, &path).unwrap();
        write_diagram(&path, bytes).unwrap();
        let loaded = read_diagram(&path).unwrap();
        assert!(loaded.imported, "{name}");
        let mut s = Session::new();
        s.open(None, &loaded.text).unwrap();
        let mut original = Session::new();
        original.open_example();
        assert_eq!(s.model.blocks.len(), original.model.blocks.len(), "{name}");
        let mut engine = BlocksEngine::spawn(|| {}).unwrap();
        run_to_end(&mut engine, &mut s);
        assert!(
            matches!(s.status, Status::Done { .. }),
            "{name}: {:?}",
            s.status
        );
    }

    // The extension chosen in a dialog is kept when it names a Simulink file, and added otherwise.
    assert_eq!(with_simulink_extension(&dir.join("a")), dir.join("a.slx"));
    assert_eq!(
        with_simulink_extension(&dir.join("a.mdl")),
        dir.join("a.mdl")
    );

    // A damaged file is an error that names what is wrong, never a panic.
    let bad = dir.join("bad.slx");
    std::fs::write(&bad, b"PK\x03\x04 not really").unwrap();
    assert!(read_diagram(&bad).is_err());
    let bad = dir.join("bad.mdl");
    std::fs::write(&bad, b"Model { Name \"x\" ").unwrap();
    assert!(read_diagram(&bad).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
