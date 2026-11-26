//! osu!standard skills, worked out the way osu!mania's are: all the compute is here in the backend,
//! the heavy per-chart part cached, and the frontend only reads the result (lib/osu-standard.ts,
//! views/skills.ts).
//!
//! osu!std has no per-play skillset calculator the way osu!mania has MinaCalc (msd.rs). This is a
//! model of its own, ported from an earlier app of the author's: rosu-pp's aim/speed difficulty
//! (through `pp::osu_difficulty`, the same fork pp uses) together with a pattern pass over the
//! `.osu` itself — wide-angle and snap jumps, flow streams, rhythm islands, slider paths, reading
//! density and sustained 15-second load. Every eligible pass demonstrates a skill to the degree the
//! chart demands it and you played it clean; a skillset's value is your best demonstrations,
//! best-weighted. The scale is provisional, not osu!'s pp or any site's percentile.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::pp;

/// Bump when the model changes, so cached rows from an older model are ignored.
const CACHE_VERSION: i64 = 4;

/// The eleven skillsets, in order: id, label, group, description. Labels/descriptions are the
/// model's own (the view shows them as given).
pub const SKILLS: [(&str, &str, &str, &str); 11] = [
    ("wide-angle-aim", "Wide angle aim", "Aim", "Weite Richtungsverläufe zwischen den tatsächlichen Objekt- und Slider-Endpositionen."),
    ("precision", "Precision", "Aim", "Aim-Schwierigkeit unter Berücksichtigung der effektiven Kreisgröße."),
    ("accuracy", "Accuracy", "Accuracy", "Erzielte Head-Trefferverteilung relativ zum tatsächlichen 300er-Zeitfenster."),
    ("snap-aim", "Snap aim", "Aim", "Räumlich anspruchsvolle Jumps außerhalb schneller Stream-Folgen."),
    ("finger-control", "Finger control", "Tapping", "Wechsel aktiver Rhythmusinseln; Pausen und Slider-Haltezeiten zählen nicht als Komplexität."),
    ("aim-stamina", "Aim stamina", "Aim", "Aim-Belastung in 15-Sekunden-Fenstern; Ruhezeiten senken die Belastung."),
    ("speed", "Speed", "Tapping", "rosu-Speed-Schwierigkeit mit Nachweisen aus zusammenhängenden schnellen Circle-Folgen."),
    ("density-reading", "Density reading", "Reading", "Typische hohe Objektdichte im effektiven AR-Fenster, einschließlich Überlagerung."),
    ("tapping-stamina", "Tapping stamina", "Tapping", "Schnelles Tapping über zusammenhängende Abschnitte und 15-Sekunden-Fenster."),
    ("slider-control", "Slider control", "Tech", "Slider-Geschwindigkeit, Kurven, Wiederholungen und tatsächlicher Slider-Anteil."),
    ("flow-aim", "Flow aim", "Aim", "Räumlich anspruchsvolle, rhythmisch stabile Streams mit kontinuierlicher Bewegung."),
];
const N_SKILLS: usize = 11;
const ACCURACY: usize = 2;
const FINGER_CONTROL: usize = 4;

pub const MODEL_VERSION: &str = "pattern-rosu-v2";

/// Provisional shared scale, logistic in log-demand: 2.5 component-equivalent units = 50, 5 = 80.
pub fn rating(d: f64) -> f64 {
    let d = d.max(0.0);
    100.0 * d * d / (d * d + 2.5 * 2.5)
}

/// A chart's rated demand (0–100) per skillset and its star rating at some mods, from the pattern
/// cache (recommend.rs): library charts reuse the pattern worked out for your own skills, so they
/// cost nothing to score. The stars are with the mods on, so a recommender can keep a map whose
/// mods (DT especially) blow its difficulty past your comfort out of the list. `read(hash)` yields
/// the `.osu` bytes when the pattern isn't cached yet.
pub fn rated_demand(read: &impl Fn(&str) -> Option<Vec<u8>>, db: &Mutex<Connection>, hash: &str, mods: &[String]) -> Option<([f64; N_SKILLS], f64)> {
    let p = pattern_for(read, db, hash, mods, rate_of(mods))?;
    let mut demand = [0.0f64; N_SKILLS];
    for k in 0..N_SKILLS {
        demand[k] = rating(p.raw_demand[k]);
    }
    Some((demand, p.stars))
}

fn finite(v: f64, f: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        f
    }
}
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}
fn clamp01(v: f64) -> f64 {
    clamp(if v.is_finite() { v } else { 0.0 }, 0.0, 1.0)
}
fn round2(v: f64) -> f64 {
    (finite(v, 0.0) * 100.0).round() / 100.0
}
fn mean(a: &[f64]) -> f64 {
    if a.is_empty() {
        0.0
    } else {
        a.iter().sum::<f64>() / a.len() as f64
    }
}
fn quantile(a: &[f64], q: f64) -> f64 {
    if a.is_empty() {
        return 0.0;
    }
    let mut v = a.to_vec();
    v.sort_by(|x, y| x.partial_cmp(y).unwrap());
    v[((v.len() - 1) as f64 * q).floor() as usize]
}
/// A skillset's value from your best-per-chart worths: 70% your single best, 30% a best-5 blend.
fn skill_value(worths: &[f64]) -> f64 {
    if worths.is_empty() {
        return 0.0;
    }
    let mut v = worths.to_vec();
    v.sort_by(|a, b| b.partial_cmp(a).unwrap());
    v.truncate(5);
    let weight: f64 = (0..v.len()).map(|i| 0.6f64.powi(i as i32)).sum();
    let confirmed = if weight == 0.0 { 0.0 } else { v.iter().enumerate().map(|(i, w)| w * 0.6f64.powi(i as i32)).sum::<f64>() / weight };
    0.7 * v[0] + 0.3 * confirmed
}

