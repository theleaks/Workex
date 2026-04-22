// P2 — Resume cost with fresh execution environment.
//
// Answers Kenton Varda's critique in workerd#6595:
//   "you presumably need to set up a new execution environment (isolate)
//    every time you want to deserialize the state to run a continuation,
//    and that is presumably pretty expensive"
//
// Three configurations × five workload sizes (XS..XL):
//
//   A: Fresh Isolate + Context per resume.            Strict isolation.
//   B: Pooled Isolate, fresh Context per resume.      Warm isolate reuse.
//   C: Pooled Context — NOT isolation-safe; verified.
//
// Pinned to V8 14.7.173.16 (the exact rev workerd uses).

#include <cassert>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <vector>

#include "common.h"

using namespace kenton_bench;

namespace {

const char kContinuationSource[] =
    "(function(state){"
    "  var total = 0;"
    "  for (var k in state) {"
    "    var v = state[k];"
    "    if (typeof v === 'number') total += v;"
    "  }"
    "  return total;"
    "})";

class Delegate : public v8::ValueSerializer::Delegate {
 public:
  void ThrowDataCloneError(v8::Local<v8::String> message) override {
    v8::Isolate::GetCurrent()->ThrowException(v8::Exception::Error(message));
  }
};

std::vector<uint8_t> BuildSerializedState(v8::Isolate* iso, SizeClass sc,
                                          uint32_t seed) {
  v8::HandleScope hs(iso);
  v8::Local<v8::Context> ctx = v8::Context::New(iso);
  v8::Context::Scope cs(ctx);
  auto state = BuildWorkload(iso, ctx, sc, seed);
  Delegate delegate;
  v8::ValueSerializer ser(iso, &delegate);
  ser.WriteHeader();
  ser.WriteValue(ctx, state).Check();
  auto buf = ser.Release();
  std::vector<uint8_t> out(buf.first, buf.first + buf.second);
  std::free(buf.first);
  return out;
}

v8::Local<v8::Function> CompileEntryPoint(v8::Isolate* iso,
                                          v8::Local<v8::Context> ctx) {
  v8::EscapableHandleScope hs(iso);
  v8::Local<v8::String> src =
      v8::String::NewFromUtf8(iso, kContinuationSource,
                               v8::NewStringType::kNormal)
          .ToLocalChecked();
  v8::Local<v8::Script> script =
      v8::Script::Compile(ctx, src).ToLocalChecked();
  v8::Local<v8::Value> fn_val = script->Run(ctx).ToLocalChecked();
  return hs.Escape(v8::Local<v8::Function>::Cast(fn_val));
}

struct ResumeSample {
  uint64_t setup_ns;
  uint64_t deserialize_ns;
  uint64_t first_instruction_ns;
};

ResumeSample RunConfigA(v8::Isolate::CreateParams& create_params,
                        const std::vector<uint8_t>& state_bytes) {
  uint64_t t0 = NowNanos();
  v8::Isolate* iso = v8::Isolate::New(create_params);
  uint64_t t_setup_end, t_deser_end, t_first_instr_end;
  {
    v8::Isolate::Scope is(iso);
    v8::HandleScope hs(iso);
    v8::Local<v8::Context> ctx = v8::Context::New(iso);
    v8::Context::Scope cs(ctx);
    auto entry = CompileEntryPoint(iso, ctx);
    t_setup_end = NowNanos();

    v8::ValueDeserializer des(iso, state_bytes.data(), state_bytes.size());
    des.ReadHeader(ctx).Check();
    v8::Local<v8::Value> state = des.ReadValue(ctx).ToLocalChecked();
    t_deser_end = NowNanos();

    v8::Local<v8::Value> args[] = {state};
    entry->Call(ctx, ctx->Global(), 1, args).ToLocalChecked();
    t_first_instr_end = NowNanos();
  }
  iso->Dispose();
  return {t_setup_end - t0, t_deser_end - t_setup_end,
          t_first_instr_end - t_deser_end};
}

ResumeSample RunConfigB(v8::Isolate* iso,
                        const std::vector<uint8_t>& state_bytes) {
  v8::Isolate::Scope is(iso);
  v8::HandleScope hs(iso);

  uint64_t t0 = NowNanos();
  v8::Local<v8::Context> ctx = v8::Context::New(iso);
  v8::Context::Scope cs(ctx);
  auto entry = CompileEntryPoint(iso, ctx);
  uint64_t t_setup_end = NowNanos();

  v8::ValueDeserializer des(iso, state_bytes.data(), state_bytes.size());
  des.ReadHeader(ctx).Check();
  v8::Local<v8::Value> state = des.ReadValue(ctx).ToLocalChecked();
  uint64_t t_deser_end = NowNanos();

  v8::Local<v8::Value> args[] = {state};
  entry->Call(ctx, ctx->Global(), 1, args).ToLocalChecked();
  uint64_t t_first_instr_end = NowNanos();

  return {t_setup_end - t0, t_deser_end - t_setup_end,
          t_first_instr_end - t_deser_end};
}

struct PooledContext {
  v8::Global<v8::Context> ctx;
  v8::Global<v8::Function> entry;
};

void InitPooledContext(v8::Isolate* iso, PooledContext* pc) {
  v8::Isolate::Scope is(iso);
  v8::HandleScope hs(iso);
  v8::Local<v8::Context> ctx = v8::Context::New(iso);
  v8::Context::Scope cs(ctx);
  auto entry = CompileEntryPoint(iso, ctx);
  pc->ctx.Reset(iso, ctx);
  pc->entry.Reset(iso, entry);
}

ResumeSample RunConfigC(v8::Isolate* iso, PooledContext* pc,
                        const std::vector<uint8_t>& state_bytes) {
  v8::Isolate::Scope is(iso);
  v8::HandleScope hs(iso);

  uint64_t t0 = NowNanos();
  v8::Local<v8::Context> ctx = pc->ctx.Get(iso);
  v8::Context::Scope cs(ctx);
  auto entry = pc->entry.Get(iso);
  uint64_t t_setup_end = NowNanos();

  v8::ValueDeserializer des(iso, state_bytes.data(), state_bytes.size());
  des.ReadHeader(ctx).Check();
  v8::Local<v8::Value> state = des.ReadValue(ctx).ToLocalChecked();
  uint64_t t_deser_end = NowNanos();

  v8::Local<v8::Value> args[] = {state};
  entry->Call(ctx, ctx->Global(), 1, args).ToLocalChecked();
  uint64_t t_first_instr_end = NowNanos();

  return {t_setup_end - t0, t_deser_end - t_setup_end,
          t_first_instr_end - t_deser_end};
}

bool VerifyIsolationA(v8::Isolate::CreateParams& create_params) {
  auto run = [&](const char* src) -> std::string {
    v8::Isolate* iso = v8::Isolate::New(create_params);
    std::string result;
    {
      v8::Isolate::Scope is(iso);
      v8::HandleScope hs(iso);
      v8::Local<v8::Context> ctx = v8::Context::New(iso);
      v8::Context::Scope cs(ctx);
      v8::Local<v8::String> s =
          v8::String::NewFromUtf8(iso, src, v8::NewStringType::kNormal)
              .ToLocalChecked();
      auto script = v8::Script::Compile(ctx, s).ToLocalChecked();
      auto v = script->Run(ctx).ToLocalChecked();
      v8::String::Utf8Value utf8(iso, v);
      if (*utf8) result = *utf8;
    }
    iso->Dispose();
    return result;
  };
  run("globalThis.__leaked_secret = 'from_A'; 'ok'");
  return run("typeof globalThis.__leaked_secret === 'undefined' ? 'clean' : 'LEAKED'") == "clean";
}

bool VerifyIsolationB(v8::Isolate* iso) {
  auto run = [&](const char* src) -> std::string {
    v8::Isolate::Scope is(iso);
    v8::HandleScope hs(iso);
    v8::Local<v8::Context> ctx = v8::Context::New(iso);
    v8::Context::Scope cs(ctx);
    auto s = v8::String::NewFromUtf8(iso, src, v8::NewStringType::kNormal)
                 .ToLocalChecked();
    auto script = v8::Script::Compile(ctx, s).ToLocalChecked();
    auto v = script->Run(ctx).ToLocalChecked();
    v8::String::Utf8Value utf8(iso, v);
    return *utf8 ? std::string(*utf8) : std::string();
  };
  run("globalThis.__leaked_secret = 'from_A'; 'ok'");
  return run("typeof globalThis.__leaked_secret === 'undefined' ? 'clean' : 'LEAKED'") == "clean";
}

bool VerifyIsolationC(v8::Isolate* iso, PooledContext* pc) {
  v8::Isolate::Scope is(iso);
  v8::HandleScope hs(iso);
  v8::Local<v8::Context> ctx = pc->ctx.Get(iso);
  v8::Context::Scope cs(ctx);
  auto run = [&](const char* src) -> std::string {
    auto s = v8::String::NewFromUtf8(iso, src, v8::NewStringType::kNormal)
                 .ToLocalChecked();
    auto script = v8::Script::Compile(ctx, s).ToLocalChecked();
    auto v = script->Run(ctx).ToLocalChecked();
    v8::String::Utf8Value utf8(iso, v);
    return *utf8 ? std::string(*utf8) : std::string();
  };
  run("globalThis.__leaked_secret = 'from_A'; 'ok'");
  std::string r =
      run("typeof globalThis.__leaked_secret === 'undefined' ? 'clean' : 'LEAKED'");
  run("delete globalThis.__leaked_secret; 'ok'");
  return r == "LEAKED";
}

struct Aggregate {
  std::vector<uint64_t> setup, deser, first, total;
};

void Push(Aggregate& a, const ResumeSample& s) {
  a.setup.push_back(s.setup_ns);
  a.deser.push_back(s.deserialize_ns);
  a.first.push_back(s.first_instruction_ns);
  a.total.push_back(s.setup_ns + s.deserialize_ns + s.first_instruction_ns);
}

void EmitConfigJson(FILE* out, const char* cfg, const char* workload,
                    bool isolation_safe, bool check_passed,
                    const Aggregate& a) {
  JsonOut j(out);
  j.KvStr("config", cfg);
  j.KvStr("workload", workload);
  j.KvBool("isolation_safe", isolation_safe);
  j.KvBool("isolation_check_passed", check_passed);
  j.KvInt("iterations", static_cast<int64_t>(a.setup.size()));
  j.KvStats("setup_ns", ComputeStats(a.setup));
  j.KvStats("deserialize_ns", ComputeStats(a.deser));
  j.KvStats("first_instruction_ns", ComputeStats(a.first));
  j.KvStats("total_resume_ns", ComputeStats(a.total));
}

size_t ItersA(SizeClass sc) {
  // Fresh-isolate path is slow; cap iterations.
  switch (sc) {
    case SizeClass::XS: return 300;
    case SizeClass::S:  return 300;
    case SizeClass::M:  return 200;
    case SizeClass::L:  return 100;
    case SizeClass::XL: return 50;
  }
  return 100;
}
size_t ItersBC(SizeClass sc) {
  switch (sc) {
    case SizeClass::XS: return 5000;
    case SizeClass::S:  return 5000;
    case SizeClass::M:  return 5000;
    case SizeClass::L:  return 1000;
    case SizeClass::XL: return 200;
  }
  return 5000;
}

}  // namespace

