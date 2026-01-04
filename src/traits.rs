use sea_orm::{
    sea_query::ArrayType, ActiveModelBehavior, ActiveModelTrait, EntityTrait, FromQueryResult,
    IntoActiveModel, Value,
};

use crate::config::{BulkInsertColumn, ClosureTreeConfig};

/// Trait implemented by SeaORM `Model` types that participate in the closure tree.
///
/// Implementations are normally provided by the `#[derive(ClosureTreeModel)]` macro.
pub trait ClosureTreeModel:
    Clone + Send + Sync + 'static + IntoActiveModel<Self::ActiveModel> + FromQueryResult
{
    type Entity: EntityTrait<Model = Self>;
    type ActiveModel: ActiveModelTrait<Entity = Self::Entity> + ActiveModelBehavior + Send;
    type Id: Clone + Send + Sync + 'static;

    type HierarchyEntity: EntityTrait<Model = Self::HierarchyModel>;
    type HierarchyModel: Clone + Send + Sync + 'static + FromQueryResult;
    type HierarchyActiveModel: ActiveModelTrait<Entity = Self::HierarchyEntity>
        + ActiveModelBehavior
        + Send;

    fn closure_tree_config() -> &'static ClosureTreeConfig;

    fn id(&self) -> Self::Id;
    fn parent_id(&self) -> Option<Self::Id>;
    fn set_parent(active: &mut Self::ActiveModel, parent: Option<Self::Id>);
    fn id_to_value(id: &Self::Id) -> Value;

    /// Sentinel value for NULL IDs in UNNEST operations.
    /// For Int: -1, for UUID: 00000000-0000-0000-0000-000000000000, etc.
    fn null_id_sentinel() -> Value;

    /// Array type for PostgreSQL UNNEST operations.
    fn id_array_type() -> ArrayType;

    /// Column definitions for bulk insert (name, array type, is nullable).
    /// Used to build UNNEST SQL with all columns.
    fn bulk_insert_columns() -> Vec<BulkInsertColumn>;

    /// Extract values for all bulk insert columns from ActiveModel.
    /// Order must match bulk_insert_columns().
    fn extract_bulk_values(active: &Self::ActiveModel) -> Vec<Value>;

    fn name(&self) -> &str;
    fn set_name(active: &mut Self::ActiveModel, name: &str);
    fn get_name_from_active(active: &Self::ActiveModel) -> Option<String>;

    fn parent_column() -> <Self::Entity as EntityTrait>::Column;
    fn id_column() -> <Self::Entity as EntityTrait>::Column;
    fn name_column() -> <Self::Entity as EntityTrait>::Column;

    fn hierarchy_ancestor_column() -> <Self::HierarchyEntity as EntityTrait>::Column;
    fn hierarchy_descendant_column() -> <Self::HierarchyEntity as EntityTrait>::Column;
    fn hierarchy_generations_column() -> <Self::HierarchyEntity as EntityTrait>::Column;

    fn hierarchy_id_to_value(id: &Self::Id) -> Value;
    fn hierarchy_model_ancestor(model: &Self::HierarchyModel) -> Self::Id;
    fn hierarchy_model_descendant(model: &Self::HierarchyModel) -> Self::Id;
    fn hierarchy_model_generations(model: &Self::HierarchyModel) -> i32;
    fn hierarchy_build_row(
        ancestor: Self::Id,
        descendant: Self::Id,
        generations: i32,
    ) -> Self::HierarchyActiveModel;
}
