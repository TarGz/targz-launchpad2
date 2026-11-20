//! Self-update.
//!
//! Two modes, picked at build time:
//! - **GitHub releases** (builds made by the GitHub workflow): the build number and
//!   `owner/repo` are baked in. The app asks GitHub for the latest release
//!   (`build-<N>`), and if N is newer it downloads the zipped .app, swaps it in
//!   place and relaunches. Nothing needs to be installed on the Mac.
//! - **git** (builds made by hand with install.sh): fetch the repo this copy was
//!   built from (or `updateRepo` in ~/.devlauncher.json), pull, rebuild, relaunch.

use crate::core::{home, Shared};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

/// `git pull --ff-only` in one of the user's projects. --ff-only so a pull can
/// never open an editor or leave a half-made merge; the timeout and
/// GIT_TERMINAL_PROMPT=0 (set by `git`) keep an HTTPS credential prompt from
/// hanging the thread. Err carries the first error line.
pub fn pull(dir: &Path) -> Result<String, String> {
    if !dir.join(".git").exists() {
        return Err("Not a git repository".into());
    }
    let out = git(dir, &["pull", "--ff-only"], Duration::from_secs(30))?;
    Ok(out.lines().next().filter(|l| !l.is_empty()).unwrap_or("Pulled").to_string())
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
    let (behind, message, repo, download) = if let (Some(gh), Some(mine)) = (GH_REPO, build_number()) {
        match latest_release(gh) {
            Ok((n, url)) if n > mine => (n - mine, format!("Build {n} available (you have {mine})"), None, Some(url)),
            Ok((n, _)) => (0, format!("Up to date (build {n})"), None, None),
            Err(e) => (0, format!("Update check failed: {e}"), None, None),
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
        (behind, message, repo, None)
    };
    let mut c = shared.lock().unwrap();
    if !c.update.busy {
        c.update.repo = repo;
        c.update.download = download;
        c.update.behind = behind;
        c.update.message = message;
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
