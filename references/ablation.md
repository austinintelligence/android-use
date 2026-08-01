# Ablation evidence

Run from the skill root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\bench-ablation.ps1 -Force
```

The generated `artifacts/final/ablation-matrix.json` is the source of truth for E0-E16. `MEASURED` means the isolated variable has direct correctness/performance evidence. `PARTIAL` means the behavior is implemented and live-tested, but the retained run combined it with other changes and cannot support causal attribution. `REJECTED` means the experiment was intentionally not kept. `PROXY_ONLY` is used only for local `o200k_base` payload counts; it is never an exact GPT-5.6 Luna token count.

Every row includes the required evidence-field checklist. Unavailable model turns, exact tokens, or isolated A/B fields remain explicit rather than being inferred from byte counts or command timings. The report therefore supports optimization decisions without turning combined-candidate evidence into a false ablation claim.
