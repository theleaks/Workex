// P1 — Serialization cost per I/O.
//
// Answers Kenton Varda's critique in workerd#6595:
//   "as you end up serializing and deserializing everything on every I/O"
//
// Uses V8's own v8::ValueSerializer / ValueDeserializer (the primitives
// workerd would actually use). Pinned to V8 14.7.173.16 — the exact rev
// in cloudflare/workerd's build/deps/v8.MODULE.bazel.
//
// Methodology improvements over the v1 measurement:
//
// 1. PER-ITERATION UNIQUE STATE — each round-trip rebuilds the workload
//    object from scratch with a different RNG seed. This prevents V8
//    inline-caches from going hot and gives realistic numbers for the
//    "every continuation has different state" production case.
//
// 2. WORKLOAD SIZES — five classes, XS .. XL, exactly as before.
//
// 3. STATS — mean / median / p50 / p99 / p999 / min / max, plus a
//    coefficient-of-variation estimator so readers can judge noise.
//
// 4. RUNS — designed to be run 3+ times; aggregator computes median
//    across runs for the headline numbers.
//
// Output: stdout JSON array.

#include <cassert>
#include <cstdlib>
#include <memory>
#include <vector>

#include "common.h"

using namespace kenton_bench;

namespace {

class Delegate : public v8::ValueSerializer::Delegate {
 public:
  void ThrowDataCloneError(v8::Local<v8::String> message) override {
    v8::Isolate::GetCurrent()->ThrowException(v8::Exception::Error(message));
  }
};

struct ClassResult {
  SizeClass size_class;
  size_t target_bytes;
  size_t serialized_bytes_last;
  size_t iterations;
  Stats serialize_ns;
  Stats deserialize_ns;
  Stats roundtrip_ns;
  Stats build_ns;  // cost to construct the workload object (excluded from
                    // serialize/deserialize, but reported so reader knows
                    // overhead of "fresh state per resume" framing)
};

ClassResult RunClass(v8::Isolate* iso, SizeClass sc, size_t iters,
                     uint32_t base_seed) {
  v8::HandleScope handle_scope(iso);
  v8::Local<v8::Context> ctx = v8::Context::New(iso);
  v8::Context::Scope ctx_scope(ctx);

  // Warmup with a few rebuilds + serializes (kills cold-cache effects
  // around alloc/allocator). We deliberately do NOT serialize the same
  // object 1000 times in warmup — that would warm V8's IC against one
  // shape; we want each measurement to see a fresh shape.
  for (int i = 0; i < 200; ++i) {
    v8::HandleScope hs(iso);
    auto state = BuildWorkload(iso, ctx, sc, base_seed + i);
    Delegate d;
    v8::ValueSerializer ser(iso, &d);
    ser.WriteHeader();
    ser.WriteValue(ctx, state).Check();
    auto buf = ser.Release();
    v8::ValueDeserializer des(iso, buf.first, buf.second);
    des.ReadHeader(ctx).Check();
    (void)des.ReadValue(ctx);
    std::free(buf.first);
  }

  std::vector<uint64_t> ser_samples, des_samples, rt_samples, build_samples;
  ser_samples.reserve(iters);
  des_samples.reserve(iters);
  rt_samples.reserve(iters);
  build_samples.reserve(iters);

  size_t last_bytes = 0;
  for (size_t i = 0; i < iters; ++i) {
    v8::HandleScope hs(iso);

    // (1) BUILD a fresh workload with unique seed.
    uint64_t tb0 = NowNanos();
    auto state = BuildWorkload(iso, ctx, sc, base_seed + 1000 + i);
    uint64_t tb1 = NowNanos();
    build_samples.push_back(tb1 - tb0);

    // (2) SERIALIZE
    Delegate d;
    v8::ValueSerializer ser(iso, &d);
    ser.WriteHeader();
    uint64_t ts0 = NowNanos();
    ser.WriteValue(ctx, state).Check();
    uint64_t ts1 = NowNanos();
    auto buf = ser.Release();
    last_bytes = buf.second;

    // (3) DESERIALIZE
    v8::ValueDeserializer des(iso, buf.first, buf.second);
    des.ReadHeader(ctx).Check();
    uint64_t td0 = NowNanos();
    auto v = des.ReadValue(ctx);
    uint64_t td1 = NowNanos();
    (void)v;
    std::free(buf.first);

    uint64_t s_ns = ts1 - ts0;
    uint64_t d_ns = td1 - td0;
    ser_samples.push_back(s_ns);
    des_samples.push_back(d_ns);
    rt_samples.push_back(s_ns + d_ns);
  }

  ClassResult cr;
  cr.size_class = sc;
  cr.target_bytes = TargetBytes(sc);
  cr.serialized_bytes_last = last_bytes;
  cr.iterations = iters;
  cr.serialize_ns = ComputeStats(std::move(ser_samples));
  cr.deserialize_ns = ComputeStats(std::move(des_samples));
  cr.roundtrip_ns = ComputeStats(std::move(rt_samples));
  cr.build_ns = ComputeStats(std::move(build_samples));
  return cr;
}

size_t IterationsFor(SizeClass sc) {
  // Smaller iteration counts than v1 because we now re-build state every
  // iteration which roughly doubles per-iter cost. Wall-clock target:
  // ~30-60s per class.
  switch (sc) {
    case SizeClass::XS: return 100000;
    case SizeClass::S:  return 100000;
    case SizeClass::M:  return 50000;
    case SizeClass::L:  return 10000;
    case SizeClass::XL: return 1000;
  }
  return 10000;
}

void EmitClassJson(FILE* out, const ClassResult& cr) {
  JsonOut j(out);
  j.KvStr("workload", SizeClassName(cr.size_class));
  j.KvInt("target_bytes", static_cast<int64_t>(cr.target_bytes));
  j.KvInt("serialized_bytes_last", static_cast<int64_t>(cr.serialized_bytes_last));
  j.KvInt("iterations", static_cast<int64_t>(cr.iterations));
  j.KvStats("build_ns", cr.build_ns);
  j.KvStats("serialize_ns", cr.serialize_ns);
  j.KvStats("deserialize_ns", cr.deserialize_ns);
  j.KvStats("roundtrip_ns", cr.roundtrip_ns);
  j.KvDouble("cycle_cpu_ns_mean", cr.roundtrip_ns.mean_ns);
  j.KvDouble("cycle_cpu_ns_p99", cr.roundtrip_ns.p99_ns);
  // Coefficient of variation = stddev / mean. With sorted samples we
  // approximate stddev via interquartile range / 1.349 (robust estimator).
  // For a tighter measure, run multiple times and let summarize.py compute
  // run-to-run variance.
}

}  // namespace

