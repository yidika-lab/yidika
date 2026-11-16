const ANIME = "./vendor/anime-4.5.0.esm.min.js";
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

function flash(button, label, done) {
  const text = button.querySelector(".copy-label") || button;
  const original = text.textContent;
  text.textContent = label;
  if (done) button.dataset.done = "";
  setTimeout(() => { text.textContent = original; delete button.dataset.done; }, 1400);
}

function selectText(node) {
  const range = document.createRange();
  range.selectNodeContents(node);
  const selection = window.getSelection();
  selection.removeAllRanges();
  selection.addRange(range);
}

function copyText(text, button, fallbackNode) {
  const done = () => flash(button, "Copied", true);
  const fail = () => { if (fallbackNode) selectText(fallbackNode); flash(button, "Press Ctrl+C"); };
  if (navigator.clipboard) navigator.clipboard.writeText(text).then(done, fail);
  else fail();
}

function wireCopyButtons() {
  document.querySelectorAll("button.copy").forEach((button) => {
    button.addEventListener("click", () => {
      const holder = button.closest("figure, .cmd");
      const node = holder.querySelector("pre, code");
      copyText(node.innerText.replace(/\n$/, ""), button, node);
    });
  });
  document.querySelectorAll("button.copy-page").forEach((button) => {
    button.addEventListener("click", () => {
      fetch(button.dataset.src).then((r) => r.text()).then((text) => copyText(text, button, null), () => flash(button, "Not available"));
    });
  });
}

function selectTab(tabs, tab) {
  tabs.forEach((t) => {
    const on = t === tab;
    t.setAttribute("aria-selected", String(on));
    t.tabIndex = on ? 0 : -1;
    document.getElementById(t.getAttribute("aria-controls")).hidden = !on;
  });
}

function wireTabs() {
  document.querySelectorAll('[role="tablist"]').forEach((list) => {
    const tabs = Array.from(list.querySelectorAll('[role="tab"]'));
    tabs.forEach((tab, i) => {
      tab.addEventListener("click", () => selectTab(tabs, tab));
      tab.addEventListener("keydown", (event) => {
        const step = { ArrowRight: 1, ArrowLeft: -1 }[event.key];
        if (!step) return;
        const next = tabs[(i + step + tabs.length) % tabs.length];
        selectTab(tabs, next);
        next.focus();
      });
    });
  });
  const windows = document.getElementById("tab-win");
  if (windows && /Windows/i.test(navigator.userAgent)) selectTab(Array.from(windows.parentElement.children), windows);
}

function wireTocSpy() {
  const links = Array.from(document.querySelectorAll(".toc a"));
  if (!links.length || !("IntersectionObserver" in window)) return;
  const byId = new Map(links.map((a) => [decodeURIComponent(a.hash.slice(1)), a]));
  const visible = new Set();
  const mark = () => {
    const first = links.find((a) => visible.has(decodeURIComponent(a.hash.slice(1))));
    links.forEach((a) => (a === first ? a.setAttribute("aria-current", "true") : a.removeAttribute("aria-current")));
  };
  const observer = new IntersectionObserver((entries) => {
    entries.forEach((e) => (e.isIntersecting ? visible.add(e.target.id) : visible.delete(e.target.id)));
    mark();
  }, { rootMargin: "-64px 0px -60% 0px" });
  byId.forEach((_, id) => { const h = document.getElementById(id); if (h) observer.observe(h); });
}

function searchScore(entry, query) {
  const title = entry.t.toLowerCase();
  if (title === query) return 0;
  if (title.startsWith(query)) return 1;
  if (title.includes(query)) return 2;
  if (entry.s.toLowerCase().includes(query)) return 3;
  return -1;
}

function searchMatches(index, query) {
  return index
    .map((entry) => ({ entry, score: searchScore(entry, query) }))
    .filter((r) => r.score >= 0)
    .sort((a, b) => a.score - b.score || (a.entry.c || 0) - (b.entry.c || 0))
    .slice(0, 12)
    .map((r) => r.entry);
}

function resultItem(entry, i) {
  const li = document.createElement("li");
  li.setAttribute("role", "option");
  li.id = `result-${i}`;
  const a = document.createElement("a");
  a.href = entry.u;
  const kind = entry.c ? "code" : entry.u.includes("#") ? "section" : "page";
  a.append(document.getElementById(`icon-${kind}`).content.cloneNode(true));
  const text = document.createElement("span");
  const title = document.createElement(entry.c ? "code" : "span");
  title.textContent = entry.t;
  const where = document.createElement("small");
  where.textContent = entry.s;
  text.append(title, where);
  a.append(text);
  li.append(a);
  return li;
}

