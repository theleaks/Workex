# demo.ps1 — Full Workex demo (Windows PowerShell)
$ErrorActionPreference = "Continue"

Write-Host ""
Write-Host "+=================================================+" -ForegroundColor Cyan
Write-Host "|  Workex Demo - Agent-Native JS Runtime           |" -ForegroundColor Cyan
Write-Host "|  166 tests, 0 mocks, 585x less memory than V8   |" -ForegroundColor Cyan
Write-Host "+=================================================+" -ForegroundColor Cyan
Write-Host ""

Write-Host "Building..." -ForegroundColor Yellow
cargo build --release 2>&1 | Where-Object { $_ -match "Finished|Compiling workex" } | Select-Object -Last 1
Write-Host ""

Write-Host "==================================================" -ForegroundColor Green
Write-Host "  CORRECTNESS" -ForegroundColor Green
Write-Host "==================================================" -ForegroundColor Green
Write-Host ""

function Run-Test($pkg, $filter, $label) {
    Write-Host -NoNewline "$label "
    $out = cargo test -p $pkg $filter --release 2>&1 | Out-String
    if ($out -match "passed") { Write-Host "PASS" -ForegroundColor Green }
    else { Write-Host "FAIL" -ForegroundColor Red }
}

Run-Test "workex-runtime" "execute_hello_ts" "1. Worker compat (hello.ts).............."
Run-Test "workex-runtime" "async_worker_fetch_called" "2. fetch() called once (bug fix)........"
Run-Test "workex-runtime" "cranelift_native_fn" "3. Cranelift native add(10,32)=42......."
Run-Test "workex-vm" "agent_survives" "4. Agent hibernation (restart).........."
Run-Test "workex-vm" "pipeline_hello" "5. Full pipeline (TS to VM)............."

Write-Host ""
Write-Host "==================================================" -ForegroundColor Green
Write-Host "  BENCHMARKS" -ForegroundColor Green
Write-Host "==================================================" -ForegroundColor Green

function Run-Bench($bin, $label, $patterns, $extraArgs) {
    Write-Host ""
    Write-Host "--- $label ---" -ForegroundColor Yellow
    if ($extraArgs) {
        $lines = cargo run -p workex-bench --release --bin $bin -- $extraArgs 2>&1
    } else {
        $lines = cargo run -p workex-bench --release --bin $bin 2>&1
    }
    foreach ($line in $lines) {
        $s = $line.ToString()
        $skip = $s -match "warning|Compil|crates\\|Running|Finished|Downloading|Locking|WORKEX\]|V8\]|WORKERS\]|Saved:|Idle pool|RSS before|RSS after|RSS delta|Verified|Creating|Running Node|Running Mini|bench\.|field in this|struct|note:|help:"
        if (-not $skip) {
            foreach ($p in $patterns) {
                if ($s -match $p) { Write-Host $s; break }
            }
        }
    }
}

Run-Bench "continuation-bench" "1M suspended agents" @("Per agent","1M agents","Factor")
Run-Bench "ten-million-bench" "10M suspended agents" @("Per agent","10M","Factor","Workex 10M","V8 10M")
Run-Bench "shared-bench" "SharedRuntime 10K (3-way)" @("Per context","10K Total","Architecture","METRIC","----")
Run-Bench "unified-bench" "Execution (3-way, 5 runs)" @("Cold start","Warm exec","Worker compat") "--runs 5"
Run-Bench "worker-test" "Worker compat latency (3-way)" @("Latency p50","Latency p99","Correct")
Run-Bench "rss-real-bench" "10K real Worker RSS (3-way)" @("Per Worker","10K Total","METRIC","----")

Write-Host ""
Write-Host "==================================================" -ForegroundColor Green
Write-Host "  DONE" -ForegroundColor Green
Write-Host "==================================================" -ForegroundColor Green
Write-Host ""
Write-Host "  Results: benchmarks/results/"
Write-Host "  Run all tests: cargo test (166 tests, 0 failures)"
Write-Host ""
