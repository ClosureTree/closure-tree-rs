use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::{
    parse_macro_input, spanned::Spanned, Attribute, Data, DeriveInput, Fields, Ident, Path, Type,
};

#[proc_macro_derive(ClosureTreeModel, attributes(closure_tree))]
pub fn derive_closure_tree_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match impl_closure_tree_model(&input) {
        Ok(tokens) => tokens,
        Err(err) => err.to_compile_error().into(),
    }
}

#[derive(Default)]
struct Options {
    id_field: Option<String>,
    id_type: Option<Type>,
    parent_field: Option<String>,
    hierarchy_module: Option<Path>,
    hierarchy_table: Option<String>,
    name_field: Option<String>,
    entity_name: Option<String>,
    hierarchy_name: Option<String>,
    ancestor_field: Option<String>,
    descendant_field: Option<String>,
    generations_field: Option<String>,
}

fn impl_closure_tree_model(input: &DeriveInput) -> syn::Result<TokenStream> {
    let struct_ident = &input.ident;

    let data_struct = match &input.data {
        Data::Struct(data) => data,
        _ => {
            return Err(syn::Error::new(
                input.span(),
                "ClosureTreeModel can only be derived for structs",
            ))
        }
    };

    let mut options = Options::default();
    let mut table_name: Option<String> = None;

    for attr in &input.attrs {
        if attr.path().is_ident("closure_tree") {
            parse_closure_tree_attr(attr, &mut options)?;
        }

        if attr.path().is_ident("sea_orm") {
            if let Some(name) = parse_sea_orm_table_name(attr)? {
                table_name = Some(name);
            }
        }
    }

    let id_field_name = options.id_field.unwrap_or_else(|| "id".to_string());
    let parent_field_name = options
        .parent_field
        .unwrap_or_else(|| "parent_id".to_string());
    let name_field_name = options.name_field.unwrap_or_else(|| "name".to_string());
    let ancestor_field_name = options
        .ancestor_field
        .unwrap_or_else(|| "ancestor_id".to_string());
    let descendant_field_name = options
        .descendant_field
        .unwrap_or_else(|| "descendant_id".to_string());
    let generations_field_name = options
        .generations_field
        .unwrap_or_else(|| "generations".to_string());

    let id_field_ident = Ident::new(&id_field_name, struct_ident.span());
    let parent_field_ident = Ident::new(&parent_field_name, struct_ident.span());
    let name_field_ident = Ident::new(&name_field_name, struct_ident.span());
    let ancestor_field_ident = Ident::new(&ancestor_field_name, struct_ident.span());
    let descendant_field_ident = Ident::new(&descendant_field_name, struct_ident.span());
    let generations_field_ident = Ident::new(&generations_field_name, struct_ident.span());

    let mut id_field_type: Option<Type> = options.id_type.clone();
    let mut bulk_insert_fields = Vec::new();

    if let Fields::Named(ref fields) = data_struct.fields {
        for field in &fields.named {
            if let Some(ident) = &field.ident {
                // Skip id field (auto-generated in DB)
                if ident != &id_field_ident {
                    bulk_insert_fields.push((ident.clone(), field.ty.clone()));
                }
            }
        }
        for field in &fields.named {
            if let Some(ident) = &field.ident {
                if ident == &id_field_ident && id_field_type.is_none() {
                    id_field_type = Some(field.ty.clone());
                }
            }
        }
    } else {
        return Err(syn::Error::new(
            data_struct.fields.span(),
            "ClosureTreeModel requires named fields",
        ));
    }

    let id_type = id_field_type.ok_or_else(|| {
        syn::Error::new(
            struct_ident.span(),
            "Unable to determine id field type; specify `id_type = ...` in #[closure_tree]",
        )
    })?;

    let hierarchy_module_path = options
        .hierarchy_module
        .ok_or_else(|| syn::Error::new(struct_ident.span(), "`hierarchy_module` must be set"))?;

    let entity_name = options
        .entity_name
        .unwrap_or_else(|| struct_ident.unraw().to_string());
    let hierarchy_name = options.hierarchy_name.unwrap_or_else(|| {
        if entity_name.ends_with("Hierarchy") {
            entity_name.clone()
        } else {
            format!("{}Hierarchy", entity_name)
        }
    });

    let base_table = table_name.unwrap_or_else(|| struct_ident.unraw().to_string());
    let hierarchy_table = options
        .hierarchy_table
        .unwrap_or_else(|| format!("{}_hierarchies", base_table));

    let id_column_variant = format_ident!("{}", to_pascal_case(&id_field_name));
    let parent_column_variant = format_ident!("{}", to_pascal_case(&parent_field_name));
    let name_column_variant = format_ident!("{}", to_pascal_case(&name_field_name));
    let ancestor_column_variant = format_ident!("{}", to_pascal_case(&ancestor_field_name));
    let descendant_column_variant = format_ident!("{}", to_pascal_case(&descendant_field_name));
    let generations_column_variant = format_ident!("{}", to_pascal_case(&generations_field_name));

    // Split bulk_insert_fields into separate vectors for quote
    let bulk_field_idents: Vec<_> = bulk_insert_fields.iter().map(|(ident, _)| ident).collect();
    let bulk_field_types: Vec<_> = bulk_insert_fields.iter().map(|(_, ty)| ty).collect();
    let bulk_field_names: Vec<String> = bulk_field_idents.iter().map(|i| i.to_string()).collect();

    let parent_column_literal = syn::LitStr::new(&parent_field_name, struct_ident.span());
    let name_column_literal = syn::LitStr::new(&name_field_name, struct_ident.span());
    let hierarchy_table_literal = syn::LitStr::new(&hierarchy_table, struct_ident.span());
    let entity_name_literal = syn::LitStr::new(&entity_name, struct_ident.span());
    let hierarchy_name_literal = syn::LitStr::new(&hierarchy_name, struct_ident.span());

    let generated = quote! {
        // Helper function to map Rust types to PostgreSQL ArrayType
        fn type_to_array_type(type_name: &str) -> ::sea_orm::sea_query::ArrayType {
            if type_name.contains("Uuid") || type_name.contains("uuid") {
                ::sea_orm::sea_query::ArrayType::Bytes
            } else if type_name.contains("i64") || type_name.contains("BigInt") {
                ::sea_orm::sea_query::ArrayType::BigInt
            } else if type_name.contains("String") || type_name.contains("str") {
                ::sea_orm::sea_query::ArrayType::String
            } else if type_name.contains("bool") {
                ::sea_orm::sea_query::ArrayType::Bool
            } else if type_name.contains("f64") || type_name.contains("Double") {
                ::sea_orm::sea_query::ArrayType::Double
            } else if type_name.contains("f32") || type_name.contains("Float") {
                ::sea_orm::sea_query::ArrayType::Float
            } else {
                // Default to Int for i32 and unknown types
                ::sea_orm::sea_query::ArrayType::Int
            }
        }


        impl ::closure_tree::ClosureTreeModel for #struct_ident {
            type Entity = Entity;
            type ActiveModel = ActiveModel;
            type Id = #id_type;

            type HierarchyEntity = #hierarchy_module_path::Entity;
            type HierarchyModel = #hierarchy_module_path::Model;
            type HierarchyActiveModel = #hierarchy_module_path::ActiveModel;

            fn closure_tree_config() -> &'static ::closure_tree::ClosureTreeConfig {
                static CONFIG: ::once_cell::sync::Lazy<::closure_tree::ClosureTreeConfig> =
                    ::once_cell::sync::Lazy::new(|| {
                        let base = ::closure_tree::ClosureTreeConfig::new(
                            #entity_name_literal,
                            #hierarchy_name_literal,
                        );
                        ::closure_tree::ClosureTreeOptions::default()
                            .parent_column(#parent_column_literal)
                            .name_column(#name_column_literal)
                            .hierarchy_table(#hierarchy_table_literal)
                            .apply(base)
                    });
                &CONFIG
            }

            fn id(&self) -> Self::Id {
                self.#id_field_ident.clone()
            }

            fn parent_id(&self) -> Option<Self::Id> {
                self.#parent_field_ident.clone()
            }

            fn set_parent(active: &mut Self::ActiveModel, parent: Option<Self::Id>) {
                active.#parent_field_ident = ::sea_orm::ActiveValue::Set(parent);
            }

            fn id_to_value(id: &Self::Id) -> ::sea_orm::Value {
                ::sea_orm::Value::from(id.clone())
            }

            fn null_id_sentinel() -> ::sea_orm::Value {
                // Type-specific sentinel values for NULL in UNNEST
                // Int types: -1, UUID: nil UUID, etc.
                match stringify!(#id_type) {
                    "Uuid" | "uuid::Uuid" => {
                        // nil UUID: 00000000-0000-0000-0000-000000000000
                        ::sea_orm::Value::Uuid(Some(Box::new(
                            ::uuid::Uuid::nil()
                        )))
                    }
                    "i64" => ::sea_orm::Value::BigInt(Some(-1)),
                    "i32" | _ => ::sea_orm::Value::Int(Some(-1)),
                }
            }

            fn id_array_type() -> ::sea_orm::sea_query::ArrayType {
                // Type-specific ArrayType for UNNEST
                match stringify!(#id_type) {
                    "Uuid" | "uuid::Uuid" => ::sea_orm::sea_query::ArrayType::Bytes,
                    "i64" => ::sea_orm::sea_query::ArrayType::BigInt,
                    "i32" | _ => ::sea_orm::sea_query::ArrayType::Int,
                }
            }

            fn name(&self) -> &str {
                self.#name_field_ident.as_str()
            }

            fn set_name(active: &mut Self::ActiveModel, name: &str) {
                active.#name_field_ident = ::sea_orm::ActiveValue::Set(name.to_owned());
            }

            fn get_name_from_active(active: &Self::ActiveModel) -> Option<String> {
                match &active.#name_field_ident {
                    ::sea_orm::ActiveValue::Set(val) => Some(val.clone()),
                    ::sea_orm::ActiveValue::Unchanged(val) => Some(val.clone()),
                    ::sea_orm::ActiveValue::NotSet => None,
                }
            }

            fn parent_column() -> <Self::Entity as ::sea_orm::EntityTrait>::Column {
                Column::#parent_column_variant
            }

            fn id_column() -> <Self::Entity as ::sea_orm::EntityTrait>::Column {
                Column::#id_column_variant
            }

            fn name_column() -> <Self::Entity as ::sea_orm::EntityTrait>::Column {
                Column::#name_column_variant
            }

            fn hierarchy_ancestor_column() -> <Self::HierarchyEntity as ::sea_orm::EntityTrait>::Column {
                #hierarchy_module_path::Column::#ancestor_column_variant
            }

            fn hierarchy_descendant_column() -> <Self::HierarchyEntity as ::sea_orm::EntityTrait>::Column {
                #hierarchy_module_path::Column::#descendant_column_variant
            }

            fn hierarchy_generations_column() -> <Self::HierarchyEntity as ::sea_orm::EntityTrait>::Column {
                #hierarchy_module_path::Column::#generations_column_variant
            }

            fn hierarchy_id_to_value(id: &Self::Id) -> ::sea_orm::Value {
                ::sea_orm::Value::from(id.clone())
            }

            fn hierarchy_model_ancestor(model: &Self::HierarchyModel) -> Self::Id {
                model.#ancestor_field_ident.clone()
            }

            fn hierarchy_model_descendant(model: &Self::HierarchyModel) -> Self::Id {
                model.#descendant_field_ident.clone()
            }

            fn hierarchy_model_generations(model: &Self::HierarchyModel) -> i32 {
                model.#generations_field_ident
            }

            fn hierarchy_build_row(
                ancestor: Self::Id,
                descendant: Self::Id,
                generations: i32,
            ) -> Self::HierarchyActiveModel {
                #[allow(clippy::needless_update)]
                {
                    #hierarchy_module_path::ActiveModel {
                        #ancestor_field_ident: ::sea_orm::ActiveValue::Set(ancestor),
                        #descendant_field_ident: ::sea_orm::ActiveValue::Set(descendant),
                        #generations_field_ident: ::sea_orm::ActiveValue::Set(generations),
                        ..::core::default::Default::default()
                    }
                }
            }

            fn bulk_insert_columns() -> Vec<::closure_tree::BulkInsertColumn> {
                // Generate column definitions for all fields except id
                vec![
                    #(
                        ::closure_tree::BulkInsertColumn::new(
                            #bulk_field_names,
                            type_to_array_type(stringify!(#bulk_field_types))
                        )
                    ),*
                ]
            }

            fn extract_bulk_values(active: &Self::ActiveModel) -> Vec<::sea_orm::Value> {
                // Helper to get sentinel for type (handles Option types)
                fn type_sentinel(type_name: &str) -> ::sea_orm::Value {
                    if type_name.contains("Uuid") || type_name.contains("uuid") {
                        ::sea_orm::Value::Uuid(Some(Box::new(::uuid::Uuid::nil())))
                    } else if type_name.contains("i64") || type_name.contains("BigInt") {
                        ::sea_orm::Value::BigInt(Some(-1))
                    } else if type_name.contains("String") {
                        ::sea_orm::Value::String(Some(Box::new(String::new())))
                    } else {
                        ::sea_orm::Value::Int(Some(-1))
                    }
                }

                // Helper to convert value ensuring consistent types in arrays
                fn to_value<T: Into<::sea_orm::Value> + Clone>(v: &T) -> ::sea_orm::Value {
                    v.clone().into()
                }

                vec![
                    #(
                        match &active.#bulk_field_idents {
                            ::sea_orm::ActiveValue::Set(v) => to_value(v),
                            ::sea_orm::ActiveValue::Unchanged(v) => to_value(v),
                            ::sea_orm::ActiveValue::NotSet => {
                                type_sentinel(stringify!(#bulk_field_types))
                            }
                        }
                    ),*
                ]
            }
        }
    };

    Ok(generated.into())
}