function wireSearch() {
  const dialog = document.querySelector("dialog.search");
  const input = document.getElementById("search-input");
  const list = document.getElementById("search-results");
  const empty = dialog.querySelector(".search-empty");
  let index = null;
  let active = 0;

  const highlight = (i) => {
    const items = Array.from(list.children);
    if (!items.length) return;
    active = (i + items.length) % items.length;
    items.forEach((li, n) => li.setAttribute("aria-selected", String(n === active)));
    input.setAttribute("aria-activedescendant", items[active].id);
    items[active].scrollIntoView({ block: "nearest" });
  };
  const render = () => {
    const query = input.value.trim().toLowerCase();
    list.replaceChildren();
    if (!index || !query) { empty.hidden = true; return; }
    const results = searchMatches(index, query);
    results.forEach((entry, i) => list.append(resultItem(entry, i)));
    empty.hidden = results.length > 0;
    highlight(0);
  };
  const open = () => {
    if (!index) fetch("search.json").then((r) => r.json()).then((data) => { index = data; render(); }, () => {});
    dialog.showModal();
    input.select();
  };

  document.querySelectorAll(".search-open").forEach((b) => b.addEventListener("click", open));
  input.addEventListener("input", render);
  input.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown") { event.preventDefault(); highlight(active + 1); }
    if (event.key === "ArrowUp") { event.preventDefault(); highlight(active - 1); }
    if (event.key === "Enter") {
      const link = list.children[active] && list.children[active].querySelector("a");
      if (link) { event.preventDefault(); window.location.href = link.href; }
    }
  });
  dialog.addEventListener("click", (event) => { if (event.target === dialog) dialog.close(); });
  document.addEventListener("keydown", (event) => {
    const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement.tagName) || document.activeElement.isContentEditable;
    const shortcut = (event.key === "/" && !typing) || (event.key.toLowerCase() === "k" && (event.metaKey || event.ctrlKey));
    if (shortcut && !dialog.open) { event.preventDefault(); open(); }
  });
}

const THEMES = ["system", "light", "dark"];
const THEME_LABELS = { system: "follows the system", light: "light", dark: "dark" };

function storedTheme() {
  try { return localStorage.getItem("yidika-theme") || "system"; } catch (e) { return "system"; }
}

function applyTheme(button, mode) {
  if (mode === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = mode;
  button.dataset.mode = mode;
  button.setAttribute("aria-label", `Color theme: ${THEME_LABELS[mode]}`);
  button.title = `Theme: ${THEME_LABELS[mode]}`;
  try {
    if (mode === "system") localStorage.removeItem("yidika-theme");
    else localStorage.setItem("yidika-theme", mode);
  } catch (e) {}
}

function wireTheme() {
  const button = document.querySelector(".theme-toggle");
  if (!button) return;
  const current = THEMES.includes(storedTheme()) ? storedTheme() : "system";
  applyTheme(button, current);
  button.addEventListener("click", () => {
    const next = THEMES[(THEMES.indexOf(button.dataset.mode) + 1) % THEMES.length];
    applyTheme(button, next);
  });
}

function onceVisible(elements, play) {
  if (!("IntersectionObserver" in window)) return;
  const observer = new IntersectionObserver((entries) => {
    entries.forEach((e) => {
      if (!e.isIntersecting) return;
      observer.unobserve(e.target);
      play(e.target);
    });
  }, { threshold: 0.3 });
  elements.forEach((el) => observer.observe(el));
}

function playOrder(A) {
  const tokens = document.querySelectorAll(".ordered .t");
  A.animate(tokens, {
    x: { from: () => A.utils.random(-140, 140) },
    y: { from: () => A.utils.random(-60, 60) },
    rotate: { from: () => A.utils.random(-35, 35) },
    opacity: { from: 0 },
    delay: A.stagger(22, { from: "random", start: 250 }),
    ease: A.spring({ bounce: 0.2, duration: 900 }),
  });
}

function playLogo(A) {
  const strand = document.querySelector("[data-logo-strand]");
  if (!strand) return;
  const drawable = A.svg.createDrawable(strand);
  A.createTimeline({ defaults: { ease: "outExpo" } })
    .add("[data-logo-part]", { opacity: { from: 0 }, scale: { from: 0.9 }, duration: 500, delay: A.stagger(90) })
    .add(drawable, { draw: ["0 0", "0 1"], duration: 700, ease: "inOutQuad" }, "-=200");
}

function playBars(A, root) {
  root.querySelectorAll(".fill").forEach((fill) => {
    A.animate(fill, { width: { from: "0%" }, duration: 1100, ease: "outExpo" });
  });
  root.querySelectorAll(".val").forEach((val) => {
    const target = parseFloat(val.dataset.value);
    const digits = target < 1 ? 3 : 1;
    const counter = { n: 0 };
    A.animate(counter, { n: target, duration: 1100, ease: "outExpo", onUpdate: () => { val.textContent = `${counter.n.toFixed(digits)} s`; } });
  });
}

function playEngines(A, root) {
  const wires = A.svg.createDrawable(root.querySelectorAll(".wire"));
  A.createTimeline({ defaults: { ease: "inOutQuad", duration: 650 } })
    .add(wires.slice(0, 2), { draw: ["0 0", "0 1"], delay: A.stagger(80) })
    .add(wires.slice(2), { draw: ["0 0", "0 1"], delay: A.stagger(80) }, "-=120")
    .add(root.querySelector("[data-engines-out]"), { strokeWidth: [1.5, 3.5, 1.5], duration: 600, ease: "outQuad" }, "-=150");
}

function playReveal(A, root) {
  const items = root.children.length > 1 ? root.children : [root];
  A.animate(items, { y: { from: 16 }, opacity: { from: 0 }, duration: 600, delay: A.stagger(60), ease: "outExpo" });
}

function startMotion(A) {
  if (document.body.classList.contains("home")) {
    playLogo(A);
    playOrder(A);
  }
  onceVisible(document.querySelectorAll("[data-bars]"), (el) => playBars(A, el));
  onceVisible(document.querySelectorAll("[data-engines]"), (el) => playEngines(A, el));
  onceVisible(document.querySelectorAll("[data-reveal]"), (el) => playReveal(A, el));
}

wireTheme();
wireCopyButtons();
wireTabs();
wireTocSpy();
wireSearch();
if (!reduceMotion) import(ANIME).then(startMotion, () => {});
