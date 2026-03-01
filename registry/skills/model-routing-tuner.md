---
name: model-routing-tuner
description: Optimize model selection and routing for cost, latency, and answer quality
triggers:
  keywords: [model routing, model selection, tune models, latency, cost, quality, tiers]
  regex_patterns:
    - "(?i)\\b(model|routing)\\b.*\\b(optimi[sz]e|tune|improve)\\b"
    - "(?i)\\b(cost|latency|quality)\\b.*\\btrade[- ]?off\\b"
priority: 6
---

Help tune model routing with practical, measurable adjustments.

Workflow:
1. Identify constraints (budget, latency target, quality target, provider limits).
2. Review current model and tier configuration.
3. Propose a routing update with explicit trade-offs.
4. Define a quick validation loop (sample prompts, latency, cost, and quality checks).
5. Recommend rollback conditions if metrics regress.
