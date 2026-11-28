//! osu!lazer on Linux: where it's installed and whether it runs.
//!
//! lazer comes as `osu.AppImage` (its own release, updated in place by Velopack, kept wherever its
//! user put it), as the Flatpak `sh.ppy.osu`, or unpacked by a distribution's package (the AUR's
//! `osu-lazer-bin`). The importer needs lazer's assemblies as files: a Flatpak's or a package's
//! folder has them; an AppImage is unpacked once per AppImage file (`--appimage-extract`, no FUSE
//! needed) into Fumen's cache, and that copy is what the importer's folder links to.

// plain std, so its tests run on every system; only Linux builds use it
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use crate::lazer_import::{find_assemblies, installed_at, Installed};

/// Folders an unpacked lazer is looked for in (a few levels down): Flatpak, distribution packages.
fn unpacked_roots(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local/share/flatpak/app/sh.ppy.osu/current/active/files"),
        PathBuf::from("/var/lib/flatpak/app/sh.ppy.osu/current/active/files"),
        PathBuf::from("/opt/osu-lazer"),
        PathBuf::from("/usr/lib/osu-lazer"),
        PathBuf::from("/usr/share/osu-lazer"),
        PathBuf::from("/opt/osu"),
    ]
}

/// Folders an `osu*.AppImage` is looked for in.
fn appimage_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("Applications"),
        home.join("AppImages"),
        home.join(".local/bin"),
        home.join("Downloads"),
        home.join("Desktop"),
        home.join("Games"),
        home.to_path_buf(),
        PathBuf::from("/opt"),
        PathBuf::from("/opt/osu-lazer"),
        PathBuf::from("/usr/local/bin"),
    ]
}

/// Whether a file name is lazer's AppImage ("osu.AppImage", "osu-lazer.AppImage", "osu!.AppImage" …).
pub fn is_lazer_appimage(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("osu") && n.ends_with(".appimage")
}

/// Fumen's cache for unpacked AppImages: `$XDG_CACHE_HOME/dev.fumen.app/lazer-appimage`.
fn appimage_cache(home: &Path) -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".cache"));
    base.join("dev.fumen.app").join("lazer-appimage")
}

/// one unpacking at a time
static UNPACKING: Mutex<()> = Mutex::new(());

/// lazer AppImages in the usual folders, and the ones desktop entries start (AppImageLauncher's,
/// Gear Lever's and the like put theirs anywhere).
fn appimages(home: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = appimage_dirs(home)
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|r| r.flatten())
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| is_lazer_appimage(&n.to_string_lossy())))
        .collect();
    let data = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    for entry in std::fs::read_dir(data.join("applications")).into_iter().flat_map(|r| r.flatten()) {
        if let Ok(text) = std::fs::read_to_string(entry.path()) {
            found.extend(text.lines().filter_map(desktop_exec).filter(|p| p.file_name().is_some_and(|n| is_lazer_appimage(&n.to_string_lossy()))));
        }
    }
    found.retain(|p| p.is_file());
    found
}

/// The program an `Exec=` line of a desktop entry starts (quoted or not).
pub fn desktop_exec(line: &str) -> Option<PathBuf> {
    let cmd = line.strip_prefix("Exec=")?.trim();
    let program = match cmd.strip_prefix('"') {
        Some(rest) => &rest[..rest.find('"')?],
        None => cmd.split_whitespace().next()?,
    };
    Some(PathBuf::from(program))
}

/// The newest of the AppImages found.
fn newest_appimage(home: &Path) -> Option<PathBuf> {
    appimages(home).into_iter().filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p))).max_by_key(|(t, _)| *t).map(|(_, p)| p)
}

/// The AppImage the running lazer was started from (AppImages tell their programs in `APPIMAGE`).
fn running_appimage() -> Option<PathBuf> {
    lazer_pids().iter().find_map(|pid| {
        let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
        env.split(|&b| b == 0).find_map(|v| v.strip_prefix(b"APPIMAGE=")).map(|v| PathBuf::from(String::from_utf8_lossy(v).into_owned()))
    })
}

/// An AppImage's assemblies, unpacked into the cache the first time it's seen (named by its size
/// and time, so an update in place is a new folder; older ones go).
fn unpack_appimage(image: &Path, home: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(image).ok()?;
    let stamp = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let cache = appimage_cache(home);
    let dir = cache.join(format!("{}-{stamp}", meta.len()));
    let _lock = UNPACKING.lock().unwrap_or_else(|e| e.into_inner());
    let root = dir.join("squashfs-root");
    if !dir.join(".fumen-ready").is_file() {
        if let Ok(entries) = std::fs::read_dir(&cache) {
            for e in entries.flatten() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
        std::fs::create_dir_all(&dir).ok()?;
        let ok = Command::new(image)
            .arg("--appimage-extract")
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        std::fs::write(dir.join(".fumen-ready"), image.to_string_lossy().as_bytes()).ok()?;
    }
    find_assemblies(&root, 3)
}

/// A picked AppImage, or the newest one in a picked folder (that has no unpacked lazer in it):
/// the AppImage and its assemblies, unpacked.
pub fn appimage_at(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let image = if path.is_file() {
        path.to_path_buf()
    } else {
        if find_assemblies(path, 3).is_some() {
            return None;
        }
        std::fs::read_dir(path)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.file_name().is_some_and(|n| is_lazer_appimage(&n.to_string_lossy())))
            .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
            .max_by_key(|(t, _)| *t)?
            .1
    };
    if !image.file_name().is_some_and(|n| is_lazer_appimage(&n.to_string_lossy())) {
        return None;
    }
    let dir = unpack_appimage(&image, &PathBuf::from(std::env::var_os("HOME")?))?;
    Some((image, dir))
}

