# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.0.0](https://github.com/bearcove/roam/compare/roam-miri-test-v3.0.0...roam-miri-test-v4.0.0) - 2026-02-21

### Other

- Pass &'static MethodDescriptor through the entire call chain ([#148](https://github.com/bearcove/roam/pull/148))
- Introduce ServiceDescriptor with precomputed RpcPlans ([#147](https://github.com/bearcove/roam/pull/147))
- Use dispatcher method descriptors instead of runtime method-name registry
- scope response recv under peeps stack; fix remaining test call signatures
- Fix cargo-shear: clean up unused/misplaced deps in roam-miri-test and roam-http-bridge
- Load testing improvements and memory safety fixes ([#136](https://github.com/bearcove/roam/pull/136))
