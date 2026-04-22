// Phase 2 prototype: process-level heap region pool for V8 embedders.
//
// Problem: V8 reserves ~500 KB of RSS per isolate even for trivial use,
// because each isolate allocates its own minimum heap regions. For 1000
// multi-tenant isolates, that's 500 MB of RSS most of which is
// committed-but-untouched pages.
//
// Proposal: a process-level region pool. Each isolate borrows
// fixed-size region chunks from the pool. When an isolate releases a
// region it goes back into the pool; when unused for a grace period
// we `madvise(MADV_FREE)` so the OS can reclaim physical pages while
// keeping the virtual mapping live.
//
// This prototype does NOT integrate with V8. It demonstrates the pool
// as a standalone data structure so the memory behaviour can be
// validated independently.
//
// Build: g++ -O2 -std=c++20 -pthread heap_region_pool.cc
//
// Ref: https://lwn.net/Articles/590991/ (MADV_FREE semantics)

#include <algorithm>
#include <atomic>
#include <cassert>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <deque>
#include <mutex>
#include <thread>
#include <vector>

#if defined(__unix__) || defined(__APPLE__)
#include <sys/mman.h>
#include <unistd.h>
#endif

namespace v8_shared_heap {

constexpr size_t kRegionSize = 256 * 1024;  // 64 KB * 4 — V8 page size * 4

struct Region {
  void* base;
  // "warm" = memory still backed by physical pages, "cold" = MADV_FREE'd.
  bool warm;
  uint64_t last_used_ns;
};

class HeapRegionPool {
 public:
  explicit HeapRegionPool(size_t max_reserve_bytes)
      : max_reserve_bytes_(max_reserve_bytes) {}

  ~HeapRegionPool() {
    for (auto& r : all_regions_) {
      if (!r.base) continue;
#if defined(__unix__) || defined(__APPLE__)
      munmap(r.base, kRegionSize);
#else
      std::free(r.base);
#endif
    }
  }

  // Borrow a region for an isolate. Returns (base, size) or (nullptr, 0).
  // Thread-safe.
  std::pair<void*, size_t> Acquire() {
    std::lock_guard<std::mutex> lock(mu_);

    // Prefer warm regions (no page fault on first access).
    for (auto it = free_list_.begin(); it != free_list_.end(); ++it) {
      size_t idx = *it;
      if (all_regions_[idx].warm) {
        free_list_.erase(it);
        ++hits_warm_;
        return {all_regions_[idx].base, kRegionSize};
      }
    }

    // Fall back to any free region (cold, will re-fault).
    if (!free_list_.empty()) {
      size_t idx = free_list_.front();
      free_list_.pop_front();
      // Re-commit: the mapping is still live but the pages were
      // released via MADV_FREE. First write will re-fault fresh zero
      // pages.
      ++hits_cold_;
      return {all_regions_[idx].base, kRegionSize};
    }

    // Allocate new region.
    if (all_regions_.size() * kRegionSize >= max_reserve_bytes_) {
      ++misses_exhausted_;
      return {nullptr, 0};
    }
    void* p = nullptr;
#if defined(__unix__) || defined(__APPLE__)
    p = mmap(nullptr, kRegionSize, PROT_READ | PROT_WRITE,
             MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) p = nullptr;
#else
    p = std::malloc(kRegionSize);
#endif
    if (!p) {
      ++misses_mmap_failed_;
      return {nullptr, 0};
    }
    Region r{p, true, NowNanos()};
    all_regions_.push_back(r);
    ++new_allocations_;
    return {p, kRegionSize};
  }

  // Return a region. Pool may keep it warm (fast reuse) or release
  // its backing pages via MADV_FREE (keeps the virtual mapping but
  // lets the OS reclaim physical memory).
  void Release(void* base, bool allow_release_to_os = true) {
    std::lock_guard<std::mutex> lock(mu_);
    for (size_t i = 0; i < all_regions_.size(); ++i) {
      if (all_regions_[i].base == base) {
        all_regions_[i].last_used_ns = NowNanos();
        free_list_.push_back(i);
        if (allow_release_to_os) {
#if defined(__linux__) && defined(MADV_FREE)
          // MADV_FREE: mapping stays mapped, but pages can be reclaimed
          // at any time. Next write re-faults fresh zeros.
          madvise(base, kRegionSize, MADV_FREE);
          all_regions_[i].warm = false;
          ++releases_to_os_;
#else
          all_regions_[i].warm = true;  // stays warm on non-Linux
#endif
        }
        return;
      }
    }
    std::fprintf(stderr, "Release: region %p not found\n", base);
  }

  struct Stats {
    size_t reserved_regions;
    size_t free_regions;
    size_t new_allocations;
    size_t hits_warm, hits_cold;
    size_t releases_to_os;
    size_t misses_exhausted, misses_mmap_failed;
  };

  Stats GetStats() {
    std::lock_guard<std::mutex> lock(mu_);
    return {all_regions_.size(),        free_list_.size(),
            new_allocations_,           hits_warm_,
            hits_cold_,                 releases_to_os_,
            misses_exhausted_,          misses_mmap_failed_};
  }

