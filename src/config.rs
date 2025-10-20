use crc32fast::Hasher;

/// Static configuration describing how a SeaORM model integrates with
/// the closure-table hierarchy.
#[derive(Clone, Debug)]
pub struct ClosureTreeConfig {
    entity_name: String,
    hierarchy_name: String,
    parent_column: String,
    name_column: String,
    hierarchy_table: String,
    dependent_behavior: DependentBehavior,
    order_strategy: Option<OrderStrategy>,
    advisory_lock_strategy: AdvisoryLockStrategy,
}

impl ClosureTreeConfig {
    /// Create a new configuration using the logical entity and hierarchy names.
    pub fn new(entity_name: impl Into<String>, hierarchy_name: impl Into<String>) -> Self {
        let entity_name = entity_name.into();
        let hierarchy_name = hierarchy_name.into();

        let default_lock = AdvisoryLockStrategy::Namespaced(AdvisoryLockKey::derived_from(
            &entity_name,
            &hierarchy_name,
        ));

        Self {
            entity_name,
            hierarchy_name,
            parent_column: "parent_id".to_string(),
            name_column: "name".to_string(),
            hierarchy_table: String::new(),
            dependent_behavior: DependentBehavior::default(),
            order_strategy: None,
            advisory_lock_strategy: default_lock,
        }
    }

    /// Merge options produced by [`ClosureTreeOptions`].
    pub(crate) fn apply_options(mut self, options: ClosureTreeOptions) -> Self {
        if let Some(parent_column) = options.parent_column {
            self.parent_column = parent_column;
        }
        if let Some(name_column) = options.name_column {
            self.name_column = name_column;
        }
        if let Some(hierarchy_table) = options.hierarchy_table {
            self.hierarchy_table = hierarchy_table;
        }
        if let Some(behavior) = options.dependent_behavior {
            self.dependent_behavior = behavior;
        }
        if let Some(order_strategy) = options.order_strategy {
            self.order_strategy = Some(order_strategy);
        }
        if let Some(strategy) = options.advisory_lock_strategy {
            self.advisory_lock_strategy = strategy;
        }
        self
    }

    /// Human-readable Rust struct name for the base entity.
    pub fn entity_name(&self) -> &str {
        &self.entity_name
    }

    /// Associated SeaORM entity name for the hierarchy model.
    pub fn hierarchy_name(&self) -> &str {
        &self.hierarchy_name
    }

    /// Column name storing the parent foreign key.
    pub fn parent_column(&self) -> &str {
        &self.parent_column
    }

    /// Column name storing the display name.
    pub fn name_column(&self) -> &str {
        &self.name_column
    }

    /// Table backing the hierarchy entity.
    pub fn hierarchy_table(&self) -> &str {
        &self.hierarchy_table
    }

    /// Dependent behavior when deleting nodes.
    pub fn dependent_behavior(&self) -> DependentBehavior {
        self.dependent_behavior
    }

    /// Ordering strategy to apply when returning descendants.
    pub fn order_strategy(&self) -> Option<&OrderStrategy> {
        self.order_strategy.as_ref()
    }

    /// Advisory lock strategy (PostgreSQL only).
    pub fn advisory_lock_strategy(&self) -> &AdvisoryLockStrategy {
        &self.advisory_lock_strategy
    }
}

/// Builder-style options consumed by the derive macro.
#[derive(Clone, Debug, Default)]
pub struct ClosureTreeOptions {
    parent_column: Option<String>,
    name_column: Option<String>,
    hierarchy_table: Option<String>,
    dependent_behavior: Option<DependentBehavior>,
    order_strategy: Option<OrderStrategy>,
    advisory_lock_strategy: Option<AdvisoryLockStrategy>,
}

impl ClosureTreeOptions {
    pub fn parent_column(mut self, value: impl Into<String>) -> Self {
        self.parent_column = Some(value.into());
        self
    }

    pub fn name_column(mut self, value: impl Into<String>) -> Self {
        self.name_column = Some(value.into());
        self
    }

    pub fn hierarchy_table(mut self, value: impl Into<String>) -> Self {
        self.hierarchy_table = Some(value.into());
        self
    }

    pub fn dependent_behavior(mut self, behavior: DependentBehavior) -> Self {
        self.dependent_behavior = Some(behavior);
        self
    }

    pub fn order_strategy(mut self, strategy: OrderStrategy) -> Self {
        self.order_strategy = Some(strategy);
        self
    }

    pub fn advisory_lock_strategy(mut self, strategy: AdvisoryLockStrategy) -> Self {
        self.advisory_lock_strategy = Some(strategy);
        self
    }

    pub fn apply(self, base: ClosureTreeConfig) -> ClosureTreeConfig {
        base.apply_options(self)
    }
}

/// Behaviour to apply to dependent nodes when destroying a record.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DependentBehavior {
    Nullify,
    Destroy,
    DeleteAll,
    None,
}

impl Default for DependentBehavior {
    fn default() -> Self {
        Self::Nullify
    }
}

/// Strategy used to generate deterministic ordering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrderStrategy {
    Manual,
    NumericColumn { column: String },
}

impl OrderStrategy {
    pub fn numeric_column(column: impl Into<String>) -> Self {
        Self::NumericColumn {
            column: column.into(),
        }
    }
}

/// Key used for PostgreSQL advisory locks.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct AdvisoryLockKey(String);

impl AdvisoryLockKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    fn derived_from(entity: &str, hierarchy: &str) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(entity.as_bytes());
        hasher.update(b"/");
        hasher.update(hierarchy.as_bytes());
        let crc = hasher.finalize();
        Self(format!("closure-tree::{entity}::{hierarchy}::{crc:x}"))
    }
}

/// Configuration describing how to acquire advisory locks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdvisoryLockStrategy {
    Disabled,
    Namespaced(AdvisoryLockKey),
}

impl AdvisoryLockStrategy {
    pub fn key(&self) -> Option<&AdvisoryLockKey> {
        match self {
            AdvisoryLockStrategy::Disabled => None,
            AdvisoryLockStrategy::Namespaced(key) => Some(key),
        }
    }
}
