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
      logo: { src: "../logo/necrassrs.svg" },
      social: [
        { icon: "github", label: "GitHub", href: "https://github.com/Necrass-Dev/NecrassRs" },
      ],
      customCss: ["./src/styles/docs.css"],
      sidebar: [
        {
          label: "Getting Started",
          items: [
            {
              label: "What is NecrassRs?",
              slug: "docs",
            },
            "docs/installation",
            "docs/tutorial",
          ],
        },
        {
          label: "Example Projects",
          items: ["docs/personnel-management", "docs/subscription-console"],
        },
        {
          label: "Guides",
          items: [
            {
              label: "Schema & Resolvers",
              items: ["docs/custom-scalars", "docs/resolver-files"],
            },
            {
              label: "Serving",
              items: ["docs/integration", "docs/graphiql"],
            },
            "docs/subscriptions",
            "docs/opentelemetry",
          ],
        },
        {
          label: "Reference",
          items: [
            "docs/cli",
            "docs/build-configuration",
            "docs/types",
            "docs/graphql-over-http",
            "docs/tracing-spans",
            {
              label: "Rust API (docs.rs)",
              link: "https://docs.rs/necrassrs/",
              attrs: { target: "_blank", rel: "noopener" },
            },
          ],
        },
        {
          label: "Concepts",
          items: [
            "docs/schema-first-workflow",
            "docs/execution-model",
            {
              label: "Reed",
              slug: "docs/reed",
            },
          ],
        },
        {
          label: "Contributing",
          items: ["docs/contributing", "docs/writing-an-adapter"],
        },
      ],
    }),
  ],
});
