// Search for the docs pages. The build (scripts/docs-site/build.mjs) writes docs/search-index.js,
// an ES module holding each page's sections as [id, heading, body tokens]. This file loads it on
// the first use of the box, so no request is made for a reader who never searches, and needs no
// network beyond the site's own files (the CSP allows only `script-src 'self'`).

/** Lower-case words, with `-` and `_` inside a word kept so a rule id stays one token. */
export const tokens = (text) => text.toLowerCase().match(/[\p{L}\p{N}_]+(?:-[\p{L}\p{N}_]+)*/gu) ?? [];

// Points for one query word against one token: whole word, start of word, anywhere in it.
const hit = (word, token) => (token === word ? 3 : token.startsWith(word) ? 2 : token.includes(word) ? 1 : 0);
const best = (word, list) => list.reduce((m, token) => Math.max(m, hit(word, token)), 0);

// A heading match outweighs any body match, and the page's own title counts for a little.
const HEADING = 100;
const TITLE = 10;

/** Turns the index into flat sections with their tokens split once. */
export function prepare(index) {
  return index.flatMap(([url, title, sections]) =>
    sections.map(([id, heading, body]) => ({
      url,
      id,
      title,
      heading,
      h: tokens(heading),
      t: tokens(title),
      b: body ? body.split(" ") : [],
    })),
  );
}

/** The best `limit` sections for a query: every word must match, in the heading, the page title or
 * the body, and one at least in the heading or body (a title alone would list a whole page), and heading matches rank first, and among those a shorter heading (closer to the query) does, then the
 * body and title matches, then index order. */
export function search(sections, query, limit = 8) {
  const words = tokens(query).filter((w) => w.length > 1);
  if (!words.length) return [];
  const found = [];
  for (const [order, s] of sections.entries()) {
    let score = 0;
    let heading = 0;
    let own = 0;
    for (const w of words) {
      const h = best(w, s.h);
      const t = best(w, s.t);
      const b = best(w, s.b);
      if (!(h || t || b)) {
        score = 0;
        break;
      }
      heading += h;
      own += h || b ? 1 : 0;
      score += h * HEADING + t * TITLE + b;
    }
    if (score && own) found.push({ heading, score, order, s });
  }
  return found
    .sort((a, b) => b.heading - a.heading || a.s.h.length - b.s.h.length || b.score - a.score || a.order - b.order)
    .slice(0, limit)
    .map((f) => f.s);
}

function init() {
  const root = document.getElementById("docs-search");
  if (!root) return;
  const input = root.querySelector("input");
  const list = root.querySelector("ul");
  const status = root.querySelector("[role=status]");
  const docs = new URL("docs/", import.meta.url);
  let sections;
  let loading;
  let shown = [];
  let active = -1;

  const open = (yes) => {
    list.hidden = !yes;
    input.setAttribute("aria-expanded", String(yes));
    if (!yes) setActive(-1);
  };
  const setActive = (i) => {
    active = i;
    list.querySelector("[aria-selected=true]")?.setAttribute("aria-selected", "false");
    const option = list.children[i];
    option?.setAttribute("aria-selected", "true");
    option?.scrollIntoView({ block: "nearest" });
    if (option) input.setAttribute("aria-activedescendant", option.id);
    else input.removeAttribute("aria-activedescendant");
  };
  const load = () =>
    (loading ??= import(`./docs/search-index.js${new URL(import.meta.url).search}`).then(
      (m) => {
        sections = prepare(m.default);
      },
      () => {
        loading = undefined;
        status.textContent = "Search could not load. The contents page lists every page.";
      },
    ));

  const render = async () => {
    const query = input.value;
    await load();
    if (!sections || query !== input.value) return;
    shown = search(sections, query);
    list.replaceChildren(
      ...shown.map((s, i) => {
        const li = document.createElement("li");
        li.id = `docs-result-${i}`;
        li.setAttribute("role", "option");
        li.setAttribute("aria-selected", "false");
        li.dataset.href = `${new URL(s.url, docs)}#${s.id}`;
        const name = document.createElement("span");
        name.textContent = s.heading;
        const page = document.createElement("span");
        page.className = "where";
        page.textContent = s.heading === s.title ? "Page" : s.title;
        li.append(name, page);
        return li;
      }),
    );
    const typed = tokens(query).some((w) => w.length > 1);
    open(typed && shown.length > 0);
    status.textContent = !typed
      ? ""
      : shown.length
        ? `${shown.length} ${shown.length === 1 ? "result" : "results"}`
        : `No results for "${query.trim()}"`;
  };

  const go = (i) => {
    const href = list.children[i]?.dataset.href;
    if (!href) return;
    open(false);
    location.href = href;
  };

  input.addEventListener("focus", load, { once: true });
  input.addEventListener("input", render);
  input.addEventListener("keydown", (e) => {
    const n = shown.length;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (!n) return;
      e.preventDefault();
      if (list.hidden) return open(true);
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive(active < 0 ? (step > 0 ? 0 : n - 1) : (active + step + n) % n);
    } else if (e.key === "Enter" && !list.hidden) {
      e.preventDefault();
      go(Math.max(active, 0));
    } else if (e.key === "Escape") {
      e.preventDefault(); // the browser's own Esc would clear a search box
      if (!list.hidden) open(false);
      else if (input.value) {
        input.value = "";
        status.textContent = "";
      }
    }
  });
  // Clicking a result must not blur the box first, or the list would close before the click lands.
  list.addEventListener("pointerdown", (e) => e.preventDefault());
  list.addEventListener("click", (e) => go([...list.children].indexOf(e.target.closest("li"))));
  root.addEventListener("focusout", (e) => {
    if (!root.contains(e.relatedTarget)) open(false);
  });
  input.addEventListener("focus", () => shown.length && tokens(input.value).length && open(true));
  document.addEventListener("keydown", (e) => {
    if (e.key !== "/" || e.ctrlKey || e.metaKey || e.altKey) return;
    const t = e.target;
    if (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)) return;
    e.preventDefault();
    input.focus();
    input.select();
  });
  root.hidden = false;
}

if (typeof document !== "undefined") init();
