import { createSignal, For, onCleanup, onMount, Show } from "solid-js";

type Phase = "initial" | "schema" | "synced";
type Props = {
  schemaBefore: string;
  schemaAfter: string;
  resolverBefore: string;
  resolverAfter: string;
};

export default function Workflow(props: Props) {
  const [phase, setPhase] = createSignal<Phase>("initial");
  const [enhanced, setEnhanced] = createSignal(false);
  let story!: HTMLElement;
  const steps: { phase: Phase; label: string }[] = [
    { phase: "initial", label: "Define" },
    { phase: "schema", label: "Evolve" },
    { phase: "synced", label: "Build" },
  ];

  onMount(() => {
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    const pinned = window.matchMedia("(min-width: 901px) and (min-height: 780px)");
    let frame = 0;
    const update = () => {
      const top = story.getBoundingClientRect().top;
      document.body.dataset.tone = top < window.innerHeight * 0.55 ? "dark" : "light";
      const progress =
        pinned.matches && !reduced.matches
          ? -top / (story.offsetHeight - window.innerHeight)
          : (window.innerHeight - top) / Math.min(window.innerHeight, story.offsetHeight);
      setPhase(progress >= 0.68 ? "synced" : progress >= 0.36 ? "schema" : "initial");
    };
    const schedule = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        update();
      });
    };
    setEnhanced(true);
    update();
    window.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    reduced.addEventListener("change", schedule);
    onCleanup(() => {
      cancelAnimationFrame(frame);
      window.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
      reduced.removeEventListener("change", schedule);
      delete document.body.dataset.tone;
    });
  });

  return (
    <section
      ref={(element) => {
        story = element;
      }}
      class="story"
      classList={{ enhanced: enhanced() }}
      id="workflow"
      aria-labelledby="workflow-title"
      data-phase={phase()}
    >
      <div class="story-stage">
        <div class="story-heading">
          <div>
            <p class="eyebrow">The contract comes first.</p>
            <h2 id="workflow-title">
              You write the schema.
              <br />
              <span>Rust gets the shape.</span>
            </h2>
          </div>
          <p class="story-description">
            Define your GraphQL API in SDL.
            <br />
            Cargo generates the contracts.
            <br />
            You bring the implementation.
          </p>
        </div>
        <div class="editors">
          <article class="editor schema-editor" aria-label="GraphQL schema">
            <header class="editor-heading">
              <span>
                <span class="file-icon" aria-hidden="true">
                  ◇
                </span>{" "}
                schema.graphql
              </span>
              <span class="file-language">GraphQL SDL</span>
            </header>
            <div class="code-versions">
              <Show when={!enhanced() || phase() === "initial"}>
                <div class="code-version" data-code="schema-before">
                  <span class="fallback-label">Initial schema</span>
                  <div innerHTML={props.schemaBefore} />
                </div>
              </Show>
              <Show when={!enhanced() || phase() !== "initial"}>
                <div class="code-version added-schema" data-code="schema-after">
                  <span class="fallback-label">Add a field</span>
                  <div innerHTML={props.schemaAfter} />
                </div>
              </Show>
            </div>
            <div class="schema-note">
              <span class="schema-symbol" aria-hidden="true">
                {"{ }"}
              </span>
              <p>
                The source of truth.
                <br />
                <span>Your API starts here.</span>
              </p>
            </div>
            <footer class="editor-footer">
              <span class="status-dot" aria-hidden="true" />
              <span>{phase() === "initial" ? "Your public contract" : "+ version: String!"}</span>
            </footer>
          </article>
          <div class="build-bridge" aria-hidden="true">
            <span>→</span>
          </div>
          <article class="editor resolver-editor" aria-label="Rust resolver implementation">
            <header class="editor-heading">
              <span>
                <span class="file-icon" aria-hidden="true">
                  ↳
                </span>{" "}
                resolvers.rs
              </span>
              <span class="file-language">Rust</span>
            </header>
            <div class="code-versions">
              <Show when={!enhanced() || phase() !== "synced"}>
                <div class="code-version" data-code="resolver-before">
                  <span class="fallback-label">Existing implementation</span>
                  <div innerHTML={props.resolverBefore} />
                </div>
              </Show>
              <Show when={!enhanced() || phase() === "synced"}>
                <div class="code-version added-resolver" data-code="resolver-after">
                  <span class="fallback-label">
                    After cargo build: a new stub, existing body preserved
                  </span>
                  <div innerHTML={props.resolverAfter} />
                </div>
              </Show>
            </div>
            <footer class="editor-footer">
              <span class="status-dot" aria-hidden="true" />
              <span>
                {phase() === "synced" ? "New stub added · hello preserved" : "Your implementation"}
              </span>
            </footer>
          </article>
        </div>
        <div class="build-line">
          <span class="mono">
            <span class="prompt" aria-hidden="true">
              $
            </span>{" "}
            cargo build
          </span>
          <span>
            {phase() === "schema"
              ? "Schema changed. Ready to build."
              : phase() === "synced"
                ? "Contracts regenerated. Resolver synchronized."
                : "SDL → Rust contracts + resolver scaffolding"}
          </span>
          <span class="build-check" aria-hidden="true">
            {phase() === "synced" ? "✓" : "↵"}
          </span>
        </div>
        <div class="story-bottom">
          <p aria-live="polite">
            {phase() === "synced"
              ? "The new stub is yours to implement. Your hello body stays intact."
              : phase() === "schema"
                ? "One new field in SDL. Build to update the Rust contract."
                : "Add a field. Watch Rust follow."}
          </p>
          <ol class="phase-indicators" aria-label="Schema evolution steps">
            <For each={steps}>
              {(step, index) => (
                <li aria-current={phase() === step.phase ? "step" : undefined}>
                  0{index() + 1} <span>{step.label}</span>
                </li>
              )}
            </For>
          </ol>
        </div>
      </div>
    </section>
  );
}
