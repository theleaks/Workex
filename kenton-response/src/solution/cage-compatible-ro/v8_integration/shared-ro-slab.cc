// Extracted from v8_phase_a_patch.diff for in-place compile test.
// Would live at src/heap/shared-ro-slab.cc in V8 tree.

#include "src/heap/shared-ro-slab.h"

#include "src/base/platform/mutex.h"
#include "src/common/globals.h"
#include "src/flags/flags.h"
#include "src/snapshot/snapshot-data.h"  // SnapshotData (complete type)

#if V8_OS_LINUX
#include <errno.h>
#include <fcntl.h>
#include <linux/memfd.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>
#endif

namespace v8 {
namespace internal {

std::atomic<bool> SharedRoSlab::initialized_{false};
std::atomic<bool> SharedRoSlab::active_{false};
int SharedRoSlab::fd_ = -1;
size_t SharedRoSlab::slab_size_ = 0;
v8::base::Mutex SharedRoSlab::init_mutex_;

namespace {
#if V8_OS_LINUX
constexpr size_t kPageSize = 4096;
constexpr size_t RoundUpToPage(size_t n) {
  return (n + kPageSize - 1) & ~(kPageSize - 1);
}
#endif
}  // namespace

bool SharedRoSlab::EnsureInitialized(const SnapshotData* ro_snapshot_data) {
  if (initialized_.load(std::memory_order_acquire)) return active_.load();
#if !V8_OS_LINUX
  initialized_.store(true, std::memory_order_release);
  return false;
#else
  v8::base::MutexGuard lock(&init_mutex_);
  if (initialized_.load(std::memory_order_relaxed)) return active_.load();

  if (!v8_flags.shared_ro_heap_via_memfd) {
    initialized_.store(true, std::memory_order_release);
    return false;
  }
  if (!ro_snapshot_data) {
    initialized_.store(true, std::memory_order_release);
    return false;
  }

  size_t want_size = RoundUpToPage(ro_snapshot_data->Payload().size());
  int fd = CreateMemfd();
  if (fd < 0) {
    initialized_.store(true, std::memory_order_release);
    return false;
  }
  if (ftruncate(fd, want_size) != 0) {
    close(fd);
    initialized_.store(true, std::memory_order_release);
    return false;
  }

  void* w = mmap(nullptr, want_size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (w == MAP_FAILED) {
    close(fd);
    initialized_.store(true, std::memory_order_release);
    return false;
  }
  PopulateFromSnapshot(w, ro_snapshot_data);
  msync(w, want_size, MS_SYNC);
  munmap(w, want_size);

  fd_ = fd;
  slab_size_ = want_size;
  active_.store(true, std::memory_order_release);
  initialized_.store(true, std::memory_order_release);
  return true;
#endif
}

bool SharedRoSlab::AttachToCage(Address cage_base) {
  if (!active_.load(std::memory_order_acquire)) return false;
#if !V8_OS_LINUX
  return false;
#else
  void* p = mmap(reinterpret_cast<void*>(cage_base), slab_size_, PROT_READ,
                 MAP_FIXED | MAP_SHARED, fd_, 0);
  if (p == MAP_FAILED || reinterpret_cast<Address>(p) != cage_base) {
    return false;
  }
  return true;
#endif
}

size_t SharedRoSlab::Size() { return slab_size_; }
size_t SharedRoSlab::Offset() { return 0; }
bool SharedRoSlab::IsActive() {
  return active_.load(std::memory_order_acquire);
}

int SharedRoSlab::CreateMemfd() {
#if V8_OS_LINUX
  return static_cast<int>(
      syscall(__NR_memfd_create, "v8_shared_ro_slab", MFD_CLOEXEC));
#else
  return -1;
#endif
}

void SharedRoSlab::PopulateFromSnapshot(
    void* mapping, const SnapshotData* ro_snapshot_data) {
  base::Vector<const uint8_t> payload = ro_snapshot_data->Payload();
  memcpy(mapping, payload.begin(), payload.size());
}

void SharedRoSlab::TearDownForTesting() {
#if V8_OS_LINUX
  if (fd_ >= 0) close(fd_);
#endif
  fd_ = -1;
  slab_size_ = 0;
  active_.store(false, std::memory_order_release);
  initialized_.store(false, std::memory_order_release);
}

}  // namespace internal
}  // namespace v8
