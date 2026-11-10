//! Typed identifiers: UUIDv7 (unique offline, time ordered). A `ProductId`
//! can never be passed where a `CustomerId` is expected.

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
                Self($crate::uuid::Uuid::now_v7())
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
