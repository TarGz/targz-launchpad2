//! Projects, config and dev-server processes. Rules follow app/MECHANICS.md.

use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_GIT_DIR: &str = "~/Documents/GIT";
pub const DEFAULT_AUTO_PORT: u16 = 2850;
/// The launcher's own repo is never listed.
/// The launcher's own repos (old and new) are never listed.
pub const SELF_REPOS: [&str; 2] = ["_targz-launchpad", "targz-launchpad2"];
/// Health-check budgets per project type (server startProject, MECHANICS.md).
pub const WAIT_NODE: Duration = Duration::from_millis(5000);
pub const WAIT_JEKYLL: Duration = Duration::from_millis(15000);
pub const WAIT_STATIC: Duration = Duration::from_millis(5000);
pub const WAIT_POLL: Duration = Duration::from_millis(300);
/// Terminal order: macOS has no "default terminal" setting to ask for, so the
/// first installed wins unless ~/.devlauncher.json names one ("terminal").
/// Terminal.app is the fallback: it lives in /System/Applications, so it never
/// shows up in the scan below, but it is always there.
const TERMINALS: [&str; 2] = ["Terminass", "iTerm"];
/// Editor order, as stored in the server's constant pool at 0x102d1db2b.
const EDITORS: [&str; 9] = [
    "Zed", "Cursor", "Visual Studio Code", "VSCodium", "Windsurf", "Sublime Text", "Nova", "WebStorm", "BBEdit",
];

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn resolve(p: &str) -> PathBuf {
    match p.strip_prefix('~') {
        Some(rest) => home().join(rest.trim_start_matches('/')),
        None => PathBuf::from(p),
    }
}

// ── Text rules (the original's regexes, by hand) ────────────────────────

