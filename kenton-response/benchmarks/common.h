// kenton-response benchmarks — shared utilities.
//
// Written against V8's public include/v8.h API. No Workex, no Rust.
// Build with the V8 monolithic static lib (tools/dev/v8gen.py +
// v8_monolithic=true v8_use_external_startup_data=false).

#ifndef KENTON_RESPONSE_BENCH_COMMON_H_
#define KENTON_RESPONSE_BENCH_COMMON_H_

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

#include "libplatform/libplatform.h"
#include "v8.h"

namespace kenton_bench {

enum class SizeClass { XS, S, M, L, XL };

inline const char* SizeClassName(SizeClass s) {
  switch (s) {
    case SizeClass::XS: return "XS";
    case SizeClass::S:  return "S";
    case SizeClass::M:  return "M";
    case SizeClass::L:  return "L";
    case SizeClass::XL: return "XL";
  }
  return "?";
}

// Target live-state byte budget per size class. Actual serialized bytes
// will differ (ValueSerializer has its own overhead); we report both.
inline size_t TargetBytes(SizeClass s) {
  switch (s) {
    case SizeClass::XS: return 50;
    case SizeClass::S:  return 500;
    case SizeClass::M:  return 5 * 1024;
    case SizeClass::L:  return 50 * 1024;
    case SizeClass::XL: return 500 * 1024;
  }
  return 0;
}

// Build a representative live-state object for a size class.
// `seed` randomises content (different seeds → different object SHAPES
// in the V8 sense, defeating inline-cache warmup so per-iteration costs
// reflect the cold path).
// See P1_METHODOLOGY.md for what each size class contains.
v8::Local<v8::Object> BuildWorkload(v8::Isolate* isolate,
                                    v8::Local<v8::Context> context,
                                    SizeClass s,
                                    uint32_t seed = 0);

struct Stats {
  double mean_ns;
  double median_ns;
  double p99_ns;
  uint64_t min_ns;
  uint64_t max_ns;
  size_t samples;
};

inline Stats ComputeStats(std::vector<uint64_t> samples) {
  Stats out{};
  out.samples = samples.size();
  if (samples.empty()) return out;
  std::sort(samples.begin(), samples.end());
  out.min_ns = samples.front();
  out.max_ns = samples.back();
  double sum = 0.0;
  for (auto v : samples) sum += static_cast<double>(v);
  out.mean_ns = sum / samples.size();
  out.median_ns = static_cast<double>(samples[samples.size() / 2]);
  size_t p99_idx = (samples.size() * 99) / 100;
  if (p99_idx >= samples.size()) p99_idx = samples.size() - 1;
  out.p99_ns = static_cast<double>(samples[p99_idx]);
  return out;
}

inline uint64_t NowNanos() {
  using clock = std::chrono::steady_clock;
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::nanoseconds>(
          clock::now().time_since_epoch())
          .count());
}

// Minimal JSON emitter — no dep on nlohmann or rapidjson so the benchmark
// builds against just V8 + stdlib.
class JsonOut {
 public:
  explicit JsonOut(FILE* f) : f_(f) { std::fputc('{', f_); }
  ~JsonOut() { std::fputc('}', f_); std::fputc('\n', f_); }

  void KeyRaw(const char* k) {
    if (!first_) std::fputc(',', f_);
    first_ = false;
    std::fprintf(f_, "\"%s\":", k);
  }
  void KvInt(const char* k, int64_t v) {
    KeyRaw(k); std::fprintf(f_, "%lld", static_cast<long long>(v));
  }
  void KvDouble(const char* k, double v) {
    KeyRaw(k); std::fprintf(f_, "%.3f", v);
  }
  void KvStr(const char* k, const char* v) {
    KeyRaw(k); std::fprintf(f_, "\"%s\"", v);
  }
  void KvBool(const char* k, bool v) {
    KeyRaw(k); std::fprintf(f_, v ? "true" : "false");
  }
  void KvStats(const char* k, const Stats& s) {
    KeyRaw(k);
    std::fprintf(f_,
                 "{\"mean\":%.1f,\"median\":%.1f,\"p99\":%.1f,"
                 "\"min\":%llu,\"max\":%llu,\"samples\":%zu}",
                 s.mean_ns, s.median_ns, s.p99_ns,
                 static_cast<unsigned long long>(s.min_ns),
                 static_cast<unsigned long long>(s.max_ns),
                 s.samples);
  }
  // Start a nested object under k — caller writes fields via the returned
  // child JsonOut; when it goes out of scope the '}' closes the object.
  class Array {
   public:
    explicit Array(FILE* f) : f_(f) { std::fputc('[', f_); }
    ~Array() { std::fputc(']', f_); }
    void Next() { if (!first_) std::fputc(',', f_); first_ = false; }
    FILE* file() const { return f_; }
   private:
    FILE* f_;
    bool first_ = true;
  };

 private:
  FILE* f_;
  bool first_ = true;
};

// RAII V8 process init. Construct once in main().
class V8Env {
 public:
  explicit V8Env(const char* argv0) {
    v8::V8::InitializeICUDefaultLocation(argv0);
    v8::V8::InitializeExternalStartupData(argv0);
    platform_ = v8::platform::NewDefaultPlatform();
    v8::V8::InitializePlatform(platform_.get());
    v8::V8::Initialize();
    create_params_.array_buffer_allocator =
        v8::ArrayBuffer::Allocator::NewDefaultAllocator();
  }
  ~V8Env() {
    v8::V8::Dispose();
    v8::V8::DisposePlatform();
    delete create_params_.array_buffer_allocator;
  }
  v8::Isolate::CreateParams& CreateParams() { return create_params_; }

 private:
  std::unique_ptr<v8::Platform> platform_;
  v8::Isolate::CreateParams create_params_;
};

}  // namespace kenton_bench

#endif  // KENTON_RESPONSE_BENCH_COMMON_H_
