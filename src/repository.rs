use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;

use sea_orm::{
    entity::prelude::*, ColumnTrait, Condition, ConnectionTrait, DbBackend, EntityTrait,
    QueryFilter, QueryOrder, Statement,
};

use sea_orm::sea_query::{Expr, ExprTrait, Value};

use crate::config::{
    BulkInsertOptions, BulkInsertResult, ClosureTreeConfig, InsertNode, OrderStrategy, ParentRef,
};
use crate::error::ClosureTreeError;
use crate::lock::LockedTransaction;
use crate::traits::ClosureTreeModel;

/// Repository exposing the higher-level closure-tree operations for a given model.
#[derive(Debug, Default)]
pub struct ClosureTreeRepository<M>
where
    M: ClosureTreeModel,
{
    _marker: PhantomData<M>,
}

impl<M> ClosureTreeRepository<M>
where
    M: ClosureTreeModel,
{
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }

    fn config(&self) -> &'static ClosureTreeConfig {
        M::closure_tree_config()
    }

    fn ensure_postgres(conn: &impl ConnectionTrait) -> Result<(), ClosureTreeError> {
        if conn.get_database_backend() == DbBackend::Postgres {
            Ok(())
        } else {
            Err(ClosureTreeError::UnsupportedBackend)
        }
    }

    pub async fn parent(
        &self,
        conn: &DatabaseConnection,
        model: &M,
    ) -> Result<Option<M>, ClosureTreeError> {
        Self::ensure_postgres(conn)?;
        match model.parent_id() {
            Some(parent_id) => {
                let parent = M::Entity::find()
                    .filter(M::id_column().eq(M::id_to_value(&parent_id)))
                    .one(conn)
                    .await?;
                Ok(parent)
            }
            None => Ok(None),
        }
    }

    pub async fn children(
        &self,
        conn: &DatabaseConnection,
        model: &M,
    ) -> Result<Vec<M>, ClosureTreeError> {
        Self::ensure_postgres(conn)?;
        let id = model.id();
        let parent_value = M::id_to_value(&id);
        let mut query = M::Entity::find().filter(M::parent_column().eq(parent_value));
        if let Some(OrderStrategy::NumericColumn { column }) = self.config().order_strategy() {
            query = query.order_by_asc(Expr::cust(column.clone()));
        }
        query = query.order_by_asc(M::name_column());
        let rows = query.all(conn).await?;
        Ok(rows)
    }

    pub async fn roots(&self, conn: &DatabaseConnection) -> Result<Vec<M>, ClosureTreeError> {
        Self::ensure_postgres(conn)?;
        let rows = M::Entity::find()
            .filter(M::parent_column().is_null())
            .order_by_asc(M::name_column())
            .all(conn)
            .await?;
        Ok(rows)
    }

    pub async fn descendants(
        &self,
        conn: &DatabaseConnection,
        model: &M,
    ) -> Result<Vec<M>, ClosureTreeError> {
        Self::ensure_postgres(conn)?;
        let rows = self.descendants_with_conn(conn, &model.id(), true).await?;
        Ok(rows)
    }

    pub async fn self_and_descendants(
        &self,
        conn: &DatabaseConnection,
        model: &M,
    ) -> Result<Vec<M>, ClosureTreeError> {
        Self::ensure_postgres(conn)?;
        let mut nodes = Vec::with_capacity(1);
        nodes.push(model.clone());
        let mut descendants = self.descendants_with_conn(conn, &model.id(), true).await?;
        nodes.append(&mut descendants);
        Ok(nodes)
    }

    pub async fn find_by_path<S: AsRef<str>>(
        &self,
        conn: &DatabaseConnection,
        segments: &[S],
    ) -> Result<Option<M>, ClosureTreeError> {
        Self::ensure_postgres(conn)?;
        self.find_by_path_on(conn, segments).await
    }

    pub async fn find_or_create_by_path<S: AsRef<str>>(
        &self,
        conn: &DatabaseConnection,
        segments: &[S],
    ) -> Result<M, ClosureTreeError> {
        Self::ensure_postgres(conn)?;

        if segments.is_empty() {
            return Err(ClosureTreeError::EmptyPath);
        }

        let strategy = self.config().advisory_lock_strategy().clone();
        let guard = LockedTransaction::acquire(&strategy, conn).await?;
        self.find_or_create_with_guard(guard, segments).await
    }

    async fn find_or_create_with_guard<S: AsRef<str>>(
        &self,
        guard: LockedTransaction,
        segments: &[S],
    ) -> Result<M, ClosureTreeError> {
        let result = self
            .find_or_create_by_path_on(guard.connection(), segments)
            .await;

        match result {
            Ok(model) => {
                guard.commit().await?;
                Ok(model)
            }
            Err(err) => {
                let _ = guard.rollback().await;
                Err(err)
            }
        }
    }

    async fn find_by_path_on<S: AsRef<str>, C: ConnectionTrait>(
        &self,
        conn: &C,
        segments: &[S],
    ) -> Result<Option<M>, ClosureTreeError> {
        if segments.is_empty() {
            return Ok(None);
        }

        let mut current_parent: Option<M::Id> = None;
        let mut current: Option<M> = None;

        for segment in segments {
            let name = segment.as_ref();
            let node = self
                .find_child_by_name(conn, current_parent.as_ref(), name)
                .await?;

            match node {
                Some(model) => {
                    current_parent = Some(model.id());
                    current = Some(model);
                }
                None => return Ok(None),
            }
        }

        Ok(current)
    }

    async fn find_or_create_by_path_on<S: AsRef<str>, C: ConnectionTrait>(
        &self,
        conn: &C,
        segments: &[S],
    ) -> Result<M, ClosureTreeError> {
        let mut current_parent: Option<M::Id> = None;
        let mut current: Option<M> = None;

        for segment in segments {
            let name = segment.as_ref();
            match self
                .find_child_by_name(conn, current_parent.as_ref(), name)
                .await?
            {
                Some(model) => {
                    current_parent = Some(model.id());
                    current = Some(model);
                }
                None => {
                    let created = self
                        .insert_child(conn, current_parent.as_ref(), name)
                        .await?;
                    current_parent = Some(created.id());
                    current = Some(created);
                }
            }
        }

        current.ok_or_else(|| ClosureTreeError::invariant("path segments produced no model"))
    }

    async fn insert_child<C: ConnectionTrait>(
        &self,
        conn: &C,
        parent_id: Option<&M::Id>,
        name: &str,
    ) -> Result<M, ClosureTreeError> {
        let mut active = M::ActiveModel::default();
        M::set_parent(&mut active, parent_id.cloned());
        M::set_name(&mut active, name);

        let model = active.insert(conn).await?;
        self.insert_hierarchy_rows(conn, &model, parent_id).await?;
        Ok(model)
    }

    async fn insert_hierarchy_rows<C: ConnectionTrait>(
        &self,
        conn: &C,
        model: &M,
        parent_id: Option<&M::Id>,
    ) -> Result<(), ClosureTreeError> {
        let mut rows = Vec::new();
        let model_id = model.id();

        rows.push(M::hierarchy_build_row(
            model_id.clone(),
            model_id.clone(),
            0,
        ));

        if let Some(parent_id) = parent_id {
            let ancestors = M::HierarchyEntity::find()
                .filter(M::hierarchy_descendant_column().eq(M::hierarchy_id_to_value(parent_id)))
                .all(conn)
                .await?;

            for ancestor in ancestors {
                let ancestor_id = M::hierarchy_model_ancestor(&ancestor);
                let generations = M::hierarchy_model_generations(&ancestor) + 1;
                rows.push(M::hierarchy_build_row(
                    ancestor_id,
                    model_id.clone(),
                    generations,
                ));
            }
        }

        M::HierarchyEntity::insert_many(rows).exec(conn).await?;
        Ok(())
    }

    async fn find_child_by_name<C: ConnectionTrait>(
        &self,
        conn: &C,
        parent_id: Option<&M::Id>,
        name: &str,
    ) -> Result<Option<M>, ClosureTreeError> {
        let mut condition = Condition::all().add(M::name_column().eq(name));

        if let Some(parent_id) = parent_id {
            condition = condition.add(M::parent_column().eq(M::id_to_value(parent_id)));
        } else {
            condition = condition.add(M::parent_column().is_null());
        }

        let model = M::Entity::find().filter(condition).one(conn).await?;
        Ok(model)
    }

    async fn descendants_with_conn<C: ConnectionTrait>(
        &self,
        conn: &C,
        ancestor_id: &M::Id,
        exclude_root: bool,
    ) -> Result<Vec<M>, ClosureTreeError> {
        let mut query = M::HierarchyEntity::find()
            .filter(M::hierarchy_ancestor_column().eq(M::hierarchy_id_to_value(ancestor_id)));

        if exclude_root {
            query = query.filter(M::hierarchy_generations_column().gt(0));
        }

        let rows = query.all(conn).await?;

        let mut descendant_ids = Vec::with_capacity(rows.len());
        for hierarchy in rows {
            let descendant = M::hierarchy_model_descendant(&hierarchy);
            descendant_ids.push(descendant);
        }

        if descendant_ids.is_empty() {
            return Ok(Vec::new());
        }

        let values = descendant_ids
            .iter()
            .map(|id| M::id_to_value(id))
            .collect::<Vec<_>>();

        let mut query = M::Entity::find().filter(M::id_column().is_in(values));
        if let Some(OrderStrategy::NumericColumn { column }) = self.config().order_strategy() {
            query = query.order_by_asc(Expr::cust(column.clone()));
        }
        query = query.order_by_asc(M::name_column());

        let models = query.all(conn).await?;
        Ok(models)
    }

    /// Topologically sort nodes into waves based on InBatch dependencies.
    /// Returns Vec<Vec<usize>> where each inner Vec is a wave of node indices
    /// that can be inserted in parallel.
    fn topological_sort_nodes(
        nodes: &[InsertNode<M>],
    ) -> Result<Vec<Vec<usize>>, ClosureTreeError> {
        let n = nodes.len();
        let mut in_degree = vec![0; n];
        let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];

        // Build dependency graph
        for (idx, node) in nodes.iter().enumerate() {
            if let Some(ParentRef::InBatch(parent_idx)) = &node.parent_ref {
                if *parent_idx >= n {
                    return Err(ClosureTreeError::InvalidBatchIndex(*parent_idx));
                }
                adjacency[*parent_idx].push(idx);
                in_degree[idx] += 1;
            }
        }

        // Kahn's algorithm
        let mut waves = Vec::new();
        let mut queue: VecDeque<usize> = in_degree
            .iter()
            .enumerate()
            .filter(|(_, &deg)| deg == 0)
            .map(|(idx, _)| idx)
            .collect();

        let mut processed = 0;
        while !queue.is_empty() {
            let wave_size = queue.len();
            let mut wave = Vec::with_capacity(wave_size);

            for _ in 0..wave_size {
                if let Some(node_idx) = queue.pop_front() {
                    wave.push(node_idx);
                    processed += 1;

                    for &child_idx in &adjacency[node_idx] {
                        in_degree[child_idx] -= 1;
                        if in_degree[child_idx] == 0 {
                            queue.push_back(child_idx);
                        }
                    }
                }
            }

            if !wave.is_empty() {
                waves.push(wave);
            }
        }

        // Check for cycles
        if processed != n {
            let cycle_node = in_degree
                .iter()
                .position(|&deg| deg > 0)
                .unwrap_or(0);
            return Err(ClosureTreeError::CycleDetected(cycle_node));
        }

        Ok(waves)
    }

    /// Resolve external keys to internal IDs by querying the database.
    ///
    /// **TODO**: Currently returns empty map. Needs trait method to extract
    /// conflict column value from models:
    ///
    /// ```ignore
    /// trait ClosureTreeModel {
    ///     fn get_column_value(&self, column: &str) -> Option<Value>;
    /// }
    /// ```
    ///
    /// Then implementation would be:
    /// ```ignore
    /// let mut map = HashMap::new();
    /// for model in models {
    ///     if let Some(key_val) = model.get_column_value(conflict_column) {
    ///         map.insert(format!("{:?}", key_val), model.id());
    ///     }
    /// }
    /// ```
    async fn resolve_external_keys<C: ConnectionTrait>(
        &self,
        conn: &C,
        external_keys: Vec<Value>,
        conflict_column: &str,
    ) -> Result<HashMap<String, M::Id>, ClosureTreeError> {
        if external_keys.is_empty() {
            return Ok(HashMap::new());
        }

        // Query nodes by conflict column
        // Note: Using Expr::cust for dynamic column name
        let column_expr = Expr::cust(conflict_column);
        let _models = M::Entity::find()
            .filter(column_expr.is_in(external_keys))
            .all(conn)
            .await?;

        // Temporary stub - needs trait method support
        Ok(HashMap::new())
    }

    /// Build and execute bulk INSERT with ON CONFLICT, returning inserted models.
    ///
    /// Uses raw SQL for true bulk insert with batched VALUES and RETURNING.
    async fn bulk_insert_wave<C: ConnectionTrait>(
        &self,
        conn: &C,
        wave_nodes: Vec<(usize, Option<M::Id>, String)>, // (idx, parent_id, name)
        options: &BulkInsertOptions,
    ) -> Result<(Vec<M>, Vec<usize>), ClosureTreeError> {
        if wave_nodes.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        let config = self.config();
        let table_name = M::Entity::default().table_name().to_string();

        // Build VALUES placeholders and collect parameter values
        let mut values = Vec::new();
        let mut placeholders = Vec::new();
        let mut original_indices = Vec::new();

        for (idx, (original_idx, parent_id, name)) in wave_nodes.iter().enumerate() {
            let parent_val = match parent_id {
                Some(id) => M::id_to_value(id),
                None => Value::Int(None),
            };
            let name_val = Value::String(Some(Box::new(name.clone())));

            let param_offset = idx * 2 + 1;
            placeholders.push(format!("(${}, ${})", param_offset, param_offset + 1));
            values.push(parent_val);
            values.push(name_val);
            original_indices.push(*original_idx);
        }

        // Build ON CONFLICT clause
        let conflict_clause = match &options.conflict_strategy {
            crate::config::ConflictStrategy::Skip => {
                format!("ON CONFLICT ({}) DO NOTHING", options.conflict_column)
            }
            crate::config::ConflictStrategy::Update(cols) => {
                let updates = cols
                    .iter()
                    .map(|col| format!("{} = EXCLUDED.{}", col, col))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "ON CONFLICT ({}) DO UPDATE SET {}",
                    options.conflict_column, updates
                )
            }
        };

        // Build full SQL statement
        let sql = format!(
            "INSERT INTO {} ({}, {}) VALUES {} {} RETURNING *",
            table_name,
            config.parent_column(),
            config.name_column(),
            placeholders.join(", "),
            conflict_clause
        );

        // Execute and deserialize
        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, &sql, values);
        let query_results = conn.query_all(stmt).await?;

        let mut inserted_models = Vec::new();
        let mut inserted_indices = Vec::new();

        for (i, row) in query_results.iter().enumerate() {
            let model = M::from_query_result(row, "")?;
            inserted_models.push(model);
            if i < original_indices.len() {
                inserted_indices.push(original_indices[i]);
            }
        }

        Ok((inserted_models, inserted_indices))
    }

    /// Bulk insert nodes with parent references and hierarchy generation.
    ///
    /// Optimized for importing large hierarchical datasets (e.g., 23K messages).
    ///
    /// # Algorithm
    /// 1. Topologically sort nodes by `InBatch` dependencies into "waves"
    /// 2. Resolve `ExternalKey` parents (TODO: currently stubbed)
    /// 3. Insert each wave with parent_id set from resolved references
    /// 4. Generate hierarchy rows for all inserted nodes
    ///
    /// # Performance
    /// - **Current**: ~3N queries for N nodes (insert + hierarchy per node)
    /// - **Target**: ~10 queries total via batched INSERT + batched hierarchy
    ///
    /// # Example
    /// ```ignore
    /// let nodes = vec![
    ///     InsertNode {
    ///         model: ActiveModel { name: "root".into(), .. },
    ///         parent_ref: None,
    ///     },
    ///     InsertNode {
    ///         model: ActiveModel { name: "child".into(), .. },
    ///         parent_ref: Some(ParentRef::InBatch(0)),
    ///     },
    /// ];
    ///
    /// let options = BulkInsertOptions::new("external_uuid");
    /// let result = repo.bulk_insert_with_parent_ids(&db, nodes, options).await?;
    /// println!("Inserted: {}, Skipped: {}", result.inserted, result.skipped);
    /// ```
    ///
    /// # Errors
    /// - `CycleDetected` if `InBatch` references form a cycle
    /// - `InvalidBatchIndex` if index is out of bounds
    /// - Database errors during insert
    pub async fn bulk_insert_with_parent_ids(
        &self,
        conn: &DatabaseConnection,
        nodes: Vec<InsertNode<M>>,
        options: BulkInsertOptions,
    ) -> Result<BulkInsertResult<M>, ClosureTreeError> {
        Self::ensure_postgres(conn)?;

        if nodes.is_empty() {
            return Ok(BulkInsertResult {
                inserted: 0,
                skipped: 0,
                models: Vec::new(),
            });
        }

        // Acquire lock if needed
        let strategy = if options.skip_advisory_locks {
            crate::config::AdvisoryLockStrategy::Disabled
        } else {
            self.config().advisory_lock_strategy().clone()
        };

        let guard = LockedTransaction::acquire(&strategy, conn).await?;
        let result = self
            .bulk_insert_with_guard(guard, nodes, options)
            .await;

        match result {
            Ok(res) => Ok(res),
            Err(err) => Err(err),
        }
    }

    async fn bulk_insert_with_guard(
        &self,
        guard: LockedTransaction,
        nodes: Vec<InsertNode<M>>,
        options: BulkInsertOptions,
    ) -> Result<BulkInsertResult<M>, ClosureTreeError> {
        let conn = guard.connection();

        // Step 1: Topological sort
        let waves = Self::topological_sort_nodes(&nodes)?;

        // Step 2: Resolve external keys
        let external_keys: Vec<Value> = nodes
            .iter()
            .filter_map(|node| {
                if let Some(ParentRef::ExternalKey(val)) = &node.parent_ref {
                    Some(val.clone())
                } else {
                    None
                }
            })
            .collect();

        let _external_map = self
            .resolve_external_keys(conn, external_keys, &options.conflict_column)
            .await?;

        // Step 3: Process waves
        let mut all_inserted = Vec::new();
        let mut idx_to_id: HashMap<usize, M::Id> = HashMap::new();
        let mut total_inserted = 0;

        for wave in waves {
            let mut wave_nodes = Vec::new();

            for &idx in &wave {
                let node = &nodes[idx];
                let active = node.model.clone();

                // Resolve parent reference
                let parent_id = match &node.parent_ref {
                    None => None,
                    Some(ParentRef::InternalId(id)) => Some(id.clone()),
                    Some(ParentRef::InBatch(parent_idx)) => {
                        idx_to_id.get(parent_idx).cloned()
                    }
                    Some(ParentRef::ExternalKey(_val)) => {
                        // TODO: Lookup from external_map
                        None
                    }
                };

                // Extract name from ActiveModel
                let name = M::get_name_from_active(&active).ok_or_else(|| {
                    ClosureTreeError::invariant("name field must be set for bulk insert")
                })?;

                wave_nodes.push((idx, parent_id, name));
            }

            let (inserted, indices) = self.bulk_insert_wave(conn, wave_nodes, &options).await?;

            // Update idx_to_id map
            for (i, &original_idx) in indices.iter().enumerate() {
                idx_to_id.insert(original_idx, inserted[i].id());
            }

            // Insert hierarchy rows for this wave BEFORE next wave
            // (next wave needs to query these ancestors)
            self.batch_insert_all_hierarchy_rows(conn, &inserted).await?;

            total_inserted += inserted.len();
            all_inserted.extend(inserted);
        }

        guard.commit().await?;

        Ok(BulkInsertResult {
            inserted: total_inserted,
            skipped: nodes.len() - total_inserted,
            models: all_inserted,
        })
    }

    /// Batch insert hierarchy rows for multiple nodes at once.
    /// Much faster than inserting per-node: 1 ancestor query + 1 bulk insert vs N queries.
    async fn batch_insert_all_hierarchy_rows<C: ConnectionTrait>(
        &self,
        conn: &C,
        models: &[M],
    ) -> Result<(), ClosureTreeError> {
        if models.is_empty() {
            return Ok(());
        }

        // Collect all unique parent IDs
        let parent_ids: Vec<M::Id> = models
            .iter()
            .filter_map(|m| m.parent_id())
            .collect();

        // Query all ancestors for all parents in one query
        let all_ancestors = if !parent_ids.is_empty() {
            let parent_values: Vec<Value> = parent_ids
                .iter()
                .map(|id| M::hierarchy_id_to_value(id))
                .collect();

            M::HierarchyEntity::find()
                .filter(M::hierarchy_descendant_column().is_in(parent_values))
                .all(conn)
                .await?
        } else {
            Vec::new()
        };

        // Generate all hierarchy rows for all nodes
        let mut all_rows = Vec::new();
        for model in models {
            let model_id = model.id();

            // Self-referential row
            all_rows.push(M::hierarchy_build_row(
                model_id.clone(),
                model_id.clone(),
                0,
            ));

            // Parent ancestor rows - match by comparing values
            if let Some(parent_id) = model.parent_id() {
                let parent_value = M::hierarchy_id_to_value(&parent_id);

                for ancestor_row in &all_ancestors {
                    let descendant_id = M::hierarchy_model_descendant(ancestor_row);
                    let descendant_value = M::hierarchy_id_to_value(&descendant_id);

                    // Check if this ancestor belongs to our parent
                    if descendant_value == parent_value {
                        let ancestor_id = M::hierarchy_model_ancestor(ancestor_row);
                        let generations = M::hierarchy_model_generations(ancestor_row);

                        all_rows.push(M::hierarchy_build_row(
                            ancestor_id,
                            model_id.clone(),
                            generations + 1,
                        ));
                    }
                }
            }
        }

        // Bulk insert all hierarchy rows at once
        if !all_rows.is_empty() {
            M::HierarchyEntity::insert_many(all_rows)
                .exec(conn)
                .await?;
        }

        Ok(())
    }
}
