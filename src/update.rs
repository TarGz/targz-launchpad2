//! Self-update.
//!
//! Two modes, picked at build time:
//! - **GitHub releases** (builds made by the GitHub workflow): the build number and
//!   `owner/repo` are baked in. The app asks GitHub for the latest release
//!   (`build-<N>`), and if N is newer it downloads the zipped .app, swaps it in
//!   place and relaunches. Nothing needs to be installed on the Mac.
//! - **git** (builds made by hand with install.sh): fetch the repo this copy was
//!   built from (or `updateRepo` in ~/.devlauncher.json), pull, rebuild, relaunch.

use crate::core::{home, Report, Shared};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Where the sources were when this binary was built.
pub const SOURCE_DIR: &str = env!("CARGO_MANIFEST_DIR");
/// Set by the GitHub workflow: run number and `owner/repo`.
pub const BUILD: Option<&str> = option_env!("LAUNCHPAD_BUILD");
pub const GH_REPO: Option<&str> = option_env!("LAUNCHPAD_REPO");
pub const CHECK_EVERY: Duration = Duration::from_secs(30 * 60);
const APP_NAME: &str = "Targz Launchpad Rust.app";

pub fn build_number() -> Option<u32> {
    BUILD.and_then(|b| b.trim().parse().ok())
}

/// Run a command with a deadline; Ok(stdout) on success, Err(first error line).
fn run(cmd: &mut Command, timeout: Duration) -> Result<String, String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            let out = child.wait_with_output().map_err(|e| e.to_string())?;
            let first = |b: &[u8]| String::from_utf8_lossy(b).trim().lines().next().unwrap_or("").to_string();
            return if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                Err(Some(first(&out.stderr)).filter(|s| !s.is_empty()).unwrap_or_else(|| first(&out.stdout)))
            };
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            return Err("timed out".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn git(repo: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    run(Command::new("git").arg("-C").arg(repo).args(args).env("GIT_TERMINAL_PROMPT", "0"), timeout)
}

fn find_repo(configured: Option<String>) -> Option<PathBuf> {
    let start = configured.map(|p| crate::core::resolve(&p)).unwrap_or_else(|| PathBuf::from(SOURCE_DIR));
    git(&start, &["rev-parse", "--show-toplevel"], Duration::from_secs(5)).ok().map(PathBuf::from)
}

/// The .app this binary runs from, if any.
fn current_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

/// Where git is, as a percentage in the bands `skew_egui::pull_step` names:
/// under 30 it says "fetching origin", 30–79 "receiving objects NN%", 80 and
/// up "fast-forwarding". Git's own phases each count 0–100 of their own work,
/// so they are mapped onto those bands and the words agree with the work.
fn phase_pct(line: &str) -> Option<u8> {
    let head = line.split('%').next()?;
    let digits: String = head.chars().rev().take_while(char::is_ascii_digit).collect();
    let n: u32 = digits.chars().rev().collect::<String>().parse().ok()?;
    let n = n.min(100);
    Some(if line.contains("Receiving objects") {
        (30 + n * 49 / 100) as u8
    } else if line.contains("Resolving deltas") || line.contains("Updating files") {
        (80 + n * 19 / 100) as u8
    } else if line.contains("Enumerating") || line.contains("Counting") || line.contains("Compressing") {
        (5 + n * 24 / 100) as u8
    } else {
        return None;
    })
}

/// `git pull --ff-only` in one of the user's projects, reporting into
/// `Core::reports` as it runs — the line the project's card shows.
///
/// `--ff-only` so a pull can never open an editor or leave a half-made merge.
/// `--progress` because git only talks when it thinks it has a terminal.
/// `GIT_TERMINAL_PROMPT=0` so a repo that wants a password fails instead of
/// hanging, `LC_ALL=C` so the phases are the English ones `phase_pct` reads,
/// and a watchdog kills a pull still running after a minute. Git reports
/// progress with `\r`, not `\n`, so stderr is split on both.
pub fn pull_reporting(shared: &Shared, name: &str, dir: &Path, repaint: impl Fn()) {
    let say = |r: Report| {
        shared.lock().unwrap().reports.insert(name.to_string(), r);
        repaint();
    };
    if !dir.join(".git").exists() {
        return say(Report::Err("not a git repository".into()));
    }
    say(Report::Run(0));
    let spawned = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["pull", "--ff-only", "--progress"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return say(Report::Err(e.to_string())),
    };
    // the watchdog: a blocking read cannot time itself out, so the deadline
    // lives in a thread that kills the process by pid
    let pid = child.id() as i32;
    let done = Arc::new(AtomicBool::new(false));
    let killed = Arc::new(AtomicBool::new(false));
    {
        let (done, killed) = (done.clone(), killed.clone());
        std::thread::spawn(move || {
            for _ in 0..600 {
                if done.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            killed.store(true, Ordering::Relaxed);
            unsafe { libc::kill(pid, libc::SIGKILL) };
        });
    }
    let mut out = child.stdout.take();
    let drain = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let mut errs = String::new();
    let mut pct = 0u8;
    if let Some(e) = child.stderr.as_mut() {
        let mut buf = [0u8; 1024];
        let mut tail = String::new();
        loop {
            match e.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    errs.push_str(&chunk);
                    tail.push_str(&chunk);
                    let mut pieces: Vec<String> = tail.split(['\r', '\n']).map(str::to_string).collect();
                    tail = pieces.pop().unwrap_or_default();
                    for piece in pieces {
                        if let Some(p) = phase_pct(&piece) {
                            if p > pct {
                                pct = p;
                                say(Report::Run(pct));
                            }
                        }
                    }
                }
            }
        }
    }
    let status = child.wait();
    done.store(true, Ordering::Relaxed);
    let stdout = drain.join().unwrap_or_default();
    let ok = matches!(&status, Ok(s) if s.success());
    if ok {
        let stat = stdout
            .lines()
            .map(str::trim)
            .find(|l| l.contains("file changed") || l.contains("files changed"))
            .map(str::to_string);
        let msg = stat.unwrap_or_else(|| {
            if stdout.contains("Already up to date") {
                "Already up to date.".to_string()
            } else {
                stdout.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Pulled").to_string()
            }
        });
        say(Report::Ok(msg));
    } else if killed.load(Ordering::Relaxed) {
        say(Report::Err("git pull timed out after 60 s".into()));
    } else {
        let first = errs
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
            .or_else(|| errs.lines().map(str::trim).find(|l| !l.is_empty() && !l.contains('%')))
            .unwrap_or("git pull failed")
            .to_string();
        say(Report::Err(first));
    }
}

