use qwick::graph::schema::MEMORY_LAYER_DDL;

#[test]
fn memory_layer_ddl_is_non_empty() {
    assert!(
        !MEMORY_LAYER_DDL.is_empty(),
        "MEMORY_LAYER_DDL must contain at least one statement",
    );
}

#[test]
fn memory_layer_ddl_uses_if_not_exists_everywhere() {
    for ddl in MEMORY_LAYER_DDL {
        assert!(
            ddl.contains("IF NOT EXISTS"),
            "every DDL statement must be idempotent; offender: {ddl}",
        );
    }
}

#[test]
fn memory_layer_ddl_defines_required_node_tables() {
    let joined = MEMORY_LAYER_DDL.join("\n");
    for table in ["Memory", "Repo", "Author", "Tag"] {
        let needle = format!("CREATE NODE TABLE IF NOT EXISTS {table}");
        assert!(
            joined.contains(&needle),
            "expected NODE TABLE for {table} in DDL, got:\n{joined}",
        );
    }
}

#[test]
fn memory_layer_ddl_defines_required_rel_tables() {
    let joined = MEMORY_LAYER_DDL.join("\n");
    for rel in [
        "InRepo",
        "AuthoredBy",
        "Tagged",
        "Supersedes",
        "ConflictsWith",
        "RelatesTo",
        "DerivedFrom",
    ] {
        let needle = format!("CREATE REL TABLE IF NOT EXISTS {rel}");
        assert!(
            joined.contains(&needle),
            "expected REL TABLE for {rel} in DDL, got:\n{joined}",
        );
    }
}

#[test]
fn memory_node_table_has_primary_key_on_id() {
    let memory_ddl = MEMORY_LAYER_DDL
        .iter()
        .find(|s| s.contains("NODE TABLE IF NOT EXISTS Memory"))
        .expect("Memory node table DDL must exist");
    assert!(
        memory_ddl.contains("PRIMARY KEY(id)"),
        "Memory must be keyed on id, got: {memory_ddl}",
    );
}
