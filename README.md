# agent-add-helper-workflow

Add Helper-specific agent launcher, review gates, and JSON-contract drift
checks for Basalt.

## Capabilities

- `AGENT_LAUNCHER`: OpenCode-based workflow agent (`opencode run --format json --auto`).
- `REVIEW_ACTIONS`: priority-200 gates that run before generic Gleam actions:
  - `gleam format --check src test` (FormatCheck)
  - `gleam test` (Test)
  - `gleam build` (Build)
- `DIAGNOSTICS`: warns when the Gleam and Kotlin JSON models drift.

## Contract drift

`diagnose` runs on `models.gleam` and `Models.kt`. It finds the counterpart
file (`android/app/src/main/java/com/adevcorn/addhelper/Models.kt` or
`src/add_helper/models.gleam`, then falls back to a workspace search), parses
`pub type` constructors and Kotlin `data class` primary constructors, and
emits a warning when:

- a type present in both files has fields only on one side, or
- a Kotlin type has no Gleam counterpart.

Field names are normalised (case-insensitive, `_` removed) so `start_time`
and `startTime` compare equal. Gleam-only types are ignored because Models.kt
intentionally mirrors only the JSON boundary.

Example warning:

```
add-helper JSON contract drift — type `TaskView` missing in Models.kt:
["itemid", "sender"] extra in Models.kt: ["direction", "id", "lane",
"sourcesender", "tags"]
```

## Build

```bash
cargo build --release --target wasm32-unknown-unknown
```

Artifact:
`target/wasm32-unknown-unknown/release/agent_add_helper_workflow.wasm`.