/// Latest GitHub release: (build number, zip download URL).
fn latest_release(repo: &str) -> Result<(u32, String), String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let body = run(
        Command::new("curl").args(["-fsSL", "--max-time", "30", "-H", "Accept: application/vnd.github+json", "-H", "User-Agent: targz-launchpad", &url]),
        Duration::from_secs(35),
    )?;
    let v: Value = serde_json::from_str(&body).map_err(|_| "unexpected answer from GitHub".to_string())?;
    let tag = v.get("tag_name").and_then(Value::as_str).unwrap_or("");
    let n: u32 = tag.trim_start_matches("build-").parse().map_err(|_| format!("latest release '{tag}' is not a build-<N> tag"))?;
    let zip = v
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().filter_map(|x| x.get("browser_download_url").and_then(Value::as_str)).find(|u| u.ends_with(".zip")))
        .ok_or("the latest release has no .zip")?;
    Ok((n, zip.to_string()))
}

/// Ask for news and record how far behind this copy is.
pub fn check(shared: &Shared) {
    let (behind, message, repo, download, latest, failed) = if let (Some(gh), Some(mine)) = (GH_REPO, build_number()) {
        match latest_release(gh) {
            Ok((n, url)) if n > mine => {
                (n - mine, format!("Build {n} available (you have {mine})"), None, Some(url), Some(format!("BUILD {n}")), false)
            }
            Ok((n, _)) => (0, format!("Up to date (build {n})"), None, None, None, false),
            Err(e) => (0, format!("Update check failed: {e}"), None, None, None, true),
        }
    } else {
        let configured = shared.lock().unwrap().cfg.get_str("updateRepo");
        let repo = find_repo(configured);
        let (behind, message) = match &repo {
            None => (0, "Not installed from a git clone or a GitHub build".to_string()),
            Some(r) => match git(r, &["fetch", "--quiet"], Duration::from_secs(30))
                .and_then(|_| git(r, &["rev-list", "--count", "HEAD..@{u}"], Duration::from_secs(5)))
            {
                Ok(n) => {
                    let n: u32 = n.parse().unwrap_or(0);
                    (n, if n == 0 { "Up to date".into() } else { format!("{n} new commit{} available", if n == 1 { "" } else { "s" }) })
                }
                Err(e) => (0, format!("Update check failed: {e}")),
            },
        };
        let failed = message.starts_with("Update check failed");
        let latest = (behind > 0).then(|| format!("{behind} COMMIT{}", if behind == 1 { "" } else { "S" }));
        (behind, message, repo, None, latest, failed)
    };
    let mut c = shared.lock().unwrap();
    if !c.update.busy {
        c.update.repo = repo;
        c.update.download = download;
        c.update.behind = behind;
        c.update.message = message;
        c.update.latest = latest;
        c.update.failed = failed;
    }
}

