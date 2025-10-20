use std::marker::PhantomData;

use sea_orm::{
    entity::prelude::*, ColumnTrait, Condition, ConnectionTrait, DbBackend, EntityTrait,
    QueryFilter, QueryOrder,
};

use sea_orm::sea_query::Expr;

use crate::config::{ClosureTreeConfig, OrderStrategy};
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
}
