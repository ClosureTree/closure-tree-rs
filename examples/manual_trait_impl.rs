// Manual ClosureTreeModel implementation - skip the derive macro
// Adapt this to your actual Message entity structure

use closure_tree::{ClosureTreeConfig, ClosureTreeModel, ClosureTreeOptions};
use once_cell::sync::Lazy;
use sea_orm::{entity::prelude::*, ActiveValue};

// ============================================================================
// Your Message Entity
// ============================================================================
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "messages")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub parent_id: Option<i32>,
    pub name: String, // Or whatever you call this field

    // Your other columns...
    // pub external_uuid: Uuid,
    // pub content: String,
    // pub created_at: DateTime,
    // etc.
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

// ============================================================================
// Message Hierarchy Entity (closure table)
// ============================================================================
pub mod message_hierarchy {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "message_hierarchies")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub ancestor_id: i32,
        #[sea_orm(primary_key)]
        pub descendant_id: i32,
        pub generations: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// ============================================================================
// Manual ClosureTreeModel Implementation
// ============================================================================
impl ClosureTreeModel for Model {
    type Entity = Entity;
    type ActiveModel = ActiveModel;
    type Id = i32; // Change if your ID is different (i64, Uuid, etc.)

    type HierarchyEntity = message_hierarchy::Entity;
    type HierarchyModel = message_hierarchy::Model;
    type HierarchyActiveModel = message_hierarchy::ActiveModel;

    fn closure_tree_config() -> &'static ClosureTreeConfig {
        static CONFIG: Lazy<ClosureTreeConfig> = Lazy::new(|| {
            let base = ClosureTreeConfig::new("Message", "MessageHierarchy");
            ClosureTreeOptions::default()
                .parent_column("parent_id") // Your parent FK column
                .name_column("name")         // Your display name column
                .hierarchy_table("message_hierarchies")
                .apply(base)
        });
        &CONFIG
    }

    fn id(&self) -> Self::Id {
        self.id
    }

    fn parent_id(&self) -> Option<Self::Id> {
        self.parent_id
    }

    fn set_parent(active: &mut Self::ActiveModel, parent: Option<Self::Id>) {
        active.parent_id = ActiveValue::Set(parent);
    }

    fn id_to_value(id: &Self::Id) -> Value {
        Value::from(*id)
    }

    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn set_name(active: &mut Self::ActiveModel, name: &str) {
        active.name = ActiveValue::Set(name.to_owned());
    }

    fn get_name_from_active(active: &Self::ActiveModel) -> Option<String> {
        match &active.name {
            ActiveValue::Set(val) => Some(val.clone()),
            ActiveValue::Unchanged(val) => Some(val.clone()),
            ActiveValue::NotSet => None,
        }
    }

    fn parent_column() -> <Self::Entity as EntityTrait>::Column {
        Column::ParentId
    }

    fn id_column() -> <Self::Entity as EntityTrait>::Column {
        Column::Id
    }

    fn name_column() -> <Self::Entity as EntityTrait>::Column {
        Column::Name
    }

    fn hierarchy_ancestor_column() -> <Self::HierarchyEntity as EntityTrait>::Column {
        message_hierarchy::Column::AncestorId
    }

    fn hierarchy_descendant_column() -> <Self::HierarchyEntity as EntityTrait>::Column {
        message_hierarchy::Column::DescendantId
    }

    fn hierarchy_generations_column() -> <Self::HierarchyEntity as EntityTrait>::Column {
        message_hierarchy::Column::Generations
    }

    fn hierarchy_id_to_value(id: &Self::Id) -> Value {
        Value::from(*id)
    }

    fn hierarchy_model_ancestor(model: &Self::HierarchyModel) -> Self::Id {
        model.ancestor_id
    }

    fn hierarchy_model_descendant(model: &Self::HierarchyModel) -> Self::Id {
        model.descendant_id
    }

    fn hierarchy_model_generations(model: &Self::HierarchyModel) -> i32 {
        model.generations
    }

    fn hierarchy_build_row(
        ancestor: Self::Id,
        descendant: Self::Id,
        generations: i32,
    ) -> Self::HierarchyActiveModel {
        message_hierarchy::ActiveModel {
            ancestor_id: ActiveValue::Set(ancestor),
            descendant_id: ActiveValue::Set(descendant),
            generations: ActiveValue::Set(generations),
        }
    }
}

// ============================================================================
// Usage Example
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use closure_tree::{BulkInsertOptions, ClosureTreeRepository, InsertNode, ParentRef};
    use sea_orm::ActiveValue;

    #[tokio::test]
    async fn example_bulk_insert() {
        // let db = /* your database connection */;

        let repo = ClosureTreeRepository::<Model>::new();

        let nodes = vec![
            InsertNode {
                model: ActiveModel {
                    id: ActiveValue::NotSet,
                    parent_id: ActiveValue::NotSet,
                    name: ActiveValue::Set("root".to_string()),
                },
                parent_ref: None,
            },
            InsertNode {
                model: ActiveModel {
                    id: ActiveValue::NotSet,
                    parent_id: ActiveValue::NotSet,
                    name: ActiveValue::Set("child".to_string()),
                },
                parent_ref: Some(ParentRef::InBatch(0)),
            },
        ];

        let options = BulkInsertOptions::new("name"); // Or "external_uuid" if unique

        // let result = repo.bulk_insert_with_parent_ids(&db, nodes, options).await?;
        // println!("Inserted {} nodes", result.inserted);
    }
}
