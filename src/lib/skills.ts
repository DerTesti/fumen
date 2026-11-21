/* skills.ts — how good you are at osu!mania, the way Etterna measures it.
 *
 * Every pass is rated by Etterna's own calculator, MinaCalc (src-tauri/src/msd.rs): the chart at
 * the rate it was played, for the accuracy it got (its Wife%, worked out from the judgements,
 * src-tauri/src/wife.rs). That's the play's SSR, one number per skillset: stream, jumpstream,
 * handstream, stamina, jackspeed, chordjack and technical. A skillset's skill is Etterna's own
 * blend of all your plays' SSRs in it (`aggregate`): your best count the most, and it takes a fair
 * number of good plays to get to your level, so one lucky play doesn't make it. Overall blends the
 * seven the same way. Each chart counts once per rate (its best play there), only passes on
 * osu!mania charts in your library, 4K to 7K, one key mode at a time, and not with mods that change
 * what you play (Hold Off, Invert, Random, …).
 *
 * Practice suggestions aim a little above your best plays in a skillset: charts in the library
 * that are mostly about it and whose MSD there lands in that band, at 1× or at a rate.
 */
import type { MapRow, MsdSkill, Play } from "./types";

/** Etterna's skillsets, in the order of the SSR arrays (after overall) */
export const SKILLSETS: readonly MsdSkill[] = ["stream", "jumpstream", "handstream", "stamina", "jackspeed", "chordjack", "technical"];

/** their names as Etterna shows them */
export const SKILL_LABEL: Record<MsdSkill, string> = {
  stream: "Stream",
  jumpstream: "Jumpstream",
  handstream: "Handstream",
  stamina: "Stamina",
  jackspeed: "Jackspeed",
  chordjack: "Chordjack",
  technical: "Technical",
};

/** below this many plays in a key mode the skills are still filling up (Etterna's blend needs
 * a fair number of good plays) */
export const FEW_PLAYS = 25;
/** plays shown behind a skillset */
export const TOP = 10;
/** mods that change what's played beyond the rate: such plays don't count */
const UNRATED = new Set(["HO", "IN", "NR", "RD", "EZ", "DS", "CO", "AT", "CN", "RX", "AP", "AS", "WU", "WD", "DA", "SG", "TP"]);

export const rateOf = (p: Play): number => p.rate ?? 1;

/** Played with mods that change what's played beyond the rate (Hold Off, Easy, Autopilot, …). */
export const changesChart = (p: Play): boolean => !!p.mods?.some((m) => UNRATED.has(m));

export const keysOf = (map: MapRow): number => Math.round(map.cs);

/* ---------- Etterna's blend ---------- */

/** erfc, to about 1e-7 (Numerical Recipes' erfcc) */
function erfc(x: number): number {
  const z = Math.abs(x);
  const t = 1 / (1 + 0.5 * z);
  const r =
    t *
    Math.exp(
      -z * z - 1.26551223 + t * (1.00002368 + t * (0.37409196 + t * (0.09678418 + t * (-0.18628806 + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277)))))))),
    );
  return x >= 0 ? r : 2 - r;
}

/**
 * Etterna's way of turning many ratings into one (MinaCalc's `aggregate_skill`): the highest
 * rating the values still "carry", a binary search where every value above the rating adds more
 * the further above it is, until that passes 2^(rating / 10). Skillsets use it with ×1.05, overall
 * on the seven skillsets with ×1.125, as Etterna does. Values at or under the rating add nothing,
 * so with them sorted highest first (`sorted`) the sum stops there.
 */
export function aggregate(values: readonly number[], deltaMultiplier = 0.1, resultMultiplier = 1.05, sorted = false): number {
  if (!values.length) return 0;
  const vs = sorted ? values : [...values].sort((a, b) => b - a);
  let rating = 0;
  let resolution = 10.24;
  for (let i = 0; i < 11; i++) {
    let sum: number;
    do {
      rating += resolution;
      sum = 0;
      for (const v of vs) {
        if (v <= rating) break;
        sum += 2 / erfc(deltaMultiplier * (v - rating)) - 2;
      }
    } while (Math.pow(2, rating * 0.1) < sum);
    rating -= resolution;
    resolution /= 2;
  }
  rating += resolution * 2;
  return Math.min(100, Math.max(0, rating * resultMultiplier));
}