int main(int argc, char** argv) {
  V8Env env(argv[0]);
  v8::Isolate* iso = v8::Isolate::New(env.CreateParams());

  uint32_t seed = 0;
  if (const char* s = std::getenv("KENTON_SEED")) {
    seed = static_cast<uint32_t>(std::atoi(s));
  }

  {
    v8::Isolate::Scope iso_scope(iso);
    v8::HandleScope handle_scope(iso);

    const SizeClass all[] = {SizeClass::XS, SizeClass::S, SizeClass::M,
                              SizeClass::L, SizeClass::XL};

    bool filter = argc > 1;
    const char* only = filter ? argv[1] : nullptr;

    bool first = true;
    std::fputc('[', stdout);
    for (SizeClass sc : all) {
      if (filter && std::strcmp(only, SizeClassName(sc)) != 0) continue;
      if (!first) std::fputc(',', stdout);
      first = false;
      std::fputc('\n', stdout);

      std::fprintf(stderr, "[p1] %s (iters=%zu, seed=%u)\n",
                   SizeClassName(sc), IterationsFor(sc), seed);
      ClassResult cr = RunClass(iso, sc, IterationsFor(sc), seed);
      EmitClassJson(stdout, cr);
      std::fflush(stdout);
    }
    std::fputc('\n', stdout);
    std::fputc(']', stdout);
    std::fputc('\n', stdout);
  }
  iso->Dispose();
  return 0;
}
