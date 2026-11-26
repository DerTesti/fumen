/* osu-standard.ts — your osu!standard skills.
 *
 * Like osu!mania's skills (lib/skills.ts), all the compute lives in the backend and is cached there
 * (src-tauri/src/osu_skills.rs, command osu_standard); this only gathers your standard passes, asks
 * the backend, and attaches each evidence play back for the view (views/skills.ts) to show.
 */
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { Play } from "./types";

/** One osu!standard pass, as the backend's model reads it. */
export interface OsuStandardPlay {
  /** the library difficulty id */
  map: string;
  mods: string[];
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
  const history = await api.getHistory();
  const plays = history.plays.filter((p) => p.mode === 0 && p.map && p.failed !== true);
  const result = await api.osuStandard(plays.map((p) => ({ map: p.map as string, mods: p.mods ?? [], hits: p.hits ?? {}, acc: p.acc, date: p.date })));
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