/// Install the update, start the new copy and quit. Running dev servers are
/// left alone; the new copy re-adopts the ones on a declared port.
pub fn apply(shared: &Shared) {
    let (repo, download, path_env) = {
        let mut c = shared.lock().unwrap();
        c.update.busy = true;
        c.update.message = "Updating…".into();
        (c.update.repo.clone(), c.update.download.clone(), c.path_env.clone())
    };
    let fail = |msg: String| {
        let mut c = shared.lock().unwrap();
        c.update.busy = false;
        c.update.message = msg.clone();
        c.toasts.push(msg);
    };
    let bundle = current_bundle();

    if let Some(url) = download {
        // GitHub release: download, unzip, swap the .app, relaunch.
        let Some(dest) = bundle else { return fail("Run the installed app to update it".into()) };
        let tmp = std::env::temp_dir().join(format!("launchpad-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        let zip = tmp.join("app.zip");
        shared.lock().unwrap().update.message = "Downloading…".into();
        if let Err(e) = run(Command::new("curl").args(["-fsSL", "--max-time", "300", "-o"]).arg(&zip).arg(&url), Duration::from_secs(310)) {
            return fail(format!("Download failed: {e}"));
        }
        if let Err(e) = run(Command::new("ditto").args(["-x", "-k"]).arg(&zip).arg(&tmp), Duration::from_secs(60)) {
            return fail(format!("Unzip failed: {e}"));
        }
        let new_app = tmp.join(APP_NAME);
        if !new_app.exists() {
            return fail(format!("The download has no {APP_NAME}"));
        }
        let script = format!(
            "sleep 1; rm -rf '{d}' && ditto '{n}' '{d}' && xattr -dr com.apple.quarantine '{d}' 2>/dev/null; open -n '{d}'; rm -rf '{t}'",
            d = dest.display(),
            n = new_app.display(),
            t = tmp.display()
        );
        let _ = Command::new("/bin/sh").args(["-c", &script]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        std::process::exit(0);
    }

    // git: pull, rebuild with install.sh (in place), relaunch.
    let Some(repo) = repo else { return fail("Not installed from a git clone".into()) };
    if let Err(e) = git(&repo, &["pull", "--ff-only"], Duration::from_secs(60)) {
        return fail(format!("Update failed: {e}"));
    }
    shared.lock().unwrap().update.message = "Building…".into();
    let mut build = Command::new("/bin/sh");
    build
        .arg(Path::new(SOURCE_DIR).join("install.sh"))
        .current_dir(SOURCE_DIR)
        .env("PATH", format!("{}:{path_env}", home().join(".cargo/bin").display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(b) = &bundle {
        build.env("DEST", b);
    }
    match build.status() {
        Ok(s) if s.success() => {}
        Ok(s) => return fail(format!("Build failed (exit {})", s.code().unwrap_or(-1))),
        Err(e) => return fail(format!("Build failed: {e}")),
    }
    let relaunch = match &bundle {
        Some(b) => format!("sleep 1; open -n '{}'", b.display()),
        None => format!("sleep 1; '{}' &", Path::new(SOURCE_DIR).join("target/release/targz-launchpad").display()),
    };
    let _ = Command::new("/bin/sh").args(["-c", &relaunch]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    std::process::exit(0);
}
