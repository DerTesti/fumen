//! Fumen's online features: logging in with osu!, the cloud, comparing with other players, and
//! everything else that goes through fumen.dertesti.dev with something that says who you are.
//! They're off until the player says yes (the first start asks; Settings › About can change it),
//! and while they're off nothing here talks to the site (`ensure`).
//!
//! With them on, every start runs the block check (`check`): the install's random id (ping.rs), a
//! code for this computer, and the osu! login when there is one. The code is a SHA-256 hash of
//! Fumen's own key and the id the system gives itself (Windows' MachineGuid, macOS' hardware UUID,
//! Linux' machine-id): the id itself never leaves the computer, and the code can't be matched with
//! what other apps send. A full block from the site is noted in `block.json`, so it holds offline
//! and with the online features turned off again, until a check finds it lifted. Turning the
//! features off asks the site to delete this computer's code (`forget_machine`).
//!
//! What the site keeps of this: site/server.mjs (/api/check), and the privacy page.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

static ON: AtomicBool = AtomicBool::new(false);

/// Switches the online features on or off for this run (Settings keeps the choice).
pub fn set(on: bool) {
    ON.store(on, Ordering::SeqCst);
}

pub fn is_on() -> bool {
    // the tests talk to their own local servers
    cfg!(test) || ON.load(Ordering::SeqCst)
}

/// Ok while the online features are on; what to tell the player when they're off.
pub fn ensure() -> Result<()> {
    if is_on() {
        Ok(())
    } else {
        Err(Error::known("online-off", "This is one of Fumen's online features, and they're turned off. You can turn them on in Settings › About."))
    }
}

/// A block from fumen.dertesti.dev: "full" (Fumen shows only this) or "online" (no login, no
/// cloud). `message` is the site's, with the reason and where to turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub scope: String,
    pub message: String,
}

/// Where a full block is noted.
const NOTE: &str = "block.json";

/// The full block noted on this computer, if there is one.
pub fn noted(data_dir: &Path) -> Option<Block> {
    let b: Block = serde_json::from_slice(&std::fs::read(data_dir.join(NOTE)).ok()?).ok()?;
    (b.scope == "full").then_some(b)
}

fn note(data_dir: &Path, block: Option<&Block>) {
    let file = data_dir.join(NOTE);
    match block {
        Some(b) => {
            if let Ok(bytes) = serde_json::to_vec(b) {
                let _ = std::fs::create_dir_all(data_dir);
                let _ = std::fs::write(file, bytes);
            }
        }
        None => {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// The code for a computer with this system id.
fn code_of(id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"fumen-machine-v1\0");
    h.update(id.trim().to_ascii_lowercase().as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// This computer's code, when the system has an id.
pub fn machine_code() -> Option<String> {
    machine_id().map(|id| code_of(&id))
}

#[cfg(windows)]
fn machine_id() -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY};
    let key: Vec<u16> = "SOFTWARE\\Microsoft\\Cryptography\0".encode_utf16().collect();
    let name: Vec<u16> = "MachineGuid\0".encode_utf16().collect();
    let mut buf = [0u16; 128];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: the names end in NUL, the buffer and its size in bytes go together
    let r = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
    };
    if r != 0 {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let id = String::from_utf16_lossy(&buf[..end]).trim().to_string();
    (!id.is_empty()).then_some(id)
}

#[cfg(target_os = "macos")]
fn machine_id() -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/ioreg").args(["-rd1", "-c", "IOPlatformExpertDevice"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // "IOPlatformUUID" = "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
    let line = text.lines().find(|l| l.contains("\"IOPlatformUUID\""))?;
    let id = line.rsplit('"').nth(1)?.trim();
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(target_os = "linux")]
fn machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn machine_id() -> Option<String> {
    None
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(crate::downloads::USER_AGENT)
        .timeout_global(Some(Duration::from_secs(15)))
        .https_only(cfg!(not(test)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// Asks `site` whether this Fumen is blocked: with the install's random id, this computer's code
/// when `machine` (only with the online features on), and the osu! login while it's current. A
/// full block is noted on disk; anything else lifts the note.
pub fn check(data_dir: &Path, site: &str, machine: bool) -> Result<Option<Block>> {
    let install = crate::ping::install_id(data_dir).ok_or_else(|| Error::known("check-failed", "Fumen couldn't make its random id."))?;
    let mut body = serde_json::json!({ "install": install });
    if machine {
        if let Some(code) = machine_code() {
            body["machine"] = code.into();
        }
    }
    let agent = agent();
    let mut req = agent.post(&format!("{site}/api/check")).header("Content-Type", "application/json");
    if let Some(token) = crate::osu_auth::current_token(data_dir) {
        req = req.header("Authorization", &format!("Bearer {token}"));
    }
    let mut res = req.send(body.to_string()).map_err(|e| Error::known("check-failed", format!("fumen.dertesti.dev didn't answer ({e})")))?;
    if res.status() != 200 {
        return Err(Error::known("check-failed", format!("fumen.dertesti.dev answered {}", res.status().as_u16())));
    }
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let answer: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let block = answer.get("blocked").and_then(|b| serde_json::from_value::<Block>(b.clone()).ok());
    note(data_dir, block.as_ref().filter(|b| b.scope == "full"));
    Ok(block)
}

/// Asks `site` to delete this computer's code wherever it keeps it (the consent for it was taken
/// back). In the background; a failure is silent.
pub fn forget_machine(site: String) {
    std::thread::spawn(move || {
        let Some(code) = machine_code() else { return };
        let body = serde_json::json!({ "machine": code });
        let _ = agent().post(&format!("{site}/api/forget-machine")).header("Content-Type", "application/json").send(body.to_string());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_computer_code_hides_the_id() {
        let a = code_of("4c4c4544-0042-3510-8051-b4c04f384d32");
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        // the same computer, however the id is written; another computer, another code
        assert_eq!(a, code_of(" 4C4C4544-0042-3510-8051-B4C04F384D32\n"));
        assert_ne!(a, code_of("4c4c4544-0042-3510-8051-b4c04f384d33"));
        assert!(!a.contains("4c4c4544"));
        // this computer has one (Windows always gives one)
        #[cfg(windows)]
        assert!(machine_code().is_some());
        if let Some(code) = machine_code() {
            assert_eq!(code.len(), 64);
            assert_eq!(machine_code(), Some(code));
        }
    }

    #[test]
    fn a_full_block_is_noted_until_lifted() {
        let dir = std::env::temp_dir().join(format!("fumen-block-{}", std::process::id()));
        let full = Block { scope: "full".into(), message: "Fumen is blocked on this computer. Reason: cheating".into() };
        assert_eq!(noted(&dir), None);
        note(&dir, Some(&full));
        assert_eq!(noted(&dir), Some(full));
        // an online block isn't one to show at the start
        note(&dir, Some(&Block { scope: "online".into(), message: "x".into() }));
        assert_eq!(noted(&dir), None);
        note(&dir, None);
        assert_eq!(noted(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
