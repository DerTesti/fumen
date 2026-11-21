/* skill-trends.ts — the Skills page's "Over time": overall and every skillset as a small chart of
 * their own, how each grew since a point in time (lib/skills' `skillHistory`, a step per day whose
 * plays raised something). Hovering one chart (or moving along it with the arrow keys) puts a line
 * through all of them at the same day: each one's value there and the step it made stand in its
 * head, the day under the chart pointed at. */
import { date, esc, fixed } from "../lib/format";
import { skillName } from "../lib/goals";
import { SKILLSETS, type SkillPoint } from "../lib/skills";
import { attachSegIndicator } from "../ui/controls";
import { t } from "../lib/i18n";

export type TrendRange = "90d" | "1y" | "all";
const RANGE_MS: Record<TrendRange, number> = { "90d": 90 * 86_400_000, "1y": 365 * 86_400_000, all: Infinity };

/** the plot's own units: 1000 wide, 100 high, room above the line for the value labels */
const W = 1000;
const H = 100;
const TOP = 16;
const BOTTOM = 6;
/** where a value sits in the plot, 0 (top) to H */
const yOf = (v: number, lo: number, hi: number): number => H - BOTTOM - ((v - lo) / (hi - lo)) * (H - TOP - BOTTOM);

interface Shown {
  from: number;
  to: number;
  points: SkillPoint[];
}

const shownOf = new WeakMap<HTMLElement, Shown>();

/** A step for the value lines: about three of them over the chart. */
const stepFor = (span: number): number => (span <= 3 ? 1 : span <= 8 ? 2 : span <= 20 ? 5 : 10);

function scale(values: number[]): { lo: number; hi: number; step: number } {
  const min = Math.min(...values);
  const max = Math.max(...values);
  const step = stepFor(Math.max(max - min, 2));
  const lo = Math.floor(min / step) * step;
  let hi = Math.ceil(max / step) * step;
  if (hi - lo < 2) hi = lo + Math.max(2, step);
  return { lo, hi, step };
}

/** One chart: line `i` of the points (0 overall, then the skillsets). */
function card(i: number, s: Shown): string {
  const key = i === 0 ? "overall" : SKILLSETS[i - 1];
  const pts = s.points.filter((p) => p.values[i] > 0);
  const last = pts[pts.length - 1];
  if (!last) return "";
  const first = pts[0];
  const change = last.values[i] - first.values[i];
  const { lo, hi, step } = scale(pts.map((p) => p.values[i]));
  const x = (d: number) => (s.to > s.from ? ((d - s.from) / (s.to - s.from)) * W : W);
  const y = (v: number) => yOf(v, lo, hi);
  let line = `M${x(first.date).toFixed(1)} ${y(first.values[i]).toFixed(1)}`;
  for (const p of pts.slice(1)) line += `H${x(p.date).toFixed(1)}V${y(p.values[i]).toFixed(1)}`;
  line += `H${W}`;
  const area = `${line}V${H}H${x(first.date).toFixed(1)}Z`;
  const grid: string[] = [];
  const labels: string[] = [];
  for (let v = lo; v <= hi + 1e-9; v += step) {
    grid.push(`<line x1="0" x2="${W}" y1="${y(v).toFixed(1)}" y2="${y(v).toFixed(1)}" vector-effect="non-scaling-stroke" />`);
    labels.push(`<span class="trend-y num" style="top:${((y(v) / H) * 100).toFixed(1)}%">${v}</span>`);
  }
  const name = skillName(key);
  return `
    <div class="panel skill-trend${i === 0 ? " is-overall" : ""}" data-line="${i}">
      <div class="skill-trend-head">
        <span class="skill-name">${name}</span>
        ${Math.abs(change) >= 0.01 ? `<span class="skill-delta ${change > 0 ? "up" : "down"} num">${change > 0 ? "+" : "−"}${fixed(Math.abs(change), 2)}</span>` : ""}
        <span class="skill-delta trend-step num" hidden></span>
        <b class="num skill-trend-now" data-now="${fixed(last.values[i], 2)}">${fixed(last.values[i], 2)}</b>
      </div>
      <div class="trend-plot" tabindex="0" role="img" aria-label="${esc(t("{skill}: {from} to {to}", { skill: name, from: fixed(first.values[i], 2), to: fixed(last.values[i], 2) }))}">
        <svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" aria-hidden="true">
          <g class="trend-grid">${grid.join("")}</g>
          <path class="trend-area" d="${area}" />
          <path class="trend-line" d="${line}" vector-effect="non-scaling-stroke" />
        </svg>
        ${labels.join("")}
        <i class="trend-cross" hidden></i>
        <i class="trend-dot" hidden data-lo="${lo}" data-hi="${hi}"></i>
        <div class="trend-tip" hidden></div>
      </div>
    </div>`;
}

/** Puts "Over time" into `host`: the range to show, then a chart for overall and each skillset.
 * Nothing when there are no counting plays yet. */
