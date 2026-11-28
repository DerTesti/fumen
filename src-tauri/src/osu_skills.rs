//! osu!standard skills, worked out the way osu!mania's are: all the compute is here in the backend,
//! the heavy per-chart part cached, and the frontend only reads the result (lib/osu-standard.ts,
//! views/skills.ts).
//!
//! osu!std has no per-play skillset calculator the way osu!mania has MinaCalc (msd.rs). This is a
//! model of its own, ported from an earlier app of the author's: rosu-pp's aim/speed difficulty
//! (through `pp::osu_difficulty`, the same fork pp uses) together with a pattern pass over the
//! `.osu` itself — wide-angle and snap jumps, flow streams, rhythm islands, slider paths, reading
//! density and sustained 15-second load. Every eligible pass demonstrates a skill to the degree the
//! chart demands it and you played it clean; a skillset's value is the mean of your best five
//! distinct-chart demonstrations, with missing evidence counted as zero. The scale is provisional,
//! not osu!'s pp or any site's percentile.

use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::pp;

/// Bump when the model changes, so cached rows from an older model are ignored.
const CACHE_VERSION: i64 = 18;

// Always accept 8 notes/s; allow slower beat-relative streams up to 180 ms as well.
const SPEED_MAX_GAP_MS: f64 = 125.0;
// Keep the existing absolute flow/rest limits, without timing-point beat fractions.
const FLOW_MAX_GAP_MS: f64 = 350.0;
const ACTIVE_MAX_GAP_MS: f64 = 1000.0;

/// The eleven skillsets, in order: id, label, group, description. Labels/descriptions are the
/// model's own (the view shows them as given).
pub const SKILLS: [(&str, &str, &str, &str); 11] = [
    ("wide-angle-aim", "Wide angle aim", "Aim", "Weite Bewegungen mit offenem Innenwinkel über 90°; jeder Jump zählt nur für Wide oder Snap."),
    ("precision", "Precision", "Aim", "Kleine Trefferflächen mit aktiven, kompakten Zielwechseln statt bloßer Jump-Distanz."),
    ("accuracy", "Accuracy", "Accuracy", "Timing-Präzision relativ zum 300er-Zeitfenster unter Tapping-, Rhythmus- und Aim-Belastung."),
    ("snap-aim", "Snap aim", "Aim", "Abrupte Bewegungswechsel bei weiten Jumps mit Innenwinkeln bis 90°; getrennt von Wide angle aim."),
    ("finger-control", "Finger control", "Tapping", "Wechsel aktiver Rhythmusinseln; Pausen und Slider-Haltezeiten zählen nicht als Komplexität."),
    ("aim-stamina", "Aim stamina", "Aim", "Aim-Belastung in 15-Sekunden-Fenstern und zusammenhängenden Passagen; Ruhezeiten unterbrechen den Nachweis."),
    ("speed", "Speed", "Tapping", "rosu-Speed-Schwierigkeit, gewichtet nach tatsächlichem Tempo zusammenhängender schneller Circle-Folgen."),
    ("density-reading", "Density reading", "Reading", "Gleichzeitig sichtbare Objekte, Überlagerungen und kreuzende Notenfolgen, gewichtet nach tatsächlich verfügbarer Sichtzeit."),
    ("tapping-stamina", "Tapping stamina", "Tapping", "Zeitbasierter Belastungsaufbau über die ganze Map: voller Aufbau bis 180 ms, linear auslaufend bis 250 ms, Erholung bei Pausen und Slidern."),
    ("slider-control", "Slider control", "Tech", "Anspruchsvolle Slider mit nachgewiesenen Lazer-Ticks und Slider-Enden; ohne Detaildaten kein Nachweis."),
    ("flow-aim", "Flow aim", "Aim", "Räumlich anspruchsvolle, rhythmisch stabile Streams mit kontinuierlicher Bewegung."),
];
const N_SKILLS: usize = 11;
const ACCURACY: usize = 2;
const FINGER_CONTROL: usize = 4;

pub const MODEL_VERSION: &str = "pattern-rosu-reading-ratios-v8";

#[derive(Deserialize)]
struct ReferenceAnchor {
    raw: f64,
    status: String,
}

#[derive(Deserialize)]
struct RatingCurves {
    curves: HashMap<String, Vec<[f64; 2]>>,
    extrapolation: String,
    anchors: HashMap<String, Vec<ReferenceAnchor>>,
}

// Fixed snapshot: offline experiments never activate a new curve automatically.
static RATING_CURVES: LazyLock<RatingCurves> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../src/lib/osu-skill-curves.json"))
        .expect("valid embedded skill curves")
});

/// Half-star map-reference calibration, with uncapped logarithmic continuation.
pub fn rating(d: f64, skill: usize) -> f64 {
    let d = finite(d, 0.0).max(0.0);
    if d == 0.0 { return 0.0 }
    let points = &RATING_CURVES.curves[SKILLS[skill].0];
    let &[last_x, last_y] = points.last().unwrap();
    if d > last_x && RATING_CURVES.extrapolation == "proportional" {
        return last_y * (d / last_x);
    }
    if d > last_x && RATING_CURVES.extrapolation == "log-tail" {
        return last_y * (1.0 + d.ln() - last_x.ln());
    }
    let upper = points.iter().position(|p| d <= p[0]).unwrap_or(points.len() - 1).max(1);
    let [x0, y0] = points[upper - 1];
    let [x1, y1] = points[upper];
    y0 + (d - x0) * (y1 - y0) / (x1 - x0)
}

fn play_rating(d: f64, skill: usize, performance: f64) -> f64 {
    // Apply the same quality penalty after calibration, rather than distorting it per curve.
    rating(d, skill) * clamp01(performance)
}

/// Undo only the two outer evidence factors used to construct the reference corpus.
/// Occurrence and evidence sufficiency remain separate; inner skill factors stay intact.
fn intensity(p: &Pattern, k: usize) -> f64 {
    if p.raw_demand[k] <= 0.0 || p.support[k] <= 0.0 || p.coverage[k] <= 0.0 { return 0.0 }
    let support_target = if k == FINGER_CONTROL { 6.0 } else { 12.0 };
    let coverage_target = if k == FINGER_CONTROL { 0.05 } else { 0.25 };
    let weight = (p.support[k] / support_target).sqrt().min(1.0)
        * (0.5 + 0.5 * (p.coverage[k] / coverage_target).sqrt()).min(1.0);
    finite(p.raw_demand[k] / weight, 0.0).max(0.0)
}

fn usable_evidence(p: &Pattern, k: usize) -> bool {
    let geometry_sensitive = matches!(k, 0 | 1 | 2 | 3 | 5 | 9 | 10);
    p.support[k] >= if k == FINGER_CONTROL { 6.0 } else { 12.0 }
        && p.coverage[k] >= if k == FINGER_CONTROL { 0.01 } else { 0.05 }
        && !(p.geometry_limited && geometry_sensitive)
}