/* ---------- your plays ---------- */

export interface Counted {
  play: Play;
  map: MapRow;
  mapIdx: number;
  /** what it's worth: overall, then the skillsets in SKILLSETS' order */
  ssr: number[];
  /** its Wife%, 0–1 */
  wife: number;
}

/** A play's SSR in skillset k (or overall at -1). */
export const ssrOf = (c: Counted, k: number): number => c.ssr[k + 1] ?? 0;

/** A play's library index when it counts for the skills in a key mode: a rated osu!mania pass on
 * a difficulty with that many keys, without mods that change the chart. */
function counting(play: Play, mapIndex: (id: string | undefined) => number | undefined, maps: MapRow[], keys: number): number | undefined {
  if (play.failed || play.mode !== 3 || !play.ssr || play.wife === undefined || changesChart(play)) return undefined;
  const mapIdx = mapIndex(play.map);
  if (mapIdx === undefined) return undefined;
  const map = maps[mapIdx];
  return map.mode === 3 && keysOf(map) === keys ? mapIdx : undefined;
}

/** Each chart's best counting play at each rate in one key mode (as Etterna keeps a best per
 * rate), best first. */
export function bestPerChart(
  plays: Play[],
  mapIndex: (id: string | undefined) => number | undefined,
  maps: MapRow[],
  keys: number,
  until = Infinity,
): Counted[] {
  const best = new Map<string, Counted>();
  for (const play of plays) {
    if (play.date > until || !play.ssr || play.wife === undefined) continue;
    const mapIdx = counting(play, mapIndex, maps, keys);
    if (mapIdx === undefined) continue;
    const map = maps[mapIdx];
    const key = `${mapIdx}@${rateOf(play).toFixed(2)}`;
    const had = best.get(key);
    if (!had || play.ssr[0] > had.ssr[0]) best.set(key, { play, map, mapIdx, ssr: play.ssr, wife: play.wife });
  }
  return [...best.values()].sort((a, b) => b.ssr[0] - a.ssr[0]);
}

export interface Skill {
  skillset: MsdSkill;
  /** Etterna's blend of the plays' SSRs in it; null without any */
  value: number | null;
  /** plays behind it */
  plays: number;
  /** its best plays, best first, with what each is worth in it */
  top: (Counted & { worth: number })[];
}

export interface SkillSheet {
  skills: Skill[];
  /** the seven blended into one, as Etterna's overall rating */
  overall: number | null;
  /** plays behind it all */
  plays: number;
}

/** The skills from each chart's best plays (see `bestPerChart`). */
export function skills(charts: Counted[]): SkillSheet {
  const list = SKILLSETS.map((skillset, k): Skill => {
    const rated = charts.filter((c) => ssrOf(c, k) > 0);
    const top = [...rated]
      .sort((a, b) => ssrOf(b, k) - ssrOf(a, k))
      .slice(0, TOP)
      .map((c) => ({ ...c, worth: ssrOf(c, k) }));
    return { skillset, value: rated.length ? aggregate(rated.map((c) => ssrOf(c, k))) : null, plays: rated.length, top };
  });
  const known = list.map((s) => s.value).filter((v): v is number => v !== null);
  return { skills: list, overall: known.length ? aggregate(known, 0.1, 1.125) : null, plays: charts.length };
}

/* ---------- Over time ---------- */

export interface SkillPoint {
  /** unix ms: the last play of a day whose plays raised a chart's best */
  date: number;
  /** overall, then the skillsets in SKILLSETS' order */
  values: number[];
}

/** the local day a time is on */
const dayOf = (ms: number): number => Math.floor((ms - new Date(ms).getTimezoneOffset() * 60_000) / 86_400_000);

