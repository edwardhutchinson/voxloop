//! Rate limits on the routes that accept a credential.
//!
//! **Limits key on source, never on the submitted account name, with a global ceiling behind
//! them** ([ADR-0025]). Keying on the name would let anyone throttle a named operator by
//! guessing at them, and **there is no auto-lock**: locking on failed attempts is a denial of
//! service aimed squarely at whoever is starting a shift. Account lock survives as a
//! deliberate administrative act; it is simply never pulled by a counter.
//!
//! The source is the peer address of the connection, and it is trustworthy here because
//! VoxLoop terminates TLS itself with no reverse proxy anywhere ([ADR-0040]) — there is no
//! forwarded-for header to be spoofed, and nothing reads one.
//!
//! [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
//! [ADR-0040]: ../../../docs/adr/0040-one-binary-one-unit-four-moving-parts.md

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What one source may spend, and how fast it comes back.
///
/// Ten in hand covers a shift change where somebody mistypes a passphrase a few times; one
/// back every six seconds is far too slow to guess anything with.
const PER_SOURCE: Allowance = Allowance {
    burst: 10.0,
    one_back_every: Duration::from_secs(6),
};

/// The ceiling behind the per-source limits, so a spread of sources cannot cost more than a
/// single one would.
const ACROSS_THE_DEPLOYMENT: Allowance = Allowance {
    burst: 60.0,
    one_back_every: Duration::from_secs(1),
};

/// How many sources are remembered before the quiet ones are forgotten.
///
/// Without a bound, attempts from a spread of addresses would grow this map until the box
/// ran out of memory, which is a cheaper attack than the one the limits exist to stop.
const SOURCES_REMEMBERED: usize = 4_096;

/// An allowance: what may be spent at once, and how quickly it returns.
struct Allowance {
    burst: f64,
    one_back_every: Duration,
}

/// What is left of one allowance.
struct Bucket {
    left: f64,
    counted_at: Instant,
}

impl Bucket {
    fn full(allowance: &Allowance, now: Instant) -> Self {
        Self {
            left: allowance.burst,
            counted_at: now,
        }
    }

    fn is_full(&self, allowance: &Allowance) -> bool {
        self.left >= allowance.burst
    }

    /// Take one, if there is one to take.
    fn take(&mut self, allowance: &Allowance, now: Instant) -> bool {
        let returned = now.saturating_duration_since(self.counted_at).as_secs_f64()
            / allowance.one_back_every.as_secs_f64();
        self.left = (self.left + returned).min(allowance.burst);
        self.counted_at = now;

        if self.left < 1.0 {
            return false;
        }

        self.left -= 1.0;
        true
    }
}

/// Whether an attempt may be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Admission {
    Permitted,
    Throttled,
}

/// The limits on every route that accepts a credential.
pub(super) struct RateLimits {
    per_source: Mutex<HashMap<IpAddr, Bucket>>,
    across_the_deployment: Mutex<Bucket>,
    source_allowance: Allowance,
    deployment_allowance: Allowance,
    sources_remembered: usize,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self::of(PER_SOURCE, ACROSS_THE_DEPLOYMENT, SOURCES_REMEMBERED)
    }
}

impl RateLimits {
    fn of(
        per_source: Allowance,
        across_the_deployment: Allowance,
        sources_remembered: usize,
    ) -> Self {
        Self {
            per_source: Mutex::new(HashMap::new()),
            across_the_deployment: Mutex::new(Bucket::full(&across_the_deployment, Instant::now())),
            source_allowance: per_source,
            deployment_allowance: across_the_deployment,
            sources_remembered,
        }
    }

    /// Whether this source may attempt something now.
    pub(super) fn admit(&self, source: IpAddr) -> Admission {
        self.admit_at(source, Instant::now())
    }

    fn admit_at(&self, source: IpAddr, now: Instant) -> Admission {
        let mut sources = self
            .per_source
            .lock()
            .unwrap_or_else(|held| held.into_inner());

        if sources.len() >= self.sources_remembered && !sources.contains_key(&source) {
            forget_the_quiet_ones(
                &mut sources,
                &self.source_allowance,
                self.sources_remembered,
            );
        }

        let bucket = sources
            .entry(source)
            .or_insert_with(|| Bucket::full(&self.source_allowance, now));

        if !bucket.take(&self.source_allowance, now) {
            return Admission::Throttled;
        }

        drop(sources);

        let mut ceiling = self
            .across_the_deployment
            .lock()
            .unwrap_or_else(|held| held.into_inner());

        if ceiling.take(&self.deployment_allowance, now) {
            Admission::Permitted
        } else {
            Admission::Throttled
        }
    }
}

