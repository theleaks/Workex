// Phase 1+2 prototype: shared RO slab + SharedContextTemplate + COW promote.
//
// This models the core data structures proposed in RFC.md:
//
//   SharedRoSlab         — process-wide, mmap'd, never freed
//   SharedContextTemplate — per-process invariant context state
//   IsolateShadow        — per-isolate mutable shadow with promote-on-write
//
// Realistic synthetic workload: 1000 tenant isolates, each runs a short
// handler. 95% never mutate built-ins. 5% mutate 1-3 properties.
//
// Build: g++ -O2 -std=c++20 -pthread data_structure_prototype.cc

#include <atomic>
#include <cassert>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <random>
#include <string>
#include <unordered_map>
#include <vector>

#if defined(__unix__) || defined(__APPLE__)
#include <sys/mman.h>
#endif

namespace v8_shared_heap {

using ObjectId = uint32_t;

// Tag stored in the high bits of a pointer/id: where does this object live?
enum class Location : uint8_t {
  kSharedRo = 0,   // the process-wide slab
  kSharedCtx = 1,  // the context template
  kPromoted = 2,   // promoted copy in isolate heap
};

struct Object {
  uint64_t type_tag;
  uint32_t size_bytes;
  uint8_t payload[48];
};

// --- Phase 1: process-wide shared read-only slab ---
class SharedRoSlab {
 public:
  explicit SharedRoSlab(size_t cap) : cap_(cap) {
#if defined(__unix__)
    base_ = static_cast<uint8_t*>(
        mmap(nullptr, cap_, PROT_READ | PROT_WRITE,
             MAP_PRIVATE | MAP_ANONYMOUS, -1, 0));
    assert(base_ != MAP_FAILED);
#else
    base_ = new uint8_t[cap_];
#endif
  }
  ~SharedRoSlab() {
#if defined(__unix__)
    munmap(base_, cap_);
#else
    delete[] base_;
#endif
  }

  ObjectId Install(const Object& obj) {
    assert(!sealed_.load());
    ObjectId id = next_id_++;
    std::memcpy(base_ + id * sizeof(Object), &obj, sizeof(Object));
    return id;
  }

  void Seal() {
    sealed_.store(true);
#if defined(__unix__)
    mprotect(base_, cap_, PROT_READ);
#endif
  }

  const Object* Get(ObjectId id) const {
    return reinterpret_cast<const Object*>(base_ + id * sizeof(Object));
  }

  uint32_t count() const { return next_id_.load(); }
  size_t byte_size() const {
    return static_cast<size_t>(next_id_.load()) * sizeof(Object);
  }

 private:
  uint8_t* base_;
  size_t cap_;
  std::atomic<ObjectId> next_id_{0};
  std::atomic<bool> sealed_{false};
};

// --- Phase 2: shared context template — invariant native context slots ---
class SharedContextTemplate {
 public:
  SharedContextTemplate() {
    // Simulate ~30 per-context slots (globalThis, built-in fn refs, etc).
    for (int i = 0; i < 30; ++i) {
      Object o{};
      o.type_tag = 0x2000 + i;
      o.size_bytes = sizeof(Object);
      std::memset(o.payload, 'P' + (i % 10), sizeof(o.payload));
      slots_.push_back(o);
    }
  }
  const Object* Get(ObjectId id) const {
    return &slots_[id % slots_.size()];
  }
  size_t count() const { return slots_.size(); }
  size_t byte_size() const { return slots_.size() * sizeof(Object); }

 private:
  std::vector<Object> slots_;
};

// --- Per-isolate shadow: COW promote on write ---
class IsolateShadow {
 public:
  IsolateShadow(const SharedRoSlab* ro, const SharedContextTemplate* ctx)
      : ro_(ro), ctx_(ctx) {}

  // Resolve: read object with (location, id). Returns pointer to either
  // shared original or promoted copy.
  const Object* Resolve(Location loc, ObjectId id) const {
    auto key = MakeKey(loc, id);
    auto it = promoted_.find(key);
    if (it != promoted_.end()) return &it->second;
    switch (loc) {
      case Location::kSharedRo: return ro_->Get(id);
      case Location::kSharedCtx: return ctx_->Get(id);
      case Location::kPromoted: return nullptr;  // shouldn't happen
    }
    return nullptr;
  }

  // Write: user code wants to modify. Promotes if needed, then applies.
  void Write(Location loc, ObjectId id, uint32_t offset, uint8_t byte) {
    auto key = MakeKey(loc, id);
    auto it = promoted_.find(key);
    if (it == promoted_.end()) {
      Object copy;
      switch (loc) {
        case Location::kSharedRo: copy = *ro_->Get(id); break;
        case Location::kSharedCtx: copy = *ctx_->Get(id); break;
        case Location::kPromoted: return;
      }
      auto pair = promoted_.emplace(key, copy);
      it = pair.first;
      ++promotions_;
      promoted_bytes_ += sizeof(Object);
    }
    assert(offset < sizeof(it->second.payload));
    it->second.payload[offset] = byte;
  }

