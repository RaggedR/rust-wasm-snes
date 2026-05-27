//! Categorical law tests for the SNES emulator.
//!
//! Tests directed container laws (Ahman-Chapman-Uustalu 2014),
//! container morphism composition, and functor naturality for
//! the Bus address space.
//!
//! Run with: cargo test --test categorical_laws

mod categorical {
    mod directed_container_laws;
    mod distributive_law;
    mod functor_laws;
}
