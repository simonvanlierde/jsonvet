// The docs collection is the repository's docs/ folder: plain Markdown with no front matter, so
// the title, summary and edit link come from the file itself.
import { i18nLoader } from "@astrojs/starlight/loaders";
import { docsSchema, i18nSchema } from "@astrojs/starlight/schema";
import { defineCollection } from "astro:content";
import { glob } from "astro/loaders";
import { resolve } from "node:path";
import { describe, REPO, ROOT } from "../lib.mjs";

const project = process.cwd();
const files = glob({
  // The decisions' README is a page of its own, at decisions/, in src/pages.
  pattern: ["*.md", "decisions/[0-9]*.md"],
  base: "../../docs",
  generateId: ({ entry }) => `docs/${entry.replace(/\.md$/, "")}`,
});
const loader = {
  ...files,
  load: (ctx) =>
    files.load({
      ...ctx,
      parseData: ({ filePath, data, ...rest }) => {
        const src = resolve(project, filePath).slice(ROOT.length + 1);
        const { title, summary } = describe(src);
        return ctx.parseData({
          ...rest,
          filePath,
          data: { title, description: summary, editUrl: `${REPO}/blob/main/${src}`, ...data },
        });
      },
    }),
};

export const collections = {
  docs: defineCollection({ loader, schema: docsSchema() }),
  // Empty: the docs are English only. Declared so Starlight finds the collection.
  i18n: defineCollection({ loader: i18nLoader(), schema: i18nSchema() }),
};
