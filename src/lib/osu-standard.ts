/* osu-standard.ts — your osu!standard skills.
 *
 * Like osu!mania's skills (lib/skills.ts), all the compute lives in the backend and is cached there
 * (src-tauri/src/osu_skills.rs, command osu_standard); this only gathers your standard passes, asks
 * the backend, and attaches each evidence play back for the view (views/skills.ts) to show.
 */
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { loadHistory, mapIndexOf } from "./history";
import { likeness, MIN_SCORE, print, type Print } from "./similar";
import { store } from "./store";
import type { Play } from "./types";

/** One osu!standard pass, as the backend's model reads it. */
export interface OsuStandardPlay {
  /** the library difficulty id */
  map: string;
  mods: string[];
  rate?: number;
  /** judgement counts (great/ok/meh/miss) */
  hits: Record<string, number>;
  /** 0–1 */
  acc: number;
  /** unix ms, for the skills-over-time graph */
  date: number;
}

/** A point on the skills-over-time graph: [overall, ...the eleven skillsets]. */
export interface OsuTrendPoint {
  date: number;
  values: number[];
}

/** A skillset as the backend returns it: its value, how much evidence stands behind it, and the
 * plays that carry it (by index into the passes sent). */
export interface OsuSkill {
  id: string;
  label: string;
  group: string;
  description: string;
  /** 0–100 on the provisional scale; 0 when nothing demonstrates it */
  value: number;
  /** 0–1 */
  confidence: number;
  evidence: { play: number; worth: number }[];
}

export interface OsuStandardSummary {
  scoreCount: number;
  analyzedBeatmapCount: number;
  metadataOnlyCount: number;
  failedDifficultyCount: number;
  method: string;
  engine: string;
  comfortStars: number;
  peakStars: number;
  preferredAr: [number, number];
  typicalLength: number;
}

/** What the backend command returns. */
export interface OsuStandardResult {
  skills: OsuSkill[];
  overall: number;
  trend: OsuTrendPoint[];
  summary: OsuStandardSummary;
}

/** A skillset with its evidence resolved to the actual plays, for the view. */
export interface OsuSkillWithPlays {
  id: string;
  label: string;
  group: string;
  description: string;
  value: number;
  confidence: number;
  best: { play: Play; worth: number }[];
}

export interface ComputedOsuStandard {
  skills: OsuSkillWithPlays[];
  overall: number;
  trend: OsuTrendPoint[];
  summary: OsuStandardSummary;
}

/* ---------- practice: what to play next (recommendations) ---------- */

/** One recommended chart to practise a skillset on, with the mods it's suggested at. */
export interface OsuRecommendation {
  setId: number;
  beatmapId: number;
  title: string;
  titleUnicode: string;
  artist: string;
  artistUnicode: string;
  version: string;
  /** the chart's demand in the skillset at the suggested mods (0–100) */
  value: number;
  mods: string[];
  rate: number;
  /** what passing it clean would make of the skillset */
  after: number;
  /** true when it's already in your library (the view opens it instead of offering to add it) */
  inLibrary?: boolean;
}

/** A library difficulty chosen in the frontend (most like the ones you play), for the backend to
 * score locally. The backend resolves its hash from `mapId` and reuses the cached pattern. */
export interface LibCandidate {
  mapId: string;
  beatmapId: number;
  setId: number;
  title: string;
  titleUnicode: string;
  artist: string;
  artistUnicode: string;
  version: string;
  stars: number;
  /** 0–1, how like your maps it is */
  similarity: number;
}

/** A weak skillset and the charts to practise it on, best first. */
export interface OsuPracticeGroup {
  skill: string;
  label: string;
  /** your value now (null without passes) and the aim band it suggests into */
  from: number | null;
  lo: number;
  hi: number;
  picks: OsuRecommendation[];
}

export type OsuPractice = OsuPracticeGroup[];

/** how many library difficulties to put forward as candidates (the backend scores them, cheaply,
 * since they reuse your own skills' cached patterns) */
const LIB_CANDIDATES = 220;

/** The library std difficulties most like the ones you play well, as candidates for the backend to
 * score. Metadata only (lib/similar.ts) — the heavy per-chart work stays in the backend. */
