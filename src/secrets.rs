//! Where every unguessable value in VoxLoop comes from, and how one is compared.
//!
//! Internal user ids, sign-in tokens and the bootstrap code are all *opaque*: their entire
//! job is to be impossible to guess and to mean nothing to whoever holds them. One place
//! mints them so that none of them is quietly weaker than the others.
//!
//! This is a facility rather than a module in the [`docs/spec/modules.md`] sense, like
//! [`crate::telemetry`]: it has no domain interface and several modules write through it.
//!
//! [`docs/spec/modules.md`]: ../../docs/spec/modules.md

use std::fmt::Write;

use sha2::{Digest, Sha256};

/// How much randomness an opaque value carries. 128 bits is past guessing and short enough
/// to be read off a log and typed into a browser.
const BITS: usize = 128;

/// Mint a value nobody can guess, rendered as lower-case hexadecimal.
///
/// The operating system is the only source of randomness here. A seeded generator that looks
/// random is the failure this function exists to make impossible to reach for.
pub(crate) fn unguessable() -> String {
    let mut bytes = [0_u8; BITS / 8];
    getrandom::fill(&mut bytes).expect("the operating system to have randomness");
    as_hexadecimal(&bytes)
}

/// A one-way fingerprint of a secret, for storing beside the thing it protects.
///
/// A sign-in token is stored as its fingerprint rather than itself: the store is one file a
/// deployment is obliged to back up, and a backup should not be a drawer full of usable
/// sign-ins. This is not password hashing and must never be used for one — a password is
/// guessable by construction and needs Argon2id ([ADR-0025]).
///
/// [ADR-0025]: ../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
pub(crate) fn fingerprint(secret: &str) -> String {
    as_hexadecimal(&Sha256::digest(secret.as_bytes()))
}

/// Render bytes the one way everything here renders them.
fn as_hexadecimal(bytes: &[u8]) -> String {
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut rendered, byte| {
            let _ = write!(rendered, "{byte:02x}");
            rendered
        },
    )
}

/// Compare two secrets in time that does not depend on how far they matched.
///
/// The bootstrap code is compared with this. Rate limiting is the defence against guessing
/// it; this is the defence against being told, one request at a time, how close a guess was.
pub(crate) fn are_the_same(presented: &str, expected: &str) -> bool {
    if presented.len() != expected.len() {
        return false;
    }

    presented
        .bytes()
        .zip(expected.bytes())
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mints_a_different_value_every_time() {
        let minted: std::collections::HashSet<String> = (0..64).map(|_| unguessable()).collect();

        assert_eq!(minted.len(), 64, "expected 64 distinct values");
    }

    #[test]
    fn mints_the_full_width_it_promises() {
        assert_eq!(unguessable().len(), BITS / 4);
    }

    #[test]
    fn fingerprints_the_same_secret_the_same_way_and_a_different_one_differently() {
        assert_eq!(fingerprint("a secret"), fingerprint("a secret"));
        assert_ne!(fingerprint("a secret"), fingerprint("a secrat"));
    }

    #[test]
    fn a_fingerprint_does_not_carry_the_secret() {
        assert!(!fingerprint("a secret").contains("a secret"));
    }

    #[test]
    fn compares_equal_secrets_as_equal_and_unequal_ones_as_unequal() {
        assert!(are_the_same("abc", "abc"));
        assert!(!are_the_same("abc", "abd"));
        assert!(!are_the_same("abc", "abcd"));
        assert!(!are_the_same("", "a"));
    }
}