int main(int argc, char** argv) {
  V8Env env(argv[0]);

  uint32_t seed = 0;
  if (const char* s = std::getenv("KENTON_SEED")) {
    seed = static_cast<uint32_t>(std::atoi(s));
  }

  // Pre-build serialized state for each size class. Reused across A/B/C
  // for that class so the deserialize cost is comparable.
  const SizeClass classes[] = {SizeClass::XS, SizeClass::S, SizeClass::M,
                                SizeClass::L, SizeClass::XL};
  std::vector<std::vector<uint8_t>> bytes_by_class;
  for (SizeClass sc : classes) {
    v8::Isolate* tmp = v8::Isolate::New(env.CreateParams());
    {
      v8::Isolate::Scope is(tmp);
      bytes_by_class.push_back(BuildSerializedState(tmp, sc, seed + 7));
    }
    tmp->Dispose();
    std::fprintf(stderr, "[p2] state %s = %zu bytes\n",
                 SizeClassName(sc), bytes_by_class.back().size());
  }

  // Filter config / workload via argv.
  bool filter_cfg = argc > 1;
  const char* only_cfg = filter_cfg ? argv[1] : nullptr;
  bool filter_wl = argc > 2;
  const char* only_wl = filter_wl ? argv[2] : nullptr;
  auto want_cfg = [&](const char* c) {
    return !filter_cfg || std::strcmp(only_cfg, c) == 0;
  };
  auto want_wl = [&](SizeClass sc) {
    return !filter_wl || std::strcmp(only_wl, SizeClassName(sc)) == 0;
  };

  std::fputc('[', stdout);
  bool first = true;
  auto comma = [&]() {
    if (!first) std::fputc(',', stdout);
    first = false;
    std::fputc('\n', stdout);
  };

  for (size_t ci = 0; ci < sizeof(classes) / sizeof(classes[0]); ++ci) {
    SizeClass sc = classes[ci];
    if (!want_wl(sc)) continue;
    const auto& state_bytes = bytes_by_class[ci];

    if (want_cfg("A")) {
      std::fprintf(stderr, "[p2] A %s: fresh isolate per resume\n",
                   SizeClassName(sc));
      bool iso_ok = VerifyIsolationA(env.CreateParams());
      Aggregate a;
      // Warmup
      for (size_t i = 0; i < 20; ++i) RunConfigA(env.CreateParams(), state_bytes);
      for (size_t i = 0; i < ItersA(sc); ++i)
        Push(a, RunConfigA(env.CreateParams(), state_bytes));
      comma();
      EmitConfigJson(stdout, "A", SizeClassName(sc), iso_ok, iso_ok, a);
      std::fflush(stdout);
    }

    if (want_cfg("B")) {
      std::fprintf(stderr, "[p2] B %s: pooled isolate, fresh context\n",
                   SizeClassName(sc));
      v8::Isolate* iso = v8::Isolate::New(env.CreateParams());
      bool iso_ok = VerifyIsolationB(iso);
      Aggregate a;
      for (size_t i = 0; i < 50; ++i) RunConfigB(iso, state_bytes);
      for (size_t i = 0; i < ItersBC(sc); ++i)
        Push(a, RunConfigB(iso, state_bytes));
      iso->Dispose();
      comma();
      EmitConfigJson(stdout, "B", SizeClassName(sc), iso_ok, iso_ok, a);
      std::fflush(stdout);
    }

    if (want_cfg("C")) {
      std::fprintf(stderr,
                   "[p2] C %s: pooled context (NOT isolation safe)\n",
                   SizeClassName(sc));
      v8::Isolate* iso = v8::Isolate::New(env.CreateParams());
      PooledContext pc;
      InitPooledContext(iso, &pc);
      bool leak = VerifyIsolationC(iso, &pc);
      Aggregate a;
      for (size_t i = 0; i < 50; ++i) RunConfigC(iso, &pc, state_bytes);
      for (size_t i = 0; i < ItersBC(sc); ++i)
        Push(a, RunConfigC(iso, &pc, state_bytes));
      pc.ctx.Reset();
      pc.entry.Reset();
      iso->Dispose();
      comma();
      EmitConfigJson(stdout, "C", SizeClassName(sc), false, leak, a);
      std::fflush(stdout);
    }
  }

  std::fputc('\n', stdout);
  std::fputc(']', stdout);
  std::fputc('\n', stdout);
  return 0;
}