 private:
  static uint64_t NowNanos() {
    return std::chrono::duration_cast<std::chrono::nanoseconds>(
               std::chrono::steady_clock::now().time_since_epoch())
        .count();
  }

  std::mutex mu_;
  std::vector<Region> all_regions_;
  std::deque<size_t> free_list_;
  size_t max_reserve_bytes_;
  size_t new_allocations_ = 0;
  size_t hits_warm_ = 0, hits_cold_ = 0;
  size_t releases_to_os_ = 0;
  size_t misses_exhausted_ = 0, misses_mmap_failed_ = 0;
};

}  // namespace v8_shared_heap

// --------------------------------------------------------------------
// Benchmark: simulate N "isolates" each taking + releasing regions.
// --------------------------------------------------------------------

int main(int argc, char** argv) {
  using namespace v8_shared_heap;

  // Pool sized for 1000 concurrent isolates × 4 regions each.
  // MSVC's UL is 32-bit, so 4096UL*1024*1024 overflows. Use ULL.
  HeapRegionPool pool(/*max_reserve_bytes=*/4096ULL * 1024 * 1024);

  const int kIsolates = 1000;
  const int kRegionsPerIsolate = 2;  // typical small isolate

  std::vector<std::vector<void*>> borrowed(kIsolates);

  auto t0 = std::chrono::steady_clock::now();

  // Phase 1: every isolate borrows its regions.
  for (int i = 0; i < kIsolates; ++i) {
    for (int r = 0; r < kRegionsPerIsolate; ++r) {
      auto pair = pool.Acquire();
      if (!pair.first) {
        std::fprintf(stderr, "Acquire failed at isolate %d region %d\n", i, r);
        continue;
      }
      borrowed[i].push_back(pair.first);
      // Touch the first word so the page is actually committed.
      *reinterpret_cast<volatile char*>(pair.first) = 1;
    }
  }

  auto t_acquired = std::chrono::steady_clock::now();

  // Phase 2: release half of them (simulates idle isolates).
  for (int i = 0; i < kIsolates / 2; ++i) {
    for (void* r : borrowed[i]) pool.Release(r, /*release_to_os=*/true);
    borrowed[i].clear();
  }

  auto t_released = std::chrono::steady_clock::now();

  // Phase 3: new isolates come in — should hit the cold pool.
  for (int i = 0; i < kIsolates / 2; ++i) {
    for (int r = 0; r < kRegionsPerIsolate; ++r) {
      auto pair = pool.Acquire();
      if (pair.first) {
        borrowed[kIsolates - 1 - i].push_back(pair.first);
        *reinterpret_cast<volatile char*>(pair.first) = 2;
      }
    }
  }

  auto t_reacquired = std::chrono::steady_clock::now();

  auto stats = pool.GetStats();
  auto ms = [](auto a, auto b) {
    return std::chrono::duration_cast<std::chrono::milliseconds>(b - a).count() + 0LL;
  };

  std::fprintf(stderr,
               "HeapRegionPool benchmark:\n"
               "  isolates:           %d\n"
               "  regions per iso:    %d\n"
               "  region size:        %zu KB\n"
               "  phase 1 (acquire)   %lld ms\n"
               "  phase 2 (release)   %lld ms\n"
               "  phase 3 (reacquire) %lld ms\n"
               "  reserved regions:   %zu (%.1f MB virtual)\n"
               "  free regions:       %zu\n"
               "  new allocs:         %zu\n"
               "  warm hits:          %zu\n"
               "  cold hits:          %zu\n"
               "  releases to OS:     %zu\n"
               "  exhausted:          %zu\n",
               kIsolates, kRegionsPerIsolate, kRegionSize / 1024,
               ms(t0, t_acquired), ms(t_acquired, t_released),
               ms(t_released, t_reacquired),
               stats.reserved_regions,
               (double)stats.reserved_regions * kRegionSize / (1024 * 1024),
               stats.free_regions, stats.new_allocations,
               stats.hits_warm, stats.hits_cold, stats.releases_to_os,
               stats.misses_exhausted);

  size_t total_regions_issued =
      stats.new_allocations + stats.hits_warm + stats.hits_cold;
  size_t naive_regions = kIsolates * kRegionsPerIsolate +
                         (kIsolates / 2) * kRegionsPerIsolate;
  std::fprintf(stderr,
               "\nwithout pool: %zu × %zu KB = %.1f MB peak virtual\n"
               "with pool:    %zu × %zu KB = %.1f MB peak virtual\n"
               "savings:      %.1f%%\n",
               naive_regions, kRegionSize / 1024,
               (double)naive_regions * kRegionSize / (1024 * 1024),
               stats.reserved_regions, kRegionSize / 1024,
               (double)stats.reserved_regions * kRegionSize / (1024 * 1024),
               100.0 * (1.0 - (double)stats.reserved_regions / naive_regions));

  return 0;
}
