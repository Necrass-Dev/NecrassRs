import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import solid from "@astrojs/solid-js";

export default defineConfig({
  site: "https://necrass.rs",
  trailingSlash: "always",
  integrations: [
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
        { label: "Start here", items: [{ label: "Introduction", slug: "docs" }] },
        { label: "Server guides", items: ["docs/types", "docs/http", "docs/graphiql"] },
        {
          label: "Design & development",
          items: [
            "docs/architecture",
            "docs/specs",
            "docs/apollo-compiler",
            "docs/experiments/default-cycle-comparison",
          ],
        },
      ],
    }),
  ],
});