function libraryCandidates(result: ComputedOsuStandard): LibCandidate[] {
  const snap = store.snap;
  if (!snap) return [];
  // anchors: the maps behind your best plays — what you already play well
  const anchorIds = new Set<string>();
  for (const s of result.skills) for (const b of s.best) if (b.play.map) anchorIds.add(b.play.map);
  const anchors: Print[] = [];
  for (const id of anchorIds) {
    const i = mapIndexOf(id);
    if (i !== undefined && snap.maps[i].mode === 0) anchors.push(print(snap.maps[i]));
  }
  if (!anchors.length) return [];
  const comfort = result.summary.comfortStars;
  const [lo, hi] = [comfort * 0.45, comfort + 0.5]; // the band discovery uses too
  const out: { sim: number; c: LibCandidate }[] = [];
  for (const m of snap.maps) {
    if (m.mode !== 0 || m.hidden || !m.onlineId) continue;
    if (!(m.sr >= lo && m.sr <= hi)) continue;
    const p = print(m);
    let sim = 0;
    for (const a of anchors) sim = Math.max(sim, likeness(a, p).score);
    if (sim < MIN_SCORE) continue;
    const set = snap.sets[m.set];
    out.push({
      sim,
      c: { mapId: m.id, beatmapId: m.onlineId, setId: set.onlineId ?? 0, title: set.title, titleUnicode: set.titleUnicode ?? "", artist: set.artist, artistUnicode: set.artistUnicode ?? "", version: m.version, stars: m.sr, similarity: Math.round(sim * 1000) / 1000 },
    });
  }
  out.sort((a, b) => b.sim - a.sim);
  return out.slice(0, LIB_CANDIDATES).map((x) => x.c);
}

/** Charts to practise your weak osu!standard skillsets on (recommend.rs): your library first — the
 * difficulties most like the ones you play — then ranked maps you don't have for discovery. Your
 * skill values (worked out by the backend) and the maps you've played guide it. */
export async function osuRecommendations(result: ComputedOsuStandard, played: number[]): Promise<OsuPractice> {
  return api.osuRecommendations(
    result.skills.map((s) => ({ id: s.id, value: s.confidence > 0 ? s.value : 0 })),
    result.summary.comfortStars,
    result.summary.peakStars,
    libraryCandidates(result),
    played,
  );
}

/* ---------- comparing with another player ---------- */

export interface ComparedOsuPlayer {
  id: number;
  name: string;
  avatar?: string;
  country?: string;
  rank?: number;
  countryRank?: number;
  pp?: number;
  accuracy?: number;
  playCount?: number;
}

/** One of their plays, for display (osu!'s cover; the library id when you have the chart). */
export interface ComparedOsuPlay {
  beatmapId: number;
  setId?: number;
  title: string;
  titleUnicode: string;
  artist: string;
  artistUnicode: string;
  version: string;
  mods: string[];
  /** 0–1 */
  acc: number;
  date: number;
  library?: string;
}

/** Another player's osu!standard skills, worked out the same way as yours. */
export interface OsuStandardComparison {
  player: ComparedOsuPlayer;
  skills: OsuSkill[];
  overall: number;
  plays: ComparedOsuPlay[];
}

export interface OsuCompareProgress {
  stage: "player" | "scores" | "charts";
  done: number;
  total: number;
}

/** Compares your osu!standard skills with another player's (backend: player::build_std). */
export async function compareOsuStandard(name: string, onProgress?: (p: OsuCompareProgress) => void): Promise<OsuStandardComparison> {
  const stop = await listen<OsuCompareProgress>("compare-progress", (e) => onProgress?.(e.payload));
  try {
    return await api.compareOsuStandard(name.trim());
  } finally {
    stop();
  }
}

/** Your osu!standard skills: the standard passes on library difficulties, rated by the backend. */
export async function computeOsuStandard(): Promise<ComputedOsuStandard> {
  // loadHistory (not api.getHistory) so the goals store's history is populated too — the standard
  // goals read osu! pp/rank from it, and they render as soon as the skills are done
  const history = await loadHistory();
  const plays = history.plays.filter((p) => p.mode === 0 && p.map && p.failed !== true);
  const result = await api.osuStandard(plays.map((p) => ({ map: p.map as string, mods: p.mods ?? [], rate: p.rate, hits: p.hits ?? {}, acc: p.acc, date: p.date })));
  const skills: OsuSkillWithPlays[] = result.skills.map((s) => ({
    id: s.id,
    label: s.label,
    group: s.group,
    description: s.description,
    value: s.value,
    confidence: s.confidence,
    // evidence.play indexes the passes we sent (same order as `plays`)
    best: s.evidence.map((e) => ({ play: plays[e.play], worth: e.worth })).filter((b) => !!b.play),
  }));
  return { skills, overall: result.overall, trend: result.trend, summary: result.summary };
}