fn parse_closure_tree_attr(attr: &Attribute, options: &mut Options) -> syn::Result<()> {
    attr.parse_nested_meta(|meta| {
        let ident = meta
            .path
            .get_ident()
            .ok_or_else(|| syn::Error::new(meta.path.span(), "Invalid option key"))?
            .to_string();

        match ident.as_str() {
            "id_field" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.id_field = Some(value.value());
            }
            "parent_field" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.parent_field = Some(value.value());
            }
            "name_field" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.name_field = Some(value.value());
            }
            "hierarchy_module" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.hierarchy_module = Some(parse_path(&value.value(), value.span())?);
            }
            "hierarchy_table" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.hierarchy_table = Some(value.value());
            }
            "entity_name" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.entity_name = Some(value.value());
            }
            "hierarchy_name" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.hierarchy_name = Some(value.value());
            }
            "ancestor_field" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.ancestor_field = Some(value.value());
            }
            "descendant_field" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.descendant_field = Some(value.value());
            }
            "generations_field" => {
                let value: syn::LitStr = meta.value()?.parse()?;
                options.generations_field = Some(value.value());
            }
            "id_type" => {
                let ty: Type = meta.value()?.parse()?;
                options.id_type = Some(ty);
            }
            other => {
                return Err(syn::Error::new(
                    meta.path.span(),
                    format!("Unsupported closure_tree option `{other}`"),
                ));
            }
        }

        Ok(())
    })
}

fn parse_sea_orm_table_name(attr: &Attribute) -> syn::Result<Option<String>> {
    let mut table_name: Option<String> = None;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("table_name") {
            let value: syn::LitStr = meta.value()?.parse()?;
            table_name = Some(value.value());
        }
        Ok(())
    })?;
    Ok(table_name)
}

fn parse_path(value: &str, span: proc_macro2::Span) -> syn::Result<Path> {
    syn::parse_str::<Path>(value).map_err(|_| syn::Error::new(span, "Invalid path"))
}

fn to_pascal_case(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}
