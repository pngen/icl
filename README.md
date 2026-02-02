# Intelligence Capital Ledger (ICL)

A deterministic, audit-grade accounting system for intelligence capital that converts inference execution into CFO-ready financial artifacts.

## Overview

ICL is a financial governance system that treats intelligence as institutional capital rather than an operational expense. It provides structured capitalization, allocation, depreciation, and return realization for intelligence assets while maintaining full auditability and compliance with accounting standards.

ICL operates above inference attribution systems (like ICAE) and below financial governance layers, formalizing the economic meaning of intelligence execution without re-computing attribution or replacing existing ERP systems.

## Architecture

<pre>
┌─────────────────┐    ┌──────────────────┐    ┌──────────────────┐
│  Inference      │    │  Intelligence    │    │  Financial       │
│  Attribution    │───▶│  Capital Ledger  │───▶│  Reporting       │
│  (ICAE)         │    │  (ICL)           │    │  Systems         │
└─────────────────┘    └──────────────────┘    └──────────────────┘
         │
         ▼
┌─────────────────┐
│  Audit &        │
│  Compliance     │
│  Tooling        │
└─────────────────┘
</pre>

## Components

### IntelligenceAsset  
A capitalized unit of intelligence capability with owner, value, and depreciation rules. Supports the full asset lifecycle from capitalization through allocation, utilization, depreciation, and retirement.

### CapitalEvent and LedgerEntry  
Discrete economic actions affecting intelligence capital (allocation, utilization, depreciation) recorded as immutable, time-ordered financial records. Append-only semantics with no silent revaluation or aggregation without traceability.

### JournalEntry  
Double-entry accounting journal entries for proper financial statement generation. All primitives map directly to standard accounting concepts.

### CapitalProof  
Machine-verifiable explanations of financial figures for audit purposes. Every transaction is traceable with reconstructable audit trails, integration-ready with compliance tooling.

### DepreciationEngine  
Calculates asset value decay using configurable methods. Deterministic calculations ensure reproducible financial outcomes across any replay window.

### LifecycleManager  
Orchestrates the complete asset lifecycle from capitalization to retirement, including allocation between business units, utilization tracking, and write-off procedures.

### IntegrityChecker  
Prevents retroactive modifications, detects and fails on invalid data, and ensures no unowned intelligence execution. Failure modes are explicit and do not compromise system integrity.

### IntegrationAdapter  
Consumes inference attribution from ICAE and emits to financial reporting systems. Supports cross-system reconciliation without assuming control over execution or finance platforms.

## Build
```bash
cargo build --release
```

## Test
```bash
cargo test
```

## Run
```bash
./icl
```

On Windows:
```bash
.\icl.exe
```

## Design Principles
1. **Deterministic** - All financial outcomes are predictable and reproducible.
2. **Audit-Ready** - Every transaction is traceable with machine-verifiable proofs.
3. **Accounting-Legible** - All primitives map directly to accounting concepts.
4. **Immutable** - Once recorded, data cannot be altered or retroactively modified.
5. **Composable** - Components can be integrated independently without tight coupling.

## Requirements
- Rust 1.56+
- Append-only, immutable ledger semantics
- Deterministic depreciation calculations
- Audit-grade proof generation