/// Overall from the known skill values: the strongest weighted most (0.8ⁿ), zeros ignored.
fn overall_of(values: &[f64]) -> f64 {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| *x > 0.0).collect();
    v.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let weight: f64 = (0..v.len()).map(|i| 0.8f64.powi(i as i32)).sum();
    if weight == 0.0 {
        0.0
    } else {
        v.iter().enumerate().map(|(i, x)| x * 0.8f64.powi(i as i32)).sum::<f64>() / weight
    }
}

/// The weighted top: the highest 20 values, weighted 0.85ⁿ.
fn weighted_top(values: &[f64]) -> f64 {
    let mut a: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    a.sort_by(|x, y| y.partial_cmp(x).unwrap());
    a.truncate(20);
    let mut w = 0.0;
    let mut s = 0.0;
    for (i, v) in a.iter().enumerate() {
        let f = 0.85f64.powi(i as i32);
        w += f;
        s += v * f;
    }
    if w == 0.0 {
        0.0
    } else {
        s / w
    }
}

/* ---------- slider geometry ---------- */

#[derive(Clone, Copy)]
struct Pt {
    x: f64,
    y: f64,
}
fn pt(x: f64, y: f64) -> Pt {
    Pt { x: clamp(finite(x, 0.0), -1e6, 1e6), y: clamp(finite(y, 0.0), -1e6, 1e6) }
}
fn dist(a: Pt, b: Pt) -> f64 {
    (b.x - a.x).hypot(b.y - a.y)
}
fn same(a: Pt, b: Pt) -> bool {
    dist(a, b) < 1e-8
}
fn mix(a: Pt, b: Pt, t: f64) -> Pt {
    pt(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn bezier(anchors: &[Pt]) -> Vec<Pt> {
    if anchors.len() < 3 {
        return anchors.to_vec();
    }
    let steps = (anchors.len() * 4).clamp(16, 96);
    let mut result = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let mut work = anchors.to_vec();
        let mut remaining = work.len() - 1;
        while remaining > 0 {
            for j in 0..remaining {
                work[j] = mix(work[j], work[j + 1], i as f64 / steps as f64);
            }
            remaining -= 1;
        }
        result.push(work[0]);
    }
    result
}

fn bezier_segments(anchors: &[Pt]) -> Vec<Pt> {
    let mut result = Vec::new();
    let mut segment: Vec<Pt> = Vec::new();
    for i in 0..anchors.len() {
        segment.push(anchors[i]);
        if i == anchors.len() - 1 || same(anchors[i], anchors[i + 1]) {
            result.extend(bezier(&segment));
            segment.clear();
        }
    }
    result
}

fn perfect(anchors: &[Pt]) -> Vec<Pt> {
    if anchors.len() != 3 {
        return bezier_segments(anchors);
    }
    let (a, b, c) = (anchors[0], anchors[1], anchors[2]);
    let (bx, by, cx, cy) = (b.x - a.x, b.y - a.y, c.x - a.x, c.y - a.y);
    let det = 2.0 * (bx * cy - by * cx);
    if det.abs() < 1e-7 {
        return anchors.to_vec();
    }
    let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
    let center = pt(a.x + (cy * b2 - by * c2) / det, a.y + (bx * c2 - cx * b2) / det);
    let radius = dist(a, center);
    if !radius.is_finite() || radius < 1e-7 || radius > 1e6 {
        return anchors.to_vec();
    }
    let start = (a.y - center.y).atan2(a.x - center.x);
    let angle = |p: Pt| -> f64 {
        let raw = ((p.y - center.y).atan2(p.x - center.x) - start).rem_euclid(2.0 * std::f64::consts::PI);
        raw
    };
    let (middle, end) = (angle(b), angle(c));
    let sweep = if middle <= end { end } else { end - 2.0 * std::f64::consts::PI };
    let steps = ((sweep.abs() * 48.0).ceil() as i64).clamp(16, 256);
    (0..=steps)
        .map(|i| {
            let t = start + sweep * i as f64 / steps as f64;
            pt(center.x + radius * t.cos(), center.y + radius * t.sin())
        })
        .collect()
}

fn catmull(anchors: &[Pt]) -> Vec<Pt> {
    let mut result = Vec::new();
    for i in 0..anchors.len().saturating_sub(1) {
        let p0 = anchors[i.saturating_sub(1)];
        let p1 = anchors[i];
        let p2 = anchors[i + 1];
        let p3 = if i + 2 < anchors.len() { anchors[i + 2] } else { pt(2.0 * p2.x - p1.x, 2.0 * p2.y - p1.y) };
        for j in 0..=16 {
            let t = j as f64 / 16.0;
            let (t2, t3) = (t * t, t * t * t);
            let coord = |a: f64, b: f64, c: f64, d: f64| 0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3);
            result.push(pt(coord(p0.x, p1.x, p2.x, p3.x), coord(p0.y, p1.y, p2.y, p3.y)));
        }
    }
    result
}

struct Geom {
    end: Pt,
    travel: f64,
    curvature: f64,
}

fn slider_geometry(o: &Obj) -> Geom {
    let start = pt(o.x, o.y);
    let controls: Vec<Pt> = o.control_points.iter().take(63).map(|&(x, y)| pt(x, y)).collect();
    let mut anchors = vec![start];
    anchors.extend(controls);
    let mut path = match o.curve_type {
        b'L' => anchors.clone(),
        b'P' => perfect(&anchors),
        b'C' => catmull(&anchors),
        _ => bezier_segments(&anchors),
    };
    path.dedup_by(|a, b| same(*a, *b));
    let declared = clamp(finite(o.length, 0.0), 0.0, 1e7);
    let repeats = (finite(o.repeats as f64, 1.0).floor() as i64).clamp(1, 10000);
    let mut span = vec![start];
    let mut remaining = declared;
    let mut actual = 0.0;
    let mut i = 1;
    while i < path.len() && remaining > 1e-8 {
        let len = dist(path[i - 1], path[i]);
        if len >= 1e-8 {
            let consumed = len.min(remaining);
            span.push(mix(path[i - 1], path[i], consumed / len));
            actual += consumed;
            remaining -= consumed;
        }
        i += 1;
    }
    if remaining > 1e-8 && span.len() > 1 {
        let last = span[span.len() - 1];
        let before = span[span.len() - 2];
        let len = dist(before, last);
        let extended = pt(last.x + (last.x - before.x) / len * remaining, last.y + (last.y - before.y) / len * remaining);
        actual += dist(last, extended);
        span.push(extended);
    }
    let mut curvature = 0.0;
    for i in 2..span.len() {
        let (a, b, c) = (span[i - 2], span[i - 1], span[i]);
        let (ax, ay, bx, by) = (b.x - a.x, b.y - a.y, c.x - b.x, c.y - b.y);
        if ax.hypot(ay) > 1e-8 && bx.hypot(by) > 1e-8 {
            curvature += (ax * by - ay * bx).atan2(ax * bx + ay * by).abs();
        }
    }
    Geom { end: if repeats % 2 == 0 { start } else { span[span.len() - 1] }, travel: actual * repeats as f64, curvature }
}

