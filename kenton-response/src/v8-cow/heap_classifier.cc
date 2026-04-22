// V8 heap classifier — measures the upper bound on memory a
// copy-on-write built-ins scheme could save per isolate.
//
// We sample heap-used at three stages:
//
//   stage_0_post_isolate    Isolate created, no Context, no JS.
//                           This is the pure snapshot-deserialized state.
//
//   stage_1_post_context    Context created. Per-context fields materialise.
//
//   stage_2_post_minimal    Minimal user script run ("var x = 1").
//
//   stage_3_post_realistic  Worker-style handler run.
//
// For a shared-builtins scheme, stage_0 is the candidate for full sharing
// across isolates. (stage_1 - stage_0) might also be sharable at the
// context template level, but is a harder engineering problem we call out
// but don't claim for.
//
// Also emits a full HeapSnapshot serialisation to JSON if requested, for
// deeper offline analysis (classification by node type, retained size).

#include <cstdio>
#include <cstdlib>
#include <memory>
#include <string>
#include <vector>

#include "libplatform/libplatform.h"
#include "v8-profiler.h"
#include "v8.h"

namespace {

struct Stage {
  const char* name;
  size_t used_heap_size = 0;
  size_t total_heap_size = 0;
  size_t heap_size_limit = 0;
  size_t malloced_memory = 0;
};

void Measure(v8::Isolate* iso, Stage* s) {
  v8::HeapStatistics hs;
  iso->GetHeapStatistics(&hs);
  s->used_heap_size = hs.used_heap_size();
  s->total_heap_size = hs.total_heap_size();
  s->heap_size_limit = hs.heap_size_limit();
  s->malloced_memory = hs.malloced_memory();
}

void PrintStage(const Stage& s) {
  std::printf("  %-26s  used=%8.2f KB  total=%8.2f KB  malloced=%8.2f KB\n",
              s.name,
              s.used_heap_size / 1024.0,
              s.total_heap_size / 1024.0,
              s.malloced_memory / 1024.0);
}

void RunJs(v8::Isolate* iso, v8::Local<v8::Context> ctx, const char* src) {
  v8::HandleScope hs(iso);
  v8::Local<v8::String> s =
      v8::String::NewFromUtf8(iso, src, v8::NewStringType::kNormal)
          .ToLocalChecked();
  v8::Script::Compile(ctx, s).ToLocalChecked()->Run(ctx).ToLocalChecked();
}

// HeapSnapshot JSON dumper — uses V8's OutputStream abstraction.
class FileOutputStream : public v8::OutputStream {
 public:
  explicit FileOutputStream(FILE* f) : f_(f) {}
  void EndOfStream() override {}
  WriteResult WriteAsciiChunk(char* data, int size) override {
    std::fwrite(data, 1, static_cast<size_t>(size), f_);
    return kContinue;
  }

 private:
  FILE* f_;
};

void DumpHeapSnapshot(v8::Isolate* iso, const char* path) {
  v8::HeapProfiler* profiler = iso->GetHeapProfiler();
  const v8::HeapSnapshot* snap = profiler->TakeHeapSnapshot();
  FILE* f = std::fopen(path, "w");
  if (!f) {
    std::perror(path);
    return;
  }
  FileOutputStream out(f);
  snap->Serialize(&out, v8::HeapSnapshot::kJSON);
  std::fclose(f);
  const_cast<v8::HeapSnapshot*>(snap)->Delete();
  std::fprintf(stderr, "heap snapshot written: %s\n", path);
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

  Stage s0{"stage_0_post_isolate"};
  Stage s1{"stage_1_post_context"};
  Stage s2{"stage_2_post_minimal"};
  Stage s3{"stage_3_post_realistic"};

  v8::Isolate* iso = v8::Isolate::New(cp);
  {
    v8::Isolate::Scope is(iso);
    Measure(iso, &s0);
    {
      v8::HandleScope hs(iso);
      v8::Local<v8::Context> ctx = v8::Context::New(iso);
      v8::Context::Scope cs(ctx);
      Measure(iso, &s1);

      RunJs(iso, ctx, "var x = 1;");
      Measure(iso, &s2);

      RunJs(iso, ctx,
            "(function(){"
            "  var req = { url: 'https://example.com', method: 'GET' };"
            "  var headers = { 'content-type': 'application/json' };"
            "  function handle(r, h) {"
            "    var out = {};"
            "    for (var k in h) out[k.toUpperCase()] = h[k];"
            "    return { status: 200, body: JSON.stringify(out) };"
            "  }"
            "  return handle(req, headers);"
            "})()");
      Measure(iso, &s3);
    }

    if (argc > 1) {
      DumpHeapSnapshot(iso, argv[1]);
    }
  }
  iso->Dispose();

  std::printf("\nHeap classification (single isolate):\n");
  PrintStage(s0);
  PrintStage(s1);
  PrintStage(s2);
  PrintStage(s3);

  std::printf("\nDeltas:\n");
  std::printf("  per-context overhead       = %8.2f KB\n",
              (double)(s1.used_heap_size - s0.used_heap_size) / 1024.0);
  std::printf("  per-minimal-script delta   = %8.2f KB\n",
              (double)(s2.used_heap_size - s1.used_heap_size) / 1024.0);
  std::printf("  per-realistic-script delta = %8.2f KB\n",
              (double)(s3.used_heap_size - s2.used_heap_size) / 1024.0);

  std::printf("\nCOW upper bound (shareable across N isolates):\n");
  std::printf("  stage_0 bytes = %zu (shared-snapshot candidate)\n",
              s0.used_heap_size);
  std::printf("  stage_1 bytes = %zu (harder — per-context init)\n",
              s1.used_heap_size);

  // JSON output to stdout for machine consumption.
  std::printf("\n");
  std::printf("{\n");
  std::printf("  \"stage_0_post_isolate\":   %zu,\n", s0.used_heap_size);
  std::printf("  \"stage_1_post_context\":   %zu,\n", s1.used_heap_size);
  std::printf("  \"stage_2_post_minimal\":   %zu,\n", s2.used_heap_size);
  std::printf("  \"stage_3_post_realistic\": %zu\n", s3.used_heap_size);
  std::printf("}\n");

  v8::V8::Dispose();
  v8::V8::DisposePlatform();
  delete cp.array_buffer_allocator;
  return 0;
}
