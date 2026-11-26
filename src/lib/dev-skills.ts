/* dev-skills.ts — DEV ONLY. Fakes osu!mania plays (and the charts they're on) so the Skills page
 * can be looked at without a real mania history. Wired in main.ts behind import.meta.env.DEV and
 * exposed as window.fumenPreviewMania(); a reload (or api reload) restores the real data.
 *
 * Not shipped: this module is only imported under the DEV guard.
 */
import { devSetHistory } from "./history";
import type { ComputedOsuStandard, OsuPractice, OsuStandardComparison } from "./osu-standard";
import { SKILLSETS } from "./skills";
import { setSnapshot, store } from "./store";
import type { History, MapRow, Play, SetRow, Snapshot } from "./types";

/** The eleven osu!standard skillsets (id, label, group), mirroring src-tauri/src/osu_skills.rs. */
const STD_SKILLS: [string, string, string][] = [
  ["wide-angle-aim", "Wide angle aim", "Aim"],
  ["precision", "Precision", "Aim"],
  ["accuracy", "Accuracy", "Accuracy"],
  ["snap-aim", "Snap aim", "Aim"],
  ["finger-control", "Finger control", "Tapping"],
  ["aim-stamina", "Aim stamina", "Aim"],
  ["speed", "Speed", "Tapping"],
  ["density-reading", "Density reading", "Reading"],
  ["tapping-stamina", "Tapping stamina", "Tapping"],
  ["slider-control", "Slider control", "Tech"],
  ["flow-aim", "Flow aim", "Aim"],
];
const round = (v: number): number => Math.round(v * 100) / 100;

/** Injects fake osu!standard skills (yours + a compared player) into the open Skills view so the
 * osu!standard tab — radar, list, graph and compare — can be looked at without an osu! login. */
export function previewStd(): void {
  const view = (window as unknown as { __fumenSkills?: { devSetStd: (s: ComputedOsuStandard, c: OsuStandardComparison | null) => void; devSetRecommendations: (p: OsuPractice) => void } }).__fumenSkills;
  if (!view) {
    // eslint-disable-next-line no-console
    console.warn("[fumenPreviewStd] open the Skills page once first, then call this again.");
    return;
  }
  const ownSkills = STD_SKILLS.map(([id, label, group], k) => ({ id, label, group, description: "Preview skill.", value: round(42 + 8 * Math.sin(k)), confidence: 0.85, best: [] as never[] }));
  const trend = Array.from({ length: 8 }, (_, i) => {
    const vals = ownSkills.map((s) => round(s.value * (0.6 + 0.05 * i)));
    return { date: Date.now() - (7 - i) * 18 * 86_400_000, values: [round(Math.max(...vals)), ...vals] };
  });
  const own: ComputedOsuStandard = {
    skills: ownSkills,
    overall: round(Math.max(...ownSkills.map((s) => s.value))),
    trend,
    summary: { scoreCount: 60, analyzedBeatmapCount: 24, metadataOnlyCount: 0, failedDifficultyCount: 0, method: "preview", engine: "preview", comfortStars: 5.2, peakStars: 6.4, preferredAr: [9, 9.6], typicalLength: 95 },
  };
  const plays = Array.from({ length: 8 }, (_, i) => ({ beatmapId: 1000 + i, setId: 1 + i, title: `Rival map ${i + 1}`, titleUnicode: "", artist: "Preview", artistUnicode: "", version: "Insane", mods: [] as string[], acc: 0.985, date: Date.now() - i * 3_600_000 }));
  const theirSkills = STD_SKILLS.map(([id, label, group], k) => ({
    id,
    label,
    group,
    description: "Preview skill.",
    value: round(38 + 9 * Math.cos(k)),
    confidence: 0.7,
    evidence: [0, 1, 2].map((j) => ({ play: (k + j) % plays.length, worth: round(40 + 9 * Math.cos(k) + j) })),
  }));
  const cmp: OsuStandardComparison = {
    player: { id: 1, name: "PreviewRival", rank: 1234, pp: 8800, accuracy: 0.991, playCount: 50_000 },
    skills: theirSkills,
    overall: round(Math.max(...theirSkills.map((s) => s.value))),
    plays,
  };
  // the four weakest skillsets, each with some practice maps
  const weak = [...ownSkills].sort((a, b) => a.value - b.value).slice(0, 4);
  const practice: OsuPractice = weak.map((s) => {
    const from = s.value;
    return {
      skill: s.id,
      label: s.label,
      from,
      lo: round(from * 1.01),
      hi: round(from * 1.12 + 2),
      picks: Array.from({ length: 4 }, (_, i) => ({
        setId: 200 + i,
        beatmapId: 900 + i,
        title: `Practice map ${i + 1}`,
        titleUnicode: "",
        artist: "Preview",
        artistUnicode: "",
        version: "Extra",
        value: round(from + 1 + i * 0.4),
        mods: i % 2 ? ["DT"] : i % 3 ? ["HD"] : [],
        rate: i % 2 ? 1.5 : 1,
        after: round(from + (1 + i * 0.4) * 0.3),
      })),
    };
  });
  view.devSetRecommendations(practice);
  view.devSetStd(own, cmp);
  // eslint-disable-next-line no-console
  console.info("[fumenPreviewStd] fake osu!standard skills + comparison + practice injected. Reload to restore.");
}

