# Stepwise Disabled Injection Design

## Goal

Prevent Codex++ from injecting the Stepwise runtime when Stepwise is disabled, and
ensure a runtime that was already active cannot recreate its floating UI after the
setting is turned off.

## Current Behavior

`injection_script_with_settings` always appends `stepwise_script()`. The injected
runtime loads settings later, so a disabled setting blocks the backend request but
does not prevent the Stepwise code from reaching the Codex page. Existing delayed
scan and DOM-ready paths can create UI after the feature is disabled.

## Design

The launcher already passes `BackendSettings` to `injection_script_with_settings`.
Use `codex_app_stepwise_enabled` there to append the Stepwise script only when it
is enabled. The renderer menu uses optional chaining for the Stepwise panel, so it
continues to render and persist the toggle when the runtime is absent.

The Stepwise runtime will additionally guard scan scheduling and DOM-ready work
with `state.settings?.enabled === true`. Its disabled path will call
`stopRuntime()`, which removes the root, style, observer, and timer.

Changing the setting changes the new-document bundle, so the UI copy will state
that restarting Codex++ is required for the change to take effect.

## Validation

- A disabled settings bundle must not contain a Stepwise runtime marker.
- An enabled settings bundle must contain the Stepwise runtime and the renderer
  toggle integration.
- The runtime source must show enabled guards before scan, scheduling, and
  DOM-ready activation paths.
- Run the targeted core bridge tests and Rust formatting checks.
