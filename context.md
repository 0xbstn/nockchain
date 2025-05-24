# Nockchain Architecture & Implementation Guide

## Overview

Nockchain is a revolutionary blockchain that replaces traditional public replication with zero-knowledge proofs for verification. Built on the Urbit technology stack, it uses a unique three-layer architecture designed for verifiable heavyweight computation.

## Core Architecture

### Technology Stack

1. **Nock Virtual Machine**
   - Minimal VM with only 12 opcodes
   - Unified data model: everything is a "noun" (atom = natural number, or cell = pair of nouns)
   - Pure axiomatic system, deterministic and serializable
   - Tree addressing system for data access

2. **Hoon Programming Language**
   - Statically typed functional language
   - Compiles directly to Nock
   - "Runic" syntax using ASCII digraphs
   - Subject-oriented programming (no implicit environment)
   - Type system stores variable names and scopes

3. **Rust Infrastructure**
   - High-performance runtime
   - "Jets" system for native acceleration
   - P2P networking with libp2p/QUIC
   - NockVM implementation with memory management

### Revolutionary Blockchain Concept

**Traditional Blockchain**: All nodes validate all transactions (public replication)
**Nockchain**: Miners generate ZK-STARK proofs that computation is correct (private proving, public verification)

Benefits:
- Lightweight verification (nodes only verify proofs, don't re-execute)
- Heavyweight verifiable applications possible
- Deterministic computation guarantees
- Universal serialization format

## Key Components

### Mining Process

```
Block Candidate → Hoon Miner Kernel → ZK-STARK Prover → Proof Generation → Block with Proof
```

1. Receives block candidate from main kernel
2. Creates temporary directory and loads mining kernel with hot state
3. Executes proof generation in Hoon
4. Returns ZK-STARK proof via effect system

### Hoon Applications

- **`dumbnet/inner.hoon`**: Main blockchain logic (validation, consensus, transaction processing)
- **`dumbnet/miner.hoon`**: Mining proof generation
- **`dumbnet/lib/`**: Core types, consensus algorithms, pending state management
- **`wallet/`**: Wallet functionality

### ZK-STARK Proof System

- **Prover** (`stark/prover.hoon`): Generates proofs with polynomial commitments and FRI protocol
- **Verifier**: Verifies proofs (much faster than generation)
- **Execution Tables**: Captures Nock execution trace for proof generation
- **Constraint System**: Ensures computation correctness

## Performance Bottlenecks (Current Issues)

### Mining Infrastructure
- Creates new kernel + tempdir for EACH mining attempt (~100ms overhead)
- Single-threaded mining (only one attempt at a time)
- No performance metrics/monitoring
- Equix PoW builder not reused efficiently

### Cryptographic Operations
- Missing jets for critical operations:
  - FFT/iFFT (used extensively in STARK generation)
  - Polynomial multiplication
  - Merkle tree construction
  - Batch field operations

### Memory Management
- Tables rebuilt for each proof
- No object pooling
- Excessive copying between noun ↔ native conversions

### Build System Issues
- Rebuilds Hoon compiler on every build
- No incremental compilation
- Assets nuked and rebuilt every time
- No caching mechanisms

## Optimization Opportunities

### 1. Mining Optimizations
- Implement kernel pool to reuse mining kernels
- Enable multi-core parallel mining
- Add performance metrics and monitoring
- Cache hot state and precomputed values

### 2. Cryptographic Jets
- Implement native FFT/iFFT operations
- Add Merkle tree construction jets
- Optimize polynomial arithmetic
- Batch field operations

### 3. Build System Improvements
- Incremental compilation
- Dependency caching
- Parallel builds
- Smart asset management

### 4. Memory Optimizations
- Object pooling for frequent allocations
- Zero-copy operations where possible
- Precomputation caches

## Unique Features

### Nock Virtual Machine
- **Axiomatic**: No dependencies, pure math (12 opcodes vs 1300+ for x86)
- **Homoiconic**: Code and data represented identically
- **Jetted**: Native implementations can replace Nock functions for performance
- **Deterministic**: Same input always produces same output
- **Universally serializable**: Single "jam" format for everything

### Hoon Language
- **Subject-oriented**: All state in the "subject" (data structure)
- **Purely functional**: No side effects, immutable data
- **Type-safe metaprogramming**: Can compile and run code at runtime safely
- **Hot code reload**: Live system updates without restart

### Blockchain Innovation
- **Proof-of-Work + ZK-STARKs**: PoW for consensus, ZK for verification
- **UTXO Model**: "Notes" system for transaction outputs
- **Deterministic Execution**: All computation reproducible
- **Upgradeable**: Live system updates through Hoon compilation

## File Organization

```
nockchain/
├── hoon/                    # Hoon source code
│   ├── apps/
│   │   ├── dumbnet/        # Main blockchain application
│   │   └── wallet/         # Wallet application
│   ├── common/             # Shared libraries
│   │   ├── stark/          # ZK-STARK implementation
│   │   ├── tx-engine.hoon  # Transaction processing
│   │   └── pow.hoon        # Proof-of-work
│   └── constraints/        # Constraint definitions
├── crates/                 # Rust codebase
│   ├── nockchain/          # Main blockchain node
│   ├── nockvm/             # Nock virtual machine
│   ├── zkvm-jetpack/       # Performance jets
│   ├── kernels/            # Compiled Hoon kernels
│   └── hoonc/              # Hoon compiler
└── assets/                 # Compiled Hoon assets
    ├── dumb.jam            # Main blockchain kernel
    ├── miner.jam           # Mining kernel
    └── wal.jam             # Wallet kernel
```

## Development Workflow

1. **Hoon Development**: Write business logic in Hoon
2. **Compilation**: Use `hoonc` to compile Hoon to Nock bytecode
3. **Asset Generation**: Create `.jam` files containing compiled kernels
4. **Rust Integration**: Rust runtime loads and executes Hoon kernels
5. **Testing**: Run integrated tests across the full stack

## Performance Targets (After Optimization)

- **Proof Generation**: < 100ms for typical transactions
- **Mining Rate**: > 1000 attempts/second/core
- **Memory Usage**: < 1GB steady state
- **Build Time**: < 30 seconds for incremental builds

This architecture represents a fundamental reimagining of blockchain technology, prioritizing verifiability over replication and enabling truly heavyweight verifiable applications while maintaining the deterministic guarantees necessary for consensus.