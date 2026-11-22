// The first start's question and the block page. Two things go to fumen.dertesti.dev only with a
// yes, both off until then: the start count (src-tauri/src/ping.rs) and the online features
// (src-tauri/src/online.rs: the osu! login, the cloud, comparing, and with them the block check).
// Everyone is asked once, also after the update that brought this. What the site keeps:
// fumen.dertesti.dev/privacy.
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, asApiError, isTauri } from "./api";
import { t } from "./i18n";
import { store } from "./store";
import type { FumenBlock } from "./types";
import { closeAnimated, toast } from "../ui/controls";

const PRIVACY = "https://fumen.dertesti.dev/privacy";

/** At the start: a block noted here first, then the question (once), the start count and the check. */
export async function startConsent(): Promise<void> {
  if (!isTauri) return;
  const noted = await api.notedBlock().catch(() => null);
  showFor(noted);
  if (!store.settings?.consentAsked) await askConsent();
  if (store.settings?.consentAsked && store.settings.startPing) await api.sendPing();
  if (store.settings?.online) await checkBlock(false);
}

/** A full block takes Fumen over; an online one is said once per start, over the app. */
let onlineSaid = false;
function showFor(b: FumenBlock | null | undefined): void {
  if (b?.scope === "full") {
    hideOnline();
    showBlock(b);
    return;
  }
  hideBlock();
  if (b?.scope === "online") {
    if (!onlineSaid) showOnline(b);
    onlineSaid = true;
  } else {
    hideOnline();
    onlineSaid = false;
  }
}

/** The block check: a full block shows the block page, an online one the notice, none takes
 * them away. Offline, what's noted holds. \`ask\`: from the block page (also with the online
 * features off). */
export async function checkBlock(ask: boolean): Promise<FumenBlock | null | undefined> {
  try {
    const b = await api.checkBlock(ask);
    showFor(b);
    return b;
  } catch (e) {
    if (ask) toast(asApiError(e).message, { kind: "danger", timeout: 6000 });
    else console.warn("[check]", asApiError(e).message);
    return undefined;
  }
}

/** The question, both switches off; Escape is "neither". */
function askConsent(): Promise<void> {
  return new Promise((resolve) => {
    const bd = document.createElement("div");
    bd.className = "modal-backdrop dialog-backdrop";
    bd.innerHTML = `
      <div class="modal-panel dialog consent" role="dialog" aria-modal="true" aria-labelledby="consent-title">
        <div class="dialog-title" id="consent-title">${t("Before you start")}</div>
        <div class="dialog-msg">${t("Fumen works on your computer, and everything in it works without these two. Both stay off unless you turn them on, and you can change them later in Settings › About.")}</div>
        <label class="consent-row">
          <span class="consent-text"><b>${t("Count me in")}</b><span>${t("At each start, Fumen tells fumen.dertesti.dev that it started: a random number made on this computer and Fumen's version. That's how many people use Fumen gets counted.")}</span></span>
          <input type="checkbox" role="switch" data-consent="count" />
        </label>
        <label class="consent-row">
          <span class="consent-text"><b>${t("Online features")}</b><span>${t("Logging in with osu!, collections in the cloud and comparing with other players. With them on, Fumen checks at each start whether it's blocked, with the random number and a code made from this computer's own id. The id never leaves your computer, and the code can't be turned back into it.")}</span><span class="consent-login" hidden>${t("You're logged in with osu!. Logging in is one of the online features, so with them off, Fumen logs you out.")}</span></span>
          <input type="checkbox" role="switch" data-consent="online" />
        </label>
        <div class="dialog-actions">
          <button class="btn btn-ghost consent-more" type="button" data-act="more">${t("What's kept, and for how long")}</button>
          <button class="btn btn-primary" type="button" data-act="ok">${t("Continue")}</button>
        </div>
      </div>`;
    document.body.appendChild(bd);
    void api
      .osuAccount()
      .then((a) => (bd.querySelector<HTMLElement>(".consent-login")!.hidden = !a))
      .catch(() => {});
    const sw = (k: string) => bd.querySelector<HTMLInputElement>(`[data-consent="${k}"]`)!;
    let done = false;
    const finish = async (count: boolean, online: boolean) => {
      if (done) return;
      done = true;
      document.removeEventListener("keydown", onKey, true);
      try {
        store.settings = await api.updateSettings({ consentAsked: true, startPing: count });
        store.settings = await api.setOnline(online);
      } catch (e) {
        toast(asApiError(e).message, { kind: "danger", timeout: 6000 });
      }
      closeAnimated(bd);
      resolve();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      void finish(false, false);
    };
    document.addEventListener("keydown", onKey, true);
    bd.querySelector('[data-act="more"]')!.addEventListener("click", () => void openUrl(PRIVACY).catch(() => {}));
    bd.querySelector('[data-act="ok"]')!.addEventListener("click", () => void finish(sw("count").checked, sw("online").checked));
    setTimeout(() => bd.querySelector<HTMLButtonElement>('[data-act="ok"]')!.focus(), 30);
  });
}

let page: HTMLElement | null = null;
/** keys don't reach Fumen behind the block page */
const swallowKeys = (e: KeyboardEvent) => {
  if (page && !page.contains(e.target as Node)) {
    e.preventDefault();
    e.stopPropagation();
  }
};

/** The site's words, one paragraph each (it parts them with a blank line). The first one says
 * what's blocked, which the page's title already does; the reason gets its own block. */
