//! "What to play next" for osu!standard: charts to practise your weak skillsets on.
//!
//! Candidates are library-first: the frontend picks the difficulties most like the ones you play
//! (lib/similar.ts, metadata only) and sends them here with their similarity; those are read
//! locally and reuse the pattern your own skills already worked out, so scoring them is nearly
//! free. A smaller set of ranked maps you don't have is mixed in for discovery. Each is scored
//! with the standard model (osu_skills) at a few mod variants, so the heavy computation stays in
//! the backend. Per weak skillset we rank by how well a chart sits a little above where you are,
//! weighted by how like your maps it is. Needs the osu! login (like comparing).

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::osu_auth::{api_get, Endpoints};
use crate::osu_skills::{self, SKILLS};
use crate::paths::Files;
use crate::player::Progress;

/// weak skillsets picked, suggestions per skillset, ranked maps mixed in for discovery
const WEAK: usize = 4;
const PER_GROUP: usize = 6;
const DISCOVERY_PAGES: usize = 3;
const DISCOVERY_MAX: usize = 36;
/// how like-your-maps a discovery pick counts as — below a typical library pick, so library leads
const DISCOVERY_SIM: f64 = 0.3;
/// how far above your hardest plays a pick (mods and all) may sit before it stops being practice —
/// the section aims "a little above your best", so this hangs off your peak stars, not your comfort
const STAR_HEADROOM: f64 = 0.5;
/// the mod combinations a candidate is tried at (nomod, then the popular difficulty-raising ones).
/// Order is the round-robin priority: picks rotate through the mods, so each skillset's list shows
/// the best of each (nomod, DT, HR, HD …) rather than six of whatever one mod fits best.
const VARIANTS: [&[&str]; 7] = [&[], &["DT"], &["HR"], &["HD"], &["HD", "DT"], &["HR", "DT"], &["HD", "HR"]];

#[derive(Deserialize)]
pub struct SkillValue {
    pub id: String,
    pub value: f64,
}

