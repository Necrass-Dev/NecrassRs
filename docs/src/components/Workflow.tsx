import { createSignal, For, onCleanup, onMount } from "solid-js";
import { gsap } from "gsap";
import { ScrollTrigger } from "gsap/ScrollTrigger";

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
  const [progress, setProgress] = createSignal(0);
  const [reducedMotion, setReducedMotion] = createSignal(false);
  let selectPhase: (phase: Phase) => void = () => {};
  let story!: HTMLElement;
  const steps: { phase: Phase; label: string }[] = [
    { phase: "initial", label: "Define" },
    { phase: "schema", label: "Evolve" },
    { phase: "synced", label: "Build" },
  ];

  onMount(() => {
    gsap.registerPlugin(ScrollTrigger);
    const media = gsap.matchMedia();
    const tone = ScrollTrigger.create({
      trigger: story,
      start: "top 55%",
      onEnter: () => {
        document.body.dataset.tone = "dark";
      },
      onLeaveBack: () => {
        document.body.dataset.tone = "light";
      },
    });
    media.add(
      {
        desktop: "(min-width: 901px)",
        reduced: "(prefers-reduced-motion: reduce)",
        motion: "(prefers-reduced-motion: no-preference)",
      },
      (context) => {
        setEnhanced(true);
        setReducedMotion(Boolean(context.conditions?.reduced));
        const select = gsap.utils.selector(story);
        const position = { value: 0 };
        if (context.conditions?.reduced) {
          selectPhase = (next) => {
            setPhase(next);
            setProgress(next === "initial" ? 0 : next === "schema" ? 0.5 : 1);
          };
        } else {
          gsap.set(select('[data-code$="after"]'), { autoAlpha: 0 });
          const timeline = gsap.timeline({
            defaults: { ease: "none" },
            scrollTrigger: {
              trigger: story,
              start: "top top",
              end: context.conditions?.desktop
                ? () => `+=${window.innerHeight * 1.6}`
                : "bottom bottom",
              pin: Boolean(context.conditions?.desktop),
              scrub: 0.35,
              invalidateOnRefresh: true,
            },
          });
          timeline
            .to(
              position,
              {
                value: 1,
                duration: 1,
                onUpdate: () => {
                  setProgress(position.value);
                  setPhase(
                    position.value < 0.36 ? "initial" : position.value < 0.72 ? "schema" : "synced",
                  );
                },
              },
              0,
            )
            .to(select('[data-code="schema-before"]'), { autoAlpha: 0, duration: 0.16 }, 0.28)
            .to(select('[data-code="schema-after"]'), { autoAlpha: 1, duration: 0.16 }, 0.28)
            .to(select('[data-code="resolver-before"]'), { autoAlpha: 0, duration: 0.16 }, 0.64)
            .to(select('[data-code="resolver-after"]'), { autoAlpha: 1, duration: 0.16 }, 0.64);
          selectPhase = (next) => {
            const trigger = timeline.scrollTrigger!;
            const target = next === "initial" ? 0.1 : next === "schema" ? 0.52 : 0.9;
            window.scrollTo({
              top: trigger.start + (trigger.end - trigger.start) * target,
              behavior: "smooth",
            });
          };
        }
        return () => {
          setEnhanced(false);
          setPhase("initial");
          setProgress(0);
        };
      },
      story,
    );
    let active = true;
    void document.fonts.ready.then(() => {
      if (active) ScrollTrigger.refresh();
    });
    onCleanup(() => {
      active = false;
      media.revert();
      tone.kill();
      delete document.body.dataset.tone;
    });
  });

  return (
    <section
      ref={(element) => {
        story = element;
      }}
      class="story"
      classList={{ enhanced: enhanced(), "reduced-motion": reducedMotion() }}
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
              <div
                class="code-version"
                data-code="schema-before"
                aria-hidden={enhanced() && phase() !== "initial"}
                inert={enhanced() && phase() !== "initial"}
              >
                <span class="fallback-label">Initial schema</span>
                <div innerHTML={props.schemaBefore} />
              </div>
              <div
                class="code-version added-schema"
                data-code="schema-after"
                aria-hidden={enhanced() && phase() === "initial"}
                inert={enhanced() && phase() === "initial"}
              >
                <span class="fallback-label">Add a field</span>
                <div innerHTML={props.schemaAfter} />
              </div>
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
              <div
                class="code-version"
                data-code="resolver-before"
                aria-hidden={enhanced() && phase() === "synced"}
                inert={enhanced() && phase() === "synced"}
              >
                <span class="fallback-label">Existing implementation</span>
                <div innerHTML={props.resolverBefore} />
              </div>
              <div
                class="code-version added-resolver"
                data-code="resolver-after"
                aria-hidden={enhanced() && phase() !== "synced"}
                inert={enhanced() && phase() !== "synced"}
              >
                <span class="fallback-label">
                  After cargo build: a new stub, existing body preserved
                </span>
                <div innerHTML={props.resolverAfter} />
              </div>
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
                ? "One new field in SDL. Keep scrolling to build."
                : "Keep scrolling to evolve your schema."}
          </p>
          <ol
            class="phase-indicators"
            aria-label="Schema evolution steps"
            style={{ "--progress": progress() }}
          >
            <For each={steps}>
              {(step, index) => (
                <li>
                  <button
                    type="button"
                    aria-current={phase() === step.phase ? "step" : undefined}
                    onClick={() => selectPhase(step.phase)}
                  >
                    0{index() + 1} <span>{step.label}</span>
                  </button>
                </li>
              )}
            </For>
          </ol>
        </div>
      </div>
    </section>
  );
}
