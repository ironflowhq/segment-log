use super::deadline::Deadline;
use std::{
    io,
    os::unix::process::ExitStatusExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Given a watchdog with locked stderr or a permanently blocked evidence callback
/// When its short deadline expires in a child process
/// Then it aborts within the outer bound and every child is reaped
/// Cases: SIM-MODEL
#[test]
fn deadlines_do_not_wait_for_diagnostic_locks() {
    if let Ok(mode) = std::env::var("SEGMENT_LOG_DEADLINE_CHILD") {
        if mode == "stderr" {
            let _locked = io::stderr().lock();
            let _deadline = Deadline::new(Duration::from_millis(20), || {});
            loop {
                thread::park();
            }
        }
        assert_eq!(mode, "callback");
        let _deadline = Deadline::new(Duration::from_millis(20), || {
            loop {
                thread::park();
            }
        });
        loop {
            thread::park();
        }
    }
    let _outer = Deadline::new(Duration::from_secs(10), || {});
    let name = concat!(
        module_path!(),
        "::deadlines_do_not_wait_for_diagnostic_locks"
    )
    .split_once("::")
    .unwrap()
    .1;
    for mode in ["stderr", "callback"] {
        let child = Command::new("sh")
            .args(["-c", "ulimit -c 0 && exec \"$@\"", "deadline-child"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env("SEGMENT_LOG_DEADLINE_CHILD", mode)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut child = Reap(child);
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert_eq!(status.signal(), Some(6), "{mode}");
                break;
            }
            assert!(Instant::now() < end, "deadline blocked on {mode}");
            thread::yield_now();
        }
    }
}