/// A library difficulty the frontend chose (most like the ones you play), to score locally.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibCandidate {
    /// the library difficulty id; the backend resolves its hash to read the `.osu` and reuse the
    /// pattern your own skills already cached under that hash
    pub map_id: String,
    pub beatmap_id: i64,
    pub set_id: i64,
    pub title: String,
    pub title_unicode: String,
    pub artist: String,
    pub artist_unicode: String,
    pub version: String,
    pub stars: f64,
    /// 0–1, how like your maps it is (lib/similar.ts)
    pub similarity: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Recommendation {
    set_id: i64,
    beatmap_id: i64,
    title: String,
    title_unicode: String,
    artist: String,
    artist_unicode: String,
    version: String,
    value: f64,
    mods: Vec<String>,
    rate: f64,
    after: f64,
    /// true when it's a difficulty from your library (the view opens it instead of offering to add)
    in_library: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Group {
    skill: String,
    label: String,
    from: Option<f64>,
    lo: f64,
    hi: f64,
    picks: Vec<Recommendation>,
}

/// A candidate to score: from your library (read locally, `notes_key` set) or a ranked map to
/// discover (downloaded, `notes_key` empty).
struct Cand {
    beatmap_id: i64,
    set_id: i64,
    stars: f64,
    title: String,
    title_unicode: String,
    artist: String,
    artist_unicode: String,
    version: String,
    similarity: f64,
    /// Some: the library chart's hash, read locally; None: a discovery map, downloaded by beatmap id
    hash: Option<String>,
    md5: Option<String>,
}

/// A library difficulty's chart hash, for reading it locally and keying its cached pattern.
fn lib_hash(db: &Mutex<Connection>, id: &str) -> Option<String> {
    db.lock()
        .unwrap_or_else(|e| e.into_inner())
        .query_row("SELECT hash FROM maps WHERE id = ?1", [id], |r| r.get::<_, Option<String>>(0))
        .optional()
        .ok()
        .flatten()
        .flatten()
}

fn rate_of(mods: &[&str]) -> f64 {
    if mods.contains(&"DT") || mods.contains(&"NC") {
        1.5
    } else if mods.contains(&"HT") || mods.contains(&"DC") {
        0.75
    } else {
        1.0
    }
}

/// Percent-encodes a cursor for the query string (osu! cursors can hold `+`, `/`, `=` …).
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn recommend(
    files: &Files,
    cache: &Path,
    db: &Mutex<Connection>,
    ep: &Endpoints,
    token: &str,
    skills: &[SkillValue],
    comfort_stars: f64,
    peak_stars: f64,
    library: &[LibCandidate],
    played: &HashSet<i64>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<Vec<u8>> {
    // the weakest skillsets you have any footing in
    let mut weak: Vec<&SkillValue> = skills.iter().filter(|s| s.value > 0.0).collect();
    weak.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap());
    weak.truncate(WEAK);
    if weak.is_empty() {
        return Ok(b"[]".to_vec());
    }
    let target = if comfort_stars > 0.0 { comfort_stars } else { 5.0 };
    // with the mods on, a pick harder than this is no practice — DT turns a 5★ map into ~9★. It
    // sits a little above your hardest plays (falls back to comfort + 2 when peak is unknown).
    let star_ceiling = if peak_stars > 0.0 { peak_stars } else { target + 2.0 } + STAR_HEADROOM;

    // 1. candidates: your library picks first (only those still in the library, by hash), then
    //    ranked maps you don't have for discovery
    let mut cands: Vec<Cand> = library
        .iter()
        .filter_map(|c| {
            Some(Cand {
                beatmap_id: c.beatmap_id,
                set_id: c.set_id,
                stars: c.stars,
                title: c.title.clone(),
                title_unicode: c.title_unicode.clone(),
                artist: c.artist.clone(),
                artist_unicode: c.artist_unicode.clone(),
                version: c.version.clone(),
                similarity: c.similarity.clamp(0.0, 1.0),
                hash: Some(lib_hash(db, &c.map_id)?),
                md5: None,
            })
        })
        .collect();
    let mut seen: HashSet<i64> = cands.iter().map(|c| c.beatmap_id).filter(|&id| id != 0).collect();
    discover(ep, token, target, played, &mut seen, &mut cands, cancel, &mut progress)?;

    let mut groups: Vec<Group> = weak
        .iter()
        .map(|s| {
            let label = SKILLS.iter().find(|(id, ..)| *id == s.id).map(|(_, l, ..)| *l).unwrap_or(s.id.as_str());
            // a band a little above where the skillset stands now; picks are ranked within it
            Group { skill: s.id.clone(), label: label.to_string(), from: Some(s.value), lo: s.value * 1.01, hi: s.value * 1.3 + 2.5, picks: Vec::new() }
        })
        .collect();
    // the point in each band we aim for, and the (score, mod bucket, recommendation) seen per skill
    let targets: Vec<f64> = weak.iter().map(|s| s.value * 1.1 + 1.0).collect();
    let mut scored: Vec<Vec<(f64, usize, Recommendation)>> = weak.iter().map(|_| Vec::new()).collect();

    // 2. read/download and score each candidate at the mod variants
    let agent = crate::downloads::chart_agent();
    let total = cands.len();
    for (i, c) in cands.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::known("cancelled", "Cancelled"));
        }
        progress(Progress { stage: "charts", done: i + 1, total });
        let bytes = match &c.hash {
            Some(h) => match files.read(h) {
                Some(b) => b,
                None => continue,
            },
            None => match crate::player::cached_chart(cache, c.beatmap_id, c.md5.as_deref(), || {
                crate::downloads::download_chart(&agent, c.beatmap_id, i % 4)
            }) {
                Ok(Some(t)) => t.into_bytes(),
                _ => continue,
            },
        };
        let read = |_: &str| Some(bytes.clone());
        // library charts score off the pattern your own skills already cached (keyed by hash)
        let hash = c.hash.clone().unwrap_or_else(|| {
            use md5::Digest;
            format!("{:x}", md5::Md5::digest(&bytes))
        });
        for (vi, variant) in VARIANTS.iter().enumerate() {
            let mods: Vec<String> = variant.iter().map(|m| m.to_string()).collect();
            let Some((demand, stars)) = osu_skills::rated_demand(&read, db, &hash, &mods) else { continue };
            // the mods, at their real difficulty, keep it within reach — else it isn't practice
            if stars > star_ceiling {
                continue;
            }
            for (gi, g) in groups.iter().enumerate() {
                let Some(k) = SKILLS.iter().position(|(id, ..)| *id == g.skill) else { continue };
                let value = demand[k];
                if value < g.lo || value > g.hi {
                    continue;
                }
                // mostly how well it sits at the band's aim; being like your maps only nudges it,
                // so an easier map worth a mod (DT, less like your maps) isn't buried under nomod picks
                let closeness = 1.0 / (1.0 + (value - targets[gi]).abs() / 2.5);
                let score = closeness * (0.5 + 0.5 * c.similarity);
                let from = g.from.unwrap_or(0.0);
                let after = from + (value - from) * 0.3;
                scored[gi].push((
                    score,
                    vi,
                    Recommendation {
                        set_id: c.set_id,
                        beatmap_id: c.beatmap_id,
                        title: c.title.clone(),
                        title_unicode: c.title_unicode.clone(),
                        artist: c.artist.clone(),
                        artist_unicode: c.artist_unicode.clone(),
                        version: c.version.clone(),
                        value: round2(value),
                        mods: mods.clone(),
                        rate: rate_of(variant),
                        after: round2(after),
                        in_library: c.hash.is_some(),
                    },
                ));
            }
        }
    }

    // per skillset, rotate through the mods (nomod, DT, HR, HD …) so the list shows the best of
    // each rather than six of one — a mod only appears when a playable map actually fits it. A set
    // shows up under one skillset only (weakest first), so the lists don't all repeat the same
    // versatile maps.
    let mut used: HashSet<i64> = HashSet::new();
    for (gi, g) in groups.iter_mut().enumerate() {
        let mut picks = std::mem::take(&mut scored[gi]);
        picks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let mut buckets: Vec<VecDeque<Recommendation>> = (0..VARIANTS.len()).map(|_| VecDeque::new()).collect();
        for (_, vi, r) in picks {
            buckets[vi].push_back(r);
        }
        loop {
            let before = g.picks.len();
            for b in buckets.iter_mut() {
                while let Some(r) = b.pop_front() {
                    if used.insert(r.set_id) {
                        g.picks.push(r);
                        break;
                    }
                }
                if g.picks.len() >= PER_GROUP {
                    break;
                }
            }
            if g.picks.len() >= PER_GROUP || g.picks.len() == before {
                break;
            }
        }
        g.lo = round2(g.lo);
        g.hi = round2(g.hi);
    }
    Ok(serde_json::to_vec(&groups)?)
}

