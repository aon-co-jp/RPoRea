//! RPoRea のアプリ(`whoami` 例)を RPoem の `open-runo-appserver`(第二のTomcat)の `Supervisor` の
//! 子プロセスとして動かす: 起動 → HTTP で応答 → 強制終了 → 自動再起動 → 停止。127.0.0.1 の固定ポートのみ。

use open_runo_appserver::{Health, RestartPolicy, RuntimeProfile, Stack, Supervisor};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn whoami_path() -> std::path::PathBuf {
    // テストの実行ファイルから親をたどり、`examples/whoami`(cargo test が作る)を探す
    let exe = std::env::current_exe().expect("current_exe");
    let name = if cfg!(windows) { "whoami.exe" } else { "whoami" };
    for a in exe.ancestors().skip(1) {
        let p = a.join("examples").join(name);
        if p.exists() {
            return p;
        }
    }
    exe.with_file_name(name)
}

fn http_get(port: u16, path: &str) -> Option<String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").ok()?;
    let mut buf = String::new();
    s.read_to_string(&mut buf).ok()?;
    Some(buf)
}

fn wait_until<F: FnMut() -> bool>(mut f: F, secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn rporea_app_runs_under_supervisor_and_restarts_after_a_crash() {
    let bin = whoami_path();
    assert!(bin.exists(), "whoami example not built at {bin:?} (cargo test builds examples)");
    let port: u16 = 19771;
    let mut profile = RuntimeProfile::template(Stack::Custom("rporea".into()), "rporea-whoami", ".", port);
    profile.command = bin.to_string_lossy().into_owned();
    profile.args = vec![port.to_string()];
    profile.env.insert("RPOREA_TRUSTED_PROXIES".into(), "127.0.0.1".into());
    let mut sup = Supervisor::new(
        profile,
        RestartPolicy {
            base_backoff: Duration::from_millis(200),
            max_backoff: Duration::from_secs(2),
            max_rapid_failures: 20,
            healthy_after: Duration::from_secs(3),
        },
    );

    assert_eq!(sup.tick(), Health::Starting);
    assert!(
        wait_until(|| http_get(port, "/").is_some_and(|r| r.starts_with("HTTP/1.1 200")), 15),
        "RPoRea app did not come up"
    );
    let body = http_get(port, "/").unwrap();
    assert!(body.contains("xff_leaked=false"), "{body}");

    // 強制終了 → Supervisor が検知して再起動する
    let pid = sup.pid().expect("pid");
    std::process::Command::new(if cfg!(windows) { "taskkill" } else { "kill" })
        .args(if cfg!(windows) { vec!["/F".to_string(), "/PID".to_string(), pid.to_string()] } else { vec!["-9".to_string(), pid.to_string()] })
        .status()
        .expect("kill");
    let (mut crashed, mut restarted) = (false, false);
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end && !(crashed && restarted) {
        match sup.tick() {
            Health::Crashed(_) => crashed = true,
            Health::Starting if crashed => restarted = true,
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(crashed && restarted, "crashed={crashed} restarted={restarted}");
    assert!(
        wait_until(|| http_get(port, "/").is_some_and(|r| r.starts_with("HTTP/1.1 200")), 15),
        "RPoRea app did not come back after the restart"
    );

    sup.stop();
    assert!(wait_until(|| http_get(port, "/").is_none(), 5), "still answering after stop()");
}