/* ---------- .osu parsing ---------- */

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Circle,
    Slider,
    Spinner,
}

struct Obj {
    x: f64,
    y: f64,
    time: f64,
    kind: Kind,
    end_time: f64,
    repeats: i64,
    length: f64,
    curve_type: u8,
    control_points: Vec<(f64, f64)>,
}

struct Timing {
    time: f64,
    beat_length: f64,
    inherited: bool,
}

struct Map {
    cs: f64,
    ar: f64,
    od: f64,
    slider_multiplier: f64,
    timing: Vec<Timing>,
    objects: Vec<Obj>,
}

fn num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<f64>().ok()
    }
}

fn parse_beatmap(text: &str) -> Map {
    let mut m = Map { cs: 4.0, ar: 5.0, od: 5.0, slider_multiplier: 1.4, timing: Vec::new(), objects: Vec::new() };
    let mut section = String::new();
    let mut explicit_ar = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_string();
            continue;
        }
        if section == "Difficulty" || section == "General" {
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim();
                if let Some(val) = num(v) {
                    match key {
                        "CircleSize" => m.cs = val,
                        "ApproachRate" => {
                            m.ar = val;
                            explicit_ar = true;
                        }
                        "OverallDifficulty" => m.od = val,
                        "SliderMultiplier" => m.slider_multiplier = val,
                        _ => {}
                    }
                }
            }
        } else if section == "TimingPoints" {
            let p: Vec<&str> = line.split(',').collect();
            if let (Some(time), Some(beat)) = (p.first().and_then(|s| num(s)), p.get(1).and_then(|s| num(s))) {
                let inherited = p.get(6).and_then(|s| num(s)).map(|v| v == 0.0).unwrap_or(false);
                m.timing.push(Timing { time, beat_length: beat, inherited });
            }
        } else if section == "HitObjects" {
            let p: Vec<&str> = line.split(',').collect();
            if p.len() < 5 || !p[..4].iter().all(|x| num(x).is_some()) {
                continue;
            }
            let ty = num(p[3]).unwrap_or(0.0) as i64;
            if ty & 1 == 0 && ty & 2 == 0 && ty & 8 == 0 {
                continue;
            }
            let time = num(p[2]).unwrap_or(0.0);
            let kind = if ty & 8 != 0 {
                Kind::Spinner
            } else if ty & 2 != 0 {
                Kind::Slider
            } else {
                Kind::Circle
            };
            let end_time = if ty & 8 != 0 { time.max(p.get(5).and_then(|s| num(s)).unwrap_or(0.0)) } else { time };
            let (repeats, length, curve_type, control_points) = if ty & 2 != 0 {
                let curve = p.get(5).copied().unwrap_or("L");
                let ct = curve.bytes().next().unwrap_or(b'L');
                let cps: Vec<(f64, f64)> = curve
                    .split('|')
                    .skip(1)
                    .filter_map(|seg| {
                        let mut it = seg.split(':');
                        match (it.next().and_then(num), it.next().and_then(num)) {
                            (Some(x), Some(y)) => Some((x, y)),
                            _ => None,
                        }
                    })
                    .collect();
                (clamp(finite(p.get(6).and_then(|s| num(s)).unwrap_or(1.0), 1.0), 1.0, 100.0) as i64, clamp(finite(p.get(7).and_then(|s| num(s)).unwrap_or(0.0), 0.0), 0.0, 100000.0), ct, cps)
            } else {
                (0, 0.0, 0u8, Vec::new())
            };
            m.objects.push(Obj { x: num(p[0]).unwrap_or(0.0), y: num(p[1]).unwrap_or(0.0), time, kind, end_time, repeats, length, curve_type, control_points });
        }
    }
    if !explicit_ar {
        m.ar = m.od;
    }
    m.objects.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    m.timing.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    m
}

/* ---------- mods and effective difficulty ---------- */

struct Settings {
    rate: f64,
    hidden: bool,
    cs: f64,
    approach: f64,
    diameter: f64,
    great_hit_window: f64,
}

fn rate_of(mods: &[String]) -> f64 {
    let has = |n: &str| mods.iter().any(|m| m == n);
    clamp(if has("DT") || has("NC") { 1.5 } else if has("HT") || has("DC") { 0.75 } else { 1.0 }, 0.1, 3.0)
}

fn effective_settings(map: &Map, mods: &[String]) -> Settings {
    let has = |n: &str| mods.iter().any(|m| m == n);
    let (hr, ez) = (has("HR"), has("EZ"));
    let scale = |base: f64, hr_f: f64| base * if hr { hr_f } else if ez { 0.5 } else { 1.0 };
    let cs = clamp(finite(scale(map.cs, 1.3), map.cs), 0.0, 10.0);
    let ar = clamp(finite(scale(map.ar, 1.4), map.ar), 0.0, 10.0);
    let od = clamp(finite(scale(map.od, 1.4), map.od), 0.0, 10.0);
    let rate = rate_of(mods);
    Settings {
        rate,
        hidden: has("HD"),
        cs,
        approach: (if ar < 5.0 { 1800.0 - 120.0 * ar } else { 1200.0 - 150.0 * (ar - 5.0) }) / rate,
        diameter: 2.0 * (54.4 - 4.48 * cs),
        great_hit_window: (80.0 - 6.0 * od) / rate,
    }
}

