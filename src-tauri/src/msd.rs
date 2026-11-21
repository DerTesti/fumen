//! MSD: Etterna's difficulty rating for osu!mania charts, from its own calculator (MinaCalc 527,
//! compiled in from `minacalc/`, see build.rs): an overall number and one per skillset (stream,
//! jumpstream, handstream, stamina, jackspeed, chordjack, technical), at any rate.
//!
//! MinaCalc reads rows: the columns pressed at a moment and when. Long notes count by their
//! heads, as when Etterna rates an osu! chart. It's made for 4K; Fumen rates 4K to 7K with it
//! (MinaCalc has its own logic for 5K, 6K and 7K) and nothing else.

use std::ffi::{c_float, c_int, c_uint, c_void};

use serde::Serialize;

use crate::chart::Chart;

#[repr(C)]
#[derive(Clone, Copy)]
struct NoteInfo {
    notes: c_uint,
    row_time: c_float,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct Ssr {
    overall: c_float,
    stream: c_float,
    jumpstream: c_float,
    handstream: c_float,
    stamina: c_float,
    jackspeed: c_float,
    chordjack: c_float,
    technical: c_float,
}

extern "C" {
    #[cfg(test)]
    fn calc_version() -> c_int;
    fn create_calc() -> *mut c_void;
    fn destroy_calc(calc: *mut c_void);
    /// mode 0: MSD (the chart's difficulty), 1: SSR (a score's rating)
    fn calc_at_rate(calc: *mut c_void, rows: *mut NoteInfo, num_rows: usize, music_rate: c_float, score_goal: c_float, keycount: c_uint, mode: c_int) -> Ssr;
}

/// A chart's MSD at a rate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Msd {
    pub overall: f32,
    pub stream: f32,
    pub jumpstream: f32,
    pub handstream: f32,
    pub stamina: f32,
    pub jackspeed: f32,
    pub chordjack: f32,
    pub technical: f32,
}

impl Msd {
    pub const SKILLS: [&'static str; 7] = ["stream", "jumpstream", "handstream", "stamina", "jackspeed", "chordjack", "technical"];

    fn skills(&self) -> [f32; 7] {
        [self.stream, self.jumpstream, self.handstream, self.stamina, self.jackspeed, self.chordjack, self.technical]
    }

    /// The skillset the chart asks most of.
    pub fn top_skill(&self) -> &'static str {
        let s = self.skills();
        let mut best = 0;
        for i in 1..s.len() {
            if s[i] > s[best] {
                best = i;
            }
        }
        Self::SKILLS[best]
    }
}

/// One calculator per thread (it keeps state between calls and isn't shared).
struct Calc(*mut c_void);

impl Drop for Calc {
    fn drop(&mut self) {
        // SAFETY: made by create_calc, dropped once
        unsafe { destroy_calc(self.0) }
    }
}

thread_local! {
    static CALC: Calc = Calc(unsafe { create_calc() });
}

/// MinaCalc's version (527).
#[cfg(test)]
pub fn version() -> i32 {
    // SAFETY: no arguments, no state
    unsafe { calc_version() }
}

/// The chart's rows for MinaCalc: the columns whose notes (or long note heads) start at each
/// moment, seconds.
fn rows(chart: &Chart) -> Vec<NoteInfo> {
    let mut out: Vec<NoteInfo> = Vec::new();
    let mut last = i32::MIN;
    let mut heads: Vec<(i32, u8)> = chart.objects.iter().map(|o| (o.time, o.lane)).collect();
    heads.sort_unstable();
    for (time, lane) in heads {
        let bit = 1u32 << lane.min(31);
        match out.last_mut() {
            Some(row) if time == last => row.notes |= bit,
            _ => {
                out.push(NoteInfo { notes: bit, row_time: time as f32 / 1000.0 });
                last = time;
            }
        }
    }
    out
}