/// First `--port[= ]<digits>` in a script.
pub fn find_port(s: &str) -> Option<u16> {
    for (i, _) in s.match_indices("--port") {
        let rest = &s[i + 6..];
        let mut chars = rest.chars();
        if matches!(chars.next(), Some('=') | Some(' ')) {
            let digits: String = rest[1..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(p) = digits.parse::<u16>() {
                return Some(p);
            }
        }
    }
    None
}

/// Replace `--port[= ]<digits>` (all, or only the first) with `with`.
fn replace_port(s: &str, with: &str, all: bool) -> String {
    let mut out = String::new();
    let mut i = 0;
    let mut done = false;
    while let Some(off) = s[i..].find("--port") {
        let at = i + off;
        let after = &s[at + 6..];
        let sep_ok = matches!(after.chars().next(), Some('=') | Some(' '));
        let digits = if sep_ok { after[1..].chars().take_while(|c| c.is_ascii_digit()).count() } else { 0 };
        if sep_ok && digits > 0 && !done {
            out.push_str(&s[i..at]);
            out.push_str(with);
            i = at + 7 + digits;
            if !all {
                done = true;
            }
        } else {
            out.push_str(&s[i..at + 6]);
            i = at + 6;
        }
    }
    out.push_str(&s[i..]);
    out
}

/// Remove `\s*<flag>` plus an optional `=value` right after it.
fn strip_flag(s: &str, flag: &str, with_value: bool) -> String {
    let mut out = s.to_string();
    while let Some(at) = out.find(flag) {
        let mut start = at;
        while start > 0 && out.as_bytes()[start - 1].is_ascii_whitespace() {
            start -= 1;
        }
        let mut end = at + flag.len();
        if with_value && out[end..].starts_with('=') {
            end += out[end..].find(char::is_whitespace).unwrap_or(out.len() - end);
        }
        out.replace_range(start..end, "");
    }
    out
}

/// `^key\s*:\s*value` in a YAML file.
fn yml_value(yml: &str, key: &str) -> Option<String> {
    for line in yml.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(v) = rest.strip_prefix(':') {
                let v = v.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn yml_port(yml: &str) -> Option<u16> {
    yml_value(yml, "port").and_then(|v| v.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok())
}

// ── Config (~/.devlauncher.json, shared with the original) ───────────────

pub struct Config {
    path: PathBuf,
    raw: Map<String, Value>,
    pub git_dir_label: String,
    pub git_dir: PathBuf,
    pub auto_port: u16,
    pub folders: Vec<String>,
    pub items: HashMap<String, Vec<String>>,
}

impl Config {
    pub fn load() -> Config {
        let path = home().join(".devlauncher.json");
        let raw: Map<String, Value> = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let git_dir_label = raw.get("gitDir").and_then(Value::as_str).unwrap_or(DEFAULT_GIT_DIR).to_string();
        let auto_port = raw.get("autoPortStart").and_then(Value::as_u64).map(|p| p as u16).unwrap_or(DEFAULT_AUTO_PORT);
        let strings = |v: &Value| -> Vec<String> {
            v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
        };
        let (folders, mut items) = if let Some(f) = raw.get("folders") {
            let folders = strings(f);
            let mut items = HashMap::new();
            if let Some(obj) = raw.get("folderItems").and_then(Value::as_object) {
                for (k, v) in obj {
                    items.insert(k.clone(), strings(v));
                }
            }
            (folders, items)
        } else {
            // First run on an old config: seed Favorites from legacy favorites.
            let fav = raw.get("favorites").map(strings).unwrap_or_default();
            (vec!["Favorites".to_string()], HashMap::from([("Favorites".to_string(), fav)]))
        };
        for f in &folders {
            items.entry(f.clone()).or_default();
        }
        items.retain(|k, _| folders.contains(k));
        let mut cfg = Config { path, raw, git_dir: resolve(&git_dir_label), git_dir_label, auto_port, folders, items };
        cfg.raw.remove("hidden");
        cfg.raw.remove("favorites");
        cfg.save();
        cfg
    }

    pub fn save(&mut self) {
        let r = &mut self.raw;
        r.entry("gitDir").or_insert(Value::from(DEFAULT_GIT_DIR));
        r.entry("port").or_insert(Value::from(2800));
        r.insert("autoPortStart".into(), Value::from(self.auto_port));
        r.insert("folders".into(), Value::from(self.folders.clone()));
        let mut obj = Map::new();
        for f in &self.folders {
            obj.insert(f.clone(), Value::from(self.items.get(f).cloned().unwrap_or_default()));
        }
        r.insert("folderItems".into(), Value::Object(obj));
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(r.clone())) {
            let _ = fs::write(&self.path, text + "\n");
        }
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        self.raw.get(key).and_then(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    fn next_port(&mut self) -> u16 {
        let p = self.auto_port;
        self.auto_port = self.auto_port.saturating_add(1);
        self.save();
        p
    }

    pub fn folders_for(&self, name: &str) -> Vec<String> {
        self.folders.iter().filter(|f| self.items.get(*f).is_some_and(|i| i.iter().any(|n| n == name))).cloned().collect()
    }

    pub fn create_folder(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Folder name required".into());
        }
        if self.folders.iter().any(|f| f == name) {
            return Err("Folder already exists".into());
        }
        self.folders.push(name.to_string());
        self.items.insert(name.to_string(), vec![]);
        self.save();
        Ok(())
    }

    pub fn rename_folder(&mut self, from: &str, to: &str) -> Result<(), String> {
        let to = to.trim();
        let Some(idx) = self.folders.iter().position(|f| f == from) else { return Err("Folder not found".into()) };
        if to.is_empty() {
            return Err("Folder name required".into());
        }
        if to != from && self.folders.iter().any(|f| f == to) {
            return Err("Folder already exists".into());
        }
        self.folders[idx] = to.to_string();
        let list = self.items.remove(from).unwrap_or_default();
        self.items.insert(to.to_string(), list);
        self.save();
        Ok(())
    }

    /// Deletes the folder only; projects are never touched.
    pub fn delete_folder(&mut self, name: &str) {
        self.folders.retain(|f| f != name);
        self.items.remove(name);
        self.save();
    }

    pub fn assign(&mut self, folder: &str, project: &str, add: bool) {
        if let Some(list) = self.items.get_mut(folder) {
            let at = list.iter().position(|n| n == project);
            match (add, at) {
                (true, None) => list.push(project.to_string()),
                (false, Some(i)) => {
                    list.remove(i);
                }
                _ => {}
            }
            self.save();
        }
    }
}

// ── Projects ────────────────────────────────────────────────────────────

/// A folder on the rack. There is no `kind` and no `description`: the card
/// dropped both — a runtime label that cannot be wrong cannot be read either,
/// and forty cards of prose is a page you scroll past. `spawn` works out how
/// to start a project by reading its own files when it starts it, so neither
/// was ever the thing that launched anything.
#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    pub default_port: Option<u16>,
    pub group: Option<String>,
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn dev_script(pkg: &Value) -> Option<String> {
    let s = pkg.get("scripts")?;
    s.get("dev").and_then(Value::as_str).or_else(|| s.get("start").and_then(Value::as_str)).map(String::from)
}

/// A directory is a project when it is one of the three things this launcher
/// knows how to start. WHICH of the three is not kept: it is answered again
/// from the same files at the moment of launching, and a copy of an answer is
/// a copy that can be wrong.
fn scan_single(name: &str, dir: &Path) -> Option<Project> {
    let mut found = false;
    let mut default_port = None;
    let pkg_path = dir.join("package.json");
    if pkg_path.exists() {
        if let Some(pkg) = read_json(&pkg_path) {
            found = true;
            default_port = dev_script(&pkg).as_deref().and_then(find_port);
        }
    }
    let jekyll = dir.join("_config.yml");
    if !found && jekyll.exists() {
        found = true;
        if let Ok(yml) = fs::read_to_string(&jekyll) {
            default_port = yml_port(&yml);
        }
        if default_port.is_none() {
            default_port = fs::read_to_string(dir.join("startlocaldev.sh")).ok().as_deref().and_then(find_port);
        }
    }
    if !found && dir.join("index.html").exists() {
        found = true;
    }
    found.then(|| Project { name: name.to_string(), path: dir.to_path_buf(), default_port, group: None })
}

fn listable(entry: &fs::DirEntry) -> Option<String> {
    let name = entry.file_name().to_string_lossy().to_string();
    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
    (is_dir && name != "node_modules" && !name.starts_with('.')).then_some(name)
}

/// Git repos at the top level; other folders are opened one level deep.
pub fn scan(git_dir: &Path) -> Vec<Project> {
    let mut out = vec![];
    let Ok(entries) = fs::read_dir(git_dir) else { return out };
    for e in entries.flatten() {
        let Some(name) = listable(&e) else { continue };
        if SELF_REPOS.contains(&name.as_str()) {
            continue;
        }
        let dir = e.path();
        if dir.join(".git").exists() {
            out.extend(scan_single(&name, &dir));
        } else if let Ok(subs) = fs::read_dir(&dir) {
            for s in subs.flatten() {
                let Some(sub) = listable(&s) else { continue };
                if let Some(mut p) = scan_single(&format!("{name}/{sub}"), &s.path()) {
                    p.group = Some(name.clone());
                    out.push(p);
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Port declared by a folder's scripts, used to adopt servers already up.
fn detect_port(dir: &Path) -> Option<u16> {
    if let Some(pkg) = read_json(&dir.join("package.json")) {
        if let Some(p) = dev_script(&pkg).as_deref().and_then(find_port) {
            return Some(p);
        }
    }
    if dir.join("_config.yml").exists() {
        if let Some(p) = fs::read_to_string(dir.join("_config.yml")).ok().as_deref().and_then(yml_port) {
            return Some(p);
        }
        return fs::read_to_string(dir.join("startlocaldev.sh")).ok().as_deref().and_then(find_port);
    }
    None
}

// ── Processes ───────────────────────────────────────────────────────────

/// Pids listening on a TCP port (listeners only, never ourselves).
pub fn listeners(port: u16) -> Vec<i32> {
    let me = std::process::id() as i32;
    Command::new("lsof")
        .args(["-ti", &format!("tcp:{port}"), "-sTCP:LISTEN"])
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.trim().parse().ok()).filter(|p| *p != me).collect())
        .unwrap_or_default()
}

fn kill_port(port: u16) {
    for pid in listeners(port) {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Any HTTP answer counts as ready (IPv4 or IPv6 localhost).
fn answers(port: u16) -> bool {
    for addr in [format!("127.0.0.1:{port}"), format!("[::1]:{port}")] {
        let Ok(sa) = addr.parse::<SocketAddr>() else { continue };
        if let Ok(mut s) = TcpStream::connect_timeout(&sa, Duration::from_millis(500)) {
            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
            if s.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n").is_ok() {
                let mut b = [0u8; 1];
                if matches!(s.read(&mut b), Ok(n) if n > 0) {
                    return true;
                }
            }
        }
    }
    false
}

fn wait_for_server(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if answers(port) {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(WAIT_POLL);
    }
}

/// PATH a Terminal would have: Finder-launched apps get a bare one, and
/// `npm` usually lives in Homebrew or nvm.
pub fn login_path() -> String {
    let base = std::env::var("PATH").unwrap_or_default();
    let fallback = format!("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:{}:{base}", home().join(".cargo/bin").display());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let Ok(mut child) = Command::new(shell)
        .args(["-ilc", "printf '\\n__PATH__%s\\n' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return fallback;
    };
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(4) {
        if let Ok(Some(_)) = child.try_wait() {
            let mut out = String::new();
            if let Some(mut so) = child.stdout.take() {
                let _ = so.read_to_string(&mut out);
            }
            if let Some(p) = out.lines().rev().find_map(|l| l.strip_prefix("__PATH__")) {
                if !p.is_empty() {
                    return format!("{p}:{fallback}");
                }
            }
            return fallback;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    fallback
}

pub struct Run {
    pub pid: i32,
    pub port: u16,
    pub child: Option<Child>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// NO `Stopping`. Stopping is synchronous — the call that asks for it has
/// already done it by the time the next frame draws — so a card could never
/// be caught in that state and nothing ever put it there.
pub enum Busy {
    Starting,
    Restarting,
}

/// What a module last reported — the line the card shows under its keys.
/// `Run` is a git pull in flight, 0–100; `Ok` and `Err` carry git's own words
/// or the launcher's, verbatim. An `Err` stays until it is answered.
#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    Run(u8),
    Ok(String),
    /// a pull failed — RETRY pulls again
    Err(String),
    /// the launcher failed — RETRY launches again
    Fail(String),
}

#[derive(Default, Clone)]
pub struct UpdateState {
    pub repo: Option<PathBuf>,
    pub behind: u32,
    pub busy: bool,
    /// a check started from the version badge is running
    pub checking: bool,
    /// zip of a newer GitHub release, when there is one
    pub download: Option<String>,
    pub message: String,
    /// what the newer version calls itself, for the UPDATE key's legend
    pub latest: Option<String>,
    /// the last check could not reach the server
    pub failed: bool,
}

pub struct Core {
    pub cfg: Config,
    pub projects: Vec<Project>,
    pub running: HashMap<String, Run>,
    pub busy: HashMap<String, Busy>,
    /// per project: the last thing it said, by project name
    pub reports: HashMap<String, Report>,
    pub toasts: Vec<String>,
    pub path_env: String,
    pub update: UpdateState,
}

pub type Shared = Arc<Mutex<Core>>;

impl Core {
    pub fn new() -> Core {
        let cfg = Config::load();
        let projects = scan(&cfg.git_dir);
        let mut core = Core {
            cfg,
            projects,
            running: HashMap::new(),
            busy: HashMap::new(),
            reports: HashMap::new(),
            toasts: vec![],
            path_env: format!("/opt/homebrew/bin:/usr/local/bin:{}", std::env::var("PATH").unwrap_or_default()),
            update: UpdateState::default(),
        };
        core.adopt_running();
        core
    }

    pub fn rescan(&mut self) {
        self.projects = scan(&self.cfg.git_dir);
    }

    /// Servers already listening on a project's declared port count as running.
    fn adopt_running(&mut self) {
        let mut claimed = vec![];
        for p in &self.projects {
            let Some(port) = detect_port(&p.path) else { continue };
            if claimed.contains(&port) {
                continue;
            }
            if let Some(pid) = listeners(port).first() {
                self.running.insert(p.name.clone(), Run { pid: *pid, port, child: None });
                claimed.push(port);
            }
        }
    }

    pub fn port_of(&self, p: &Project) -> Option<u16> {
        self.running.get(&p.name).map(|r| r.port).or(p.default_port)
    }

    fn port_taken(&self, port: u16, name: &str) -> bool {
        self.running.iter().any(|(k, v)| v.port == port && k != name)
    }

    /// Picks the port and spawns the command. Returns (port, health timeout).
    fn spawn(&mut self, name: &str) -> Result<(u16, Duration), String> {
        let dir = self.cfg.git_dir.join(name);
        let pkg_path = dir.join("package.json");
        let jekyll = dir.join("_config.yml");
        let base_path = self.path_env.clone();

        if pkg_path.exists() {
            let pkg = read_json(&pkg_path).ok_or("package.json is not valid JSON")?;
            if let Some(dev) = dev_script(&pkg) {
                let mut port = match find_port(&dev) {
                    Some(p) if p > 0 => p,
                    _ => self.cfg.next_port(),
                };
                if self.port_taken(port, name) {
                    port = self.cfg.next_port();
                }
                kill_port(port);
                let mut cmd = strip_flag(&dev, "--open", true);
                cmd = replace_port(&cmd, &format!("--port={port}"), true);
                if cmd.contains("live-server") && !cmd.contains("--no-browser") {
                    cmd.push_str(" --no-browser");
                }
                let path = format!("{}:{base_path}", dir.join("node_modules/.bin").display());
                let child = Command::new("sh")
                    .args(["-c", &cmd])
                    .current_dir(&dir)
                    .env("PATH", path)
                    .env("PORT", port.to_string())
                    .env("PORT_PROJECT", port.to_string())
                    .env("BROWSER", "none")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .process_group(0)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                self.register(name, child, port);
                return Ok((port, WAIT_NODE));
            }
        }

        if jekyll.exists() && !pkg_path.exists() {
            let script = dir.join("startlocaldev.sh");
            let found = fs::read_to_string(&script)
                .ok()
                .as_deref()
                .and_then(find_port)
                .or_else(|| fs::read_to_string(&jekyll).ok().as_deref().and_then(yml_port));
            let mut port = match found {
                Some(p) => p,
                None => self.cfg.next_port(),
            };
            if self.port_taken(port, name) {
                port = self.cfg.next_port();
            }
            kill_port(port);
            let cmd = if let Ok(sh) = fs::read_to_string(&script) {
                let mut sh = replace_port(&sh, &format!("--port {port}"), false);
                while let Some(at) = sh.find("--livereload-port") {
                    let mut start = at;
                    while start > 0 && sh.as_bytes()[start - 1].is_ascii_whitespace() {
                        start -= 1;
                    }
                    let rest = &sh[at + 17..];
                    let ws = rest.len() - rest.trim_start().len();
                    let digits = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).count();
                    sh.replace_range(start..at + 17 + ws + digits, "");
                }
                sh = strip_flag(&sh, "--livereload", false);
                sh = strip_flag(&sh, "--open", false);
                sh.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).collect::<Vec<_>>().join(" && ")
            } else if dir.join("Gemfile").exists() {
                format!("bundle exec jekyll serve --port {port}")
            } else {
                format!("jekyll serve --port {port}")
            };
            let h = home();
            let mut path = base_path;
            for extra in ["/opt/homebrew/opt/ruby/bin".into(), "/usr/local/opt/ruby/bin".into(), h.join(".rbenv/shims"), h.join(".rvm/bin")] {
                let extra: PathBuf = extra;
                if extra.exists() {
                    path = format!("{}:{path}", extra.display());
                }
            }
            let child = Command::new("/bin/sh")
                .args(["-c", &cmd])
                .current_dir(&dir)
                .env("PATH", path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .map_err(|e| e.to_string())?;
            self.register(name, child, port);
            return Ok((port, WAIT_JEKYLL));
        }

        if dir.join("index.html").exists() {
            let mut port = self.cfg.next_port();
            if self.port_taken(port, name) {
                port = self.cfg.next_port();
            }
            let child = Command::new("npx")
                .args(["live-server", ".", &format!("--port={port}"), "--no-browser", "--quiet"])
                .current_dir(&dir)
                .env("PATH", base_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .map_err(|e| e.to_string())?;
            self.register(name, child, port);
            return Ok((port, WAIT_STATIC));
        }
        Err("No runnable config found".into())
    }

    fn register(&mut self, name: &str, child: Child, port: u16) {
        let stale: Vec<String> = self.running.iter().filter(|(k, v)| v.port == port && *k != name).map(|(k, _)| k.clone()).collect();
        for k in stale {
            self.running.remove(&k);
        }
        let pid = child.id() as i32;
        self.running.insert(name.to_string(), Run { pid, port, child: Some(child) });
    }

    pub fn stop(&mut self, name: &str) {
        let Some(run) = self.running.remove(name) else { return };
        unsafe {
            if libc::kill(-run.pid, libc::SIGTERM) != 0 {
                libc::kill(run.pid, libc::SIGTERM);
            }
        }
        kill_port(run.port);
        if let Some(mut child) = run.child {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    pub fn stop_all(&mut self) {
        let names: Vec<String> = self.running.keys().cloned().collect();
        for n in names {
            self.stop(&n);
        }
    }

    /// Drop servers that died on their own; re-adopt one that re-forked.
    pub fn check_alive(&mut self) {
        let names: Vec<String> = self.running.keys().filter(|n| !self.busy.contains_key(*n)).cloned().collect();
        for n in names {
            let Some(run) = self.running.get_mut(&n) else { continue };
            let gone = match run.child.as_mut() {
                Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                None => !alive(run.pid),
            };
            if gone {
                match listeners(run.port).first() {
                    Some(pid) => {
                        run.pid = *pid;
                        run.child = None;
                    }
                    None => {
                        self.running.remove(&n);
                    }
                }
            }
        }
    }

    pub fn running_list(&self) -> Vec<(String, u16)> {
        let mut v: Vec<(String, u16)> = self.running.iter().map(|(k, r)| (k.clone(), r.port)).collect();
        v.sort();
        v
    }
}

// ── Actions run off the UI thread ──────────────────────────────────────

pub fn start_async(shared: &Shared, name: String, on_done: impl Fn() + Send + 'static) {
    let shared = shared.clone();
    {
        let mut c = shared.lock().unwrap();
        if c.busy.contains_key(&name) {
            return;
        }
        c.busy.insert(name.clone(), Busy::Starting);
    }
    std::thread::spawn(move || {
        start_blocking(&shared, &name);
        shared.lock().unwrap().busy.remove(&name);
        on_done();
    });
}

fn start_blocking(shared: &Shared, name: &str) {
    let spawned = {
        let mut c = shared.lock().unwrap();
        if c.running.contains_key(name) {
            return;
        }
        c.spawn(name)
    };
    match spawned {
        Ok((port, timeout)) => {
            let answered = wait_for_server(port, timeout);
            let mut c = shared.lock().unwrap();
            match answered {
                // a started server needs no line: the port strip is the receipt
                true => {
                    c.reports.remove(name);
                }
                false => {
                    c.reports.insert(name.to_string(), Report::Fail(format!("no answer on :{port} yet")));
                }
            }
        }
        Err(e) => {
            shared.lock().unwrap().reports.insert(name.to_string(), Report::Fail(e));
        }
    }
}

pub fn restart_async(shared: &Shared, name: String, on_done: impl Fn() + Send + 'static) {
    let shared = shared.clone();
    {
        let mut c = shared.lock().unwrap();
        if c.busy.contains_key(&name) {
            return;
        }
        c.busy.insert(name.clone(), Busy::Restarting);
        c.stop(&name);
    }
    std::thread::spawn(move || {
        start_blocking(&shared, &name);
        shared.lock().unwrap().busy.remove(&name);
        on_done();
    });
}

// ── Opening things ─────────────────────────────────────────────────────

pub fn open_url(port: u16) {
    let _ = Command::new("open").arg(format!("http://localhost:{port}")).spawn();
}

pub fn open_finder(dir: &Path) {
    let _ = Command::new("open").arg(dir).spawn();
}

/// First installed editor from a fixed list, unless the config names one.
pub fn open_editor(cfg: &Config, dir: &Path) -> Result<String, String> {
    let editor = cfg.get_str("editor").or_else(|| {
        let dirs = [PathBuf::from("/Applications"), home().join("Applications")];
        EDITORS.iter().find(|app| dirs.iter().any(|d| d.join(format!("{app}.app")).exists())).map(|s| s.to_string())
    });
    let editor = editor.ok_or("No code editor found in /Applications")?;
    Command::new("open").arg("-a").arg(&editor).arg(dir).spawn().map_err(|e| e.to_string())?;
    Ok(editor)
}

/// Open a project folder in a terminal. `open -a <term> <dir>` only: osascript
/// would make the signed binary ask for Automation permission on first run.
pub fn open_terminal(cfg: &Config, dir: &Path) -> Result<String, String> {
    let term = cfg
        .get_str("terminal")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let dirs = [PathBuf::from("/Applications"), home().join("Applications")];
            TERMINALS.iter().find(|app| dirs.iter().any(|d| d.join(format!("{app}.app")).exists())).map(|s| s.to_string())
        })
        .unwrap_or_else(|| "Terminal".to_string());
    Command::new("open").arg("-a").arg(&term).arg(dir).spawn().map_err(|e| e.to_string())?;
    Ok(term)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ports() {
        assert_eq!(find_port("exec node server.js --port 5173"), Some(5173));
        assert_eq!(find_port("vite --port=3000 --open"), Some(3000));
        assert_eq!(find_port("vite"), None);
        assert_eq!(replace_port("vite --port 3000 --x", "--port=2851", true), "vite --port=2851 --x");
        assert_eq!(strip_flag("vite --open --port 1", "--open", true), "vite --port 1");
        assert_eq!(strip_flag("vite --open=/a --host", "--open", true), "vite --host");
    }
}