/* ---------- pattern analysis ---------- */

struct Ev {
    index: usize,
    time: f64,
    gap: f64,
    beat_length: f64,
    active: bool,
    circle_pair: bool,
    fast: bool,
    distance: f64,
    velocity: f64,
    internal_angle: f64,
    jump: f64,
    wide: f64,
    sustained_tap: f64,
}

fn rolling_load(events: &[Ev], get: impl Fn(&Ev) -> f64, seconds: f64) -> f64 {
    let mut bins: BTreeMap<i64, f64> = BTreeMap::new();
    for e in events {
        let v = get(e);
        if !(v > 0.0) {
            continue;
        }
        let t = (e.time / 1000.0).floor() as i64;
        *bins.entry(t).or_insert(0.0) += v * (0.25f64).min(e.gap / 1000.0);
    }
    let keys: Vec<i64> = bins.keys().copied().collect();
    let mut start = 0usize;
    let mut sum = 0.0f64;
    let mut max = 0.0f64;
    for end in 0..keys.len() {
        sum += bins[&keys[end]];
        while (keys[end] - keys[start]) as f64 >= seconds {
            sum -= bins[&keys[start]];
            start += 1;
        }
        max = max.max(sum / seconds);
    }
    max
}

/// What a chart demands, before a play's accuracy scales it. `accuracy`'s raw demand is left at 0
/// here (a per-play judgement thing, filled per play in `skills`). Cached to the database.
#[derive(Clone, Serialize, Deserialize)]
struct Pattern {
    raw_demand: [f64; N_SKILLS],
    support: [f64; N_SKILLS],
    coverage: [f64; N_SKILLS],
    object_count: usize,
    duration: f64,
    great_hit_window: f64,
    stars: f64,
    ar: f64,
}

struct AnObj {
    x: f64,
    y: f64,
    time: f64,
    end_time: f64,
    kind: Kind,
    beat_length: f64,
    end: Pt,
    geom: Option<Geom>,
}

