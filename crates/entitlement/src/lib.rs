#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Which paid export features a build may use.
//!
//! The product's rule is one sentence: **free is everything on your machine, Pro is anything that
//! leaves it**. Export of print-quality artifacts is the Pro half. This crate is the single place
//! that answers "is this build entitled to that feature?", so the desktop application and the CLI
//! enforce the same answer; neither may invent its own.
//!
//! Signed tokens and their offline grace period arrive with S03. Until then an [`Entitlement`] is
//! constructed as free or Pro directly. Every caller already takes one, so adding token
//! verification later will not change a single call site.

/// The plan a build is running under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// The free tier: everything local, with a watermarked 1× preview.
    Free,
    /// The paid tier: print-quality and other artifacts that leave the machine.
    Pro,
}

/// A paid capability the free tier does not grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    /// A print-quality PNG at 2–4×.
    PrintPng,
    /// A vector SVG file.
    Svg,
    /// A paginated PDF.
    Pdf,
    /// A self-contained HTML bundle.
    Html,
}

impl Feature {
    /// A short name for messages.
    pub fn name(self) -> &'static str {
        match self {
            Feature::PrintPng => "print-quality PNG export",
            Feature::Svg => "SVG export",
            Feature::Pdf => "PDF export",
            Feature::Html => "HTML export",
        }
    }
}

/// What this build may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entitlement {
    plan: Plan,
}

impl Entitlement {
    /// The free tier.
    pub fn free() -> Self {
        Entitlement { plan: Plan::Free }
    }

    /// A Pro build, as a verified token would grant.
    pub fn pro() -> Self {
        Entitlement { plan: Plan::Pro }
    }

    /// The plan this entitlement carries.
    pub fn plan(self) -> Plan {
        self.plan
    }

    /// Checks that a paid feature is available.
    ///
    /// # Errors
    ///
    /// Returns [`EntitlementError::RequiresPro`] on the free tier.
    pub fn require(self, feature: Feature) -> Result<(), EntitlementError> {
        match self.plan {
            Plan::Pro => Ok(()),
            Plan::Free => Err(EntitlementError::RequiresPro { feature }),
        }
    }
}

/// Why an entitlement check failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EntitlementError {
    /// The feature needs a Pro plan and this build is free.
    #[error("{feature} needs SlickBI Pro", feature = .feature.name())]
    RequiresPro {
        /// The feature that was refused.
        feature: Feature,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_free_tier_refuses_print_export() {
        let error = Entitlement::free()
            .require(Feature::PrintPng)
            .expect_err("free must refuse");
        assert_eq!(
            error,
            EntitlementError::RequiresPro {
                feature: Feature::PrintPng
            }
        );
        assert!(error.to_string().contains("Pro"));
        assert!(error.to_string().contains("print-quality"));
    }

    #[test]
    fn pro_allows_every_paid_feature() {
        let pro = Entitlement::pro();
        assert_eq!(pro.plan(), Plan::Pro);
        for feature in [Feature::PrintPng, Feature::Svg, Feature::Pdf, Feature::Html] {
            assert!(
                pro.require(feature).is_ok(),
                "{feature:?} should be allowed"
            );
        }
    }

    #[test]
    fn free_reports_its_plan() {
        assert_eq!(Entitlement::free().plan(), Plan::Free);
    }
}
