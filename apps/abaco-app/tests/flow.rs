// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The window's logic end to end, without a window: the reducer, the engine's worker thread and
//! real files.

use std::time::{Duration, Instant};

use abaco_app::engine::{Engine, Notice, Outcome};
use abaco_app::workbook::{Kind, Msg, Workbook};

/// Applies `msg`, sends what the reducer asks for and folds replies in until the engine is idle.
fn step(wb: &mut Workbook, engine: &mut Engine, msg: Msg) {
    for request in wb.update(msg) {
        engine.send(request).expect("worker is alive");
    }
    settle(wb, engine);
}

fn settle(wb: &mut Workbook, engine: &mut Engine) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for reply in engine.poll() {
            wb.apply(reply);
        }
        if engine.in_flight() == 0 {
            return;
        }
        assert!(Instant::now() < deadline, "the worker did not answer");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn type_into(wb: &mut Workbook, engine: &mut Engine, text: &str) {
    let id = wb.selected.expect("a cell is selected");
    wb.cell_mut(id).expect("cell").buffer = text.to_owned();
    step(wb, engine, Msg::Edited(id));
}

fn value_of(wb: &Workbook, index: usize) -> String {
    match &wb.cells[index].outcome {
        Outcome::Ok { value: Some(v), .. } => v.text.clone(),
        other => panic!("no value: {other:?}"),
    }
}

#[test]
fn typing_adding_and_saving_a_document_round_trips_through_a_file() {
    let mut engine = Engine::spawn(|| {}).expect("spawn");
    let mut wb = Workbook::new();
    step(&mut wb, &mut engine, Msg::NewBlank);
    assert_eq!(wb.cells.len(), 1);
    assert_eq!(wb.cells[0].kind, Kind::Math);

    wb.selected = Some(wb.cells[0].id);
    type_into(&mut wb, &mut engine, "mass = 10 kg");
    step(&mut wb, &mut engine, Msg::EvaluateAndAdd);
    assert_eq!(wb.cells.len(), 2, "Ctrl+Enter adds a cell");
    assert_eq!(wb.selected, Some(wb.cells[1].id), "and selects it");
    type_into(&mut wb, &mut engine, "mass * 9.81 m/s^2 -> N");
    assert_eq!(value_of(&wb, 1), "98.1 N");
    assert!(wb.dirty);

    // Changing m recomputes the cell that uses it.
    wb.selected = Some(wb.cells[0].id);
    type_into(&mut wb, &mut engine, "mass = 20 kg");
    assert_eq!(value_of(&wb, 1), "196.2 N");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("beam.abaco");
    step(&mut wb, &mut engine, Msg::Save(Some(path.clone())));
    assert!(
        wb.notices.contains(&Notice::Saved(path.clone())),
        "{:?}",
        wb.notices
    );
    assert!(!wb.dirty);
    assert!(path.is_file());

    let mut other = Workbook::new();
    step(&mut other, &mut engine, Msg::Open(path.clone()));
    assert!(other.notices.contains(&Notice::Opened(path)));
    assert_eq!(other.cells.len(), 2);
    assert_eq!(other.cells[0].buffer, "mass = 20 kg");
    assert_eq!(value_of(&other, 1), "196.2 N");
    assert!(!other.dirty);
}

#[test]
fn an_error_is_shown_in_place_with_its_position() {
    let mut engine = Engine::spawn(|| {}).expect("spawn");
    let mut wb = Workbook::new();
    step(&mut wb, &mut engine, Msg::NewBlank);
    wb.selected = Some(wb.cells[0].id);
    type_into(&mut wb, &mut engine, "x = 1 m\ny = x + 2 s");
    match &wb.cells[0].outcome {
        Outcome::Err { error, .. } => {
            assert_eq!(error.position.map(|p| p.0), Some(2), "{error:?}");
            assert!(error.quote.is_some());
        }
        other => panic!("expected an error, got {other:?}"),
    }
    assert_eq!(wb.errors, 1);
    // Fixing the cell clears it.
    type_into(&mut wb, &mut engine, "x = 1 m\ny = x + 2 m");
    assert_eq!(wb.errors, 0);
}

#[test]
fn the_sample_opens_without_errors_and_a_bad_file_is_a_notice() {
    let mut engine = Engine::spawn(|| {}).expect("spawn");
    let mut wb = Workbook::new();
    step(&mut wb, &mut engine, Msg::NewSample);
    assert!(wb.cells.len() > 10);
    assert_eq!(wb.errors, 0);
    assert!(!wb.dirty);

    let dir = tempfile::tempdir().expect("tempdir");
    let bad = dir.path().join("broken.abaco");
    std::fs::write(&bad, "{ not json").expect("write");
    let before = wb.cells.len();
    step(&mut wb, &mut engine, Msg::Open(bad.clone()));
    assert!(
        wb.notices
            .iter()
            .any(|n| matches!(n, Notice::OpenFailed { path, .. } if *path == bad)),
        "{:?}",
        wb.notices
    );
    assert_eq!(wb.cells.len(), before, "the document is kept");
}

#[test]
fn reordering_and_removing_cells_keep_results_attached() {
    let mut engine = Engine::spawn(|| {}).expect("spawn");
    let mut wb = Workbook::new();
    step(&mut wb, &mut engine, Msg::NewBlank);
    wb.selected = Some(wb.cells[0].id);
    type_into(&mut wb, &mut engine, "a = 2 m");
    step(&mut wb, &mut engine, Msg::EvaluateAndAdd);
    type_into(&mut wb, &mut engine, "b = a * 3");
    let first = wb.cells[0].id;
    step(&mut wb, &mut engine, Msg::MoveDown(first));
    assert_eq!(wb.cells[1].buffer, "a = 2 m");
    assert_eq!(wb.cells.len(), 2);
    step(&mut wb, &mut engine, Msg::Remove(first));
    assert_eq!(wb.cells.len(), 1);
    assert_eq!(wb.cells[0].buffer, "b = a * 3");
}