/** Where `v` goes in a list sorted highest first. */
function slot(list: number[], v: number): number {
  let lo = 0;
  let hi = list.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (list[mid] > v) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** How the skills grew in a key mode, oldest first: overall and every skillset after each day whose
 * plays raised a chart's best (what `skills` would have said that evening). */
export function skillHistory(plays: Play[], mapIndex: (id: string | undefined) => number | undefined, maps: MapRow[], keys: number): SkillPoint[] {
  const list: { play: Play; ssr: number[]; key: string }[] = [];
  for (const play of plays) {
    const mapIdx = counting(play, mapIndex, maps, keys);
    if (mapIdx !== undefined && play.ssr) list.push({ play, ssr: play.ssr, key: `${mapIdx}@${rateOf(play).toFixed(2)}` });
  }
  list.sort((a, b) => a.play.date - b.play.date);
  const best = new Map<string, number[]>();
  // each skillset's values, highest first, kept as plays come in
  const sorted: number[][] = SKILLSETS.map(() => []);
  const out: SkillPoint[] = [];
  let moved = false;
  for (let j = 0; j < list.length; j++) {
    const { play, ssr, key } = list[j];
    const had = best.get(key);
    if (!had || ssr[0] > had[0]) {
      best.set(key, ssr);
      for (let k = 0; k < SKILLSETS.length; k++) {
        const old = had?.[k + 1] ?? 0;
        if (old > 0) sorted[k].splice(slot(sorted[k], old), 1);
        const v = ssr[k + 1] ?? 0;
        if (v > 0) sorted[k].splice(slot(sorted[k], v), 0, v);
      }
      moved = true;
    }
    const next = list[j + 1];
    if (moved && (!next || dayOf(next.play.date) !== dayOf(play.date))) {
      const each = sorted.map((v) => aggregate(v, 0.1, 1.05, true));
      out.push({ date: play.date, values: [aggregate(each.filter((v) => v > 0), 0.1, 1.125), ...each] });
      moved = false;
    }
  }
  return out;
}

/** Key modes with counting plays, most played first. */
export function keyModes(plays: Play[], mapIndex: (id: string | undefined) => number | undefined, maps: MapRow[]): { keys: number; plays: number }[] {
  const n = new Map<number, number>();
  for (const p of plays) {
    if (p.mode !== 3 || !p.ssr) continue;
    const i = mapIndex(p.map);
    if (i === undefined || maps[i].mode !== 3) continue;
    const k = keysOf(maps[i]);
    n.set(k, (n.get(k) ?? 0) + 1);
  }
  return [...n.entries()].map(([keys, plays]) => ({ keys, plays })).sort((a, b) => b.plays - a.plays || a.keys - b.keys);
}

/* ---------- Practice next ---------- */

/** A chart's MSD at 0.7× … 2.0×, a tenth apart: [overall, the skillsets] each */
export type MsdRows = number[][];

/** The MSD at a rate, in a straight line between the tenths; null outside 0.7× … 2.0×. */
export function msdAt(rows: MsdRows, rate: number): number[] | null {
  const x = (rate - 0.7) / 0.1;
  if (x < -1e-6 || x > 13 + 1e-6 || rows.length < 14) return null;
  const i = Math.min(12, Math.max(0, Math.floor(x + 1e-9)));
  const f = Math.min(1, Math.max(0, x - i));
  return rows[i].map((v, j) => v + (rows[i + 1][j] - v) * f);
}

/** Rates a suggestion may use */
const PRACTICE_RATES = Array.from({ length: 16 }, (_, i) => (75 + i * 5) / 100);
/** where practice aims, as factors of your best plays in the skillset: enough to move it, close enough to pass */
const AIM = { lo: 1.01, ideal: 1.05, hi: 1.1 } as const;
/** a skillset without plays starts this far under the lowest base there is */
const FIRST_AIM = 0.92;
/** a chart is about a skillset when its MSD there is at least this share of its highest */
export const FOCUS = 0.93;
/** your best plays a skillset's aim starts from */
const BASE_PLAYS = 5;
/** suggestions shown per skillset, and at most after "Show more" */
export const SUGGESTIONS = 5;
export const MORE_SUGGESTIONS = 12;

export interface Aim {
  /** the skill now; null without plays in it */
  from: number | null;
  /** what your best plays in it are worth (their average), where the band starts from */
  base: number;
  /** MSDs (in the skillset, at the rate) practice aims for: the band and its sweet spot */
  lo: number;
  ideal: number;
  hi: number;
}

export interface Suggestion {
  map: MapRow;
  mapIdx: number;
  /** the rate to play it at */
  rate: number;
  /** its MSD in the skillset at that rate */
  msd: number;
  /** how much of the chart's difficulty is the skillset, 0–1 (its MSD there over the highest) */
  focus: number;
  /** your best counting play on it, if any (below the aim, or it wouldn't be here) */
  best?: Counted;
  /** the skillset's skill after a pass at Etterna's reference accuracy (93%, where SSR = MSD) */
  after: number;
}

export interface Practice {
  skill: Skill;
  aim: Aim;
  /** best first, up to MORE_SUGGESTIONS */
  picks: Suggestion[];
}

const baseOf = (s: Skill): number | null => (s.top.length ? s.top.slice(0, BASE_PLAYS).reduce((sum, c) => sum + c.worth, 0) / Math.min(BASE_PLAYS, s.top.length) : null);

/** Where practice for a skillset aims: a little above your best plays in it, or for one without
 * plays a little under the lowest of the others. Null when there are no plays at all. */
export function aimFor(skill: Skill, all: Skill[]): Aim | null {
  let base = baseOf(skill);
  if (base === null) {
    const known = all.map(baseOf).filter((v): v is number => v !== null);
    if (!known.length) return null;
    base = Math.min(...known) * FIRST_AIM;
  }
  return { from: skill.value, base, lo: base * AIM.lo, ideal: base * AIM.ideal, hi: base * AIM.hi };
}

/** How much a chart at a rate is about skillset k: its MSD there over its highest skillset. */
const focusOf = (msd: number[], k: number): number => {
  let top = 0;
  for (let j = 1; j < msd.length; j++) top = Math.max(top, msd[j]);
  return top > 0 ? msd[k + 1] / top : 0;
};

/** The rate that puts a chart nearest the aim's sweet spot in skillset k, with its MSD there: 1×
 * whenever that's within the aim. Null when no rate gets it in (or it isn't about k there). */
export function rateFor(rows: MsdRows, k: number, aim: Aim): { rate: number; msd: number; focus: number } | null {
  const at = (rate: number) => {
    const m = msdAt(rows, rate);
    return m ? { rate, msd: m[k + 1], focus: focusOf(m, k) } : null;
  };
  const one = at(1);
  if (one && one.msd >= aim.lo && one.msd <= aim.hi && one.focus >= FOCUS) return one;
  let pick: { rate: number; msd: number; focus: number } | null = null;
  for (const rate of PRACTICE_RATES) {
    const c = at(rate);
    if (!c || c.msd < aim.lo || c.msd > aim.hi || c.focus < FOCUS) continue;
    if (!pick || Math.abs(c.msd - aim.ideal) < Math.abs(pick.msd - aim.ideal)) pick = c;
  }
  return pick;
}

/** The charts practice draws on, per library and key mode: osu!mania charts that lazer shows.
 * Snapshots are replaced, never changed, so this holds as long as the snapshot does. */
const pools = new WeakMap<MapRow[], Map<number, number[]>>();
function poolOf(maps: MapRow[], keys: number): number[] {
  let byKeys = pools.get(maps);
  if (!byKeys) pools.set(maps, (byKeys = new Map()));
  let pool = byKeys.get(keys);
  if (!pool) {
    pool = [];
    for (let i = 0; i < maps.length; i++) {
      const m = maps[i];
      if (m.mode === 3 && !m.hidden && keysOf(m) === keys) pool.push(i);
    }
    byKeys.set(keys, pool);
  }
  return pool;
}

/** Skillset k's skill once a chart is passed at a rate at Etterna's reference accuracy (its MSD
 * there), replacing that chart's best at that rate. */
function skillWith(charts: Counted[], k: number, mapIdx: number, rate: number, msd: number): number {
  const values: number[] = [];
  let had = 0;
  for (const c of charts) {
    const v = ssrOf(c, k);
    if (v <= 0) continue;
    if (c.mapIdx === mapIdx && Math.abs(rateOf(c.play) - rate) < 0.005) had = v;
    else values.push(v);
  }
  values.push(Math.max(msd, had));
  return aggregate(values);
}

/** Charts to practise skillset k on, best first: in the key mode, mostly about k at a rate that puts
 * them in the aim, and not played that well yet. Charts most about it lead, then those near the
 * sweet spot; untouched charts and 1× go before replays and rates. One difficulty per set.
 * `msdOf` gives a chart's MSD at every rate (undefined: not worked out). */
export function suggest(
  k: number,
  aim: Aim,
  charts: Counted[],
  maps: MapRow[],
  keys: number,
  msdOf: (mapIdx: number) => MsdRows | undefined,
  opts: {
    played?: Set<number>;
    exclude?: Set<number>;
    limit?: number;
    /** your best on one of `maps`, when `maps` isn't the library `charts` index into */
    bestOf?: (mapIdx: number) => Counted | undefined;
  } = {},
): Suggestion[] {
  const byIdx = new Map<number, Counted>();
  for (const c of charts) if (!byIdx.has(c.mapIdx) || ssrOf(c, k) > ssrOf(byIdx.get(c.mapIdx)!, k)) byIdx.set(c.mapIdx, c);
  const bestOf = opts.bestOf ?? ((i: number) => byIdx.get(i));
  const perSet = new Map<number, { pick: Omit<Suggestion, "after">; score: number }>();
  for (const mapIdx of poolOf(maps, keys)) {
    if (opts.exclude?.has(mapIdx)) continue;
    const rows = msdOf(mapIdx);
    if (!rows) continue;
    const best = bestOf(mapIdx);
    if (best && ssrOf(best, k) >= aim.lo) continue;
    const at = rateFor(rows, k, aim);
    if (!at) continue;
    const map = maps[mapIdx];
    const score =
      at.focus +
      0.5 * (1 - Math.abs(at.msd - aim.ideal) / (aim.hi - aim.lo)) -
      (at.rate === 1 ? 0 : 0.1 + Math.abs(at.rate - 1) / 2) -
      (best ? 0.15 : opts.played?.has(mapIdx) ? 0.05 : 0);
    const had = perSet.get(map.set);
    if (!had || score > had.score) perSet.set(map.set, { pick: { map, mapIdx, ...at, best }, score });
  }
  return [...perSet.values()]
    .sort((a, b) => b.score - a.score)
    .slice(0, opts.limit ?? SUGGESTIONS)
    .map(({ pick }) => ({ ...pick, after: skillWith(charts, k, pick.mapIdx, pick.rate, pick.msd) }));
}

/** Practice for every skillset the library has charts for, weakest first (those without plays
 * before all), those with nothing in reach last. A chart is suggested once: to the first skillset
 * that shows it, where `shown` says how many of a skillset's picks are on screen. */
export function practice(
  now: Skill[],
  charts: Counted[],
  maps: MapRow[],
  keys: number,
  msdOf: (mapIdx: number) => MsdRows | undefined,
  opts: { played?: Set<number>; shown?: (s: MsdSkill) => number } = {},
): Practice[] {
  const order = now.map((skill, k) => ({ skill, k }));
  order.sort((a, b) => (a.skill.value ?? -1) - (b.skill.value ?? -1));
  const exclude = new Set<number>();
  const out: Practice[] = [];
  for (const { skill, k } of order) {
    const aim = aimFor(skill, now);
    if (!aim) continue;
    const picks = suggest(k, aim, charts, maps, keys, msdOf, { played: opts.played, exclude, limit: MORE_SUGGESTIONS });
    picks.slice(0, opts.shown?.(skill.skillset) ?? SUGGESTIONS).forEach((p) => exclude.add(p.mapIdx));
    out.push({ skill, aim, picks });
  }
  return out.sort((a, b) => Number(!a.picks.length) - Number(!b.picks.length));
}
