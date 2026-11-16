function copyText(text, button) {
  const done = () => {
    const label = button.textContent;
    button.textContent = "Copied";
    setTimeout(() => { button.textContent = label; }, 1500);
  };
  if (navigator.clipboard) {
    navigator.clipboard.writeText(text).then(done, () => selectFallback(button));
  } else {
    selectFallback(button);
  }
}

function selectFallback(button) {
  const pre = button.closest("figure") && button.closest("figure").querySelector("pre");
  if (!pre) return;
  const range = document.createRange();
  range.selectNodeContents(pre);
  const selection = window.getSelection();
  selection.removeAllRanges();
  selection.addRange(range);
}

function wireCopyButtons() {
  document.querySelectorAll("button.copy").forEach((button) => {
    button.addEventListener("click", () => {
      const code = button.closest("figure").querySelector("pre").innerText;
      copyText(code, button);
    });
  });
}

function wireCopyPage() {
  document.querySelectorAll("button.copy-page").forEach((button) => {
    button.addEventListener("click", () => {
      fetch(button.dataset.src).then((r) => r.text()).then((text) => copyText(text, button));
    });
  });
}

function selectTab(tabs, tab) {
  tabs.forEach((t) => {
    const selected = t === tab;
    t.setAttribute("aria-selected", String(selected));
    t.tabIndex = selected ? 0 : -1;
    document.getElementById(t.getAttribute("aria-controls")).hidden = !selected;
  });
}

function wireTabs() {
  document.querySelectorAll('[role="tablist"]').forEach((list) => {
    const tabs = Array.from(list.querySelectorAll('[role="tab"]'));
    tabs.forEach((tab, i) => {
      tab.addEventListener("click", () => selectTab(tabs, tab));
      tab.addEventListener("keydown", (e) => {
        const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
        if (!step) return;
        const next = tabs[(i + step + tabs.length) % tabs.length];
        selectTab(tabs, next);
        next.focus();
      });
    });
    if (/Win/.test(navigator.platform)) selectTab(tabs, tabs[1]);
  });
}

wireCopyButtons();
wireCopyPage();
wireTabs();