/// Make room, keeping whatever is still worth knowing.
///
/// A bucket that has refilled says exactly what an absent one says, so those go first. If
/// that is not enough, the least recently seen go too — which hands those sources a fresh
/// allowance, and is the trade this accepts: the deployment-wide ceiling is what holds
/// against a spread of addresses, and an unbounded map would be a cheaper attack than the
/// one being defended against.
fn forget_the_quiet_ones(
    sources: &mut HashMap<IpAddr, Bucket>,
    allowance: &Allowance,
    keep_at_most: usize,
) {
    sources.retain(|_, bucket| !bucket.is_full(allowance));

    if sources.len() < keep_at_most {
        return;
    }

    let mut seen_at: Vec<(IpAddr, Instant)> = sources
        .iter()
        .map(|(source, bucket)| (*source, bucket.counted_at))
        .collect();
    seen_at.sort_unstable_by_key(|(_, at)| *at);

    for (source, _) in seen_at.into_iter().take(sources.len() / 2) {
        sources.remove(&source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_source(last: u8) -> IpAddr {
        IpAddr::from([192, 0, 2, last])
    }

    fn generous() -> Allowance {
        Allowance {
            burst: 1_000.0,
            one_back_every: Duration::from_secs(1),
        }
    }

    fn three_at_a_time() -> Allowance {
        Allowance {
            burst: 3.0,
            one_back_every: Duration::from_secs(10),
        }
    }

    fn limits_of(per_source: Allowance, across_the_deployment: Allowance) -> RateLimits {
        RateLimits::of(per_source, across_the_deployment, SOURCES_REMEMBERED)
    }

    #[test]
    fn admits_a_source_until_it_has_spent_its_allowance() {
        let limits = limits_of(three_at_a_time(), generous());
        let now = Instant::now();

        for attempt in 1..=3 {
            assert_eq!(
                limits.admit_at(a_source(1), now),
                Admission::Permitted,
                "attempt {attempt} was throttled"
            );
        }

        assert_eq!(limits.admit_at(a_source(1), now), Admission::Throttled);
    }

    #[test]
    fn throttles_one_source_without_throttling_another() {
        let limits = limits_of(three_at_a_time(), generous());
        let now = Instant::now();
        for _ in 0..3 {
            limits.admit_at(a_source(1), now);
        }

        assert_eq!(limits.admit_at(a_source(1), now), Admission::Throttled);
        assert_eq!(limits.admit_at(a_source(2), now), Admission::Permitted);
    }

    #[test]
    fn lets_a_throttled_source_back_in_as_its_allowance_returns() {
        let limits = limits_of(three_at_a_time(), generous());
        let now = Instant::now();
        for _ in 0..3 {
            limits.admit_at(a_source(1), now);
        }

        assert_eq!(limits.admit_at(a_source(1), now), Admission::Throttled);
        assert_eq!(
            limits.admit_at(a_source(1), now + Duration::from_secs(10)),
            Admission::Permitted
        );
    }

    #[test]
    fn the_ceiling_holds_however_many_sources_are_trying() {
        let limits = limits_of(generous(), three_at_a_time());
        let now = Instant::now();

        for source in 1..=3 {
            assert_eq!(limits.admit_at(a_source(source), now), Admission::Permitted);
        }

        assert_eq!(limits.admit_at(a_source(4), now), Admission::Throttled);
    }

    #[test]
    fn remembers_no_more_sources_than_it_said_it_would() {
        let limits = RateLimits::of(three_at_a_time(), generous(), 8);
        let now = Instant::now();

        for source in 0..=255 {
            limits.admit_at(a_source(source), now);
        }

        assert!(
            limits.per_source.lock().expect("the map").len() <= 8,
            "the map grew past what it promised to hold"
        );
    }

    #[test]
    fn forgets_a_source_that_has_stopped_trying_before_one_that_has_not() {
        let limits = RateLimits::of(three_at_a_time(), generous(), 2);
        let now = Instant::now();
        limits.admit_at(a_source(1), now);
        let much_later = now + Duration::from_secs(600);

        // The quiet source has refilled and says nothing; the busy one is out of allowance.
        for _ in 0..3 {
            limits.admit_at(a_source(2), much_later);
        }
        limits.admit_at(a_source(3), much_later);

        assert_eq!(
            limits.admit_at(a_source(2), much_later),
            Admission::Throttled,
            "the source that had spent its allowance was forgotten first"
        );
    }
}
