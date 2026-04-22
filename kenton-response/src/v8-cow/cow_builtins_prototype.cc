// A standalone prototype of the core data structure for copy-on-write
// built-ins. NOT integrated into V8 — see RFC.md for the integration plan.
//
// This file implements the "easy half" of the proposal in pure C++ so we
// can validate the idea before proposing it as a V8 patch:
//
//   1. A process-wide SharedBuiltinsHeap, mmap'd once.
//   2. A per-isolate writable shadow heap.
//   3. An ObjectRef that resolves to shared unless the object has been
//      promoted.
//   4. A Write() path that copies-on-first-write into the shadow heap.
//
// Build: see build_prototype.sh (doesn't need V8).
//
// This is not a working JS engine. It's a data-structure proof that the
// sharing + promotion path is non-pathological. The V8 integration would
// plug this scheme into v8::internal::ReadOnlyHeap + the object model.

#include <atomic>
#include <cassert>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <unordered_map>
#include <vector>

#if defined(__unix__) || defined(__APPLE__)
#include <sys/mman.h>
#include <unistd.h>
#endif

namespace cow {

// Object identifier — in V8 this would be a HeapObject pointer. For the
// prototype, a uint64_t index.
using ObjectId = uint64_t;

// Placeholder "object": a small slab of bytes. Real built-ins are
// more structured (Map + properties + in-object fields) but the sharing
// behavior is orthogonal.
struct Object {
  uint64_t type_tag;
  uint32_t size_bytes;
  uint8_t payload[48];  // inline to keep the prototype simple
};

// Process-wide shared, read-only built-ins. Constructed once at startup.
class SharedBuiltinsHeap {
 public:
  SharedBuiltinsHeap(size_t capacity_bytes) : cap_(capacity_bytes) {
#if defined(__unix__) || defined(__APPLE__)
    // mmap anonymous shared — real impl would mmap the snapshot file
    // and MAP_PRIVATE | MAP_FIXED_NOREPLACE into a known address range.
    base_ = static_cast<uint8_t*>(
        mmap(nullptr, cap_, PROT_READ | PROT_WRITE,
             MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));
    assert(base_ != MAP_FAILED);
#else
    base_ = new uint8_t[cap_];
#endif
  }

  ~SharedBuiltinsHeap() {
#if defined(__unix__) || defined(__APPLE__)
    munmap(base_, cap_);
#else
    delete[] base_;
#endif
  }

  // Write phase — called once, before any isolate runs.
  ObjectId InstallBuiltin(const Object& obj) {
    assert(!sealed_.load());
    ObjectId id = next_id_++;
    size_t off = id * sizeof(Object);
    assert(off + sizeof(Object) <= cap_);
    std::memcpy(base_ + off, &obj, sizeof(Object));
    return id;
  }

  // Freeze: flip to read-only. After this, any write will segfault.
  void Seal() {
    sealed_.store(true);
#if defined(__unix__) || defined(__APPLE__)
    mprotect(base_, cap_, PROT_READ);
#endif
  }

  const Object* Get(ObjectId id) const {
    return reinterpret_cast<const Object*>(base_ + id * sizeof(Object));
  }

  uint64_t count() const { return next_id_; }

 private:
  uint8_t* base_;
  size_t cap_;
  std::atomic<uint64_t> next_id_{0};
  std::atomic<bool> sealed_{false};
};

// Per-isolate writable shadow heap.
class IsolateShadow {
 public:
  explicit IsolateShadow(const SharedBuiltinsHeap* shared) : shared_(shared) {}

  // Resolve: give me the current state of the object with ID `id`.
  // If promoted, return the shadow copy; else return the shared copy.
  const Object* Resolve(ObjectId id) const {
    auto it = promoted_.find(id);
    if (it != promoted_.end()) return &it->second;
    return shared_->Get(id);
  }

  // Write: the user code attempts to mutate property `offset` of
  // object `id`. On first write for that `id`, copy the shared object
  // into the shadow heap, then apply the mutation.
  //
  // This is the "write barrier" equivalent — in V8 terms, SetProperty
  // would route through this when the target resides in kReadOnlyShared.
  void Write(ObjectId id, uint32_t offset, uint8_t byte) {
    auto it = promoted_.find(id);
    if (it == promoted_.end()) {
      // COW promote: copy the shared object.
      Object copy = *shared_->Get(id);
      auto pair = promoted_.emplace(id, copy);
      it = pair.first;
      ++promotions_;
    }
    assert(offset < sizeof(it->second.payload));
    it->second.payload[offset] = byte;
  }

  size_t promotion_count() const { return promotions_; }
  size_t promoted_bytes() const { return promoted_.size() * sizeof(Object); }

 private:
  const SharedBuiltinsHeap* shared_;
  std::unordered_map<ObjectId, Object> promoted_;
  size_t promotions_ = 0;
};

}  // namespace cow

// -----------------------------------------------------------------------
// Demo + self-check.
// -----------------------------------------------------------------------
int main() {
  using namespace cow;

  // 1) Process startup: install 30 "built-ins" into the shared heap.
  SharedBuiltinsHeap shared(4 * 1024 * 1024);  // 4 MB
  std::vector<ObjectId> builtin_ids;
  for (int i = 0; i < 30; ++i) {
    Object o{};
    o.type_tag = 0x1000 + i;
    o.size_bytes = sizeof(Object);
    std::memset(o.payload, 'A' + (i % 26), sizeof(o.payload));
    builtin_ids.push_back(shared.InstallBuiltin(o));
  }
  shared.Seal();
  std::fprintf(stderr, "shared: installed %llu built-ins, sealed.\n",
               static_cast<unsigned long long>(shared.count()));

  // 2) Simulate 1000 isolates, each doing a modest mix of reads and
  // (occasional) writes. Report total promoted bytes across all isolates.
  size_t total_promoted_bytes = 0;
  size_t total_isolates = 1000;
  size_t total_reads = 0;
  size_t total_writes = 0;
  for (size_t i = 0; i < total_isolates; ++i) {
    IsolateShadow shadow(&shared);
    // Most isolates only READ built-ins — realistic for serverless.
    for (size_t r = 0; r < 100; ++r) {
      const Object* o = shadow.Resolve(builtin_ids[r % builtin_ids.size()]);
      (void)o;
      ++total_reads;
    }
    // 10% of isolates mutate one built-in (e.g., monkey-patching
    // Array.prototype.foo). This promotes exactly one object.
    if (i % 10 == 0) {
      shadow.Write(builtin_ids[0], 0, 'X');
      ++total_writes;
    }
    total_promoted_bytes += shadow.promoted_bytes();
  }

  // 3) Report.
  size_t per_isolate_builtins_if_not_shared =
      shared.count() * sizeof(Object);
  size_t naive_total =
      total_isolates * per_isolate_builtins_if_not_shared;
  size_t cow_total = /* shared slab */ shared.count() * sizeof(Object) +
                     total_promoted_bytes;
  std::fprintf(stderr,
               "summary:\n"
               "  isolates:                 %zu\n"
               "  shared built-ins (once):  %zu B\n"
               "  naive per-isolate cost:   %zu B\n"
               "  naive total:              %zu B\n"
               "  COW total:                %zu B\n"
               "  savings:                  %.2f%%\n"
               "  reads: %zu  writes: %zu\n",
               total_isolates, per_isolate_builtins_if_not_shared,
               per_isolate_builtins_if_not_shared, naive_total, cow_total,
               100.0 * (1.0 - (double)cow_total / naive_total),
               total_reads, total_writes);
  return 0;
}