fn analyze_beatmap(map: &Map, mods: &[String], diff: Option<&pp::OsuDiff>) -> Pattern {
    let s = effective_settings(map, mods);
    let (rate, diameter, approach) = (s.rate, s.diameter, s.approach);
    let mut beat_length = 500.0;
    let mut sv = 1.0;
    let mut ti = 0;
    let objects: Vec<AnObj> = map
        .objects
        .iter()
        .map(|o| {
            while ti < map.timing.len() && map.timing[ti].time <= o.time {
                let p = &map.timing[ti];
                if !p.inherited && p.beat_length > 0.0 {
                    beat_length = p.beat_length;
                    sv = 1.0;
                } else if p.inherited && p.beat_length < 0.0 {
                    sv = clamp(-100.0 / p.beat_length, 0.1, 10.0);
                }
                ti += 1;
            }
            let geom = if o.kind == Kind::Slider { Some(slider_geometry(o)) } else { None };
            let duration = if o.kind == Kind::Slider { o.length / (100.0 * map.slider_multiplier.max(0.1) * sv) * beat_length * o.repeats as f64 } else { 0.0 };
            let end = geom.as_ref().map(|g| g.end).unwrap_or(Pt { x: o.x, y: o.y });
            AnObj {
                x: o.x,
                y: o.y,
                time: o.time / rate,
                end_time: (if o.kind == Kind::Slider { o.time + duration } else { o.end_time }) / rate,
                kind: o.kind,
                beat_length: beat_length / rate,
                end,
                geom,
            }
        })
        .collect();

    let mut events: Vec<Ev> = Vec::new();
    let mut sliders: Vec<f64> = Vec::new();
    let mut densities: Vec<f64> = Vec::new();
    let mut overlaps: Vec<f64> = Vec::new();
    let mut left = 0usize;
    for i in 0..objects.len() {
        let o = &objects[i];
        if o.kind == Kind::Spinner {
            continue;
        }
        // a reading window at least this wide, so a high AR (short approach) doesn't hide how fast
        // dense notes actually come — otherwise a top player on high-AR/DT maps reads as low-density
        // ponytail: 700ms floor is a calibration knob
        while left < i && o.time - objects[left].time > approach.max(700.0) {
            left += 1;
        }
        let from = left.max(i.saturating_sub(128));
        let visible: Vec<&AnObj> = objects[from..=i].iter().filter(|v| v.kind != Kind::Spinner).collect();
        densities.push(visible.len() as f64);
        overlaps.push(visible.iter().filter(|v| !std::ptr::eq(**v, o) && (o.x - v.x).hypot(o.y - v.y) < diameter).count() as f64);
        if let Some(g) = &o.geom {
            let velocity = g.travel / diameter / (0.02f64).max((o.end_time - o.time) / 1000.0);
            sliders.push((velocity / 4.0).sqrt() * (1.0 + 0.15 * (3.0f64).min(g.curvature / std::f64::consts::PI)));
        }
        if i == 0 {
            continue;
        }
        let p = &objects[i - 1];
        if p.kind == Kind::Spinner || o.time <= p.time {
            continue;
        }
        let gap = o.time - p.time;
        let move_time = (25.0f64).max(o.time - p.end_time);
        let distance = (o.x - p.end.x).hypot(o.y - p.end.y) / diameter;
        let active = gap <= (1000.0f64).min(o.beat_length * 2.0);
        let circle_pair = p.kind == Kind::Circle && o.kind == Kind::Circle;
        let fast = circle_pair && gap >= 25.0 && gap <= (220.0f64).min(o.beat_length * 0.34);
        let velocity = if active { distance / (move_time / 1000.0) } else { 0.0 };
        let mut internal_angle = 0.0;
        if i >= 2 {
            let before = &objects[i - 2];
            if before.kind != Kind::Spinner && p.time - before.end_time < (1000.0f64).min(p.beat_length * 2.0) {
                let (ax, ay) = (p.end.x - before.end.x, p.end.y - before.end.y);
                let (bx, by) = (o.x - p.end.x, o.y - p.end.y);
                let len = ax.hypot(ay) * bx.hypot(by);
                if len > 0.0 {
                    internal_angle = 1.0 - clamp((ax * bx + ay * by) / len, -1.0, 1.0).acos() / std::f64::consts::PI;
                }
            }
        }
        events.push(Ev {
            index: i,
            time: o.time,
            gap,
            beat_length: o.beat_length,
            active,
            circle_pair,
            fast,
            distance,
            velocity,
            internal_angle,
            jump: if active && !fast && distance >= 1.3 { velocity } else { 0.0 },
            wide: if active && distance >= 1.3 { velocity * internal_angle } else { 0.0 },
            sustained_tap: if active && circle_pair && gap >= 25.0 && gap <= 300.0 { 1000.0 / gap } else { 0.0 },
        });
    }

    // consecutive fast circle runs
    let mut fast_runs: Vec<Vec<usize>> = Vec::new();
    let mut run: Vec<usize> = Vec::new();
    for (ei, e) in events.iter().enumerate() {
        if !e.fast || (!run.is_empty() && e.index != events[*run.last().unwrap()].index + 1) {
            if run.len() >= 3 {
                fast_runs.push(run.clone());
            }
            run.clear();
        }
        if e.fast {
            run.push(ei);
        }
    }
    if run.len() >= 3 {
        fast_runs.push(run);
    }
    let fast_events = fast_runs.iter().flatten().count();

    // flow runs: smooth, rhythmically steady streams
    let mut flow_runs: Vec<Vec<usize>> = Vec::new();
    let mut flowing: Vec<usize> = Vec::new();
    for (ei, e) in events.iter().enumerate() {
        let valid = e.circle_pair && e.active && e.gap <= (350.0f64).min(e.beat_length * 0.6) && e.distance >= 0.35 && e.distance <= 3.5 && e.internal_angle > 0.5;
        let breaks = !flowing.is_empty() && (e.index != events[*flowing.last().unwrap()].index + 1 || (e.gap / events[*flowing.last().unwrap()].gap).log2().abs() > 0.3);
        if !valid || breaks {
            if flowing.len() >= 4 {
                flow_runs.push(flowing.clone());
            }
            flowing.clear();
        }
        if valid {
            flowing.push(ei);
        }
    }
    if flowing.len() >= 4 {
        flow_runs.push(flowing);
    }
    let flow_velocities: Vec<f64> = flow_runs.iter().flatten().map(|&ei| events[ei].velocity).collect();

    // rhythm islands and their changes (finger control)
    struct Island {
        gap: f64,
        count: i64,
        start_index: usize,
        end_index: usize,
    }
    let mut islands: Vec<Island> = Vec::new();
    let mut have_island = false;
    for e in &events {
        if !e.circle_pair || !e.active || e.gap < 25.0 || e.gap > 350.0 {
            have_island = false;
            continue;
        }
        if have_island {
            let last = islands.last_mut().unwrap();
            if e.index == last.end_index + 1 && (e.gap - last.gap).abs() <= (8.0f64).max(last.gap * 0.08) {
                last.count += 1;
                last.end_index = e.index;
                continue;
            }
        }
        islands.push(Island { gap: e.gap, count: 1, start_index: e.index, end_index: e.index });
        have_island = true;
    }
    let mut rhythm: Vec<f64> = Vec::new();
    for i in 1..islands.len() {
        let (a, b) = (&islands[i - 1], &islands[i]);
        if b.start_index != a.end_index + 1 || a.gap.min(b.gap) > 200.0 {
            continue;
        }
        let ratio = a.gap.max(b.gap) / a.gap.min(b.gap);
        if ratio < 1.2 || ratio > 3.5 || a.count.max(b.count) > 16 {
            continue;
        }
        let parity = if a.count % 2 != b.count % 2 { 0.3 } else { 0.0 };
        let changing = (1.0f64).min((a.count - b.count).abs() as f64 / 4.0);
        let non_binary = if (ratio - 2.0).abs() > 0.18 { 0.3 } else { 0.0 };
        rhythm.push((0.2 + 0.35 * changing + parity + non_binary) * (1.0f64).min(a.count.min(b.count) as f64 / 2.0) * (1.0f64).min(180.0 / a.gap.min(b.gap)));
    }

    let non_spinners = objects.iter().filter(|o| o.kind != Kind::Spinner).count();
    let n = (events.len().max(1)) as f64;
    let mut support = [0.0f64; N_SKILLS];
    support[0] = events.iter().filter(|e| e.wide > 0.0).count() as f64; // wide-angle-aim
    support[1] = events.iter().filter(|e| e.distance > 0.35 && e.active).count() as f64; // precision
    support[ACCURACY] = non_spinners as f64; // accuracy
    support[3] = events.iter().filter(|e| e.jump > 0.0).count() as f64; // snap-aim
    support[FINGER_CONTROL] = rhythm.len() as f64; // finger-control
    support[5] = events.iter().filter(|e| e.velocity > 2.0).count() as f64; // aim-stamina
    support[6] = fast_events as f64; // speed
    support[7] = non_spinners as f64; // density-reading
    support[8] = events.iter().filter(|e| e.sustained_tap > 0.0).count() as f64; // tapping-stamina
    support[9] = sliders.len() as f64; // slider-control
    support[10] = flow_runs.iter().flatten().count() as f64; // flow-aim
    let mut coverage = [0.0f64; N_SKILLS];
    for k in 0..N_SKILLS {
        coverage[k] = clamp01(support[k] / n);
    }
    let support_factor = |k: usize| {
        let target = if k == FINGER_CONTROL { 6.0 } else { 12.0 };
        (1.0f64).min((support[k] / target).sqrt())
    };
    let coverage_factor = |k: usize| {
        if support[k] == 0.0 {
            return 0.0;
        }
        let target = if k == FINGER_CONTROL { 0.05 } else { 0.25 };
        (1.0f64).min(0.5 + 0.5 * (coverage[k] / target).sqrt())
    };

    let peak_velocity = weighted_top(&events.iter().map(|e| e.velocity).collect::<Vec<_>>());
    let aim_base = diff.map(|d| d.aim).filter(|v| v.is_finite()).unwrap_or_else(|| (peak_velocity / 4.0).sqrt());
    let fast_nps = weighted_top(&fast_runs.iter().map(|r| 1000.0 / mean(&r.iter().map(|&ei| events[ei].gap).collect::<Vec<_>>())).collect::<Vec<_>>());
    let speed_base = diff.map(|d| d.speed).filter(|v| v.is_finite()).unwrap_or(fast_nps / 3.5);
    let sustained_aim = rolling_load(&events, |e| e.velocity, 15.0);
    let sustained_tap = rolling_load(&events, |e| e.sustained_tap, 15.0);

    let pv = if peak_velocity == 0.0 { 1.0 } else { peak_velocity };
    let mut base = [0.0f64; N_SKILLS];
    base[0] = 1.25 * aim_base * (weighted_top(&events.iter().map(|e| e.wide).collect::<Vec<_>>()) / pv).sqrt();
    base[1] = aim_base * (s.cs / 4.0).sqrt();
    base[ACCURACY] = 30.0 / (4.0f64).max(s.great_hit_window / 2.0);
    base[3] = aim_base * (weighted_top(&events.iter().map(|e| e.jump).collect::<Vec<_>>()) / pv).sqrt();
    base[FINGER_CONTROL] = speed_base * (1.6f64).min(weighted_top(&rhythm) * 1.5);
    base[5] = aim_base * clamp01(sustained_aim / pv).sqrt();
    base[6] = speed_base;
    base[7] = (quantile(&densities, 0.9).sqrt() / 1.3) * (1.0 + 0.06 * (4.0f64).min(quantile(&overlaps, 0.9))) * if s.hidden { 1.05 } else { 1.0 };
    // tapping-stamina: the sustained tapping load against a stamina reference, on the speed scale.
    // Was gated by sustained_tap / fast_nps (the burst peak), which docked anything that wasn't a
    // pure held stream — so long streams (real stamina) read low.
    // ponytail: 10.0 is the sustained taps/s load of demanding stamina, a calibration knob
    base[8] = (if speed_base != 0.0 { speed_base } else { sustained_tap / 3.5 }) * clamp01(sustained_tap / 10.0).sqrt();
    base[9] = weighted_top(&sliders);
    // flow-aim on the aim scale, gated by how fast the flow streams are against a flow reference —
    // not the map's peak velocity, which the jumps set and which suppressed flow on any mixed map
    // ponytail: 10.0 is the flow-velocity (≈diameters/s) of demanding streams, a calibration knob
    base[10] = aim_base * clamp01(weighted_top(&flow_velocities) / 10.0).sqrt();

    let mut raw_demand = [0.0f64; N_SKILLS];
    for k in 0..N_SKILLS {
        raw_demand[k] = finite(base[k], 0.0).max(0.0) * support_factor(k) * coverage_factor(k);
    }

    Pattern {
        raw_demand,
        support,
        coverage,
        object_count: non_spinners,
        duration: if objects.is_empty() { 0.0 } else { (objects[objects.len() - 1].end_time - objects[0].time) / 1000.0 },
        great_hit_window: s.great_hit_window,
        stars: diff.map(|d| d.stars).unwrap_or(0.0),
        ar: diff.map(|d| d.ar).unwrap_or(0.0),
    }
}

