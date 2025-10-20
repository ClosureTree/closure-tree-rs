//! SeaORM-centric closure tree implementation.
//!
//! This crate mirrors the behaviour of the Ruby `closure_tree` gem while exposing
//! an asynchronous API that composes with SeaORM entities. At this stage the
//! implementation focuses on PostgreSQL support; the public API is kept backend
//! agnostic so MySQL can follow.

pub mod config;
pub mod error;
pub mod lock;
pub mod repository;
pub mod traits;

pub mod prelude {
    //! Convenient re-exports for consumers.
    pub use crate::config::{
        AdvisoryLockStrategy, ClosureTreeConfig, ClosureTreeOptions, DependentBehavior,
        OrderStrategy,
    };
    pub use crate::traits::ClosureTreeModel;
}

pub use closure_tree_macros::ClosureTreeModel as ClosureTreeModelDerive;
#[doc(hidden)]
pub use closure_tree_macros::ClosureTreeModel;
pub use config::{
    AdvisoryLockKey, AdvisoryLockStrategy, ClosureTreeConfig, ClosureTreeOptions,
    DependentBehavior, OrderStrategy,
};
pub use error::ClosureTreeError;
pub use repository::ClosureTreeRepository;
pub use traits::ClosureTreeModel;
