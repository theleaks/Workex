// kenton-response benchmarks — shared utilities impl.

#include "common.h"

#include <random>

namespace kenton_bench {

namespace {

std::string MakeText(size_t approx_bytes, uint32_t seed) {
  std::mt19937 rng(seed);
  std::string s;
  s.reserve(approx_bytes);
  const char* alphabet = "abcdefghijklmnopqrstuvwxyz ";
  const size_t alen = 27;
  while (s.size() < approx_bytes) {
    s.push_back(alphabet[rng() % alen]);
  }
  return s;
}

v8::Local<v8::String> V8Str(v8::Isolate* iso, const char* s) {
  return v8::String::NewFromUtf8(iso, s, v8::NewStringType::kNormal)
      .ToLocalChecked();
}

v8::Local<v8::String> V8Str(v8::Isolate* iso, const std::string& s) {
  return v8::String::NewFromUtf8(iso, s.c_str(), v8::NewStringType::kNormal,
                                  static_cast<int>(s.size()))
      .ToLocalChecked();
}

// V8 14.x dropped Context::GetIsolate(), so callers pass iso explicitly.
void Set(v8::Isolate* iso, v8::Local<v8::Context> ctx,
         v8::Local<v8::Object> obj, const char* k,
         v8::Local<v8::Value> v) {
  obj->Set(ctx, V8Str(iso, k), v).Check();
}

v8::Local<v8::Object> BuildRequestHeaders(v8::Isolate* iso,
                                          v8::Local<v8::Context> ctx,
                                          uint32_t seed) {
  auto h = v8::Object::New(iso);
  std::mt19937 rng(seed);
  const char* cts[] = {"application/json", "text/plain",
                       "application/octet-stream", "text/html"};
  const char* uas[] = {"Mozilla/5.0 (X11; Linux x86_64) agent/1.0",
                       "curl/8.5.0", "Go-http-client/1.1",
                       "workerd/1.20260101.0"};
  Set(iso, ctx, h, "content-type", V8Str(iso, cts[rng() % 4]));
  Set(iso, ctx, h, "user-agent", V8Str(iso, uas[rng() % 4]));
  Set(iso, ctx, h, "accept", V8Str(iso, "*/*"));
  return h;
}

}  // namespace

v8::Local<v8::Object> BuildWorkload(v8::Isolate* iso,
                                    v8::Local<v8::Context> ctx,
                                    SizeClass s,
                                    uint32_t seed) {
  auto o = v8::Object::New(iso);
  std::mt19937 rng(seed);

  switch (s) {
    case SizeClass::XS: {
      Set(iso, ctx, o, "n", v8::Number::New(iso, 42.5 + (rng() % 1000)));
      Set(iso, ctx, o, "s", V8Str(iso, MakeText(8, seed + 1)));
      return o;
    }
    case SizeClass::S: {
      Set(iso, ctx, o, "url",
          V8Str(iso, "https://api.example.com/v1/agents/" +
                          MakeText(16, seed + 2) + "/resume"));
      Set(iso, ctx, o, "method", V8Str(iso, (seed % 2) ? "POST" : "GET"));
      Set(iso, ctx, o, "headers", BuildRequestHeaders(iso, ctx, seed + 3));
      return o;
    }
    case SizeClass::M: {
      Set(iso, ctx, o, "url",
          V8Str(iso, "https://api.example.com/v1/agents/" +
                          MakeText(16, seed + 4) + "/resume"));
      Set(iso, ctx, o, "method", V8Str(iso, "POST"));
      Set(iso, ctx, o, "headers", BuildRequestHeaders(iso, ctx, seed + 5));

      auto cache = v8::Object::New(iso);
      Set(iso, ctx, cache, "etag",
          V8Str(iso, "\"" + MakeText(11, seed + 6) + "\""));
      Set(iso, ctx, cache, "body", V8Str(iso, MakeText(2048, seed + 7)));
      Set(iso, ctx, o, "cache", cache);

      Set(iso, ctx, o, "body", V8Str(iso, MakeText(2048, seed + 8)));
      return o;
    }
    case SizeClass::L: {
      auto m = BuildWorkload(iso, ctx, SizeClass::M, seed);
      auto vec = v8::Array::New(iso, 1024);
      std::mt19937 vrng(seed + 100);
      for (uint32_t i = 0; i < 1024; ++i) {
        double x = (static_cast<double>(vrng()) / 4294967296.0) * 2.0 - 1.0;
        vec->Set(ctx, i, v8::Number::New(iso, x)).Check();
      }
      Set(iso, ctx, m, "embedding", vec);
      Set(iso, ctx, m, "doc", V8Str(iso, MakeText(40 * 1024, seed + 200)));
      return m;
    }
    case SizeClass::XL: {
      auto m = BuildWorkload(iso, ctx, SizeClass::M, seed);
      auto chat = v8::Array::New(iso, 30);
      for (uint32_t i = 0; i < 30; ++i) {
        auto msg = v8::Object::New(iso);
        Set(iso, ctx, msg, "role",
            V8Str(iso, (i % 2 == 0) ? "user" : "assistant"));
        Set(iso, ctx, msg, "content",
            V8Str(iso, MakeText(15 * 1024, seed + 1000 + i)));
        chat->Set(ctx, i, msg).Check();
      }
      Set(iso, ctx, m, "chat", chat);
      return m;
    }
  }
  return o;
}

}  // namespace kenton_bench