fn uncertain_reference(d: f64, k: usize) -> bool {
    let anchors = &RATING_CURVES.anchors[SKILLS[k].0];
    let lower = anchors.iter().rev().find(|a| a.raw <= d);
    let upper = anchors.iter().find(|a| a.raw >= d);
    match (lower, upper) {
        (Some(a), Some(b)) => a.status != "measured" || b.status != "measured",
        _ => true,
    }
}

/// A chart's open-ended demand per skillset and its star rating at some mods, from the pattern
/// cache (recommend.rs): library charts reuse the pattern worked out for your own skills, so they
/// cost nothing to score. The stars are with the mods on, so a recommender can keep a map whose
/// mods (DT especially) blow its difficulty past your comfort out of the list. `read(hash)` yields
/// the `.osu` bytes when the pattern isn't cached yet.
pub fn rated_demand(read: &impl Fn(&str) -> Option<Vec<u8>>, db: &Mutex<Connection>, hash: &str, mods: &[String]) -> Option<([f64; N_SKILLS], f64)> {
    let p = pattern_for(read, db, hash, mods, rate_of(mods))?;
    let mut demand = [0.0f64; N_SKILLS];
    for k in 0..N_SKILLS {
        demand[k] = if usable_evidence(&p, k) { rating(intensity(&p, k), k) } else { 0.0 };
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
/// Confirmed level: five equally weighted best-per-chart demonstrations.
/// Missing demonstrations count as zero: one exceptional play contributes only one fifth.
fn skill_value(worths: &[f64]) -> f64 {
    let mut v = worths.to_vec();
    v.sort_by(|a, b| b.partial_cmp(a).unwrap());
    v.truncate(5);
    v.iter().sum::<f64>() / 5.0
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

fn play_rate(rate: Option<f64>, mods: &[String]) -> f64 {
    rate.filter(|r| r.is_finite() && *r > 0.0).unwrap_or_else(|| rate_of(mods))
}

fn effective_settings(map: &Map, mods: &[String], rate: f64) -> Settings {
    let has = |n: &str| mods.iter().any(|m| m == n);
    let (hr, ez) = (has("HR"), has("EZ"));
    let scale = |base: f64, hr_f: f64| base * if hr { hr_f } else if ez { 0.5 } else { 1.0 };
    let cs = clamp(finite(scale(map.cs, 1.3), map.cs), 0.0, 10.0);
    let ar = clamp(finite(scale(map.ar, 1.4), map.ar), 0.0, 10.0);
    let od = clamp(finite(scale(map.od, 1.4), map.od), 0.0, 10.0);
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
    active: bool,
    circle_pair: bool,
    fast: bool,
    distance: f64,
    velocity: f64,
    internal_angle: f64,
    jump: f64,
    wide: f64,
    sustained_tap: f64,
    precision: f64,
}

/// Mutually exclusive evidence: open internal angles are wide; sharp turns are snap.
/// An unknown angle must not default to a reversal and create false snap evidence.
fn jump_demands(active: bool, distance: f64, velocity: f64, angle: Option<f64>, acceleration: f64) -> (f64, f64) {
    if !active || distance < 1.3 { return (0.0, 0.0) }
    match angle {
        Some(a) if a > 0.5 => (0.0, velocity * a.powi(2)),
        Some(_) => (acceleration, 0.0),
        None => (0.0, 0.0),
    }
}

fn tapping_weight(e: &Ev) -> f64 {
    if !e.circle_pair || e.gap < 25.0 { return 0.0 }
    clamp01((250.0 - e.gap) / 70.0)
}

/// X is measured in equivalent tapping seconds, not notes. Solve dX/dt = w - X/tau
/// exactly for each interval: w fades linearly from 1 at 180ms to 0 at 250ms;
/// tau fades from 60s under full load to 5s at rest. Sliders/spinners are recovery.
/// Repeated runs preserve residual load, and constant full tapping approaches X=60.
fn tapping_peak(events: &[Ev]) -> f64 {
    let mut x: f64 = 0.0;
    let mut peak: f64 = 0.0;
    let mut previous_time = None;
    for e in events {
        let elapsed = previous_time.map(|t| e.time - t).unwrap_or(e.gap).max(0.0) / 1000.0;
        previous_time = Some(e.time);
        let interval = (e.gap / 1000.0).min(elapsed);
        // Missing events (e.g. spinner heads) must still contribute their elapsed recovery time.
        x *= (-(elapsed - interval) / 5.0).exp();
        let w = tapping_weight(e);
        let tau = 5.0 + 55.0 * w;
        let decay = (-interval / tau).exp();
        x = x * decay + w * tau * (1.0 - decay);
        peak = peak.max(x);
    }
    peak
}

/// Longest uninterrupted qualifying passage, in played seconds. Every nonqualifying event,
/// skipped object or long rest resets it, so bursts cannot be stitched across recovery time.
fn longest_passage(events: &[Ev], qualifies: impl Fn(&Ev) -> bool) -> f64 {
    let mut previous = None;
    let mut duration: f64 = 0.0;
    let mut longest: f64 = 0.0;
    for e in events {
        if !qualifies(e) {
            duration = 0.0;
            previous = None;
            continue;
        }
        if previous != Some(e.index.saturating_sub(1)) {
            duration = 0.0;
        }
        duration += e.gap / 1000.0;
        longest = longest.max(duration);
        previous = Some(e.index);
    }
    longest
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

/// What a chart demands, before a play's accuracy scales it. Accuracy has a window-based chart
/// estimate; `skills` replaces it with the judgement-based timing demand times the cached load.
/// Cached to the database.
#[derive(Clone, Serialize, Deserialize)]
struct Pattern {
    raw_demand: [f64; N_SKILLS],
    support: [f64; N_SKILLS],
    coverage: [f64; N_SKILLS],
    object_count: usize,
    duration: f64,
    great_hit_window: f64,
    accuracy_load_factor: f64,
    slider_ticks: Option<u32>,
    tapping_peak: f64,
    stars: f64,
    ar: f64,
    geometry_limited: bool,
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

/// Strict crossings of the chronological head-to-head order in the current screen.
/// Shared endpoints, collinear stacks and gaps left by expired heads aren't crossings.
fn reading_crossings(objects: &[AnObj], visible: &[usize]) -> f64 {
    let edges: Vec<_> = visible.windows(2).filter(|p| p[1] == p[0] + 1).collect();
    let turn = |a: &AnObj, b: &AnObj, c: &AnObj| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let mut crossings = 0;
    for (i, a) in edges.iter().enumerate() {
        for b in edges.iter().skip(i + 1) {
            if a[1] == b[0] { continue }
            let (p, q, r, s) = (&objects[a[0]], &objects[a[1]], &objects[b[0]], &objects[b[1]]);
            if turn(p, q, r) * turn(p, q, s) < 0.0 && turn(r, s, p) * turn(r, s, q) < 0.0 {
                crossings += 1;
            }
        }
    }
    crossings as f64 / edges.len().max(1) as f64
}

fn reading_load(visible: usize, overlaps: usize, crossings_per_edge: f64, approach: f64, hidden: bool) -> f64 {
    // 600 ms is a unit reference (AR9 NM), not a visibility floor. The fourth root gives
    // time pressure a smaller effect than crowding; doubling playback rate adds ~19%.
    let time_pressure = (600.0 / approach).powf(0.25);
    let ambiguity = 1.0 + 0.06 * overlaps.min(4) as f64 + 0.35 * crossings_per_edge.sqrt();
    (visible as f64).sqrt() / 1.3 * ambiguity * time_pressure * if hidden { 1.05 } else { 1.0 }
}

fn analyze_beatmap(map: &Map, mods: &[String], rate: f64, diff: Option<&pp::OsuDiff>) -> Pattern {
    let s = effective_settings(map, mods, rate);
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
    let mut reading_loads: Vec<f64> = Vec::new();
    let mut visible_indices: Vec<usize> = Vec::new();
    for i in 0..objects.len() {
        let o = &objects[i];
        if o.kind == Kind::Spinner {
            continue;
        }
        // Sample when this head first appears. Earlier circles remain until their hit time;
        // sliders remain until their actual end (including repeats), not just their head time.
        // Expire each object independently: a long slider can outlive many newer circles.
        let reading_time = o.time - approach;
        visible_indices.retain(|&j| objects[j].end_time >= reading_time);
        visible_indices.push(i);
        let visible: Vec<&AnObj> = visible_indices.iter().map(|&j| &objects[j]).collect();
        let overlap = visible.iter().filter(|v| !std::ptr::eq(**v, o) && (o.x - v.x).hypot(o.y - v.y) < diameter).count();
        reading_loads.push(reading_load(visible.len(), overlap, reading_crossings(&objects, &visible_indices), approach, s.hidden));
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
        let active = gap <= ACTIVE_MAX_GAP_MS;
        let circle_pair = p.kind == Kind::Circle && o.kind == Kind::Circle;
        let fast = circle_pair && gap >= 25.0
            && (gap <= SPEED_MAX_GAP_MS || (gap <= 180.0 && gap <= o.beat_length * 0.34));
        let velocity = if active { distance / (move_time / 1000.0) } else { 0.0 };
        let mut internal_angle = 0.0;
        let mut known_angle = None;
        let mut snap_acceleration = 0.0;
        if i >= 2 {
            let before = &objects[i - 2];
            if before.kind != Kind::Spinner && p.time - before.end_time < ACTIVE_MAX_GAP_MS {
                let (ax, ay) = (p.end.x - before.end.x, p.end.y - before.end.y);
                let (bx, by) = (o.x - p.end.x, o.y - p.end.y);
                let len = ax.hypot(ay) * bx.hypot(by);
                if len > 0.0 {
                    internal_angle = 1.0 - clamp((ax * bx + ay * by) / len, -1.0, 1.0).acos() / std::f64::consts::PI;
                    known_angle = Some(internal_angle);
                }
                // Compare actual inter-object movement vectors, in diameters/s. Slider travel
                // itself is not an incoming jump: use the previous head and preceding end.
                let previous_time = p.time - before.end_time;
                if previous_time > 0.0 && previous_time < ACTIVE_MAX_GAP_MS {
                    let previous_seconds = previous_time.max(25.0) / 1000.0;
                    let seconds = move_time / 1000.0;
                    let previous_vx = (p.x - before.end.x) / diameter / previous_seconds;
                    let previous_vy = (p.y - before.end.y) / diameter / previous_seconds;
                    let vx = (o.x - p.end.x) / diameter / seconds;
                    let vy = (o.y - p.end.y) / diameter / seconds;
                    snap_acceleration = (vx - previous_vx).hypot(vy - previous_vy)
                        / ((previous_seconds + seconds) / 2.0);
                }
            }
        }
        let (jump, wide) = jump_demands(active, distance, velocity, known_angle, snap_acceleration);
        events.push(Ev {
            index: i,
            time: o.time,
            gap,
            active,
            circle_pair,
            fast,
            distance,
            velocity,
            internal_angle,
            jump,
            wide,
            sustained_tap: if active && circle_pair && gap >= 25.0 && gap <= 300.0 { 1000.0 / gap } else { 0.0 },
            // Small targets + active target changes; compact placement separates this from
            // pure large-distance jumps. All references are provisional calibration choices.
            precision: if active && distance > 0.35 {
                clamp01((s.cs - 2.0) / 5.0) * clamp01(velocity / 10.0)
                    / (1.0 + (distance / 2.0).powi(2))
            } else { 0.0 },
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
        let valid = e.circle_pair && e.active && e.gap >= 25.0 && e.gap <= FLOW_MAX_GAP_MS && e.distance >= 0.35 && e.distance <= 3.5 && e.internal_angle > 0.5;
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
    support[1] = events.iter().filter(|e| e.precision > 0.0).count() as f64; // precision
    support[ACCURACY] = non_spinners as f64; // accuracy
    support[3] = events.iter().filter(|e| e.jump > 0.0).count() as f64; // snap-aim
    support[FINGER_CONTROL] = rhythm.len() as f64; // finger-control
    support[5] = events.iter().filter(|e| e.velocity > 2.0).count() as f64; // aim-stamina
    support[6] = fast_events as f64; // speed
    support[7] = non_spinners as f64; // density-reading
    support[8] = events.iter().filter(|e| tapping_weight(e) > 0.0).count() as f64; // tapping-stamina
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
    let aim_passage = longest_passage(&events, |e| e.active && e.velocity > 2.0);
    let tapping_peak = tapping_peak(&events);
    // Geometry/timing only: no OD, stars or rosu components (which may themselves include OD).
    // Reuse supported fast runs so a single fast pair cannot supply the tempo bonus.
    let tempo_load = fast_nps * support_factor(6) * coverage_factor(6);
    let rhythm_load = weighted_top(&rhythm) * support_factor(FINGER_CONTROL) * coverage_factor(FINGER_CONTROL);
    let accuracy_load_factor = accuracy_load_factor(tempo_load, rhythm_load, sustained_tap, sustained_aim);

    let pv = if peak_velocity == 0.0 { 1.0 } else { peak_velocity };
    let mut base = [0.0f64; N_SKILLS];
    // Separate physical demands instead of the same rosu aim base multiplied by similar gates.
    // Provisional references: 4 diameters/s (wide) and 40 diameters/s² (snap).
    base[0] = 1.25 * (weighted_top(&events.iter().map(|e| e.wide).collect::<Vec<_>>()) / 4.0).sqrt();
    base[1] = aim_base * (s.cs / 4.0).sqrt()
        * weighted_top(&events.iter().map(|e| e.precision).collect::<Vec<_>>());
    base[ACCURACY] = 30.0 / (4.0f64).max(s.great_hit_window / 2.0) * accuracy_load_factor;
    base[3] = (weighted_top(&events.iter().map(|e| e.jump).collect::<Vec<_>>()) / 40.0).sqrt()
        * weighted_top(&events.iter().map(|e| clamp01(e.jump / 80.0)).collect::<Vec<_>>());
    base[FINGER_CONTROL] = speed_base * (1.6f64).min(weighted_top(&rhythm) * 1.5);
    // A short peak is insufficient: both window occupancy and uninterrupted duration matter.
    base[5] = aim_base * clamp01(sustained_aim / pv) * clamp01(aim_passage / 15.0);
    // Tempo is a pattern-strength multiplier, not just a yes/no support gate.
    base[6] = speed_base * (fast_nps / 10.0).powf(1.5).min(1.5);
    // Quantile of actual simultaneous loads, rather than unrelated density/overlap quantiles.
    base[7] = quantile(&reading_loads, 0.9);
    // 15 equivalent tapping seconds is a provisional scale reference, not a duration cap.
    let tap_base = if speed_base > 0.0 { speed_base } else {
        weighted_top(&events.iter().map(|e| e.sustained_tap).collect::<Vec<_>>()) / 3.5
    };
    base[8] = tap_base * tapping_peak / 15.0;
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
        accuracy_load_factor,
        slider_ticks: diff.map(|d| d.large_ticks),
        tapping_peak,
        stars: diff.map(|d| d.stars).unwrap_or(0.0),
        ar: diff.map(|d| d.ar).unwrap_or(0.0),
        geometry_limited: map.objects.iter().any(|o| o.control_points.len() > 63),
    }
}

/* ---------- per play ---------- */

/// Provisional load calibration, bounded to keep timing performance dominant.
/// References: 8 taps/s, rhythm complexity 1, sustained 8 taps/s and 10 diameters/s.
/// Sparse charts receive 0.5x; fully demanding charts at most 1.25x before rating().
fn accuracy_load_factor(tempo: f64, rhythm: f64, sustained_tap: f64, sustained_aim: f64) -> f64 {
    let load = 0.40 * clamp01(tempo / 8.0)
        + 0.25 * clamp01(rhythm)
        + 0.25 * clamp01(sustained_tap / 8.0)
        + 0.10 * clamp01(sustained_aim / 10.0);
    0.5 + 0.75 * load
}

struct Hits {
    great: f64,
    miss: f64,
    total: f64,
    accuracy: f64,
}

/// Strict slider following needs actual lazer results, not inferred success from head accuracy.
/// Stable/Classic and missing detail counts are unknown and do not demonstrate this skill.
fn slider_performance(play: &StdPlay, pattern: &Pattern) -> Option<f64> {
    if play.stable || play.mods.iter().any(|m| m == "CL") {
        return None;
    }
    let ratio = |key: &str, total: f64| -> Option<f64> {
        if total == 0.0 { return Some(1.0) }
        let hit = *play.hits.get(key)?;
        (hit.is_finite() && hit >= 0.0 && hit <= total).then_some(hit / total)
    };
    let sliders = pattern.support[9];
    if sliders <= 0.0 { return None }
    let tails = ratio("slider_tail_hit", sliders)?;
    let ticks = ratio("large_tick_hit", pattern.slider_ticks? as f64)?;
    Some(tails * ticks)
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
    /// Actual clock rate, including custom lazer rates.
    #[serde(default)]
    pub rate: Option<f64>,
    #[serde(default)]
    pub stable: bool,
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
    pub details: serde_json::Value,
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
    let pattern = analyze_beatmap(&map, mods, rate, diff.as_ref());
    db_store(db, hash, &key, rk, &pattern);
    let mut kept = KEPT.lock().unwrap();
    kept.insert(0, Cached { hash: hash.to_string(), key, rate: bits, pattern: pattern.clone() });
    kept.truncate(KEEP);
    Some(pattern)
}

/// Pre-compute and cache a chart's pattern for the given mods, from text already in hand. The
/// compare workers call this in parallel so the heavy analysis doesn't fall to the single-threaded
/// `skills` pass afterwards (which then finds everything in cache). Same keys `skills` reads under.
pub fn warm(db: &Mutex<Connection>, hash: &str, text: &[u8], mods: &[String], rate: Option<f64>) {
    let read = |_: &str| Some(text.to_vec());
    let rate = play_rate(rate, mods);
    let _ = pattern_for(&read, db, hash, mods, rate);
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
    let mut diagnostics = HashMap::new();
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
        let rate = play_rate(play.rate, &play.mods);
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
            Some(t) => pattern.raw_demand[ACCURACY] = t * pattern.accuracy_load_factor,
            None => {
                pattern.raw_demand[ACCURACY] = 0.0;
                pattern.support[ACCURACY] = 0.0;
            }
        }
        let mut worth = [0.0f64; N_SKILLS];
        for k in 0..N_SKILLS {
            if !usable_evidence(&pattern, k) || pattern.raw_demand[k] <= 0.0 {
                continue;
            }
            let performance = if k == ACCURACY {
                (-4.0 * miss_rate).exp()
            } else if k == 9 {
                let Some(following) = slider_performance(play, &pattern) else { continue };
                mastery * following
            } else { mastery };
            let demand = intensity(&pattern, k);
            worth[k] = play_rating(demand, k, performance);
            diagnostics.insert((i, k), serde_json::json!({
                "rawDemand": pattern.raw_demand[k], "intensity": demand, "calibratedDemand": rating(demand, k),
                "uncertain": uncertain_reference(demand, k), "scale": "map-reference-halfstar-v1", "performance": performance,
                "headAccuracy": hits.accuracy, "displayAccuracy": play.acc, "missRate": miss_rate,
                "stars": pattern.stars, "rate": rate, "support": pattern.support[k], "coverage": pattern.coverage[k]
            }));
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
            evidence: best.into_iter().map(|(play, worth)| Evidence { play, worth: round2(worth), details: diagnostics.remove(&(play, k)).unwrap_or_default() }).collect(),
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

    #[test]
    fn skill_level_requires_five_distinct_map_demonstrations() {
        assert_eq!(skill_value(&[]), 0.0);
        assert_eq!(skill_value(&[100.0]), 20.0);
        assert_eq!(skill_value(&[80.0; 5]), 80.0);
        assert_eq!(skill_value(&[100.0, 60.0, 60.0, 60.0, 60.0]), 68.0);
        assert_eq!(skill_value(&[1.0, 60.0, 100.0, 60.0, 60.0, 60.0]), 68.0);
        assert!(skill_value(&[80.0, 80.0]) > skill_value(&[80.0]));
    }

    const MAP: &str = "[General]\nMode: 0\n[Difficulty]\nCircleSize:4\nApproachRate:9\nOverallDifficulty:8\nSliderMultiplier:1.4\n[TimingPoints]\n0,300,4,1,0,100,1,0\n[HitObjects]\n256,192,0,1,0\n120,120,120,1,0\n400,260,240,1,0\n180,80,360,1,0\n256,192,480,2,0,L|356:192,1,100\n";

    #[test]
    fn half_star_reference_curves_are_monotone_and_remain_open_ended() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../../src/lib/osu-skill-curves.json")).unwrap();
        assert_eq!(config["scale"], "map-reference-halfstar-v1");
        assert_eq!(config["input"], "conditional-intensity");
        assert_eq!(config["extrapolation"], "log-tail");
        assert_eq!(config["patternVersion"].as_i64(), Some(CACHE_VERSION));
        assert_eq!(RATING_CURVES.curves.len(), N_SKILLS);
        for k in 0..N_SKILLS {
            let points = &RATING_CURVES.curves[SKILLS[k].0];
            assert_eq!(points[0], [0.0, 0.0]);
            for &[x, y] in points { assert!((rating(x, k) - y).abs() < 1e-9); }
            for pair in points.windows(2) {
                assert!(pair[1][0] > pair[0][0] && pair[1][1] > pair[0][1]);
                let mid = (pair[0][0] + pair[1][0]) / 2.0;
                assert!((rating(mid, k) - (pair[0][1] + pair[1][1]) / 2.0).abs() < 1e-9);
            }
            // Check both inside segments and across knots / the extrapolation boundary.
            for step in 1..100 {
                let raw = points.last().unwrap()[0] * step as f64 / 25.0;
                for multiplier in [1.01, 1.5, 2.0, 5.0] {
                    assert!(rating(raw * multiplier, k) >= rating(raw, k), "skill {k}, raw {raw}");
                }
            }
            let [x, y] = *points.last().unwrap();
            assert!(rating(x * 2.0, k) > y);
            assert!((rating(x * 2.0, k) - y * (1.0 + 2.0f64.ln())).abs() < 1e-8);
            assert!(uncertain_reference(x * 2.0, k));
            assert!(rating(10000.0, k) > 100.0);
            assert_eq!(rating(-1.0, k), 0.0);
            assert_eq!(rating(f64::NAN, k), 0.0);
            assert_eq!(rating(f64::INFINITY, k), 0.0);
            assert!((play_rating(x, k, 0.9) - y * 0.9).abs() < 1e-9);
        }
    }

    #[test]
    fn reference_intensity_separates_difficulty_from_evidence() {
        let mut p = analyze_beatmap(&parse_beatmap(MAP), &[], 1.0, None);
        p.raw_demand[0] = 2.0; p.support[0] = 3.0; p.coverage[0] = 0.0625;
        assert!((intensity(&p, 0) - 2.0 / 0.375).abs() < 1e-9);
        assert!(!usable_evidence(&p, 0));
        p.support[0] = 12.0; p.coverage[0] = 0.25;
        assert_eq!(intensity(&p, 0), 2.0);
        assert!(usable_evidence(&p, 0));
        p.geometry_limited = true;
        assert!(!usable_evidence(&p, 0));
        p.support[8] = 12.0; p.coverage[8] = 0.25;
        assert!(usable_evidence(&p, 8));
        p.raw_demand[0] = 0.0;
        assert_eq!(intensity(&p, 0), 0.0);
    }

    #[test]
    fn every_embedded_half_star_anchor_preserves_its_target() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../../src/lib/osu-skill-curves.json")).unwrap();
        for k in 0..N_SKILLS {
            let anchors = config["anchors"][SKILLS[k].0].as_array().unwrap();
            for (i, anchor) in anchors.iter().enumerate() {
                let stars = anchor["stars"].as_f64().unwrap();
                let raw = anchor["raw"].as_f64().unwrap();
                assert!((rating(raw, k) - (5.0 * stars - 5.0)).abs() < 1e-8);
                if i > 0 { assert_eq!(stars - anchors[i - 1]["stars"].as_f64().unwrap(), 0.5); }
            }
        }
    }

    #[test]
    fn slider_control_requires_real_end_and_tick_results() {
        let mut pattern = analyze_beatmap(&parse_beatmap(MAP), &[], 1.0, None);
        pattern.support[9] = 137.0;
        pattern.slider_ticks = Some(3);
        let mut play: StdPlay = serde_json::from_str(r#"{"map":"test","hits":{"slider_tail_hit":137,"large_tick_hit":3}}"#).unwrap();
        assert_eq!(slider_performance(&play, &pattern), Some(1.0));
        play.hits.insert("slider_tail_hit".into(), 100.0);
        let dropped_ends = slider_performance(&play, &pattern).unwrap();
        assert!(dropped_ends < 1.0);
        play.hits.insert("large_tick_hit".into(), 2.0);
        assert!(slider_performance(&play, &pattern).unwrap() < dropped_ends);
        play.hits.insert("slider_tail_hit".into(), 0.0);
        assert_eq!(slider_performance(&play, &pattern), Some(0.0));
        play.hits.remove("slider_tail_hit");
        assert_eq!(slider_performance(&play, &pattern), None);
        play.hits.insert("slider_tail_hit".into(), 137.0);
        play.stable = true;
        assert_eq!(slider_performance(&play, &pattern), None);
        play.stable = false;
        play.mods.push("CL".into());
        assert_eq!(slider_performance(&play, &pattern), None);
        play.mods.clear();
        play.hits.remove("large_tick_hit");
        assert_eq!(slider_performance(&play, &pattern), None);
        pattern.slider_ticks = Some(0);
        assert_eq!(slider_performance(&play, &pattern), Some(1.0));
        play.hits.insert("slider_tail_hit".into(), 138.0);
        assert_eq!(slider_performance(&play, &pattern), None);
    }

    #[test]
    fn parses_and_analyzes() {
        let map = parse_beatmap(MAP);
        assert_eq!(map.objects.len(), 5);
        assert_eq!(map.cs, 4.0);
        assert_eq!(map.ar, 9.0);
        let p = analyze_beatmap(&map, &[], 1.0, None);
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

    #[test]
    fn reading_density_uses_actual_ar_window_below_700ms() {
        let mut map = parse_beatmap(MAP);
        // Keep the same spacing and positions: only the visibility window changes.
        for (i, o) in map.objects.iter_mut().enumerate() {
            o.kind = Kind::Circle;
            o.time = i as f64 * 160.0;
            o.end_time = o.time;
        }
        map.ar = 9.0; // 600 ms: four objects fit at 160 ms spacing.
        let ar9 = analyze_beatmap(&map, &[], 1.0, None);
        map.ar = 10.0; // 450 ms: only three objects fit.
        let ar10 = analyze_beatmap(&map, &[], 1.0, None);
        assert!(ar9.raw_demand[7] > ar10.raw_demand[7]);
        // Equal crowding at DT, but less time to decode the same screen.
        let dt = analyze_beatmap(&map, &["DT".into()], 1.5, None);
        assert!((dt.raw_demand[7] / ar10.raw_demand[7] - 1.5f64.powf(0.25)).abs() < 1e-9);
        let custom_dt = analyze_beatmap(&map, &["DT".into()], 1.2, None);
        assert!((custom_dt.raw_demand[7] / ar10.raw_demand[7] - 1.2f64.powf(0.25)).abs() < 1e-9);
    }

    #[test]
    fn reading_keeps_long_sliders_until_their_end_without_a_128_object_cutoff() {
        let mut map = parse_beatmap(MAP);
        map.ar = 10.0;
        map.objects = (0..2000).map(|i| Obj {
            x: 256.0, y: 192.0, time: i as f64 * 100.0, kind: Kind::Circle,
            end_time: i as f64 * 100.0, repeats: 0, length: 0.0, curve_type: 0, control_points: Vec::new(),
        }).collect();
        let circles = analyze_beatmap(&map, &[], 1.0, None);
        let o = &mut map.objects[0];
        o.kind = Kind::Slider;
        o.curve_type = b'L';
        o.control_points = vec![(356.0, 192.0)];
        o.length = 1400.0;
        o.repeats = 100; // 300 seconds, outliving the other 1999 heads.
        let long = analyze_beatmap(&map, &[], 1.0, None);
        assert!(long.raw_demand[7] > circles.raw_demand[7]);
        let dt = analyze_beatmap(&map, &["DT".into()], 1.2, None);
        assert!((dt.raw_demand[7] / long.raw_demand[7] - 1.2f64.powf(0.25)).abs() < 1e-9);
        map.objects[0].repeats = 1;
        let expired = analyze_beatmap(&map, &[], 1.0, None);
        assert_eq!(expired.raw_demand[7], circles.raw_demand[7]);
    }

    #[test]
    fn reading_distinguishes_crossed_order_from_equal_crowding() {
        let make = |xy: &[(f64, f64)]| {
            let mut map = parse_beatmap(MAP);
            map.ar = 9.0;
            map.objects = (0..100).map(|i| Obj {
                x: xy[i % xy.len()].0, y: xy[i % xy.len()].1,
                time: i as f64 * 125.0, end_time: i as f64 * 125.0,
                kind: Kind::Circle, repeats: 0, length: 0.0, curve_type: 0, control_points: Vec::new(),
            }).collect();
            analyze_beatmap(&map, &[], 1.0, None)
        };
        let simple = make(&[(50.0,50.0),(50.0,330.0),(460.0,330.0),(460.0,50.0)]);
        let crossed = make(&[(50.0,50.0),(460.0,330.0),(50.0,330.0),(460.0,50.0)]);
        assert!(crossed.raw_demand[7] > simple.raw_demand[7]);
        assert!(reading_load(6, 0, 0.25, 600.0, false) > reading_load(6, 0, 0.0, 600.0, false));
        // Density still matters independently of the time-pressure term.
        assert!(reading_load(8, 0, 0.0, 600.0, false) > reading_load(4, 0, 0.0, 600.0, false));
    }

    #[test]
    fn accuracy_rewards_pattern_load_but_still_requires_clean_timing() {
        let easy = speed_pattern(500.0, 500.0, 200, &[]);
        let hard = speed_pattern(500.0, 100.0, 200, &[]);
        let clean = Hits { great: 990.0, miss: 0.0, total: 1000.0, accuracy: 0.9933333333 };
        let poor = Hits { great: 500.0, miss: 0.0, total: 1000.0, accuracy: 0.6666666667 };
        let value = |hits: &Hits, p: &Pattern| rating(accuracy_demand(hits, p.great_hit_window).unwrap() * p.accuracy_load_factor, ACCURACY);
        assert_eq!(easy.great_hit_window, hard.great_hit_window);
        assert!(value(&clean, &hard) > value(&clean, &easy));
        assert!(value(&poor, &hard) < value(&clean, &easy));
        assert!(speed_pattern(500.0, 100.0, 4, &[]).accuracy_load_factor < hard.accuracy_load_factor);
        assert!(accuracy_demand(&Hits { great: 0.0, miss: 0.0, total: 0.0, accuracy: 0.0 }, 20.0).is_none());
    }

    #[test]
    fn accuracy_load_is_bounded_and_independent_of_od() {
        assert_eq!(accuracy_load_factor(0.0, 0.0, 0.0, 0.0), 0.5);
        assert_eq!(accuracy_load_factor(100.0, 100.0, 100.0, 100.0), 1.25);
        let baseline = accuracy_load_factor(4.0, 0.0, 0.0, 0.0);
        assert!(accuracy_load_factor(8.0, 0.0, 0.0, 0.0) > baseline);
        assert!(accuracy_load_factor(4.0, 1.0, 0.0, 0.0) > baseline);
        assert!(accuracy_load_factor(4.0, 0.0, 8.0, 0.0) > baseline);
        assert!(accuracy_load_factor(4.0, 0.0, 0.0, 10.0) > baseline);
        let mut map = parse_beatmap(MAP);
        map.od = 4.0;
        let low_od = analyze_beatmap(&map, &[], 1.0, None);
        map.od = 10.0;
        let high_od = analyze_beatmap(&map, &[], 1.0, None);
        assert_eq!(low_od.accuracy_load_factor, high_od.accuracy_load_factor);
        assert_ne!(low_od.great_hit_window, high_od.great_hit_window);
        let encoded = serde_json::to_vec(&high_od).unwrap();
        let cached: Pattern = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(cached.accuracy_load_factor, high_od.accuracy_load_factor);
    }

    #[test]
    fn custom_rate_overrides_mod_defaults_and_changes_pattern() {
        let mods = vec!["DT".to_string()];
        assert_eq!(play_rate(Some(1.2), &mods), 1.2);
        assert_eq!(play_rate(None, &mods), 1.5);
        assert_eq!(play_rate(Some(f64::NAN), &mods), 1.5);
        assert_eq!(play_rate(Some(0.0), &mods), 1.5);
        let mut map = parse_beatmap(MAP);
        for (i, o) in map.objects.iter_mut().enumerate() {
            o.kind = Kind::Circle;
            o.time = i as f64 * 180.0;
            o.end_time = o.time;
        }
        let custom = analyze_beatmap(&map, &mods, 1.2, None);
        let default = analyze_beatmap(&map, &mods, 1.5, None);
        assert_eq!(custom.support[6], 0.0);
        assert!(default.support[6] > 0.0);
        assert!((custom.duration / default.duration - 1.25).abs() < 1e-9);
        assert!((custom.great_hit_window / default.great_hit_window - 1.25).abs() < 1e-9);
        assert_ne!(rate_db_key(1.2), rate_db_key(1.5));
        let play: StdPlay = serde_json::from_str(r#"{"map":"test","mods":["DT"],"rate":1.2}"#).unwrap();
        assert_eq!(play_rate(play.rate, &play.mods), 1.2);
    }

    fn speed_pattern(beat_length: f64, gap: f64, count: usize, mods: &[String]) -> Pattern {
        let mut map = parse_beatmap(MAP);
        map.timing[0].beat_length = beat_length;
        map.objects = (0..count).map(|i| Obj {
            x: if i % 2 == 0 { 100.0 } else { 200.0 },
            y: 192.0,
            time: i as f64 * gap,
            kind: Kind::Circle,
            end_time: i as f64 * gap,
            repeats: 0,
            length: 0.0,
            curve_type: 0,
            control_points: Vec::new(),
        }).collect();
        analyze_beatmap(&map, mods, rate_of(mods), None)
    }

    #[test]
    fn speed_uses_absolute_intervals_independent_of_bpm() {
        let quarter_180 = speed_pattern(60000.0 / 180.0, 60000.0 / 720.0, 16, &[]);
        let half_360 = speed_pattern(60000.0 / 360.0, 60000.0 / 720.0, 16, &[]);
        assert!(quarter_180.raw_demand[6] > 0.0);
        assert_eq!(quarter_180.support[6], half_360.support[6]);
        assert_eq!(quarter_180.raw_demand[6], half_360.raw_demand[6]);
        assert!(speed_pattern(60000.0 / 280.0, 60000.0 / 560.0, 16, &[]).raw_demand[6] > 0.0);
    }

    fn pattern_with_fixed_components(count: usize, gap: f64, cs: f64, spacing: f64, rest_every: Option<usize>) -> Pattern {
        let mut map = parse_beatmap(MAP);
        map.cs = cs;
        map.objects = (0..count).map(|i| {
            let time = i as f64 * gap + rest_every.map(|n| (i / n) as f64 * 2000.0).unwrap_or(0.0);
            Obj { x: if i % 2 == 0 { 80.0 } else { 80.0 + spacing }, y: 192.0,
                time, kind: Kind::Circle, end_time: time, repeats: 0, length: 0.0,
                curve_type: 0, control_points: Vec::new() }
        }).collect();
        // Identical rosu components: differences must come from pattern recognition.
        let diff = pp::OsuDiff { aim: 3.0, speed: 3.0, stars: 5.0, ar: 9.0, great_hit_window: 20.0, large_ticks: 0 };
        analyze_beatmap(&map, &[], 1.0, Some(&diff))
    }

    #[test]
    fn precision_requires_small_targets_and_target_changes_not_just_aim() {
        let compact = pattern_with_fixed_components(200, 150.0, 8.0, 80.0, None);
        let large_targets = pattern_with_fixed_components(200, 150.0, 4.0, 80.0, None);
        let big_jumps = pattern_with_fixed_components(200, 150.0, 8.0, 320.0, None);
        let stationary = pattern_with_fixed_components(200, 150.0, 8.0, 0.0, None);
        assert!(compact.raw_demand[1] > large_targets.raw_demand[1]);
        assert!(compact.raw_demand[1] > big_jumps.raw_demand[1]);
        assert_eq!(stationary.raw_demand[1], 0.0);
        assert!(big_jumps.raw_demand[3] > compact.raw_demand[3]);
    }

    #[test]
    fn speed_and_stamina_distinguish_bursts_sustained_load_and_recovery() {
        let burst = pattern_with_fixed_components(24, 60.0, 4.0, 160.0, None);
        let stream = pattern_with_fixed_components(240, 120.0, 4.0, 160.0, None);
        let fragmented = pattern_with_fixed_components(240, 120.0, 4.0, 160.0, Some(24));
        assert!(burst.raw_demand[6] > stream.raw_demand[6]);
        assert!(stream.raw_demand[8] > burst.raw_demand[8] * 3.0);
        assert!(stream.raw_demand[5] > burst.raw_demand[5] * 3.0);
        assert!(stream.raw_demand[8] > fragmented.raw_demand[8]);
        assert!(stream.raw_demand[5] > fragmented.raw_demand[5] * 3.0);
        let slow = pattern_with_fixed_components(240, 250.0, 4.0, 160.0, None);
        assert_eq!(slow.raw_demand[8], 0.0);
    }

    #[test]
    fn jumps_never_contribute_to_both_wide_and_snap() {
        for angle in [0.0, 0.25, 0.5, 0.500001, 0.75, 1.0] {
            let (snap, wide) = jump_demands(true, 2.0, 10.0, Some(angle), 100.0);
            assert_ne!(snap > 0.0, wide > 0.0);
            assert_eq!(wide > 0.0, angle > 0.5);
        }
        assert_eq!(jump_demands(true, 2.0, 10.0, None, 100.0), (0.0, 0.0));
        assert_eq!(jump_demands(false, 2.0, 10.0, Some(1.0), 100.0), (0.0, 0.0));
    }

    fn tapping_test_event(time: f64, gap: f64, circle_pair: bool) -> Ev {
        Ev { index: 1, time, gap, active: true, circle_pair, fast: false,
            distance: 1.0, velocity: 0.0, internal_angle: 0.0, jump: 0.0, wide: 0.0,
            sustained_tap: if circle_pair { 1000.0 / gap } else { 0.0 }, precision: 0.0 }
    }

    #[test]
    fn tapping_weight_is_linear_and_x_is_time_based() {
        for (gap, weight) in [(100.0, 1.0), (180.0, 1.0), (215.0, 0.5), (250.0, 0.0), (300.0, 0.0)] {
            assert_eq!(tapping_weight(&tapping_test_event(gap, gap, true)), weight);
        }
        assert_eq!(tapping_weight(&tapping_test_event(100.0, 100.0, false)), 0.0);
        let run = |count: usize, gap: f64| (1..=count).map(|i| tapping_test_event(i as f64 * gap, gap, true)).collect::<Vec<_>>();
        let a = tapping_peak(&run(100, 100.0));
        let b = tapping_peak(&run(200, 50.0));
        assert!((a - b).abs() < 1e-9); // twice as many notes, same ten seconds of full load
        assert!((a - 60.0 * (1.0 - (-10.0f64 / 60.0).exp())).abs() < 1e-9);
        assert!(tapping_peak(&run(900, 100.0)) > tapping_peak(&run(150, 100.0)));
        assert!(tapping_peak(&run(6000, 100.0)) < 60.0);
        assert!(pattern_with_fixed_components(200, 215.0, 4.0, 160.0, None).raw_demand[8] > 0.0);
    }

    #[test]
    fn tapping_runs_keep_residual_x_and_sliders_allow_faster_recovery() {
        let repeated = |pause: f64| {
            let mut events = Vec::new();
            let mut time = 0.0;
            for block in 0..2 {
                if block == 1 { time += pause; events.push(tapping_test_event(time, pause, false)); }
                for _ in 0..50 { time += 100.0; events.push(tapping_test_event(time, 100.0, true)); }
            }
            events
        };
        let short = tapping_peak(&repeated(500.0));
        let long = tapping_peak(&repeated(25000.0));
        assert!(short > long);
        let mut events = repeated(500.0);
        let before = tapping_peak(&events);
        events.push(tapping_test_event(events.last().unwrap().time + 120000.0, 120000.0, false));
        assert_eq!(tapping_peak(&events), before); // empty time adds no stamina
    }

    fn flow_pattern(beat_length: f64, gap: f64, rate: f64, zigzag: bool) -> Pattern {
        let mut map = parse_beatmap(MAP);
        map.timing[0].beat_length = beat_length;
        map.objects = (0..24).map(|i| {
            // Smooth circular path stays on the playfield; zigzag reverses every movement.
            let angle = i as f64 * 0.4;
            let (x, y) = if zigzag {
                (if i % 2 == 0 { 100.0 } else { 200.0 }, 192.0)
            } else {
                (256.0 + 150.0 * angle.cos(), 192.0 + 150.0 * angle.sin())
            };
            Obj { x, y, time: i as f64 * gap, kind: Kind::Circle, end_time: i as f64 * gap,
                repeats: 0, length: 0.0, curve_type: 0, control_points: Vec::new() }
        }).collect();
        analyze_beatmap(&map, &[], rate, None)
    }

    #[test]
    fn wide_and_snap_distinguish_continuous_motion_from_reversals() {
        let mut map = parse_beatmap(MAP);
        // Three wide transitions per passage; rests separate passages and reset velocity history.
        let make = |reverse: bool| (0..48).map(|i| {
            let step = i % 4;
            let x = if reverse { if step % 2 == 0 { 80.0 } else { 400.0 } }
                else { 40.0 + step as f64 * 140.0 };
            let time = (i / 4) as f64 * 2000.0 + step as f64 * 200.0;
            Obj { x, y: 192.0, time, kind: Kind::Circle, end_time: time,
                repeats: 0, length: 0.0, curve_type: 0, control_points: Vec::new() }
        }).collect();
        map.objects = make(false);
        let continuous = analyze_beatmap(&map, &[], 1.0, None);
        map.objects = make(true);
        let reversing = analyze_beatmap(&map, &[], 1.0, None);
        assert!(continuous.raw_demand[0] > 0.0);
        assert_eq!(continuous.raw_demand[3], 0.0);
        assert_eq!(reversing.raw_demand[0], 0.0);
        assert!(reversing.raw_demand[3] > 0.0);
        map.timing[0].beat_length = 80.0;
        let different_bpm = analyze_beatmap(&map, &[], 1.0, None);
        assert_eq!(reversing.raw_demand[3], different_bpm.raw_demand[3]);
        let faster = analyze_beatmap(&map, &[], 1.2, None);
        assert!(faster.raw_demand[3] > reversing.raw_demand[3]);
    }

    #[test]
    fn flow_is_independent_of_bpm_including_shared_activity_and_angles() {
        let slow_bpm = flow_pattern(500.0, 200.0, 1.0, false);
        // Previously fails both the direct 0.6-beat and shared 2-beat gates.
        let fast_bpm = flow_pattern(80.0, 200.0, 1.0, false);
        assert!(slow_bpm.raw_demand[10] > 0.0);
        assert_eq!(slow_bpm.support[10], fast_bpm.support[10]);
        assert_eq!(slow_bpm.raw_demand[10], fast_bpm.raw_demand[10]);
        assert!(flow_pattern(500.0, 350.0, 1.0, false).support[10] > 0.0);
        assert_eq!(flow_pattern(500.0, 351.0, 1.0, false).support[10], 0.0);
        assert_eq!(flow_pattern(500.0, 400.0, 1.0, false).support[10], 0.0);
        assert!(flow_pattern(500.0, 400.0, 1.2, false).support[10] > 0.0);
        assert_eq!(flow_pattern(500.0, 100.0, 1.0, true).support[10], 0.0);
    }

    #[test]
    fn speed_interval_boundary_mods_and_minimum_run() {
        assert!(speed_pattern(500.0, 125.0, 4, &[]).raw_demand[6] > 0.0);
        assert_eq!(speed_pattern(300.0, 126.0, 16, &[]).support[6], 0.0);
        assert_eq!(speed_pattern(500.0, 83.0, 3, &[]).support[6], 0.0);
        assert!(speed_pattern(500.0, 150.0, 16, &["DT".into()]).raw_demand[6] > 0.0);
        assert_eq!(speed_pattern(200.0, 100.0, 16, &["HT".into()]).support[6], 0.0);
    }

    #[test]
    fn speed_accepts_slower_beat_relative_runs_with_absolute_ceiling() {
        assert!(speed_pattern(500.0, 150.0, 16, &[]).support[6] > 0.0);
        assert_eq!(speed_pattern(300.0, 150.0, 16, &[]).support[6], 0.0);
        assert!(speed_pattern(600.0, 180.0, 16, &[]).support[6] > 0.0);
        assert_eq!(speed_pattern(600.0, 181.0, 16, &[]).support[6], 0.0);
        assert!(speed_pattern(500.0, 170.0, 16, &[]).support[6] > 0.0);
        assert_eq!(speed_pattern(500.0, 171.0, 16, &[]).support[6], 0.0);
        // Both gap and beat length must be rate-adjusted; original gap exceeds 180 ms.
        assert!(speed_pattern(600.0, 200.0, 16, &["DT".into()]).support[6] > 0.0);
        assert_eq!(speed_pattern(400.0, 200.0, 16, &["DT".into()]).support[6], 0.0);
    }
}
