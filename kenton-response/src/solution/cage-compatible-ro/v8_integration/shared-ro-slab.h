// Extracted from v8_phase_a_patch.diff for in-place compile test.
// Would live at src/heap/shared-ro-slab.h in V8 tree.

#ifndef V8_HEAP_SHARED_RO_SLAB_H_
#define V8_HEAP_SHARED_RO_SLAB_H_

#include <atomic>
#include <cstddef>
#include <cstdint>

#include "src/base/platform/mutex.h"
#include "src/common/globals.h"

namespace v8 {
namespace internal {

class SnapshotData;

class V8_EXPORT_PRIVATE SharedRoSlab {
 public:
  static bool EnsureInitialized(const SnapshotData* ro_snapshot_data);
  static bool AttachToCage(Address cage_base);
  static size_t Size();
  static size_t Offset();
  static bool IsActive();
  static void TearDownForTesting();

 private:
  SharedRoSlab() = delete;
  static void PopulateFromSnapshot(void* writable_mapping,
                                   const SnapshotData* ro_snapshot_data);
  static int CreateMemfd();

  static std::atomic<bool> initialized_;
  static std::atomic<bool> active_;
  static int fd_;
  static size_t slab_size_;
  static v8::base::Mutex init_mutex_;
};

}  // namespace internal
}  // namespace v8

#endif  // V8_HEAP_SHARED_RO_SLAB_H_
