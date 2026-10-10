import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import solid from "@astrojs/solid-js";
import mermaid from "astro-mermaid";

export default defineConfig({
  site: "https://necrass.rs",
  trailingSlash: "always",
  integrations: [
    mermaid({ enableLog: false }),
    solid(),
    starlight({
      title: "NecrassRs",
      description: "Your schema leads. Rust delivers.",
      logo: { src: "../logo/logo.svg" },
      social: [
        { icon: "github", label: "GitHub", href: "https://github.com/Necrass-Dev/NecrassRs" },
      ],
      customCss: ["./src/styles/docs.css"],
      sidebar: [
        {
          label: "Introduction",
          items: [
            {
              label: "What is NecrassRs?",
              slug: "docs",
            },
          ],
        },
        {
          label: "Guide",
          items: ["docs/installation", "docs/tutorial", "docs/personnel-management"],
        },
        {
          label: "Manual",
          items: ["docs/types", "docs/resolver-files", "docs/graphiql", "docs/integration"],
        },
      ],
    }),
  ],
});
