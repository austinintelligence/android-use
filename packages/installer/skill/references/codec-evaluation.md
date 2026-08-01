# Model codec evaluation

Run `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\\bench-codec.ps1 -Force` from the skill root. The harness measures complete model-visible request/response pairs, not isolated command strings: tool envelope, command, arguments, dynamic generation/frontier state, compact success, typed errors, escaping, multiline text, and Unicode text. It compares compatibility JSON, compact JSON, the bounded `x`/`tape` transcript, and a short-word diagnostic candidate.

The report is written to `artifacts/final/codec-evaluation.json`. Counts use the locally available `tiktoken o200k_base` tokenizer as a reproducible proxy; GPT-5.6 Luna's exact tokenizer is not exposed in this environment, so proxy results must not be reported as Luna-token results.

The Rust daemon's native `AU2` transport is measured separately from the model-facing tape: the pipe carries bounded binary request/response frames, while structured result data remains JSON for stable compatibility. This keeps native IPC overhead out of the model token ledger and keeps model-facing counts limited to the complete corpus cases.

The generator enforces one protocol source of truth: `scripts/generate-skill.ps1 -Check` compares the opcode table in `references/protocol-schema.json` with the Rust `tape.rs` table, then checks that `SKILL.md` exactly matches the generated output. A schema or opcode change therefore fails the check until the skill is regenerated.