/// A mania chart's MSD at `rate` (0.7 to 2.0 is what Etterna lists); None for other modes, key
/// counts outside 4K to 7K and charts too short to rate.
pub fn rate(chart: &Chart, rate: f64) -> Option<Msd> {
    if chart.mode != 3 || !(4..=7).contains(&chart.keys) {
        return None;
    }
    let mut rows = rows(chart);
    if rows.len() < 2 {
        return None;
    }
    let ssr = CALC.with(|c| {
        // SAFETY: the rows live through the call, the calculator is this thread's own
        unsafe { calc_at_rate(c.0, rows.as_mut_ptr(), rows.len(), rate.clamp(0.5, 3.0) as f32, 0.93, chart.keys as c_uint, 0) }
    });
    let r2 = |v: f32| if v.is_finite() { (v * 100.0).round() / 100.0 } else { 0.0 };
    let msd = Msd {
        overall: r2(ssr.overall),
        stream: r2(ssr.stream),
        jumpstream: r2(ssr.jumpstream),
        handstream: r2(ssr.handstream),
        stamina: r2(ssr.stamina),
        jackspeed: r2(ssr.jackspeed),
        chordjack: r2(ssr.chordjack),
        technical: r2(ssr.technical),
    };
    (msd.overall > 0.0).then_some(msd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::parse;

    fn chart(keys: u8, notes: &[(i32, u8)]) -> Chart {
        let width = 512 / keys as i32;
        let objects: String = notes.iter().map(|&(t, lane)| format!("{},192,{t},1,0,0:0:0:0:\n", lane as i32 * width + width / 2)).collect();
        parse(&format!("osu file format v14\n\n[General]\nMode: 3\n\n[Difficulty]\nCircleSize:{keys}\nOverallDifficulty:8\n\n[TimingPoints]\n0,250,4,2,0,100,1,0\n\n[HitObjects]\n{objects}"))
    }

    #[test]
    fn version_is_527() {
        assert_eq!(version(), 527);
    }

    #[test]
    fn rows_join_notes_at_one_moment() {
        let c = chart(4, &[(0, 0), (0, 2), (125, 1), (250, 3)]);
        let r = rows(&c);
        assert_eq!(r.iter().map(|r| r.notes).collect::<Vec<_>>(), vec![0b0101, 0b0010, 0b1000]);
        assert_eq!(r[1].row_time, 0.125);
    }

    #[test]
    fn faster_is_harder() {
        // a minute of 1/4 jumpstream at 180 BPM
        let notes: Vec<(i32, u8)> = (0..720)
            .flat_map(|i| {
                let t = i * 83;
                let a = (i % 4) as u8;
                if i % 2 == 0 { vec![(t, a), (t, (a + 2) % 4)] } else { vec![(t, a)] }
            })
            .collect();
        let c = chart(4, &notes);
        let one = rate(&c, 1.0).expect("rated");
        let fast = rate(&c, 1.5).expect("rated");
        assert!(one.overall > 5.0 && one.overall < 40.0, "{one:?}");
        assert!(fast.overall > one.overall * 1.2, "{one:?} {fast:?}");
        assert!(rate(&chart(4, &[(0, 0)]), 1.0).is_none());
        assert!(rate(&chart(9, &notes.iter().map(|&(t, l)| (t, l)).collect::<Vec<_>>()), 1.0).is_none());
    }

    /// Fumen's MSD against a reference build of the same sources (mania-ai's DLL), on real charts:
    /// `FUMEN_MSD_REF=<json: [{"p": .osu path, "m": overall at 1×}]> cargo test msd::tests::matches_reference -- --ignored`
    #[test]
    #[ignore = "needs real charts and reference values"]
    fn matches_reference() {
        let Some(path) = std::env::var_os("FUMEN_MSD_REF") else { return };
        let refs: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for r in refs {
            let text = std::fs::read_to_string(r["p"].as_str().unwrap()).unwrap();
            let got = rate(&parse(&text), 1.0).map_or(0.0, |m| m.overall);
            let want = r["m"].as_f64().unwrap() as f32;
            eprintln!("{got:6.2} {want:6.2}");
            assert!((got - want).abs() < 0.02, "{got} vs {want}");
        }
    }
}
