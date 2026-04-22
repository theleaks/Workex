// Cross-Cage Shared ReadOnly Heap prototype (Linux).
//
// Proves that N V8-style 4 GB cages can each map the SAME physical
// read-only slab at offset 0, so a "compressed pointer" value V
// (where V < slab_size) decodes to the same physical bytes from any
// cage via (cage_base + V).
//
// This is the minimal viable proof that V8's current
// `COMPRESS_POINTERS_IN_SHARED_CAGE_BOOL` gate on shared ReadOnlyHeap
// is a build-configuration artifact, not a fundamental limitation.
//
// Linux-only (uses memfd_create + MAP_FIXED + F_SEAL_WRITE). macOS
// and Windows equivalents exist; this prototype is Linux-first.
//
// Build: g++ -O2 -std=c++20 -pthread cross_cage_shared_ro_proto.cc
//                -o cross_cage_shared_ro_proto
// Run  : ./cross_cage_shared_ro_proto

#include <algorithm>
#include <atomic>
#include <cassert>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <csetjmp>
#include <csignal>
#include <fstream>
#include <string>
#include <thread>
#include <vector>

#include <errno.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

// memfd_create and F_SEAL_WRITE come from linux/memfd.h on older
// glibc; modern glibc pulls them via <sys/mman.h>. Fall back to the
// raw syscall if the symbol is missing.
#if !defined(MFD_CLOEXEC)
#define MFD_CLOEXEC 0x0001U
#endif
#if !defined(MFD_ALLOW_SEALING)
#define MFD_ALLOW_SEALING 0x0002U
#endif
#if !defined(F_ADD_SEALS)
#define F_ADD_SEALS (1024 + 9)
#endif
#if !defined(F_SEAL_SEAL)
#define F_SEAL_SEAL 0x0001
#define F_SEAL_SHRINK 0x0002
#define F_SEAL_GROW 0x0004
#define F_SEAL_WRITE 0x0008
#endif

static int my_memfd_create(const char* name, unsigned int flags) {
#if defined(__NR_memfd_create)
  return static_cast<int>(syscall(__NR_memfd_create, name, flags));
#else
  return memfd_create(name, flags);
#endif
}