/// Adds ranked maps you don't have, near your comfort stars, for discovery.
fn discover(
    ep: &Endpoints,
    token: &str,
    target: f64,
    played: &HashSet<i64>,
    seen: &mut HashSet<i64>,
    cands: &mut Vec<Cand>,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    let (lo_star, hi_star) = ((target * 0.45).max(2.0), target + 0.5);
    let mut found: Vec<Cand> = Vec::new();
    for sort in ["plays_desc", "ranked_desc"] {
        let mut cursor: Option<String> = None;
        for _ in 0..DISCOVERY_PAGES {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::known("cancelled", "Cancelled"));
            }
            let path = match &cursor {
                Some(c) => format!("beatmapsets/search?m=0&s=ranked&sort={sort}&cursor_string={}", enc(c)),
                None => format!("beatmapsets/search?m=0&s=ranked&sort={sort}"),
            };
            let res = api_get(ep, token, &path)?;
            for set in res["beatmapsets"].as_array().into_iter().flatten() {
                let set_id = set["id"].as_i64().unwrap_or(0);
                for b in set["beatmaps"].as_array().into_iter().flatten() {
                    let id = b["id"].as_i64().unwrap_or(0);
                    let is_osu = b["mode_int"].as_i64() == Some(0) || b["mode"].as_str() == Some("osu");
                    let stars = b["difficulty_rating"].as_f64().unwrap_or(0.0);
                    if id == 0 || !is_osu || played.contains(&id) || !seen.insert(id) || stars < lo_star || stars > hi_star {
                        continue;
                    }
                    found.push(Cand {
                        beatmap_id: id,
                        set_id,
                        stars,
                        title: set["title"].as_str().unwrap_or_default().to_string(),
                        title_unicode: set["title_unicode"].as_str().unwrap_or_default().to_string(),
                        artist: set["artist"].as_str().unwrap_or_default().to_string(),
                        artist_unicode: set["artist_unicode"].as_str().unwrap_or_default().to_string(),
                        version: b["version"].as_str().unwrap_or_default().to_string(),
                        similarity: DISCOVERY_SIM,
                        hash: None,
                        md5: b["checksum"].as_str().map(str::to_string),
                    });
                }
            }
            progress(Progress { stage: "scores", done: found.len(), total: 0 });
            cursor = res["cursor_string"].as_str().map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
    }
    // nearest comfort stars first, capped
    found.sort_by(|a, b| (a.stars - target).abs().partial_cmp(&(b.stars - target).abs()).unwrap());
    found.truncate(DISCOVERY_MAX);
    cands.extend(found);
    Ok(())
}
