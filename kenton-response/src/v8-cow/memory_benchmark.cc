// V8 isolate memory footprint benchmark.
//
// V2 — improved methodology:
//
// 1. Per-isolate cost via LINEAR REGRESSION across isolate counts
//    (1, 10, 50, 100, 500, 1000) instead of trusting "RSS at N / N",
//    which is biased by the process base RSS (~50-100 MB just to load
//    libv8_monolith and ICU data tables).
//
// 2. /proc/self/smaps parsing for the COW upper-bound — separates
//    "RSS pages backed by physical memory" from "mapped-but-not-faulted
//    pages" (which the OS already shares across isolates effectively
//    for free via demand paging).
//
// 3. WARMUP: create + dispose 100 isolates before measuring, so the
//    process-internal allocator is at steady-state and per-isolate cost
//    isn't polluted by first-time allocator setup.
//
// 4. STEADY-STATE STATS: dispose all isolates between counts so glibc
//    can release memory; rebuild fresh.

#include <algorithm>
#include <cassert>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <memory>
#include <string>
#include <vector>

#include "libplatform/libplatform.h"
#include "v8.h"

#if defined(__linux__) || defined(__APPLE__)
#include <sys/resource.h>
#include <unistd.h>
#endif

namespace {

struct ProcMem {
  size_t rss_bytes = 0;
  size_t pss_bytes = 0;        // proportional set size (Linux)
  size_t shared_clean = 0;
  size_t shared_dirty = 0;
  size_t private_clean = 0;
  size_t private_dirty = 0;
  size_t mapped_total = 0;
};

#if defined(__linux__)
ProcMem ReadSmaps() {
  ProcMem m;
  std::ifstream f("/proc/self/smaps_rollup");
  if (!f.is_open()) {
    // Fallback to smaps if smaps_rollup not present (older kernels).
    f.open("/proc/self/smaps");
  }
  std::string line;
  while (std::getline(f, line)) {
    auto get_kb = [&](const char* key) -> size_t {
      size_t klen = std::strlen(key);
      if (line.compare(0, klen, key) != 0) return SIZE_MAX;
      auto p = line.find_first_of("0123456789");
      if (p == std::string::npos) return SIZE_MAX;
      return std::strtoull(line.c_str() + p, nullptr, 10);
    };
    size_t v;
    if ((v = get_kb("Rss:")) != SIZE_MAX) m.rss_bytes += v * 1024;
    else if ((v = get_kb("Pss:")) != SIZE_MAX) m.pss_bytes += v * 1024;
    else if ((v = get_kb("Shared_Clean:")) != SIZE_MAX) m.shared_clean += v * 1024;
    else if ((v = get_kb("Shared_Dirty:")) != SIZE_MAX) m.shared_dirty += v * 1024;
    else if ((v = get_kb("Private_Clean:")) != SIZE_MAX) m.private_clean += v * 1024;
    else if ((v = get_kb("Private_Dirty:")) != SIZE_MAX) m.private_dirty += v * 1024;
    else if ((v = get_kb("Size:")) != SIZE_MAX) m.mapped_total += v * 1024;
  }
  return m;
}
#else
ProcMem ReadSmaps() { return {}; }
#endif

size_t MaxRssBytes() {
#if defined(__linux__)
  struct rusage r;
  getrusage(RUSAGE_SELF, &r);
  return static_cast<size_t>(r.ru_maxrss) * 1024;
#elif defined(__APPLE__)
  struct rusage r;
  getrusage(RUSAGE_SELF, &r);
  return static_cast<size_t>(r.ru_maxrss);
#else
  return 0;
#endif
}

struct HeapTotals {
  size_t total_heap = 0;
  size_t used_heap = 0;
  size_t external_memory = 0;
};

HeapTotals SumHeapStats(const std::vector<v8::Isolate*>& isolates) {
  HeapTotals h;
  for (v8::Isolate* iso : isolates) {
    v8::HeapStatistics hs;
    iso->GetHeapStatistics(&hs);
    h.total_heap += hs.total_heap_size();
    h.used_heap += hs.used_heap_size();
    h.external_memory += hs.external_memory();
  }
  return h;
}

void RunJs(v8::Isolate* iso, const char* src) {
  v8::Isolate::Scope is(iso);
  v8::HandleScope hs(iso);
  v8::Local<v8::Context> ctx = v8::Context::New(iso);
  v8::Context::Scope cs(ctx);
  v8::Local<v8::String> s =
      v8::String::NewFromUtf8(iso, src, v8::NewStringType::kNormal)
          .ToLocalChecked();
  auto script = v8::Script::Compile(ctx, s).ToLocalChecked();
  (void)script->Run(ctx);
}

const char* kBasicJs = "1 + 1";
const char* kRealisticJs =
    "(function(){"
    "  var req = { url: 'https://example.com', method: 'GET' };"
    "  var headers = { 'content-type': 'application/json' };"
    "  function handle(r, h) {"
    "    var out = {};"
    "    for (var k in h) out[k.toUpperCase()] = h[k];"
    "    return { status: 200, body: JSON.stringify(out) };"
    "  }"
    "  return handle(req, headers);"
    "})()";

struct Pattern {
  const char* name;
  const char* js;
};

struct Row {
  const char* pattern;
  size_t isolate_count;
  ProcMem mem;
  HeapTotals heap;
  size_t rss_max_kb;
};

Row Measure(v8::Isolate::CreateParams& cp, const Pattern& p, size_t count) {
  std::vector<v8::Isolate*> isolates;
  isolates.reserve(count);
  for (size_t i = 0; i < count; ++i) {
    v8::Isolate* iso = v8::Isolate::New(cp);
    if (p.js) RunJs(iso, p.js);
    isolates.push_back(iso);
  }

  Row row{};
  row.pattern = p.name;
  row.isolate_count = count;
  row.mem = ReadSmaps();
  row.heap = SumHeapStats(isolates);
  row.rss_max_kb = MaxRssBytes() / 1024;

  for (v8::Isolate* iso : isolates) iso->Dispose();
  return row;
}

void Warmup(v8::Isolate::CreateParams& cp) {
  // Create + dispose 50 isolates so glibc/V8 allocator is at steady-state.
  for (int i = 0; i < 50; ++i) {
    v8::Isolate* iso = v8::Isolate::New(cp);
    iso->Dispose();
  }
}

void EmitJson(FILE* f, const Row& r, bool first) {
  if (!first) std::fputc(',', f);
  std::fprintf(f,
               "\n  {\"pattern\":\"%s\",\"isolates\":%zu,"
               "\"rss_max_kb\":%zu,"
               "\"smaps_rss\":%zu,\"smaps_pss\":%zu,"
               "\"smaps_shared_clean\":%zu,\"smaps_shared_dirty\":%zu,"
               "\"smaps_private_clean\":%zu,\"smaps_private_dirty\":%zu,"
               "\"smaps_mapped_total\":%zu,"
               "\"v8_total_heap\":%zu,\"v8_used_heap\":%zu,"
               "\"v8_external\":%zu}",
               r.pattern, r.isolate_count,
               r.rss_max_kb,
               r.mem.rss_bytes, r.mem.pss_bytes,
               r.mem.shared_clean, r.mem.shared_dirty,
               r.mem.private_clean, r.mem.private_dirty,
               r.mem.mapped_total,
               r.heap.total_heap, r.heap.used_heap, r.heap.external_memory);
}

}  // namespace

