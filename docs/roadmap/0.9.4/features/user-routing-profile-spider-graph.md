# 0.9.4 Feature: User-Tunable Routing Profile + Spider Graph

## Objective

Expose a user-facing control surface for routing tradeoffs and make routing decisions inspectable in real time.

## Scope

- Add a three-axis routing profile in the dashboard:
  - `correctness`
  - `cost`
  - `speed`
- Render a spider graph for the active profile.
- Map profile sliders to safe, existing routing controls:
  - `models.routing.accuracy_floor`
  - `models.routing.cost_aware`
  - `models.routing.cost_weight`
  - `models.routing.confidence_threshold`
  - `models.routing.estimated_output_tokens`
- Add an explorable model-decision graph driven by routing telemetry.
- Refresh metrics view on both `model_selection` and `model_shift` websocket events.

## Mapping Rules (v0.9.4)

- `correctness -> accuracy_floor`
- `cost -> cost_weight` and `cost_aware = (cost > 0.01)`
- `speed -> confidence_threshold` via `0.95 - (0.35 * speed)`
- `speed -> estimated_output_tokens` via `clamp(200, 1200 - 800 * speed, 1200)`

## Guardrails

- Slider inputs are clamped to `[0.0, 1.0]`.
- Config updates go through `PUT /api/config`, preserving existing validation and runtime reload behavior.
- No direct mutation of provider keys or auth settings from this control surface.

## Telemetry UX

- Metrics page includes:
  - spider graph for current routing profile
  - explorable graph of model transitions over recent routing events
  - focus mode for a model node or a transition edge

## Acceptance Criteria

- Operators can adjust profile sliders and apply changes without restarting the daemon.
- `/api/config` validation rejects unsafe values.
- Metrics page updates when model routing or fallback shifts occur.
- Users can inspect transition continuity visually from recent model decisions.
