import { defineCollection } from "astro:content";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

export const collections = {
  docs: defineCollection({
    loader: docsLoader({
      // Keep public URLs stable while source files follow the sidebar sections.
      generateId: ({ entry }) =>
        entry
          .replace(/\.[^.]+$/, "")
          .replace(/^docs\/(?:introduction|guide|manual)\//, "docs/")
          .replace(/\/index$/, ""),
    }),
    schema: docsSchema(),
  }),
};
