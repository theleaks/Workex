# V8-COW hooks — edits required in existing V8 source

`patch_skeleton.diff` is additive only (adds `cow-shared-builtins.{h,cc}`
under `src/heap/`). To actually activate the feature, four edits to
existing V8 files are required. They're documented here rather than in
the diff because exact line numbers drift per V8 revision, so a
line-numbered hunk would fail to apply on anything but the exact tree
it was written against.

Target V8: `branch-heads/12.8`. Adjust if your tree differs.

## 1. Flag definition

**File:** `src/flags/flag-definitions.h`

Add alongside the other experimental isolate flags:

```cpp
// Copy-on-write shared built-ins. See RFC in
// cloudflare/workerd#6595-response/src/v8-cow/RFC.md
DEFINE_BOOL(shared_readonly_builtins, false,
            "share built-in objects across isolates in the same process "
            "via a read-only mmap'd slab; copy-on-write on first "
            "mutation. EXPERIMENTAL.")
```

## 2. BUILD.gn — compile the new files

**File:** `BUILD.gn` (top-level)

Add `"src/heap/cow-shared-builtins.cc"` and
`"src/heap/cow-shared-builtins.h"` to the `v8_base_without_compiler`
sources list (the group that includes other `src/heap/*.cc` files).

## 3. Wire roots on isolate setup

**File:** `src/heap/read-only-heap.cc`

Locate `ReadOnlyHeap::SetUp` (or whichever method is canonically called
once per isolate to populate read-only space from the snapshot). At the
tail of that method, add:

```cpp
// COW: if --shared-readonly-builtins is on, redirect the RO root
// pointers into the process-wide shared slab instead of keeping the
// per-isolate RO space contents. Leave the space allocated-but-empty
// so existing assertions still hold.
if (v8_flags.shared_readonly_builtins) {
  CowSharedBuiltins::WireIsolateRoots(isolate);
}
```

Add at the top:

```cpp
#include "src/heap/cow-shared-builtins.h"
```

## 4. Intercept writes to shared objects

**File:** `src/objects/js-objects.cc`

Locate `JSReceiver::SetPropertyOrElement` (or the canonical hot-path
property-set used by `Set`). Early in the function, after the receiver
is known but before the store actually commits, add the COW check:

```cpp
// COW: if the target is in the shared read-only slab, promote it into
// the isolate's mutable heap before storing. This check must be cheap
// in the common case — target NOT in shared slab — so the predicate
// should be a single-bit tag check on the object's page header rather
// than a range comparison.
if (v8_flags.shared_readonly_builtins &&
    HeapLayout::InSharedReadOnlyBuiltinSlab(*object)) {
  object = handle(
      CowSharedBuiltins::PromoteOnWrite(isolate, *object), isolate);
}
```

(The predicate `HeapLayout::InSharedReadOnlyBuiltinSlab` is itself part
of what needs to be implemented — a new method on `HeapLayout` that
checks whether a pointer falls inside `[CowSharedBuiltins::base(),
CowSharedBuiltins::base() + CowSharedBuiltins::size())`. In a real
implementation this would be a page-header tag check so it's O(1) and
branchless.)

Add at the top:

```cpp
#include "src/heap/cow-shared-builtins.h"
```

## 5. (Optional) Test status file

**File:** `test/mjsunit/mjsunit.status`

Once `PromoteOnWrite` is implemented, triage the mjsunit suite under
`--shared-readonly-builtins` and add a comment block documenting any
expected skips. The goal is zero skips — COW must be observationally
invisible to JS.

## Verifying the edits

After applying `patch_skeleton.diff` and making the above edits:

```bash
cd vendor/v8
ninja -C out/x64.release v8_monolith
# Expect: builds cleanly. Flag is registered but a no-op at runtime.
#
# Enabling --shared-readonly-builtins will hit UNIMPLEMENTED() in
# PromoteOnWrite — this is the expected state for the skeleton.
```

That's the signal the skeleton has landed correctly. The real work —
populating the slab from snapshot, implementing `PromoteOnWrite`,
wiring the root table — is the 4-8 weeks the RFC is asking permission
to spend.