/* ---------- per play ---------- */

struct Hits {
    great: f64,
    miss: f64,
    total: f64,
    accuracy: f64,
}

fn hit_results(play: &StdPlay) -> Hits {
    let g = |k: &str, alt: &str| play.hits.get(k).or_else(|| play.hits.get(alt)).copied().unwrap_or(0.0);
    let great = g("great", "count_300");
    let ok = g("ok", "count_100");
    let meh = g("meh", "count_50");
    let miss = g("miss", "count_miss");
    let total = great + ok + meh + miss;
    Hits { great, miss, total, accuracy: if total > 0.0 { (great + ok / 3.0 + meh / 6.0) / total } else { clamp01(play.acc) } }
}

/// Conservative accuracy demand from head judgements (Wilson lower bound → normal spread).
fn accuracy_demand(hits: &Hits, window: f64) -> Option<f64> {
    if hits.total == 0.0 {
        return None;
    }
    let n = hits.total;
    let p = hits.great / n;
    let z = 1.645;
    let z2 = z * z;
    let lower = (p + z2 / (2.0 * n) - z * ((p * (1.0 - p)) / n + z2 / (4.0 * n * n)).sqrt()) / (1.0 + z2 / n);
    let x = clamp(lower, 0.001, 0.99999);
    let log = (1.0 - x * x).ln();
    let a = 0.147;
    let t = 2.0 / (std::f64::consts::PI * a) + log / 2.0;
    let inverse = ((t * t - log / a).sqrt() - t).sqrt();
    Some(30.0 / (3.0f64).max(window / (std::f64::consts::SQRT_2 * inverse)))
}

