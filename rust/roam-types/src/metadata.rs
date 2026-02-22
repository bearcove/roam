use facet::Facet;

/// Metadata value.
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub enum MetadataValue {
    String(String) = 0,
    Bytes(Vec<u8>) = 1,
    U64(u64) = 2,
}

/// Metadata entry flags.
///
/// Flags control metadata handling behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct MetadataFlags(u64);

impl MetadataFlags {
    /// No special handling.
    pub const NONE: Self = Self(0);

    /// Value MUST NOT be logged, traced, or included in error messages.
    pub const SENSITIVE: Self = Self(1 << 0);

    /// Value MUST NOT be forwarded to downstream calls.
    pub const NO_PROPAGATE: Self = Self(1 << 1);

    /// Returns `true` if all flags in `other` are set in `self`.
    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl std::ops::BitOr for MetadataFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for MetadataFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAnd for MetadataFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::BitAndAssign for MetadataFlags {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

/// Metadata entry: (key, value, flags).
pub type Metadata = Vec<(String, MetadataValue, MetadataFlags)>;
