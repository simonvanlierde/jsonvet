// Satteri Markdown plugins for the docs, which apply to the files of docs/ only:
// - docLinks: a link goes to the site's page, or to GitHub, as in lib.mjs, and the h1 is dropped,
//   as the page's title is Starlight's.
// - docBoxes: a wide table or code block scrolls on its own, in a box a keyboard can reach, named
//   by the heading above it, so each landmark name is unique on the page.
import { relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { ROOT, rewrite, sources, submodules } from "./lib.mjs";

const pages = new Set(sources());
let subs;

const srcOf = (fileURL) => {
  if (!fileURL) return;
  const src = relative(ROOT, fileURLToPath(fileURL)).split(sep).join("/");
  return pages.has(src) ? src : undefined;
};

export const docLinks = ({ fileURL }) => {
  const src = srcOf(fileURL);
  if (!src) return;
  subs ??= submodules();
  const go = (node, ctx) => {
    ctx.setProperty(node, "url", rewrite(node.url, src, pages, subs));
  };
  const h1 = (node, ctx) => {
    if (node.depth === 1) ctx.removeNode(node);
  };
  return { name: "doc-links", link: go, definition: go, heading: h1 };
};

export const docBoxes = ({ fileURL }) => {
  if (!srcOf(fileURL)) return;
  let heading = "";
  const uses = new Map();
  const label = (kind) => {
    const key = `${kind}: ${heading}`;
    const n = (uses.get(key) ?? 0) + 1;
    uses.set(key, n);
    return (heading ? key : kind) + (n > 1 ? ` (${n})` : "");
  };
  return {
    name: "doc-boxes",
    element: [
      {
        filter: ["h1", "h2", "h3", "h4", "h5", "h6"],
        visit(node, ctx) {
          heading = ctx.textContent(node).replace(/\s+/g, " ").trim();
        },
      },
      {
        filter: ["pre"],
        visit(node, ctx) {
          ctx.setProperty(node, "tabIndex", 0);
          ctx.setProperty(node, "role", "region");
          ctx.setProperty(node, "ariaLabel", label("Code"));
        },
      },
      {
        filter: ["table"],
        visit(node, ctx) {
          ctx.wrapNode(node, {
            type: "element",
            tagName: "div",
            properties: { className: ["scroll"], tabIndex: 0, role: "region", ariaLabel: label("Table") },
            children: [],
          });
        },
      },
    ],
  };
};