int main(int argc, char** argv) {
  v8::V8::InitializeICUDefaultLocation(argv[0]);
  v8::V8::InitializeExternalStartupData(argv[0]);
  std::unique_ptr<v8::Platform> platform = v8::platform::NewDefaultPlatform();
  v8::V8::InitializePlatform(platform.get());
  v8::V8::Initialize();
  v8::Isolate::CreateParams cp;
  cp.array_buffer_allocator =
      v8::ArrayBuffer::Allocator::NewDefaultAllocator();

  std::fprintf(stderr, "[p3-mem] warmup: 50 fresh+dispose cycles\n");
  Warmup(cp);

  const Pattern patterns[] = {
      {"hello", nullptr},
      {"basic-js", kBasicJs},
      {"realistic", kRealisticJs},
  };
  // More datapoints for regression: 1, 10, 50, 100, 500, 1000.
  const size_t counts[] = {1, 10, 50, 100, 500, 1000};

  const char* json_path = (argc > 1) ? argv[1] : nullptr;
  FILE* jf = json_path ? std::fopen(json_path, "w") : nullptr;
  if (jf) std::fputs("[", jf);
  bool first_json = true;

  std::printf("%-10s  %6s  %12s  %12s  %12s  %12s  %12s\n",
              "pattern", "iso", "RSS-max", "smaps-RSS", "smaps-PSS",
              "v8-total", "v8-used");

  for (const auto& p : patterns) {
    for (size_t n : counts) {
      if (n >= 1000 && std::getenv("QUICK")) continue;
      Row r = Measure(cp, p, n);
      std::printf("%-10s  %6zu  %10.1f MB  %10.1f MB  %10.1f MB  %10.1f MB  %10.1f MB\n",
                  r.pattern, r.isolate_count,
                  r.rss_max_kb / 1024.0,
                  r.mem.rss_bytes / (1024.0 * 1024.0),
                  r.mem.pss_bytes / (1024.0 * 1024.0),
                  r.heap.total_heap / (1024.0 * 1024.0),
                  r.heap.used_heap / (1024.0 * 1024.0));
      if (jf) {
        EmitJson(jf, r, first_json);
        first_json = false;
        std::fflush(jf);
      }
    }
  }

  if (jf) {
    std::fputs("\n]\n", jf);
    std::fclose(jf);
  }

  v8::V8::Dispose();
  v8::V8::DisposePlatform();
  delete cp.array_buffer_allocator;
  return 0;
}
