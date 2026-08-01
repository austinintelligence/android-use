# Troubleshooting

For repeatable timing evidence, run scripts\bench.ps1 -Serial ENDPOINT -Samples 30 -Warmup 5; it writes a bounded JSON report under the AU artifact directory.

`E_DEVICE` means no online endpoint reported the pinned hardware serial. Run `au d -j`; do not force a same-model device. `E_STALE` means refresh the UI snapshot. `E_CAPABILITY` means enable the named helper service/permission or fall back only to supported coordinate/read-only operations.

For ADB authorization, unlock the tablet, confirm the debugging prompt, then retry. For daemon recovery, inspect `au daemon status`; a stale PID/state file is not enough to terminate anything. `au doctor` reports owned forwards, helper availability, and location recovery state.

After some Android 13 tablet reboots, `dumpsys user` may report `State: RUNNING_LOCKED`. In that state Android can list installed components while returning `not found` for explicit activities and services. Wake and unlock the tablet normally; do not reinstall the helper or clear its data to address this condition. Re-check `dumpsys user` until User 0 is unlocked, then retry `ui snap`.