/* ---------- input / output ---------- */

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StdPlay {
    /// the library difficulty id
    pub map: String,
    #[serde(default)]
    pub mods: Vec<String>,
    #[serde(default)]
    pub hits: HashMap<String, f64>,
    #[serde(default)]
    pub acc: f64,
    /// unix ms, for the skills-over-time graph
    #[serde(default)]
    pub date: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendPoint {
    /// unix ms (the last play of a day whose plays raised a chart's best)
    pub date: f64,
    /// overall, then the eleven skillsets in SKILLS order
    pub values: Vec<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// index into the plays given
    pub play: usize,
    pub worth: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillOut {
    pub id: String,
    pub label: String,
    pub group: String,
    pub description: String,
    pub value: f64,
    pub confidence: f64,
    pub evidence: Vec<Evidence>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub score_count: usize,
    pub analyzed_beatmap_count: usize,
    pub metadata_only_count: usize,
    pub failed_difficulty_count: usize,
    pub method: String,
    pub engine: String,
    pub comfort_stars: f64,
    pub peak_stars: f64,
    pub preferred_ar: [f64; 2],
    pub typical_length: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Computed {
    pub skills: Vec<SkillOut>,
    pub overall: f64,
    /// how the skills grew, oldest first (overall + the eleven skillsets at each day)
    pub trend: Vec<TrendPoint>,
    pub summary: Summary,
}

/* ---------- per-chart cache ---------- */

struct Cached {
    hash: String,
    key: String,
    rate: u64,
    pattern: Pattern,
}
static KEPT: Mutex<Vec<Cached>> = Mutex::new(Vec::new());
const KEEP: usize = 512;

fn mods_key(mods: &[String]) -> String {
    let mut m: Vec<&str> = mods.iter().map(|s| s.as_str()).collect();
    m.sort_unstable();
    m.join("")
}

/// The rate as a stable integer key (×1000), so the cache lookup doesn't hinge on float bits.
fn rate_db_key(rate: f64) -> i64 {
    (rate * 1000.0).round() as i64
}

fn db_load(db: &Mutex<Connection>, hash: &str, key: &str, rate: i64) -> Option<Pattern> {
    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
    conn.query_row(
        "SELECT data FROM osu_skill_charts WHERE hash=?1 AND mods=?2 AND rate=?3 AND version=?4",
        params![hash, key, rate, CACHE_VERSION],
        |r| r.get::<_, Vec<u8>>(0),
    )
    .optional()
    .ok()
    .flatten()
    .and_then(|b| serde_json::from_slice(&b).ok())
}

fn db_store(db: &Mutex<Connection>, hash: &str, key: &str, rate: i64, pattern: &Pattern) {
    if let Ok(data) = serde_json::to_vec(pattern) {
        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = conn.execute(
            "INSERT OR REPLACE INTO osu_skill_charts (hash, mods, rate, version, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![hash, key, rate, CACHE_VERSION, data],
        );
    }
}

/// The chart's demand, difficulty and metrics, worked out once per (chart, mods, rate): from this
/// run's memory, else the database (survives restarts), else computed and stored in both. The DB
/// lock is only taken for the quick read and write, never during the heavy analysis.
fn pattern_for(read: &impl Fn(&str) -> Option<Vec<u8>>, db: &Mutex<Connection>, hash: &str, mods: &[String], rate: f64) -> Option<Pattern> {
    let key = mods_key(mods);
    let bits = rate.to_bits();
    if let Some(c) = KEPT.lock().unwrap().iter().find(|c| c.hash == hash && c.key == key && c.rate == bits) {
        return Some(c.pattern.clone());
    }
    let rk = rate_db_key(rate);
    if let Some(p) = db_load(db, hash, &key, rk) {
        KEPT.lock().unwrap().insert(0, Cached { hash: hash.to_string(), key, rate: bits, pattern: p.clone() });
        return Some(p);
    }
    let bytes = read(hash)?;
    let text = String::from_utf8_lossy(&bytes);
    let map = parse_beatmap(&text);
    if map.objects.len() < 3 {
        return None;
    }
    let diff = pp::osu_difficulty_from(|| Ok(bytes.clone()), hash, mods, rate).ok().flatten();
    let pattern = analyze_beatmap(&map, mods, diff.as_ref());
    db_store(db, hash, &key, rk, &pattern);
    let mut kept = KEPT.lock().unwrap();
    kept.insert(0, Cached { hash: hash.to_string(), key, rate: bits, pattern: pattern.clone() });
    kept.truncate(KEEP);
    Some(pattern)
}

/// Pre-compute and cache a chart's pattern for the given mods, from text already in hand. The
/// compare workers call this in parallel so the heavy analysis doesn't fall to the single-threaded
/// `skills` pass afterwards (which then finds everything in cache). Same keys `skills` reads under.
pub fn warm(db: &Mutex<Connection>, hash: &str, text: &[u8], mods: &[String]) {
    let read = |_: &str| Some(text.to_vec());
    let _ = pattern_for(&read, db, hash, mods, rate_of(mods));
}

const ASSISTED: [&str; 5] = ["RX", "AP", "AT", "CN", "TD"];

/// Your osu!standard skills from your passes. `hashes` maps each play's `map` id to its `.osu`
/// hash (resolved by the caller, which knows the library). All the compute is here.
pub fn skills(read: impl Fn(&str) -> Option<Vec<u8>>, db: &Mutex<Connection>, hashes: &HashMap<String, String>, plays: &[StdPlay]) -> Result<Computed> {
    struct Row {
        play: usize,
        map: String,
        date: f64,
        /// what the play is worth in each skillset (0 where it shows nothing of it)
        worth: [f64; N_SKILLS],
        stars: f64,
        ar: f64,
        duration: f64,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut missing = 0usize;
    let mut failed = 0usize;
    for (i, play) in plays.iter().enumerate() {
        if play.mods.iter().any(|m| ASSISTED.contains(&m.as_str())) {
            continue;
        }
        let hash = match hashes.get(&play.map) {
            Some(h) => h,
            None => {
                missing += 1;
                continue;
            }
        };
        let rate = rate_of(&play.mods);
        let mut pattern = match pattern_for(&read, db, hash, &play.mods, rate) {
            Some(p) => p,
            None => {
                failed += 1;
                continue;
            }
        };
        let hits = hit_results(play);
        let miss_rate = hits.miss / (1.0f64).max(if hits.total > 0.0 { hits.total } else { pattern.object_count as f64 });
        let mastery = hits.accuracy.powi(2) * (-4.0 * miss_rate).exp();
        match accuracy_demand(&hits, pattern.great_hit_window) {
            Some(t) => pattern.raw_demand[ACCURACY] = t,
            None => {
                pattern.raw_demand[ACCURACY] = 0.0;
                pattern.support[ACCURACY] = 0.0;
            }
        }
        let mut worth = [0.0f64; N_SKILLS];
        for k in 0..N_SKILLS {
            if pattern.support[k] == 0.0 || pattern.raw_demand[k] <= 0.0 {
                continue;
            }
            let performance = if k == ACCURACY { (-4.0 * miss_rate).exp() } else { mastery };
            worth[k] = rating(pattern.raw_demand[k] * performance);
        }
        rows.push(Row { play: i, map: play.map.clone(), date: play.date, worth, stars: pattern.stars, ar: pattern.ar, duration: pattern.duration });
    }

    let mut skills: Vec<SkillOut> = Vec::with_capacity(N_SKILLS);
    for k in 0..N_SKILLS {
        // the best demonstration per chart
        let mut by_map: HashMap<&str, (usize, f64)> = HashMap::new();
        for row in &rows {
            if row.worth[k] <= 0.0 {
                continue;
            }
            let e = by_map.entry(row.map.as_str()).or_insert((row.play, -1.0));
            if row.worth[k] > e.1 {
                *e = (row.play, row.worth[k]);
            }
        }
        let map_count = by_map.len();
        let mut best: Vec<(usize, f64)> = by_map.into_values().collect();
        best.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        best.truncate(5);
        let value = skill_value(&best.iter().map(|b| b.1).collect::<Vec<_>>());
        let (id, label, group, desc) = SKILLS[k];
        skills.push(SkillOut {
            id: id.to_string(),
            label: label.to_string(),
            group: group.to_string(),
            description: desc.to_string(),
            value: round2(value),
            confidence: round2((1.0f64).min(map_count as f64 / 10.0)),
            evidence: best.into_iter().map(|(play, worth)| Evidence { play, worth: round2(worth) }).collect(),
        });
    }
    let overall = overall_of(&skills.iter().filter(|s| s.confidence > 0.0).map(|s| s.value).collect::<Vec<_>>());

    // how the skills grew: replay plays by date, keep each chart's best per skill, and at the end of
    // every day whose plays raised one, record what the skills would have said that evening
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| rows[a].date.partial_cmp(&rows[b].date).unwrap());
    let mut bests: Vec<HashMap<&str, f64>> = (0..N_SKILLS).map(|_| HashMap::new()).collect();
    let mut trend: Vec<TrendPoint> = Vec::new();
    let mut moved = false;
    for (oi, &ri) in order.iter().enumerate() {
        let row = &rows[ri];
        for k in 0..N_SKILLS {
            if row.worth[k] <= 0.0 {
                continue;
            }
            let e = bests[k].entry(row.map.as_str()).or_insert(0.0);
            if row.worth[k] > *e {
                *e = row.worth[k];
                moved = true;
            }
        }
        let day = (row.date / 86_400_000.0).floor();
        let next_day = order.get(oi + 1).map(|&n| (rows[n].date / 86_400_000.0).floor());
        if moved && next_day != Some(day) {
            let skill_vals: Vec<f64> = (0..N_SKILLS).map(|k| skill_value(&bests[k].values().copied().collect::<Vec<_>>())).collect();
            let mut values = Vec::with_capacity(N_SKILLS + 1);
            values.push(round2(overall_of(&skill_vals)));
            values.extend(skill_vals.iter().map(|v| round2(*v)));
            trend.push(TrendPoint { date: row.date, values });
            moved = false;
        }
    }

    let stars: Vec<f64> = rows.iter().map(|r| r.stars).collect();
    let ars: Vec<f64> = rows.iter().map(|r| r.ar).collect();
    let analyzed: std::collections::HashSet<&str> = rows.iter().map(|r| r.map.as_str()).collect();
    Ok(Computed {
        skills,
        overall: round2(overall),
        trend,
        summary: Summary {
            score_count: plays.len(),
            analyzed_beatmap_count: analyzed.len(),
            metadata_only_count: missing,
            failed_difficulty_count: failed,
            method: MODEL_VERSION.to_string(),
            engine: "rosu-pp (fumen backend)".to_string(),
            comfort_stars: round2(quantile(&stars, 0.5)),
            peak_stars: round2(quantile(&stars, 0.95)),
            preferred_ar: [round2(quantile(&ars, 0.25)), round2(quantile(&ars, 0.75))],
            typical_length: round2(quantile(&rows.iter().map(|r| r.duration).collect::<Vec<_>>(), 0.5)),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = "[General]\nMode: 0\n[Difficulty]\nCircleSize:4\nApproachRate:9\nOverallDifficulty:8\nSliderMultiplier:1.4\n[TimingPoints]\n0,300,4,1,0,100,1,0\n[HitObjects]\n256,192,0,1,0\n120,120,120,1,0\n400,260,240,1,0\n180,80,360,1,0\n256,192,480,2,0,L|356:192,1,100\n";

    #[test]
    fn rating_is_bounded_and_rising() {
        assert_eq!(rating(0.0), 0.0);
        assert!((rating(2.5) - 50.0).abs() < 1e-6);
        assert!(rating(1000.0) > 99.0 && rating(1000.0) <= 100.0);
        assert!(rating(5.0) > rating(2.5));
    }

    #[test]
    fn parses_and_analyzes() {
        let map = parse_beatmap(MAP);
        assert_eq!(map.objects.len(), 5);
        assert_eq!(map.cs, 4.0);
        assert_eq!(map.ar, 9.0);
        let p = analyze_beatmap(&map, &[], None);
        assert_eq!(p.object_count, 5);
        for k in 0..N_SKILLS {
            assert!(p.raw_demand[k].is_finite() && p.raw_demand[k] >= 0.0, "skill {k}");
        }
    }

    #[test]
    fn rate_speeds_up() {
        assert!((rate_of(&["DT".to_string()]) - 1.5).abs() < 1e-9);
        assert_eq!(rate_of(&[]), 1.0);
    }
}