export function renderTrends(host: HTMLElement, points: SkillPoint[], range: TrendRange, onRange: (r: TrendRange) => void): void {
  if (!points.length) {
    host.hidden = true;
    return;
  }
  host.hidden = false;
  const to = Date.now();
  const span = RANGE_MS[range];
  const from = span === Infinity ? points[0].date : to - span;
  // where things stood when the range starts, then what changed in it
  let carry: SkillPoint | undefined;
  const inside: SkillPoint[] = [];
  for (const p of points) {
    if (p.date < from) carry = p;
    else inside.push(p);
  }
  const shown: Shown = { from, to, points: carry ? [{ date: from, values: carry.values }, ...inside] : inside };
  shownOf.set(host, shown);
  const opts: [TrendRange, string][] = [
    ["90d", t("90 days")],
    ["1y", t("Year")],
    ["all", t("All")],
  ];
  host.innerHTML = `
    <div class="practice-head">
      <div>
        <h2 class="section-title">${t("Over time")}</h2>
        <div class="practice-sub">${inside.length ? t("How your skills grew, a step for every day your plays raised one.") : t("Nothing moved in this time.")}</div>
      </div>
      <div class="seg-pill trend-range" role="tablist" aria-label="${t("Time range")}">${opts.map(([r, label]) => `<button type="button" data-range="${r}" class="${r === range ? "active" : ""}">${label}</button>`).join("")}</div>
    </div>
    <div class="trends-grid">${[0, ...SKILLSETS.map((_, k) => k + 1)].map((i) => card(i, shown)).join("")}</div>
    <div class="trend-x num"><span>${date(from)}</span><span>${t("Today")}</span></div>`;
  attachSegIndicator(host.querySelector<HTMLElement>(".trend-range")!);
  host.onclick = (e) => {
    const b = (e.target as Element).closest<HTMLElement>("[data-range]");
    if (b && b.dataset.range !== range) onRange(b.dataset.range as TrendRange);
  };
  host.onpointermove = (e) => {
    const plot = (e.target as Element).closest<HTMLElement>(".trend-plot");
    if (!plot) return clear(host);
    const r = plot.getBoundingClientRect();
    point(host, nearest(shown, (e.clientX - r.left) / Math.max(1, r.width)), plot);
  };
  host.onpointerleave = () => clear(host);
  // the host is new with every render of the page, so these go on once
  host.addEventListener("focusin", (e) => {
    const plot = (e.target as Element).closest<HTMLElement>(".trend-plot");
    if (plot) point(host, shown.points.length - 1, plot);
  });
  host.addEventListener("focusout", () => clear(host));
  host.onkeydown = (e) => {
    const plot = (e.target as Element).closest<HTMLElement>(".trend-plot");
    if (!plot || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    e.preventDefault();
    const at = Number(host.dataset.at ?? shown.points.length - 1);
    point(host, Math.max(0, Math.min(shown.points.length - 1, at + (e.key === "ArrowLeft" ? -1 : 1))), plot);
  };
}

/** The point nearest to a spot along the charts (0–1). */
function nearest(s: Shown, f: number): number {
  const d = s.from + f * (s.to - s.from);
  let best = 0;
  for (let j = 1; j < s.points.length; j++) if (Math.abs(s.points[j].date - d) < Math.abs(s.points[best].date - d)) best = j;
  return best;
}

/** A line through every chart at point `j`: each chart's value there and the step it made in its
 * head, the day under `active`. */
function point(host: HTMLElement, j: number, active: HTMLElement): void {
  const s = shownOf.get(host);
  const p = s?.points[j];
  if (!s || !p) return;
  host.dataset.at = String(j);
  host.classList.add("is-pointing");
  const fx = s.to > s.from ? (p.date - s.from) / (s.to - s.from) : 1;
  for (const plot of host.querySelectorAll<HTMLElement>(".trend-plot")) {
    const card = plot.closest<HTMLElement>("[data-line]")!;
    const i = Number(card.dataset.line);
    const v = p.values[i];
    const cross = plot.querySelector<HTMLElement>(".trend-cross")!;
    const dot = plot.querySelector<HTMLElement>(".trend-dot")!;
    const tip = plot.querySelector<HTMLElement>(".trend-tip")!;
    const now = card.querySelector<HTMLElement>(".skill-trend-now")!;
    const step = card.querySelector<HTMLElement>(".trend-step")!;
    cross.hidden = false;
    cross.style.left = `${(fx * 100).toFixed(2)}%`;
    if (plot === active) {
      tip.hidden = false;
      tip.textContent = date(p.date);
      const w = plot.clientWidth;
      const half = tip.offsetWidth / 2;
      tip.style.left = `${Math.max(half, Math.min(w - half, fx * w))}px`;
    } else tip.hidden = true;
    if (!(v > 0)) {
      dot.hidden = step.hidden = true;
      now.textContent = "–";
      continue;
    }
    const lo = Number(dot.dataset.lo);
    const hi = Number(dot.dataset.hi);
    dot.hidden = false;
    dot.style.left = `${(fx * 100).toFixed(2)}%`;
    dot.style.top = `${((yOf(v, lo, hi) / H) * 100).toFixed(2)}%`;
    now.textContent = fixed(v, 2);
    // the step this day made, from the one before it
    const before = j > 0 ? s.points[j - 1].values[i] : 0;
    const d = before > 0 ? v - before : 0;
    step.hidden = Math.abs(d) < 0.005;
    step.className = `skill-delta trend-step num ${d < 0 ? "down" : "up"}`;
    step.textContent = `${d < 0 ? "−" : "+"}${fixed(Math.abs(d), 2)}`;
  }
}

function clear(host: HTMLElement): void {
  delete host.dataset.at;
  host.classList.remove("is-pointing");
  for (const el of host.querySelectorAll<HTMLElement>(".trend-cross, .trend-dot, .trend-tip, .trend-step")) el.hidden = true;
  for (const el of host.querySelectorAll<HTMLElement>(".skill-trend-now")) el.textContent = el.dataset.now ?? "";
}
