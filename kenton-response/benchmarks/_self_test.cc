// Self-test for the non-V8 parts of common.h: Stats, JsonOut, NowNanos.
// Compile: cl /nologo /std:c++17 /EHsc _self_test.cc
// (no V8 needed).

#define COMMON_H_NO_V8
// We can't include common.h directly because it pulls in v8.h. Instead,
// inline just the utilities under test. If common.h's implementations
// diverge from this copy, the compile-checking approach won't catch it —
// so keep these kept-in-sync manually if common.h changes.

#include <algorithm>
#include <cassert>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

namespace kenton_bench {

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

 private:
  FILE* f_;
  bool first_ = true;
};

}  // namespace kenton_bench

using namespace kenton_bench;

static int failures = 0;

#define EXPECT(cond) do { \
  if (!(cond)) { \
    std::fprintf(stderr, "FAIL: %s at line %d\n", #cond, __LINE__); \
    ++failures; \
  } \
} while (0)

#define EXPECT_EQ(a, b) EXPECT((a) == (b))

void test_stats_empty() {
  Stats s = ComputeStats({});
  EXPECT_EQ(s.samples, size_t(0));
  EXPECT_EQ(s.mean_ns, 0.0);
}

void test_stats_single() {
  Stats s = ComputeStats({42});
  EXPECT_EQ(s.samples, size_t(1));
  EXPECT_EQ(s.mean_ns, 42.0);
  EXPECT_EQ(s.median_ns, 42.0);
  EXPECT_EQ(s.p99_ns, 42.0);
  EXPECT_EQ(s.min_ns, uint64_t(42));
  EXPECT_EQ(s.max_ns, uint64_t(42));
}

void test_stats_many() {
  std::vector<uint64_t> v;
  for (uint64_t i = 1; i <= 100; ++i) v.push_back(i);
  Stats s = ComputeStats(v);
  EXPECT_EQ(s.samples, size_t(100));
  EXPECT_EQ(s.mean_ns, 50.5);
  EXPECT_EQ(s.min_ns, uint64_t(1));
  EXPECT_EQ(s.max_ns, uint64_t(100));
  // median: samples[50] = 51 (0-indexed)
  EXPECT_EQ(s.median_ns, 51.0);
  // p99 idx = (100*99)/100 = 99, samples[99] = 100
  EXPECT_EQ(s.p99_ns, 100.0);
}

void test_stats_unordered() {
  std::vector<uint64_t> v = {5, 3, 9, 1, 7, 2, 8, 6, 4};
  Stats s = ComputeStats(v);
  EXPECT_EQ(s.min_ns, uint64_t(1));
  EXPECT_EQ(s.max_ns, uint64_t(9));
  EXPECT_EQ(s.mean_ns, 5.0);
}

void test_nownanos_monotonic() {
  uint64_t a = NowNanos();
  // Spin to force a tick.
  for (volatile int i = 0; i < 1000000; ++i) {}
  uint64_t b = NowNanos();
  EXPECT(b >= a);
}

void test_json_empty() {
  FILE* f = std::fopen("_self_test_json_empty.json", "w");
  { JsonOut j(f); }
  std::fclose(f);
  FILE* g = std::fopen("_self_test_json_empty.json", "r");
  char buf[16]; size_t n = std::fread(buf, 1, sizeof(buf)-1, g); buf[n] = 0;
  std::fclose(g);
  EXPECT(std::strcmp(buf, "{}\n") == 0);
}

void test_json_flat() {
  FILE* f = std::fopen("_self_test_json_flat.json", "w");
  {
    JsonOut j(f);
    j.KvStr("name", "hello");
    j.KvInt("count", 42);
    j.KvBool("ok", true);
    j.KvDouble("ratio", 0.5);
  }
  std::fclose(f);
  FILE* g = std::fopen("_self_test_json_flat.json", "r");
  char buf[256]; size_t n = std::fread(buf, 1, sizeof(buf)-1, g); buf[n] = 0;
  std::fclose(g);
  std::fprintf(stderr, "flat json: %s", buf);
  EXPECT(std::strstr(buf, "\"name\":\"hello\"") != nullptr);
  EXPECT(std::strstr(buf, "\"count\":42") != nullptr);
  EXPECT(std::strstr(buf, "\"ok\":true") != nullptr);
  EXPECT(std::strstr(buf, "\"ratio\":0.500") != nullptr);
  // Must start with { and end with }
  EXPECT(buf[0] == '{');
  // Last non-newline char should be }
  size_t len = std::strlen(buf);
  while (len > 0 && (buf[len-1] == '\n' || buf[len-1] == '\r')) --len;
  EXPECT(len > 0 && buf[len-1] == '}');
}

void test_json_stats() {
  FILE* f = std::fopen("_self_test_json_stats.json", "w");
  Stats s{100.5, 99.0, 250.0, 50, 500, 1000};
  {
    JsonOut j(f);
    j.KvStats("latency_ns", s);
  }
  std::fclose(f);
  FILE* g = std::fopen("_self_test_json_stats.json", "r");
  char buf[512]; size_t n = std::fread(buf, 1, sizeof(buf)-1, g); buf[n] = 0;
  std::fclose(g);
  std::fprintf(stderr, "stats json: %s", buf);
  EXPECT(std::strstr(buf, "\"latency_ns\"") != nullptr);
  EXPECT(std::strstr(buf, "\"mean\":100.5") != nullptr);
  EXPECT(std::strstr(buf, "\"median\":99.0") != nullptr);
  EXPECT(std::strstr(buf, "\"p99\":250.0") != nullptr);
  EXPECT(std::strstr(buf, "\"min\":50") != nullptr);
  EXPECT(std::strstr(buf, "\"max\":500") != nullptr);
  EXPECT(std::strstr(buf, "\"samples\":1000") != nullptr);
}

int main() {
  test_stats_empty();
  test_stats_single();
  test_stats_many();
  test_stats_unordered();
  test_nownanos_monotonic();
  test_json_empty();
  test_json_flat();
  test_json_stats();

  if (failures == 0) {
    std::fprintf(stderr, "all tests passed\n");
    return 0;
  }
  std::fprintf(stderr, "%d test(s) failed\n", failures);
  return 1;
}
