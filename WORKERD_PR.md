# RFC: Pluggable Runtime Interface for workerd

## Problem

V8 isolates allocate ~183KB per execution context regardless of whether
the Worker is actively executing or waiting for I/O. For agent workloads
where 99% of agents are suspended (awaiting LLM/API responses), this
creates a hard ceiling:

- 24M concurrent agents x 183KB = **4.1 TB RAM** (physically impossible)

## Proposal

A minimal interface to allow alternative execution models in workerd:

```cpp
class JsRuntime {
public:
  struct ExecutionResult {
    enum class Kind { Done, Suspended, Error };
    Kind kind;
    kj::Maybe<Response> response;
    kj::Maybe<Continuation> cont;
    kj::Maybe<IoRequest> io_request;
    kj::Maybe<kj::String> error;
  };

  virtual ExecutionResult executeFetch(
    const Request& request,
    const WorkerEnv& env
  ) = 0;

  virtual ExecutionResult resume(
    const Continuation& cont,
    const IoResult& result
  ) = 0;

  virtual size_t memoryUsageBytes() const = 0;
};
```

## Implementation

We've implemented this interface as Workex (github.com/user/workex).

Benchmark results with the continuation approach:
- 1M suspended agents: 182 MB vs V8's 174.5 GB (981x less)
- 10M suspended agents: 2.99 GB vs V8's 1,745 GB (585x less)
- 24M agents: ~22 GB vs V8's ~4.1 TB

162 tests. Zero mocks. Workers API compatible.

## Backward Compatibility

- Existing Workers continue using V8 (no change)
- Interface is opt-in per Worker
- No changes to wrangler or deployment pipeline