function blockParagraphs(message: string): HTMLElement[] {
  const parts = message.split(/\n\s*\n/).map((p) => p.trim()).filter(Boolean);
  const shown = parts.length > 1 ? parts.slice(1) : parts;
  return shown.map((text) => {
    const reason = /^Reason:\s*(.*)$/s.exec(text);
    const contact = /^If you think that's a mistake:\s*(.*)$/s.exec(text);
    if (reason) {
      const box = document.createElement("div");
      box.className = "fumen-block-reason";
      const label = document.createElement("span");
      label.textContent = t("Reason");
      const words = document.createElement("p");
      words.textContent = reason[1];
      box.append(label, words);
      return box;
    }
    const p = document.createElement("p");
    p.textContent = contact ? `${t("If you think that's a mistake:")} ${contact[1]}` : text;
    return p;
  });
}

/** The card both block screens share: the badge on its striped head, then the words. */
function blockCard(o: { eyebrow: string; title: string; note: string; actions: string; small?: string }): string {
  return `
    <div class="fumen-block-panel" role="alertdialog" aria-modal="true" aria-labelledby="fumen-block-title">
      <div class="fumen-block-card">
        <div class="fumen-block-head">
          <div class="fumen-block-badge" aria-hidden="true">
            <svg viewBox="0 0 48 48"><circle class="ring" cx="24" cy="24" r="17" pathLength="1"/><path class="slash" d="M12 12 36 36" pathLength="1"/></svg>
          </div>
        </div>
        <div class="fumen-block-body">
          <div class="fumen-block-eyebrow">${o.eyebrow}</div>
          <div class="fumen-block-title" id="fumen-block-title">${o.title}</div>
          <div class="fumen-block-msg"></div>
          <p class="fumen-block-note">${o.note}</p>
          <div class="fumen-block-actions">${o.actions}</div>
        </div>
      </div>
      ${o.small ? `<p class="fumen-block-small">${o.small}</p>` : ""}
    </div>`;
}

/** The card's words from the site, and its privacy button. */
function fillCard(el: HTMLElement, b: FumenBlock): void {
  el.querySelector(".fumen-block-msg")!.replaceChildren(...blockParagraphs(b.message));
  el.querySelector('[data-act="privacy"]')!.addEventListener("click", () =>
    void openUrl(PRIVACY).catch((e) => toast(asApiError(e).message, { kind: "danger" })),
  );
}

let notice: HTMLElement | null = null;
const noticeKeys = (e: KeyboardEvent) => {
  if (e.key === "Escape" || e.key === "Enter") {
    e.preventDefault();
    e.stopPropagation();
    hideOnline();
  }
};

/** An online block, said over the app: the login, the cloud and comparing are off, the rest works. */
function showOnline(b: FumenBlock): void {
  hideOnline(true);
  notice = document.createElement("div");
  notice.className = "fumen-block is-online";
  notice.innerHTML = blockCard({
    eyebrow: t("Online blocked"),
    title: t("Fumen's online features are blocked"),
    note: t("Logging in, the cloud and comparing are off. Everything else in Fumen works as usual."),
    actions: `
      <button class="btn btn-ghost" type="button" data-act="privacy">${t("Privacy")}</button>
      <button class="btn btn-danger" type="button" data-act="close">${t("Got it")}</button>`,
  });
  fillCard(notice, b);
  notice.querySelector('[data-act="close"]')!.addEventListener("click", () => hideOnline());
  notice.addEventListener("click", (e) => {
    if (e.target === notice) hideOnline();
  });
  document.body.appendChild(notice);
  document.addEventListener("keydown", noticeKeys, true);
  notice.querySelector<HTMLButtonElement>('[data-act="close"]')!.focus({ preventScroll: true });
}

function hideOnline(now = false): void {
  if (!notice) return;
  const el = notice;
  notice = null;
  document.removeEventListener("keydown", noticeKeys, true);
  if (now) return el.remove();
  el.classList.add("closing");
  setTimeout(() => el.remove(), 200);
}

/** Fumen behind a page with the site's reason and where to turn; the files stay as they are. */
function showBlock(b: FumenBlock): void {
  if (!page) {
    page = document.createElement("div");
    page.className = "fumen-block";
    document.body.appendChild(page);
    document.body.classList.add("fumen-blocked");
    document.addEventListener("keydown", swallowKeys, true);
  }
  page.innerHTML = blockCard({
    eyebrow: t("Blocked"),
    title: t("Fumen is blocked on this computer"),
    note: t("Your files stay as they are. Fumen hasn't changed or deleted anything."),
    actions: `
      <button class="btn btn-ghost" type="button" data-act="privacy">${t("Privacy")}</button>
      <button class="btn btn-danger" type="button" data-act="ask">${t("Ask again")}</button>`,
    small: store.settings?.online
      ? t("Asks fumen.dertesti.dev whether the block was lifted.")
      : t("Asks fumen.dertesti.dev whether the block was lifted, with Fumen's random number only."),
  });
  fillCard(page, b);
  const ask = page.querySelector<HTMLButtonElement>('[data-act="ask"]')!;
  ask.addEventListener("click", async () => {
    ask.disabled = true;
    const now = await checkBlock(true);
    // an online block left over shows its own notice (showFor)
    if (now === null) toast(t("The block is lifted."), { kind: "success" });
    else if (now?.scope === "full") {
      toast(t("Still blocked."));
      // a short shake again, so the answer is felt
      page?.classList.remove("again");
      void page?.offsetWidth;
      page?.classList.add("again");
    }
    ask.disabled = false;
  });
}

function hideBlock(): void {
  if (!page) return;
  page.remove();
  page = null;
  document.body.classList.remove("fumen-blocked");
  document.removeEventListener("keydown", swallowKeys, true);
}
