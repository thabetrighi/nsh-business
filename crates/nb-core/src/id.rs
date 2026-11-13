//! Typed identifiers: UUIDv7 (unique offline, time ordered). A `ProductId`
//! can never be passed where a `CustomerId` is expected.

/// 16 bytes from the system's secure random source (122 random bits; the
/// browser's Web Crypto API on the web), e.g. for password salts.
pub fn random_bytes() -> [u8; 16] {
    uuid::Uuid::new_v4().into_bytes()
}

/// Identifiers made inside an operation (M11/M0, ADR-0006): while
/// [`deterministic`] runs, every new identifier is derived from the
/// operation's id and time, so running the same operation again (on the
/// server, or on another replica) makes exactly the same rows. Outside it,
/// identifiers are ordinary random UUIDv7.
pub mod idgen {
    use std::cell::Cell;

    use uuid::Uuid;

    #[derive(Clone, Copy)]
    struct Gen {
        ms: u64,
        seed: u64,
        n: u64,
    }

    thread_local! {
        static GEN: Cell<Option<Gen>> = const { Cell::new(None) };
    }

    /// SplitMix64: a fixed, well-spread mix (not cryptographic; the seed
    /// comes from a random operation id).
    fn mix(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A new identifier: derived inside [`deterministic`], random outside.
    pub fn uuid() -> Uuid {
        GEN.with(|g| match g.get() {
            None => Uuid::now_v7(),
            Some(mut s) => {
                s.n += 1;
                g.set(Some(s));
                // 12 bits of order within the operation, then 64 bits mixed
                // from the seed and the counter (unique across operations).
                let n12 = (s.n & 0x0FFF) as u16;
                let tail = mix(s.seed ^ mix(s.n)).to_be_bytes();
                let mut r = [0u8; 10];
                r[..2].copy_from_slice(&n12.to_be_bytes());
                r[2..].copy_from_slice(&tail);
                uuid::Builder::from_unix_timestamp_millis(s.ms, &r).into_uuid()
            }
        })
    }

    /// Restores the previous generator when dropped (also on panic).
    struct Restore(Option<Gen>);

    impl Drop for Restore {
        fn drop(&mut self) {
            GEN.with(|g| g.set(self.0));
        }
    }

    /// Runs `f` with identifiers derived from `op` (an operation id) and its
    /// time `at_ms`.
    pub fn deterministic<R>(op: Uuid, at_ms: i64, f: impl FnOnce() -> R) -> R {
        let (hi, lo) = op.as_u64_pair();
        let state = Gen {
            ms: u64::try_from(at_ms).unwrap_or(0),
            seed: mix(hi) ^ lo,
            n: 0,
        };
        let _restore = Restore(GEN.with(|g| g.replace(Some(state))));
        f()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_same_operation_makes_the_same_ids() {
            let op = Uuid::now_v7();
            let a: Vec<Uuid> =
                deterministic(op, 1_700_000_000_000, || (0..5).map(|_| uuid()).collect());
            let b: Vec<Uuid> =
                deterministic(op, 1_700_000_000_000, || (0..5).map(|_| uuid()).collect());
            assert_eq!(a, b);
            // Time ordered within the operation, all distinct.
            assert!(a.windows(2).all(|w| w[0] < w[1]));
            let other: Vec<Uuid> = deterministic(Uuid::now_v7(), 1_700_000_000_000, || {
                (0..5).map(|_| uuid()).collect()
            });
            assert!(a.iter().all(|x| !other.contains(x)));
            assert_eq!(a[0].get_version_num(), 7);
            // Outside: random again.
            assert_ne!(uuid(), uuid());
            // Nested: the outer generator comes back.
            let (x, y) = deterministic(op, 1, || {
                let x = uuid();
                deterministic(Uuid::now_v7(), 2, uuid);
                (x, uuid())
            });
            assert!(x < y);
        }
    }
}

/// Declares a typed UUIDv7 identifier.
#[macro_export]
macro_rules! typed_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($crate::uuid::Uuid);

        impl $name {
            /// A new, time-ordered identifier.
            pub fn new() -> Self {
                Self($crate::id::idgen::uuid())
            }

            pub fn from_uuid(u: $crate::uuid::Uuid) -> Self {
                Self(u)
            }

            pub fn uuid(&self) -> $crate::uuid::Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl $crate::serde::Serialize for $name {
            fn serialize<S: $crate::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(&self.0)
            }
        }

        impl<'de> $crate::serde::Deserialize<'de> for $name {
            fn deserialize<D: $crate::serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <::std::string::String as $crate::serde::Deserialize>::deserialize(d)?;
                s.parse().map_err(<D::Error as $crate::serde::de::Error>::custom)
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::CoreError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                $crate::uuid::Uuid::parse_str(s).map(Self).map_err(|_| $crate::CoreError::InvalidId(s.to_owned()))
            }
        }
    };
}

typed_id!(
    /// Identifies one business operation for idempotency: the same id applied twice has one effect.
    OperationId
);
typed_id!(
    /// Identifies a registered device (POS terminal, phone, browser).
    DeviceId
);
typed_id!(
    /// Identifies an organization (tenant).
    OrganizationId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_ordered_and_parse() -> Result<(), crate::CoreError> {
        let a = OperationId::new();
        let b = OperationId::new();
        assert_ne!(a, b);
        assert!(a <= b, "v7 ids are time ordered");
        assert_eq!(a.to_string().parse::<OperationId>()?, a);
        assert!("not-a-uuid".parse::<OperationId>().is_err());
        Ok(())
    }
}