/// lazer's assemblies and version: an unpacked install first, else the AppImage that runs, else
/// the newest AppImage found.
pub fn installed() -> Option<Installed> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if let Some(found) = unpacked_roots(&home).iter().find_map(|r| installed_at(r)) {
        return Some(found);
    }
    let image = running_appimage().filter(|p| p.is_file()).or_else(|| newest_appimage(&home))?;
    installed_at(&image)
}

/// Whether a process' name (`/proc/<pid>/comm`) is lazer's.
pub fn is_lazer_process(comm: &str) -> bool {
    comm.trim() == "osu!"
}

/// Whether lazer runs: a process named `osu!` (the AppImage's, the Flatpak's and the packages'
/// alike). A process list that can't be read counts as running.
pub fn running() -> bool {
    let Ok(procs) = std::fs::read_dir("/proc") else { return true };
    procs.flatten().filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit())).any(|e| {
        std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| is_lazer_process(&c))
    })
}

/// The `osu!` processes' ids.
fn lazer_pids() -> Vec<String> {
    let Ok(procs) = std::fs::read_dir("/proc") else { return Vec::new() };
    procs
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.bytes().all(|b| b.is_ascii_digit()))
        .filter(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| is_lazer_process(&c)))
        .collect()
}

/// Asks lazer to quit (SIGTERM, which it takes like its window's close button); how many
/// processes were asked.
pub fn close() -> usize {
    lazer_pids()
        .iter()
        .filter(|pid| std::process::Command::new("kill").arg("-TERM").arg(pid.as_str()).status().is_ok_and(|s| s.success()))
        .count()
}

/// How lazer is started: the Flatpak, the newest AppImage, or a package's `osu-lazer` / `osu!`.
pub fn launch_command() -> Option<std::process::Command> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if let Some(picked) = crate::lazer_import::picked() {
        if let Some(c) = picked_command(&picked) {
            return Some(c);
        }
    }
    let flatpak = [home.join(".local/share/flatpak/app/sh.ppy.osu"), PathBuf::from("/var/lib/flatpak/app/sh.ppy.osu")];
    if flatpak.iter().any(|d| d.is_dir()) {
        let mut c = std::process::Command::new("flatpak");
        c.args(["run", "sh.ppy.osu"]);
        return Some(c);
    }
    let image = newest_appimage(&home);
    if let Some(image) = image {
        return Some(std::process::Command::new(image));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|d| [d.join("osu-lazer"), d.join("osu!")])
        .find(|p| p.is_file())
        .map(std::process::Command::new)
}

/// How a picked lazer is started: its AppImage (picked, or the newest in the picked folder), else
/// the `osu!` next to its assemblies.
fn picked_command(picked: &Path) -> Option<std::process::Command> {
    let is_image = |p: &Path| p.is_file() && p.file_name().is_some_and(|n| is_lazer_appimage(&n.to_string_lossy()));
    if is_image(picked) {
        return Some(std::process::Command::new(picked));
    }
    let dir = if picked.is_file() { picked.parent()? } else { picked };
    if let Some(image) = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| is_image(p)) {
        return Some(std::process::Command::new(image));
    }
    let exe = find_assemblies(dir, 3)?.join("osu!");
    exe.is_file().then(|| std::process::Command::new(exe))
}

/// lazer's data folder: `$XDG_DATA_HOME/osu` (`~/.local/share/osu`), or the Flatpak's.
pub fn data_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let data = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let own = data.join("osu");
    let flatpak = home.join(".var/app/sh.ppy.osu/data/osu");
    let has = |d: &Path| d.join("client.realm").is_file() || d.join("storage.ini").is_file();
    Some(if !has(&own) && has(&flatpak) { flatpak } else { own })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appimage_names() {
        assert!(is_lazer_appimage("osu.AppImage"));
        assert!(is_lazer_appimage("osu-lazer.appimage"));
        assert!(is_lazer_appimage("osu!.AppImage"));
        assert!(!is_lazer_appimage("Fumen.AppImage"));
        assert!(!is_lazer_appimage("osu.zip"));
    }

    #[test]
    fn process_names() {
        assert!(is_lazer_process("osu!\n"));
        assert!(!is_lazer_process("osu-wine\n"));
        assert!(!is_lazer_process("fumen\n"));
    }

    #[test]
    fn desktop_entries() {
        assert_eq!(desktop_exec("Exec=/home/a/AppImages/osu.appimage %U"), Some(PathBuf::from("/home/a/AppImages/osu.appimage")));
        assert_eq!(desktop_exec("Exec=\"/home/a/My Apps/osu.AppImage\" %U"), Some(PathBuf::from("/home/a/My Apps/osu.AppImage")));
        assert_eq!(desktop_exec("Name=osu!"), None);
    }

    #[test]
    fn finds_assemblies_below() {
        let root = std::env::temp_dir().join(format!("fumen-lazer-find-{}", std::process::id()));
        let bin = root.join("squashfs-root").join("usr").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("osu.Game.dll"), b"").unwrap();
        std::fs::write(bin.join("sq.version"), b"<package><version>2026.1005.0-lazer</version></package>").unwrap();
        assert_eq!(find_assemblies(&root, 3).unwrap(), bin);
        let found = installed_at(&root).unwrap();
        assert_eq!(found.dir, bin);
        assert_eq!(found.version, "2026.1005.0-lazer");
        let _ = std::fs::remove_dir_all(&root);
    }
}
