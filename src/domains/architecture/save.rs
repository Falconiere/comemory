//! Saving a model: validate it against the index, then write it through the
//! ordinary memory save path so it inherits content-addressed ids, the
//! markdown mirror, sync, and the supersede chain. Nothing here writes a row.

use crate::domains::architecture::current;
use crate::domains::architecture::model::{Model, TAG};
use crate::domains::architecture::validate::validate;
use crate::domains::memories::Kind;
use crate::domains::memories::save::{Request, run as save_memory};
use crate::prelude::*;
use crate::store::indexed_files;
use crate::utilities::context::Ctx;

/// What a save did.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Saved {
    /// 8-hex id of the memory now holding the model.
    pub id: String,
    /// On-disk markdown path.
    pub path: String,
    /// `false` when an identical model was already stored under this id.
    pub created: bool,
    /// The model id this one replaced, when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded: Option<String>,
    /// Component count stored.
    pub components: usize,
    /// Edge count stored.
    pub edges: usize,
}

/// Validate and store `model` as the architecture model of `repo`.
pub fn run(ctx: &mut Ctx<'_>, repo: &str, model: &Model) -> Result<Saved> {
    let conn = ctx.conn()?;
    let indexed: Vec<String> = indexed_files::list_for_repo(conn, repo)?
        .into_iter()
        .map(|(path, _blob)| path)
        .collect();
    if indexed.is_empty() {
        return Err(Error::Usage(format!(
            "no indexed files for repo {repo:?}; run `comemory index-code --repo {repo}` first"
        )));
    }
    validate(model, repo, &indexed)?;
    let superseded = current::find(conn, repo)?.map(|c| c.id);

    let response = save_memory(
        ctx,
        Request {
            body: body_for(repo, model)?,
            title: None,
            kind: Kind::Note,
            repo: repo.to_string(),
            tags: vec![TAG.to_string()],
            author: String::new(),
            quality: 3,
            supersedes: superseded.iter().cloned().collect(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )?;
    Ok(Saved {
        id: response.id,
        path: response.path,
        created: response.created,
        superseded,
        components: model.components.len(),
        edges: model.edges.len(),
    })
}

/// The memory body: the title line every memory is read by, then the model in
/// a ```json fence the console and [`current::parse_body`] both read.
fn body_for(repo: &str, model: &Model) -> Result<String> {
    let json = serde_json::to_string_pretty(model)?;
    Ok(format!(
        "Architecture model — {repo}\n\nComponent-level architecture of {repo}, \
         {} components and {} edges. Generated {}, source {}.\n\n```json\n{json}\n```\n",
        model.components.len(),
        model.edges.len(),
        model.generated_at,
        serde_json::to_string(&model.source)?.trim_matches('"'),
    ))
}
