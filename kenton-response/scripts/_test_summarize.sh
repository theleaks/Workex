#!/usr/bin/env bash
# Dry-run test of summarize.py with synthetic results.

set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
t=$(mktemp -d)
trap "rm -rf $t" EXIT

# Synthetic P1
cat > "$t/p1.json" <<'EOF'
[
  {"workload":"XS","target_bytes":50,"serialized_bytes":64,"iterations":200000,
   "serialize_ns":{"mean":420.5,"median":400.0,"p99":850.0,"min":300,"max":12000,"samples":200000},
   "deserialize_ns":{"mean":380.0,"median":360.0,"p99":720.0,"min":280,"max":9000,"samples":200000},
   "roundtrip_ns":{"mean":800.5,"median":760.0,"p99":1570.0,"min":580,"max":21000,"samples":200000},
   "cycle_cpu_ns_mean":800.5,"cycle_cpu_ns_p99":1570.0},
  {"workload":"M","target_bytes":5120,"serialized_bytes":4956,"iterations":100000,
   "serialize_ns":{"mean":4200.0,"median":4100.0,"p99":8900.0,"min":3800,"max":95000,"samples":100000},
   "deserialize_ns":{"mean":3900.0,"median":3800.0,"p99":8200.0,"min":3500,"max":88000,"samples":100000},
   "roundtrip_ns":{"mean":8100.0,"median":7900.0,"p99":17100.0,"min":7300,"max":183000,"samples":100000},
   "cycle_cpu_ns_mean":8100.0,"cycle_cpu_ns_p99":17100.0},
  {"workload":"XL","target_bytes":512000,"serialized_bytes":498432,"iterations":2000,
   "serialize_ns":{"mean":420000.0,"median":410000.0,"p99":890000.0,"min":380000,"max":1200000,"samples":2000},
   "deserialize_ns":{"mean":390000.0,"median":380000.0,"p99":820000.0,"min":350000,"max":1100000,"samples":2000},
   "roundtrip_ns":{"mean":810000.0,"median":790000.0,"p99":1710000.0,"min":730000,"max":2300000,"samples":2000},
   "cycle_cpu_ns_mean":810000.0,"cycle_cpu_ns_p99":1710000.0}
]
EOF

# Synthetic P2
cat > "$t/p2.json" <<'EOF'
[
  {"config":"A","isolation_safe":true,"isolation_check_passed":true,"iterations":500,
   "setup_ns":{"mean":3800000.0,"median":3700000.0,"p99":7800000.0,"min":3500000,"max":12000000,"samples":500},
   "deserialize_ns":{"mean":82000.0,"median":80000.0,"p99":170000.0,"min":75000,"max":290000,"samples":500},
   "first_instruction_ns":{"mean":120000.0,"median":115000.0,"p99":240000.0,"min":100000,"max":380000,"samples":500},
   "total_resume_ns":{"mean":4002000.0,"median":3895000.0,"p99":8210000.0,"min":3675000,"max":12670000,"samples":500}},
  {"config":"B","isolation_safe":true,"isolation_check_passed":true,"iterations":10000,
   "setup_ns":{"mean":62000.0,"median":60000.0,"p99":125000.0,"min":55000,"max":190000,"samples":10000},
   "deserialize_ns":{"mean":80000.0,"median":79000.0,"p99":165000.0,"min":72000,"max":270000,"samples":10000},
   "first_instruction_ns":{"mean":115000.0,"median":113000.0,"p99":235000.0,"min":100000,"max":370000,"samples":10000},
   "total_resume_ns":{"mean":257000.0,"median":252000.0,"p99":525000.0,"min":227000,"max":830000,"samples":10000}},
  {"config":"C","isolation_safe":false,"isolation_check_passed":true,"iterations":10000,
   "setup_ns":{"mean":80.0,"median":75.0,"p99":160.0,"min":60,"max":450,"samples":10000},
   "deserialize_ns":{"mean":79000.0,"median":78000.0,"p99":160000.0,"min":72000,"max":260000,"samples":10000},
   "first_instruction_ns":{"mean":88000.0,"median":86000.0,"p99":180000.0,"min":80000,"max":290000,"samples":10000},
   "total_resume_ns":{"mean":167080.0,"median":164075.0,"p99":340160.0,"min":152060,"max":550450,"samples":10000}}
]
EOF

# Synthetic P3 memory
cat > "$t/p3_memory.json" <<'EOF'
[
  {"pattern":"hello","isolates":1,"rss_bytes":52428800,"total_heap_size":4194304,"used_heap_size":2097152,"external_memory":0},
  {"pattern":"hello","isolates":10,"rss_bytes":104857600,"total_heap_size":41943040,"used_heap_size":20971520,"external_memory":0},
  {"pattern":"hello","isolates":100,"rss_bytes":556793856,"total_heap_size":419430400,"used_heap_size":209715200,"external_memory":0},
  {"pattern":"hello","isolates":1000,"rss_bytes":5368709120,"total_heap_size":4194304000,"used_heap_size":2097152000,"external_memory":0},
  {"pattern":"realistic","isolates":1,"rss_bytes":57671680,"total_heap_size":8388608,"used_heap_size":5242880,"external_memory":0}
]
EOF

cat > "$t/p3_heap_classifier.txt" <<'EOF'

Heap classification (single isolate):
  stage_0_post_isolate        used=  512.00 KB  total=  1024.00 KB
  stage_1_post_context        used=  640.00 KB  total=  1024.00 KB
  stage_2_post_minimal        used=  642.00 KB  total=  1024.00 KB
  stage_3_post_realistic      used=  755.00 KB  total=  1024.00 KB

{
  "stage_0_post_isolate":   524288,
  "stage_1_post_context":   655360,
  "stage_2_post_minimal":   657408,
  "stage_3_post_realistic": 773120
}
EOF

cat > "$t/p3_prototype.txt" <<'EOF'
shared: installed 30 built-ins, sealed.
summary:
  isolates:                 1000
  shared built-ins (once):  1920 B
  naive total:              1920000 B
  COW total:                8320 B
  savings:                  99.57%
EOF

cat > "$t/env.txt" <<'EOF'
date: 2026-04-22T00:00:00Z
host: Linux container x86_64
v8: abcdef1234567890
EOF

python3 "$here/summarize.py" "$t" 2>&1 | tail -60
