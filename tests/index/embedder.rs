use qwick_memory::index::Embedder;

#[test]
fn nomic_embeds_to_768d() {
    let mut emb = Embedder::nomic_text().unwrap();
    let v = emb.embed_one("hello world").unwrap();
    assert_eq!(v.len(), 768);
    let mag: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(mag > 0.0, "expected non-zero magnitude, got {mag}");
}

#[test]
fn deterministic_same_input_same_output() {
    let mut emb = Embedder::nomic_text().unwrap();
    let a = emb.embed_one("postgres migration race").unwrap();
    let b = emb.embed_one("postgres migration race").unwrap();
    assert_eq!(a, b, "embedder should be deterministic for the same input");
}
