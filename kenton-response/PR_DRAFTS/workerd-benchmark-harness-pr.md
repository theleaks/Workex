# workerd PR — isolate benchmark harness

**Target branch:** `cloudflare/workerd:main`
**Kind:** additive only; no runtime changes
**Size:** ~500 lines of C++ in a new `src/workerd/tests/benchmarks/` folder
**Associated issue:** cloudflare/workerd#6595

## PR title

```
Add isolate serialisation + resume-cost benchmark harness
```

## PR description

Adds two benchmark tools to workerd's test tree so anyone working on
isolate memory or resume costs can measure them without writing new
instrumentation each time.

### What's added

`src/workerd/tests/benchmarks/p1_serialization_cost.cc` — measures V8
ValueSerializer round-trip cost across five live-state sizes (50 B,
500 B, 5 KB, 50 KB, 500 KB). Emits JSON with mean/median/p99 for
serialize, deserialize, and round-trip.

`src/workerd/tests/benchmarks/p2_resume_cost.cc` — measures fresh-env
resume cost in three isolation configurations (fresh isolate, pooled
isolate + fresh context, pooled context). Includes an isolation leak
test for the "pooled context" config so its optimistic numbers come
with the correct caveat.

Neither binary is run as part of `bazel test //...`; they live under
`benchmarks/` and are opt-in via their own BUILD target.

### Why

cloudflare/workerd#6595 surfaced that there was no shared tool for
measuring these costs, which made it easy for outside contributors
(including the original reporter of #6595 — me) to measure the wrong
thing and make overclaims. A repo-local harness that the workerd team
controls and can point people at would prevent this.

### What's *not* in this PR

- No workerd runtime changes.
- No claims about any specific alternative runtime.
- No changes to existing tests or CI.

### Testing

- `bazel build //src/workerd/tests/benchmarks:p1_serialization_cost`
  passes on Linux x86_64.
- `bazel build //src/workerd/tests/benchmarks:p2_resume_cost` passes
  on Linux x86_64.
- Running `bazel run //...:p1_serialization_cost` emits a
  well-formed JSON array covering all five size classes.
- Running `bazel run //...:p2_resume_cost` emits a well-formed JSON
  array covering configs A/B/C, with the C run correctly reporting
  `isolation_safe: false`.

### Follow-ups (separate PRs if this one's accepted)

- Hook benchmarks into a nightly job and archive JSON results so
  regressions are visible.
- Add an XL workload class (500 KB) and a "streaming" one (built up
  incrementally across 100 awaits).