/** charts faked per skillset (so the radar has variety and the skills fill up past FEW_PLAYS) */
const PER_SKILL = 5;

/** Injects fake 4K mania plays and charts, then lets the Skills view redraw. */
export function previewMania(): void {
  const base = store.snap;
  const sets: SetRow[] = base ? [...base.sets] : [];
  const maps: MapRow[] = base ? [...base.maps] : [];
  const plays: Play[] = [];
  let n = 0;

  for (let si = 0; si < SKILLSETS.length; si++) {
    for (let c = 0; c < PER_SKILL; c++) {
      const id = `fake-${n}`;
      const setIdx = sets.length;
      const mapIdx = maps.length;
      const strength = 16 + c * 2 + si; // 16–30ish, so skillsets differ
      sets.push({ id: `fake-set-${n}`, status: 1, title: `${SKILLSETS[si]} chart ${c + 1}`, artist: "Preview", creator: "fumen", maps: [mapIdx] });
      maps.push({ set: setIdx, id, version: `${SKILLSETS[si]} ${strength}`, mode: 3, status: 1, sr: 4 + si * 0.3, bpm: 170 + c * 10, length: 90_000, objects: 1200, holds: 150, hp: 8, cs: 4, od: 8, ar: 5, notesKey: `fake-notes-${n}` });

      // two passes on the chart, the better one a touch higher
      for (let p = 0; p < 2; p++) {
        const ssr = ssrFor(si, strength + p * 0.4);
        plays.push({
          mode: 3,
          map: id,
          date: Date.now() - (n * 2 + p) * 3_600_000 - si * 20 * 86_400_000,
          rank: 4,
          acc: 0.95 + p * 0.01,
          score: 940_000,
          combo: 1200,
          source: "lazer",
          mods: [],
          wife: 0.93 + p * 0.01,
          ssr,
        });
      }
      n++;
    }
  }

  const snap: Snapshot = {
    source: "lazer",
    sets,
    maps,
    collections: base?.collections ?? [],
    user: base?.user,
    mania: base?.mania ?? {},
    scannedAt: base?.scannedAt,
  };
  setSnapshot(snap);
  plays.sort((a, b) => b.date - a.date);
  const history: History = { plays, mostPlayed: [], stats: {}, beatmaps: [] };
  devSetHistory(history);
  // eslint-disable-next-line no-console
  console.info(`[fumenPreviewMania] injected ${plays.length} fake mania passes on ${n} charts. Reload to restore.`);
}

/** An SSR array ([overall, ...SKILLSETS]) that leans on skillset `si`. */
function ssrFor(si: number, strength: number): number[] {
  const out = [0];
  for (let k = 0; k < SKILLSETS.length; k++) {
    const lean = k === si ? 1 : 0.55 + 0.1 * Math.cos(k + si);
    out.push(Math.round(strength * lean * 10) / 10);
  }
  out[0] = Math.max(...out.slice(1)); // overall ≈ the strongest
  return out;
}
