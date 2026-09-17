# AI usage policy

NecrassRs permits AI assistance with code, tests, design, reviews, and documentation. Contributors remain responsible for understanding and validating their submissions. The same quality standards apply with or without AI assistance.

See [AGENTS.md](AGENTS.md) for agent workflow and [docs/architecture.md](docs/architecture.md) for product design. Official repository documentation, including AI disclosures in repository documents, must be written in English.

## 1. Human responsibility and review

Every contribution must have a responsible human owner. AI authorship or review is not evidence of correctness.

Before submission, contributors must personally review the complete change and be able to explain its behavior and design choices. Check generated explanations against the facts and edit them where needed. AI self-review does not replace human review.

Local drafting and validation are allowed. Do not label unreviewed work as human-reviewed or treat it as a final submission.

## 2. Validation and quality

- Verify changed behavior with runnable checks. Generator changes must also validate the generated consumer code.
- Identify the platforms, features, and commands actually checked. Do not claim success for unrun checks or untested environments.
- Distinguish implemented features, design goals, and unresolved decisions. Verify external technical claims against sources and applicable versions.
- Expose and resolve conflicts between design and implementation. AI-generated convenience is not a reason to bypass contracts.
- Divide large changes into reviewable scopes. Avoid unrelated cleanup and speculative features.
- Do not conceal defects by removing tests, distorting expectations, or making unsupported performance or specification-compliance claims.

## 3. Disclosure

AI-assisted pull requests must identify the tools and models used, their scope, direct human edits, human review status, and validation results. Minor assistance such as wording corrections can be described in one line. Full conversations and verbatim prompts are not required.

Use these trailers for AI-assisted commits:

```text
Assisted-by: <tool>:<model identifier>
Prompt-summary: <request and material follow-up instructions relevant to this commit>
Human-changes: <direct human edits or None>
```

- Include an `Assisted-by` trailer for each assisting tool.
- Use verified tool and model identifiers. If the model is unavailable, record `unknown` rather than guessing.
- Keep `Prompt-summary` relevant to the committed changes.
- `Human-changes` records direct edits. Review, approval, and running tests are not direct edits.
- Use `None` when there were no direct human edits. Do not infer authorship from an existing diff; clarify uncertain attribution before finalizing the commit message.
- Reserve `Co-authored-by` for human contributors, not AI tools.

A pull request can summarize disclosure as follows. Contributions without AI assistance may state that none was used.

```text
AI assistance: Tools, models, and scope
Direct human edits: Changes or none
Human review: Complete or incomplete, with scope
Validation: Commands, results, and checks not run
```

Do not invent review or editing history. AI-drafted descriptions must not claim human review that has not been confirmed.

## 4. Information and source material

Do not send secrets, tokens, personal information, production data, or material without permission to disclose it to external AI services or public artifacts. Use minimal reproductions and synthetic data.

Do not paste private development journals or experiment records into public pull requests or prompt summaries. Include only shareable design conclusions and validation evidence. Check permissions and licenses for source material; AI generation does not remove the need to verify provenance.

For AI-generated images or diagrams in documentation, identify the tool and purpose and have a human check their accuracy. An explanatory diagram is not evidence of execution or measurement.

## 5. Application

Maintainers may request revision or defer contributions that lack validation, contain false disclosures, exceed a reviewable scope, or violate the design. This policy supports trustworthy, reviewable contributions and does not require any particular AI tool.

## Reference

This policy was informed by [Gelite's AI usage policy](https://github.com/gelite-dev/gelite/blob/main/AI_POLICY.md), particularly its disclosure, commit trailers, and human responsibility rules, and adapted to NecrassRs. Upstream policy changes do not automatically change this document.
