//! Three-rank role type. No I/O, no framework, no crate dependencies.
//!
//! Copy this file into another project, or depend on `gw-role`. Persistence,
//! HTTP and password hashing stay in the host app.
//!
//! Stored strings (the only values that belong in a `role` column):
//! `user` · `admin` · `super_admin`.

use core::fmt;

/// The three ranks this algebra knows. Unknown stored strings are not staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    User,
    Admin,
    SuperAdmin,
}

impl Role {
    pub const ALL: [Self; 3] = [Self::User, Self::Admin, Self::SuperAdmin];

    /// Parse a stored or requested role. Empty / unknown is an error — use
    /// [`Role::from_stored`] when reading a column that must never fail.
    pub fn parse(raw: &str) -> Result<Self, ParseError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "user" => Ok(Self::User),
            "admin" => Ok(Self::Admin),
            "super_admin" => Ok(Self::SuperAdmin),
            "" => Err(ParseError::Empty),
            _ => Err(ParseError::Unknown),
        }
    }

    /// Column → rank. Empty and unknown collapse to [`Role::User`] so they
    /// never pass a staff gate.
    #[must_use]
    pub fn from_stored(raw: &str) -> Self {
        match Self::parse(raw) {
            Ok(role) => role,
            Err(ParseError::Empty | ParseError::Unknown) => Self::User,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Admin => "admin",
            Self::SuperAdmin => "super_admin",
        }
    }

    #[must_use]
    pub const fn is_staff(self) -> bool {
        matches!(self, Self::Admin | Self::SuperAdmin)
    }

    #[must_use]
    pub const fn is_super_admin(self) -> bool {
        matches!(self, Self::SuperAdmin)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why [`Role::parse`] rejected the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_strings_round_trip() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Ok(role));
            assert_eq!(Role::from_stored(role.as_str()), role);
        }
    }

    #[test]
    fn parse_ignores_case_and_surrounding_space() {
        assert_eq!(Role::parse(" Super_Admin "), Ok(Role::SuperAdmin));
        assert_eq!(Role::parse("ADMIN"), Ok(Role::Admin));
    }

    #[test]
    fn unknown_and_empty_stored_values_are_not_staff() {
        for raw in ["", "   ", "superadmin", "root", "administrator"] {
            let role = Role::from_stored(raw);
            assert!(!role.is_staff(), "{raw:?} must not be staff");
            assert_eq!(role, Role::User);
        }
    }

    #[test]
    fn staff_is_admin_or_super_admin() {
        assert!(!Role::User.is_staff());
        assert!(Role::Admin.is_staff());
        assert!(Role::SuperAdmin.is_staff());
        assert!(Role::SuperAdmin.is_super_admin());
        assert!(!Role::Admin.is_super_admin());
    }
}
