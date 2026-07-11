# Chrome and feed operations

The feed runtime launches its own headless Chrome/Chromium process with a
temporary user-data directory and a bounded startup timeout. It creates one
Live target for Football and one for Basketball; existing browser targets are
never navigated or closed.

Set `OwnedBrowserConfig::executable` when Chrome is not discoverable on `PATH`.
Otherwise the runtime checks `google-chrome`, `chromium`,
`chromium-browser`, and `chrome`. Health is reported as `Starting`, `Healthy`,
`Stale`, `Disconnected`, or `Stopped`. A shutdown closes only registered
targets, terminates the owned process, and removes its temporary profile.

Source handling prefers network responses, then the application store, then
DOM snapshots. Each source must identify the Live filter, the expected sport,
match identity, status, competition, and both teams. A shape mismatch is
reported as `SOURCE_CHANGED` and is not converted into a finished match.

If startup or health checks fail, verify the executable path and that the
temporary profile location is writable. Do not point the runtime at a user's
remote-debugging endpoint; the ownership boundary is intentional.
