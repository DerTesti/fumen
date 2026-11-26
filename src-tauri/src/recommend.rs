//! "What to play next" for osu!standard: charts to practise your weak skillsets on. Candidates come
//! from osu!'s ranked maps (not your library), each downloaded once and analysed with the standard
//! model (osu_skills) at a few mod variants, so the whole computation stays in the backend. Ported
//! from the author's recommendations approach. Needs the osu! login (like comparing).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::osu_auth::{api_get, Endpoints};
use crate::osu_skills::{self, SKILLS};
use crate::player::Progress;

/// pages of each search sort, candidates analysed, weak skillsets picked, suggestions per skillset
const PAGES: usize = 3;
const CANDIDATES: usize = 40;
const WEAK: usize = 4;
const PER_GROUP: usize = 6;
/// the mods a candidate is tried at (nomod, then common difficulty-raising ones)
const VARIANTS: [&[&str]; 4] = [&[], &["HD"], &["HR"], &["DT"]];

#[derive(Deserialize)]
pub struct SkillValue {
    pub id: String,
    pub value: f64,
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

struct Cand {
    beatmap_id: i64,
    set_id: i64,
    stars: f64,
    title: String,
    title_unicode: String,
    artist: String,
    artist_unicode: String,
    version: String,
    play_count: i64,
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

pub fn recommend(ep: &Endpoints, token: &str, skills: &[SkillValue], comfort_stars: f64, played: &HashSet<i64>, cancel: &AtomicBool, mut progress: impl FnMut(Progress)) -> Result<Vec<u8>> {
    // the weakest skillsets you have any footing in
    let mut weak: Vec<&SkillValue> = skills.iter().filter(|s| s.value > 0.0).collect();
    weak.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap());
    weak.truncate(WEAK);
    if weak.is_empty() {
        return Ok(b"[]".to_vec());
    }
    let target = if comfort_stars > 0.0 { comfort_stars } else { 5.0 };

    // 1. candidates from osu!'s ranked search (most played, most recent)
    let mut cands: Vec<Cand> = Vec::new();
    let mut seen: HashSet<i64> = HashSet::new();
    for sort in ["plays_desc", "ranked_desc"] {
        let mut cursor: Option<String> = None;
        for _ in 0..PAGES {
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
                let play_count = set["play_count"].as_i64().unwrap_or(0);
                for b in set["beatmaps"].as_array().into_iter().flatten() {
                    let id = b["id"].as_i64().unwrap_or(0);
                    let is_osu = b["mode_int"].as_i64() == Some(0) || b["mode"].as_str() == Some("osu");
                    if id == 0 || !is_osu || played.contains(&id) || !seen.insert(id) {
                        continue;
                    }
                    cands.push(Cand {
                        beatmap_id: id,
                        set_id,
                        stars: b["difficulty_rating"].as_f64().unwrap_or(0.0),
                        title: set["title"].as_str().unwrap_or_default().to_string(),
                        title_unicode: set["title_unicode"].as_str().unwrap_or_default().to_string(),
                        artist: set["artist"].as_str().unwrap_or_default().to_string(),
                        artist_unicode: set["artist_unicode"].as_str().unwrap_or_default().to_string(),
                        version: b["version"].as_str().unwrap_or_default().to_string(),
                        play_count,
                    });
                }
            }
            progress(Progress { stage: "scores", done: cands.len(), total: 0 });
            cursor = res["cursor_string"].as_str().map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
    }
    // keep those nearest your comfort stars, most played breaking ties
    cands.sort_by(|a, b| (a.stars - target).abs().partial_cmp(&(b.stars - target).abs()).unwrap().then(b.play_count.cmp(&a.play_count)));
    cands.truncate(CANDIDATES);

    let mut groups: Vec<Group> = weak
        .iter()
        .map(|s| {
            let label = SKILLS.iter().find(|(id, ..)| *id == s.id).map(|(_, l, ..)| *l).unwrap_or(s.id.as_str());
            // a band a little above where the skillset stands now
            Group { skill: s.id.clone(), label: label.to_string(), from: Some(s.value), lo: s.value * 1.01, hi: s.value * 1.12 + 2.0, picks: Vec::new() }
        })
        .collect();

    // 2. download and analyse each candidate at the mod variants
    let agent = crate::downloads::chart_agent();
    for (i, c) in cands.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::known("cancelled", "Cancelled"));
        }
        progress(Progress { stage: "charts", done: i + 1, total: cands.len() });
        let text = match crate::downloads::download_chart(&agent, c.beatmap_id, i % 4) {
            Ok(Some(t)) => t,
            _ => continue,
        };
        for variant in VARIANTS {
            let mods: Vec<String> = variant.iter().map(|m| m.to_string()).collect();
            let Some(demand) = osu_skills::analyze(&text, &mods) else { continue };
            for g in &mut groups {
                let Some(k) = SKILLS.iter().position(|(id, ..)| *id == g.skill) else { continue };
                let value = demand[k];
                if value >= g.lo && value <= g.hi {
                    // a clean pass here lifts the skillset a little above where it stands (your best
                    // count most, so one chart moves it only so far)
                    let from = g.from.unwrap_or(0.0);
                    let after = from + (value - from) * 0.3;
                    g.picks.push(Recommendation {
                        set_id: c.set_id,
                        beatmap_id: c.beatmap_id,
                        title: c.title.clone(),
                        title_unicode: c.title_unicode.clone(),
                        artist: c.artist.clone(),
                        artist_unicode: c.artist_unicode.clone(),
                        version: c.version.clone(),
                        value: (value * 100.0).round() / 100.0,
                        mods: mods.clone(),
                        rate: rate_of(variant),
                        after: (after * 100.0).round() / 100.0,
                    });
                }
            }
        }
    }
    // the best per skillset, one difficulty per set, nearest the top of the band first
    for g in &mut groups {
        g.picks.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap());
        let mut sets: HashSet<i64> = HashSet::new();
        g.picks.retain(|p| sets.insert(p.set_id));
        g.picks.truncate(PER_GROUP);
        g.lo = (g.lo * 100.0).round() / 100.0;
        g.hi = (g.hi * 100.0).round() / 100.0;
    }
    Ok(serde_json::to_vec(&groups)?)
}
