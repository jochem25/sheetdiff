//! De wachtrij met een nepwerker: een hangend of wegvallend blad raakt alleen zichzelf.

use sheetdiff_core::parallel::{run_pool, Outcome};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fake_worker() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap();
    dir.join("examples")
        .join(format!("fake_worker{}", std::env::consts::EXE_SUFFIX))
}

fn run(tasks: &[&str], workers: usize) -> Vec<(String, Outcome)> {
    let tasks = tasks
        .iter()
        .map(|t| (t.to_string(), t.to_string()))
        .collect();
    let mut got = Vec::new();
    let job = PathBuf::from("niet-gebruikt");
    let timeout = Duration::from_millis(1500);
    run_pool(&fake_worker(), &job, workers, tasks, timeout, |t, o| {
        got.push((t, o))
    });
    got.sort_by(|x, y| x.0.cmp(&y.0));
    got
}

#[test]
fn a_hanging_task_times_out_and_the_rest_goes_on() {
    let t0 = Instant::now();
    let got = run(&["a", "hang", "b", "c"], 1);
    assert!(t0.elapsed() < Duration::from_secs(20));
    assert_eq!(got.len(), 4);
    assert_eq!(got[0].1, Outcome::Line("a".into()));
    assert_eq!(got[1].1, Outcome::Line("b".into()));
    assert_eq!(got[2].1, Outcome::Line("c".into()));
    assert_eq!(got[3].1, Outcome::TimedOut);
}

#[test]
fn a_worker_that_dies_costs_only_its_own_task() {
    let got = run(&["a", "stop", "b"], 1);
    assert_eq!(got[0].1, Outcome::Line("a".into()));
    assert_eq!(got[1].1, Outcome::Line("b".into()));
    assert!(matches!(got[2].1, Outcome::Died(_)));
}

#[test]
fn every_task_gets_one_outcome_when_the_worker_cannot_start() {
    let tasks = vec![("x".to_string(), 1), ("y".to_string(), 2)];
    let (mut n, exe) = (0, PathBuf::from("bestaat-niet.exe"));
    let job = PathBuf::from("niet-gebruikt");
    run_pool(&exe, &job, 2, tasks, Duration::from_secs(1), |_, o| {
        assert!(matches!(o, Outcome::Died(_)));
        n += 1;
    });
    assert_eq!(n, 2);
}