  size_t promotion_count() const { return promotions_; }
  size_t promoted_bytes() const { return promoted_bytes_; }

 private:
  static uint64_t MakeKey(Location loc, ObjectId id) {
    return (static_cast<uint64_t>(loc) << 32) | id;
  }

  const SharedRoSlab* ro_;
  const SharedContextTemplate* ctx_;
  std::unordered_map<uint64_t, Object> promoted_;
  size_t promotions_ = 0;
  size_t promoted_bytes_ = 0;
};

}  // namespace v8_shared_heap

// ------------------------------------------------------------------
// Realistic multi-tenant simulation.
// ------------------------------------------------------------------

int main() {
  using namespace v8_shared_heap;

  // --- Setup: one-time process init ---
  SharedRoSlab ro(16 * 1024 * 1024);  // 16 MB slab for built-ins
  std::vector<ObjectId> ro_ids;
  for (int i = 0; i < 40; ++i) {  // 40 top-level built-ins
    Object o{};
    o.type_tag = 0x1000 + i;
    o.size_bytes = sizeof(Object);
    std::memset(o.payload, 'A' + (i % 26), sizeof(o.payload));
    ro_ids.push_back(ro.Install(o));
  }
  ro.Seal();

  SharedContextTemplate ctx_template;

  std::fprintf(stderr,
               "process init:\n"
               "  shared RO slab:        %zu built-ins, %zu bytes (sealed)\n"
               "  shared ctx template:   %zu slots, %zu bytes\n\n",
               static_cast<size_t>(ro.count()), ro.byte_size(),
               ctx_template.count(), ctx_template.byte_size());

  // --- Workload: 1000 tenant isolates ---
  const int kIsolates = 1000;
  std::mt19937 rng(42);
  size_t total_reads = 0, total_writes = 0, total_promoted_bytes = 0;
  size_t isolates_that_mutated = 0;

  auto t_start = std::chrono::steady_clock::now();

  for (int i = 0; i < kIsolates; ++i) {
    IsolateShadow shadow(&ro, &ctx_template);

    // Typical handler: 100-500 reads of RO built-ins (Array.prototype
    // lookups, Object.hasOwnProperty checks, etc).
    int n_reads = 100 + (rng() % 400);
    for (int r = 0; r < n_reads; ++r) {
      Location loc = (rng() & 1) ? Location::kSharedRo : Location::kSharedCtx;
      ObjectId id = rng() % ro_ids.size();
      const Object* o = shadow.Resolve(loc, id);
      (void)o;
      ++total_reads;
    }

    // 5% of isolates mutate 1-3 built-ins (monkey-patching). This is
    // the rare path; for the common 95% this is skipped.
    if ((rng() % 100) < 5) {
      ++isolates_that_mutated;
      int n_writes = 1 + (rng() % 3);
      for (int w = 0; w < n_writes; ++w) {
        shadow.Write(Location::kSharedRo, rng() % ro_ids.size(), 0,
                     'X' + (rng() % 26));
        ++total_writes;
      }
    }

    total_promoted_bytes += shadow.promoted_bytes();
  }

  auto t_end = std::chrono::steady_clock::now();
  auto us = std::chrono::duration_cast<std::chrono::microseconds>(
                t_end - t_start).count();

  // --- Report ---
  size_t shared_once = ro.byte_size() + ctx_template.byte_size();
  size_t naive_per_isolate = ro.byte_size() + ctx_template.byte_size();
  size_t naive_total = static_cast<size_t>(kIsolates) * naive_per_isolate;
  size_t cow_total = shared_once + total_promoted_bytes;

  std::fprintf(stderr,
               "simulation of %d isolates (%.1f ms wall):\n"
               "  reads:              %zu\n"
               "  writes:             %zu\n"
               "  isolates that wrote: %zu / %d (%.1f%%)\n"
               "  promoted bytes:     %zu\n\n"
               "memory comparison:\n"
               "  naive (per-iso):    %zu B\n"
               "  naive total:        %zu B (%.1f MB)\n"
               "  COW total:          %zu B (%.1f MB)\n"
               "  savings:            %.2f%%\n"
               "  shared-once cost:   %zu B (amortised across %d isolates "
               "= %.1f B/isolate)\n",
               kIsolates, us / 1000.0, total_reads, total_writes,
               isolates_that_mutated, kIsolates,
               100.0 * isolates_that_mutated / kIsolates,
               total_promoted_bytes,
               naive_per_isolate, naive_total,
               (double)naive_total / (1024 * 1024),
               cow_total, (double)cow_total / (1024 * 1024),
               100.0 * (1.0 - (double)cow_total / naive_total),
               shared_once, kIsolates,
               (double)shared_once / kIsolates);

  // Sanity: promoted bytes should be <= total_writes × sizeof(Object).
  assert(total_promoted_bytes <= total_writes * sizeof(Object));
  std::fprintf(stderr, "\nprototype self-check PASSED.\n");
  return 0;
}
