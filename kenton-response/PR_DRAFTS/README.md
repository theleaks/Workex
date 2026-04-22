# PR / RFC drafts

Three artifacts, each with its own audience and venue.

| File | Where it goes | What it is |
|---|---|---|
| `workerd-issue-6595-response.md` | Comment on cloudflare/workerd#6595 | The honest response — what was wrong, what's done about it, where the work lives |
| `v8-dev-rfc.md` | v8-dev@googlegroups.com | RFC seeking go/no-go on the COW built-ins work before writing the full V8 patch |
| `workerd-benchmark-harness-pr.md` | PR description for cloudflare/workerd | If workerd team wants the P1/P2 benchmarks in-tree |

## Suggested order

1. **First** — post the `workerd-issue-6595-response.md` comment on
   #6595. This is the acknowledgement and the pointer to the work.
   It costs nothing to send and is the bit that's *owed*.

2. **Second** — if Kenton / the workerd team signals interest, send
   the `v8-dev-rfc.md` to v8-dev@. Don't send it speculatively —
   v8-dev@ is a busy list, and the RFC is strongest when it shows up
   with "the workerd folks think this would help us, here's the
   shape, would v8 consider it."

3. **Third** — only if the workerd team explicitly wants the benchmark
   tooling in-tree, open the harness PR. Otherwise keep it in this
   repo as the reference implementation.

## What to replace before sending

- `[your name]` in `v8-dev-rfc.md`
- `bedirhanplatinplus/workex` URL in both files — replace if the repo
  moves or renames.
- Double-check the V8 branch (`branch-heads/12.8`) matches what's
  current at send time.
