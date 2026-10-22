//! Isolated backend comparison; not a claim about native GUI event latency.
use chda_session::{Client, Session};
use chda_term::{Event, SessionOptions};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Server {
    child: std::process::Child,
    data: PathBuf,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.data);
        }
    }
}

fn wait(session: &Session, text: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let frame = session.frame();
        if (0..frame.size.rows).any(|y| frame.row_text(y).contains(text)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "missing {text}; visible: {:?}",
            (0..frame.size.rows)
                .map(|y| frame.row_text(y))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_micros(250));
    }
}
fn options(data: &std::path::Path, script: &str) -> SessionOptions {
    SessionOptions {
        command: Some(vec![
            "/usr/bin/env".into(),
            "-i".into(),
            format!("HOME={}", data.display()),
            "PATH=/usr/bin:/bin".into(),
            "/bin/sh".into(),
            "-c".into(),
            script.into(),
        ]),
        cwd: Some(data.to_owned()),
        scrollback: 1000,
        ..Default::default()
    }
}
fn create(
    client: Option<&Client>,
    pane: u64,
    opts: SessionOptions,
) -> (Session, mpsc::Receiver<Event>) {
    let (tx, rx) = mpsc::channel();
    let s = match client {
        Some(c) => Session::Remote(c.create(pane, opts, tx, || {}).unwrap()),
        None => Session::spawn(opts, tx, || {}).unwrap(),
    };
    (s, rx)
}
fn latency(client: Option<&Client>, data: &std::path::Path) -> serde_json::Value {
    let (s, _events) = create(
        client,
        1,
        options(
            data,
            "echo ready; while read x; do printf '<ack-%s>\\n' \"$x\"; done",
        ),
    );
    wait(&s, "ready");
    let mut times = Vec::new();
    for n in 0..60 {
        let start = Instant::now();
        s.write(format!("{n}\n").into_bytes());
        wait(&s, &format!("<ack-{n}>"));
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    s.terminate();
    drop(s);
    times.sort_by(f64::total_cmp);
    serde_json::json!({"samples":times.len(), "median_ms":times[times.len()/2], "p95_ms":times[times.len()*95/100]})
}
fn throughput(client: Option<&Client>, data: &std::path::Path) -> serde_json::Value {
    let start = Instant::now();
    let sessions: Vec<_>=(2..6).map(|n|create(client,n,options(data,&format!("i=0; while [ $i -lt 2000 ]; do echo output-$i; i=$((i+1)); done; echo done-{n}; read x")))).collect();
    for (n, (s, _)) in (2..6).zip(&sessions) {
        wait(s, &format!("done-{n}"));
    }
    let elapsed = start.elapsed().as_secs_f64();
    for (s, _) in sessions {
        s.terminate();
    }
    serde_json::json!({"panes":4,"lines":8000,"seconds":elapsed,"lines_per_second":8000.0/elapsed})
}
fn bytes(path: &std::path::Path) -> u64 {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            if e.path().is_dir() {
                bytes(&e.path())
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}
fn main() {
    let executable = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("pass the candidate chda-session executable"),
    );
    let data = std::env::temp_dir().join(format!("chda-measure-{}", std::process::id()));
    std::fs::create_dir(&data).unwrap();
    let child = Command::new(executable)
        .arg("serve")
        .arg(&data)
        .env_clear()
        .env("HOME", &data)
        .env("PATH", "/usr/bin:/bin")
        .env("CHDA_HOOK_DATA_DIR", &data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let server = Server {
        child,
        data: data.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let client = loop {
        if let Ok(c) = Client::connect(&data) {
            break c;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    eprintln!("local input");
    let local_latency = latency(None, &data);
    eprintln!("service input: local={local_latency}");
    let remote_latency = latency(Some(&client), &data);
    eprintln!("local output: service input={remote_latency}");
    let local_output = throughput(None, &data);
    eprintln!("service output: local={local_output}");
    let remote_output = throughput(Some(&client), &data);
    for pane in 6..10 {
        let (detached, _events) = create(
            Some(&client),
            pane,
            options(
                &data,
                "echo detach-ready; i=0; while :; do j=0; while [ $j -lt 20 ]; do echo detached-$i-$j; j=$((j+1)); done; i=$((i+1)); sleep .1; done",
            ),
        );
        wait(&detached, "detach-ready");
        drop(detached);
    }
    drop(client);
    let rss = || {
        Command::new("/bin/ps")
            .args(["-p", &server.child.id().to_string(), "-o", "rss="])
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
    };
    let mut samples = Vec::new();
    for seconds in [0, 15, 30] {
        if seconds > 0 {
            std::thread::sleep(Duration::from_secs(15));
        }
        samples
            .push(serde_json::json!({"seconds":seconds,"rss_kib":rss(),"disk_bytes":bytes(&data)}));
    }
    let report = serde_json::json!({"scope":"same-candidate debug local backend vs service; not native GUI latency or historical signed binary", "local_input":local_latency,"service_input":remote_latency,
        "local_output":local_output,"service_output":remote_output,"detached_continuous_output_panes":4,"detached_samples":samples});
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
