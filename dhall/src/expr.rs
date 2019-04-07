use crate::imports::ImportRoot;
use dhall_core::*;

macro_rules! derive_other_traits {
    ($ty:ident) => {
        impl std::cmp::PartialEq for $ty {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl std::fmt::Display for $ty {
            fn fmt(
                &self,
                f: &mut std::fmt::Formatter,
            ) -> Result<(), std::fmt::Error> {
                self.0.fmt(f)
            }
        }
    };
}

#[derive(Debug, Clone, Eq)]
pub struct Parsed(pub(crate) SubExpr<X, Import>, pub(crate) ImportRoot);
derive_other_traits!(Parsed);

#[derive(Debug, Clone, Eq)]
pub struct Resolved(pub(crate) SubExpr<X, X>);
derive_other_traits!(Resolved);

#[derive(Debug, Clone, Eq)]
pub struct Typed(pub(crate) SubExpr<X, X>, pub(crate) Type);
derive_other_traits!(Typed);

#[derive(Debug, Clone, Eq)]
pub struct Normalized(pub(crate) SubExpr<X, X>, pub(crate) Type);
derive_other_traits!(Normalized);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type(pub(crate) TypeInternal);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeInternal {
    Expr(Box<Normalized>),
    Untyped,
}