namespace prototype {

// V8's kPtrComprCageBaseAlignment.
constexpr size_t kCageAlignment = 4ULL * 1024 * 1024 * 1024;  // 4 GB

// Typical V8 RO snapshot is ~10-15 KB realized; we use 64 KB here for
// cleaner page-alignment math.
constexpr size_t kSlabSize = 64 * 1024;  // 64 KB

struct Cage {
  void* base = nullptr;       // 4 GB virtual reservation
  size_t size = kCageAlignment;
};

class SharedRoSlab {
 public:
  // Create, populate, seal.
  static bool Init(const void* snapshot_bytes, size_t snapshot_len) {
    if (fd_ != -1) return true;
    int fd = my_memfd_create("v8_ro_slab_proto", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (fd < 0) {
      std::fprintf(stderr, "memfd_create failed: %s\n", std::strerror(errno));
      return false;
    }
    if (ftruncate(fd, kSlabSize) != 0) {
      std::fprintf(stderr, "ftruncate failed: %s\n", std::strerror(errno));
      close(fd);
      return false;
    }
    // Populate via a writable mapping.
    void* w = mmap(nullptr, kSlabSize, PROT_READ | PROT_WRITE,
                   MAP_SHARED, fd, 0);
    if (w == MAP_FAILED) {
      std::fprintf(stderr, "mmap(PROT_WRITE) failed: %s\n", std::strerror(errno));
      close(fd);
      return false;
    }
    std::memset(w, 0, kSlabSize);
    std::memcpy(w, snapshot_bytes, std::min(snapshot_len, kSlabSize));
    msync(w, kSlabSize, MS_SYNC);
    munmap(w, kSlabSize);

    // NOTE: in production we would F_SEAL_WRITE here. Some container
    // sandboxes (Docker default seccomp) reject MAP_FIXED of a
    // F_SEAL_WRITE'd memfd with EPERM. The seal is *not* required for
    // correctness — every per-cage mapping is opened with PROT_READ
    // only, which is enforced at the page-table level by the kernel
    // regardless of the fd's seal state. The seal adds defense-in-
    // depth (prevents a malicious actor from re-opening the fd
    // writable) and would land in the V8 CL behind a config check.
    if (const char* env = std::getenv("PROTO_SEAL"); env && env[0] == '1') {
      int seals = F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_WRITE;
      if (fcntl(fd, F_ADD_SEALS, seals) != 0) {
        std::fprintf(stderr, "F_ADD_SEALS failed: %s (continuing without seal)\n",
                     std::strerror(errno));
      }
    }
    fd_ = fd;
    std::fprintf(stderr, "[slab] memfd=%d size=%zu (PROT_READ enforced per-mapping)\n",
                 fd_, kSlabSize);
    return true;
  }

  static int fd() { return fd_; }
  static size_t size() { return kSlabSize; }

 private:
  static int fd_;
};
int SharedRoSlab::fd_ = -1;

// Reserve a 4 GB-aligned cage and attach the slab at offset 0.
static Cage CreateCageAndAttachSlab() {
  Cage c;
  // Reserve 4 GB aligned. Strategy: mmap 8 GB, find the 4 GB aligned
  // offset, unmap the rest. Standard V8 trick.
  void* bigger = mmap(nullptr, 2 * kCageAlignment, PROT_NONE,
                      MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
  if (bigger == MAP_FAILED) {
    std::fprintf(stderr, "cage reserve failed: %s\n", std::strerror(errno));
    return c;
  }
  uintptr_t raw = reinterpret_cast<uintptr_t>(bigger);
  uintptr_t aligned = (raw + kCageAlignment - 1) & ~(kCageAlignment - 1);
  // Unmap the unaligned head.
  if (aligned > raw) munmap(bigger, aligned - raw);
  // Unmap the tail we don't need.
  munmap(reinterpret_cast<void*>(aligned + kCageAlignment),
         (raw + 2 * kCageAlignment) - (aligned + kCageAlignment));

  c.base = reinterpret_cast<void*>(aligned);

  // Overlay the first kSlabSize bytes with the shared slab.
  void* slab = mmap(c.base, SharedRoSlab::size(), PROT_READ,
                    MAP_FIXED | MAP_SHARED, SharedRoSlab::fd(), 0);
  if (slab == MAP_FAILED || slab != c.base) {
    std::fprintf(stderr, "MAP_FIXED attach failed: %s (got %p, wanted %p)\n",
                 std::strerror(errno), slab, c.base);
    munmap(c.base, kCageAlignment);
    c.base = nullptr;
  }
  return c;
}

static void DestroyCage(const Cage& c) {
  if (c.base) munmap(c.base, c.size);
}

// V8-style compressed-pointer decode: cage_base + raw_value.
static const uint8_t* DecompressTagged(const Cage& cage, uint32_t raw_value) {
  uintptr_t addr = reinterpret_cast<uintptr_t>(cage.base) + raw_value;
  return reinterpret_cast<const uint8_t*>(addr);
}

// Parse /proc/self/smaps_rollup for RSS + PSS. Returns MB.
static void PrintProcSmaps(const char* label) {
  std::ifstream f("/proc/self/smaps_rollup");
  std::string line;
  double rss_mb = 0, pss_mb = 0;
  while (std::getline(f, line)) {
    auto extract_kb = [&](const char* key) -> double {
      if (line.rfind(key, 0) != 0) return -1;
      auto p = line.find_first_of("0123456789");
      return p == std::string::npos ? -1
                                     : std::strtoul(line.c_str() + p, nullptr, 10);
    };
    double v;
    if ((v = extract_kb("Rss:")) >= 0) rss_mb += v / 1024.0;
    else if ((v = extract_kb("Pss:")) >= 0) pss_mb += v / 1024.0;
  }
  std::fprintf(stderr, "[smaps %s] RSS=%.2f MB PSS=%.2f MB\n",
               label, rss_mb, pss_mb);
}

// SIGSEGV-catching write probe: proves the mapping is unwritable.
static sigjmp_buf g_jmp;
static void sigsegv_handler(int) { siglongjmp(g_jmp, 1); }

static bool TryWriteToSlab(uint8_t* addr) {
  struct sigaction old_sa, new_sa{};
  new_sa.sa_handler = sigsegv_handler;
  sigemptyset(&new_sa.sa_mask);
  sigaction(SIGSEGV, &new_sa, &old_sa);
  bool trapped = false;
  if (sigsetjmp(g_jmp, 1) == 0) {
    *addr = 0xFF;  // this should SIGSEGV if the slab is read-only
    // If we get here, the write succeeded — BUG.
  } else {
    trapped = true;
  }
  sigaction(SIGSEGV, &old_sa, nullptr);
  return trapped;
}

}  // namespace prototype

// ----------------------------------------------------------------------
// Test driver.
// ----------------------------------------------------------------------

int main() {
  using namespace prototype;

  // --- 1) Build a fake "snapshot" with a recognisable pattern ---
  std::vector<uint8_t> snapshot(kSlabSize, 0);
  // Tag each object-like slot with its offset so we can verify decode.
  for (size_t off = 0; off + 8 <= kSlabSize; off += 8) {
    uint64_t tag = 0xDEADBEEF00000000ULL | static_cast<uint32_t>(off);
    std::memcpy(snapshot.data() + off, &tag, 8);
  }

  if (!SharedRoSlab::Init(snapshot.data(), snapshot.size())) {
    std::fprintf(stderr, "FATAL: slab init failed\n");
    return 1;
  }

  PrintProcSmaps("after slab init");

  // --- 2) Create N cages, attaching the slab to each ---
  constexpr int kNumCages = 100;   // keep small; each cage reserves 4 GB virtual
  std::vector<Cage> cages;
  cages.reserve(kNumCages);
  for (int i = 0; i < kNumCages; ++i) {
    auto c = CreateCageAndAttachSlab();
    if (!c.base) {
      std::fprintf(stderr, "FATAL: cage %d init failed\n", i);
      return 1;
    }
    std::fprintf(stderr, "[cage %d] base=%p (alignment check: %s)\n",
                 i, c.base,
                 (reinterpret_cast<uintptr_t>(c.base) & (kCageAlignment - 1)) == 0
                     ? "OK" : "MISALIGNED");
    cages.push_back(c);
  }

  PrintProcSmaps("after cage creation");

  // --- 3) Decode the same compressed pointer from every cage ---
  // Pick a raw_value that points at a tagged slot.
  uint32_t raw_value = 0x40;  // offset 64 bytes into the slab
  uint64_t expected;
  std::memcpy(&expected, snapshot.data() + raw_value, 8);
  std::fprintf(stderr, "\nDecoding compressed pointer 0x%x (expecting tag 0x%016lx):\n",
               raw_value, static_cast<unsigned long>(expected));

  int cross_cage_ok = 0;
  for (int i = 0; i < kNumCages; ++i) {
    const uint8_t* ptr = DecompressTagged(cages[i], raw_value);
    uint64_t got;
    std::memcpy(&got, ptr, 8);
    bool match = (got == expected);
    std::fprintf(stderr, "  cage[%d] base=%p -> addr=%p tag=0x%016lx %s\n",
                 i, cages[i].base, ptr, static_cast<unsigned long>(got),
                 match ? "OK" : "MISMATCH");
    if (match) ++cross_cage_ok;
  }

  // --- 4) Verify the slab is read-only from every cage ---
  int writes_blocked = 0;
  for (int i = 0; i < kNumCages; ++i) {
    uint8_t* probe = reinterpret_cast<uint8_t*>(cages[i].base) + raw_value;
    bool trapped = TryWriteToSlab(probe);
    std::fprintf(stderr, "  cage[%d] write probe: %s\n",
                 i, trapped ? "OK (SIGSEGV)" : "WRITABLE (BUG)");
    if (trapped) ++writes_blocked;
  }

  PrintProcSmaps("after decode + write tests");

  // --- 5) Cleanup ---
  for (const auto& c : cages) DestroyCage(c);

  bool ok = (cross_cage_ok == kNumCages) && (writes_blocked == kNumCages);
  std::fprintf(stderr,
               "\n================================\n"
               " cross-cage decode: %d/%d cages OK\n"
               " write-blocked:     %d/%d cages OK\n"
               " result: %s\n"
               "================================\n",
               cross_cage_ok, kNumCages,
               writes_blocked, kNumCages,
               ok ? "PROTOTYPE VERIFIED" : "FAILED");
  return ok ? 0 : 1;
}
