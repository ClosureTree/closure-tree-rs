use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, EntityTrait, FromQueryResult, IntoActiveModel, Value,
};

use crate::config::ClosureTreeConfig;

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

    fn name(&self) -> &str;
    fn set_name(active: &mut Self::ActiveModel, name: &str);

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
