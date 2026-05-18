use qwick_memory::index::schema::{code_schema, memory_schema, CODE_TABLE, MEMORY_TABLE};

#[test]
fn memory_table_name_is_correct() {
    assert_eq!(MEMORY_TABLE, "memory_chunks");
}

#[test]
fn memory_schema_has_nine_fields() {
    let schema = memory_schema(768);
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(
        names,
        vec![
            "id",
            "body",
            "kind",
            "repo",
            "tags",
            "created",
            "quality",
            "content_hash",
            "embedding"
        ]
    );
}

#[test]
fn embedding_dim_matches_arg() {
    let schema = memory_schema(768);
    let field = schema.field_with_name("embedding").unwrap();
    let arrow_schema::DataType::FixedSizeList(_, dim) = field.data_type() else {
        panic!("embedding should be FixedSizeList");
    };
    assert_eq!(*dim, 768);
}

#[test]
fn code_table_name_is_correct() {
    assert_eq!(CODE_TABLE, "code_chunks");
}

#[test]
fn code_schema_has_seven_fields() {
    let schema = code_schema(768);
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(
        names,
        vec![
            "qualified",
            "snippet",
            "language",
            "file",
            "symbol_kind",
            "ast_hash",
            "embedding",
        ]
    );
}

#[test]
fn code_embedding_dim_matches_arg() {
    let schema = code_schema(768);
    let field = schema.field_with_name("embedding").unwrap();
    let arrow_schema::DataType::FixedSizeList(_, dim) = field.data_type() else {
        panic!("embedding should be FixedSizeList");
    };
    assert_eq!(*dim, 768);
}
