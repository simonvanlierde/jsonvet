// The theme picked with the colophon's sun and moon, set before the page paints: a classic script
// in <head>, as the page's CSP allows no inline script. With no choice stored, the page follows the
// system's setting. The choice is a convenience, so a browser that refuses storage just forgets it.
(() => {
  const root = document.documentElement;
  const system = matchMedia("(prefers-color-scheme: dark)");
  try {
    const saved = localStorage.getItem("theme");
    if (saved === "light" || saved === "dark") root.dataset.theme = saved;
  } catch {}
  const current = () => root.dataset.theme ?? (system.matches ? "dark" : "light");

  addEventListener("DOMContentLoaded", () => {
    const button = document.getElementById("theme");
    const paper = getComputedStyle(root);
    const paint = () => {
      const theme = current();
      const other = theme === "dark" ? "light" : "dark";
      button.dataset.theme = theme;
      button.setAttribute("aria-label", `Switch to the ${other} theme`);
      button.title = `Switch to the ${other} theme`;
      // The browser's toolbar takes the paper colour of the theme in use.
      for (const meta of document.querySelectorAll('meta[name="theme-color"]')) {
        meta.content = paper.getPropertyValue("--paper").trim();
      }
    };
    button.addEventListener("click", () => {
      const theme = current() === "dark" ? "light" : "dark";
      root.dataset.theme = theme;
      try {
        localStorage.setItem("theme", theme);
      } catch {}
      paint();
    });
    system.addEventListener("change", paint);
    paint();
  });
})();
