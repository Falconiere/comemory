#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/authority.rs`, against a real temp
//! data directory. Every out-of-envelope verb is refused before the store (no
//! row, no event, no telemetry row, and a lazy context never creates its
//! database); every admitted verb runs as the envelope's actor; and an
//! unrecognised capability empties the whole list without being logged.
//!
//! Verbs whose cores land in later tasks are driven by [`Probe`], a command
//! that names any verb and whose body is the real create core, so an
//! admission is observable as rows and a refusal as their absence.

use std::io::Write;
use std::sync::{Arc, Mutex};

use super::{Actor, Capabilities, Capability, Command, Envelope, Rule, Tier, Verb, run, sealed};
use comemory::config::{Config, Paths};
use comemory::domains::projects::{create, list, show};
use comemory::errors::Result;
use comemory::store::{Connection, connection, project_read};
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::classify;

/// Any verb, executed by the real create core.
struct Probe {
    verb: Verb,
    key: String,
}

impl sealed::Sealed for Probe {}

impl Command for Probe {
    type Response = create::Response;

    fn verb(&self) -> Verb {
        self.verb
    }

    fn execute(self, ctx: &mut Ctx<'_>, actor: &Actor) -> Result<create::Response> {
        charter(&self.key).execute(ctx, actor)
    }
}

/// A minimal charter keyed `key`.
fn charter(key: &str) -> create::Request {
    serde_json::from_value(serde_json::json!({
        "name": format!("Project {key}"), "keyPrefix": key, "outcome": "Ship it"
    }))
    .unwrap()
}

/// A migrated data directory and its open connection.
struct Home {
    _dir: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure_dirs().unwrap();
        let conn = connection::open(paths.db_path()).unwrap();
        Self {
            _dir: dir,
            paths,
            cfg: Config::defaults(),
            conn,
        }
    }

    fn exec<C: Command>(&mut self, envelope: &Envelope, command: C) -> Result<C::Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        run(&mut ctx, envelope, command)
    }

    fn probe(&mut self, envelope: &Envelope, verb: Verb, key: &str) -> Result<create::Response> {
        let key = key.to_string();
        self.exec(envelope, Probe { verb, key })
    }

    /// Rows of every table a create writes or a refusal could.
    fn counts(&self) -> [i64; 3] {
        ["projects", "project_activity_events", "activity_log"].map(|table| {
            self.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        })
    }

    /// Every distinct `kind:id` actor on a project activity event.
    fn event_actors(&self) -> Vec<String> {
        self.conn
            .prepare(
                "SELECT DISTINCT actor_principal_type || ':' || actor_principal_id
                 FROM project_activity_events ORDER BY 1",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
}

/// The refusal's code word and message.
fn refusal<T: std::fmt::Debug>(result: Result<T>) -> (&'static str, String) {
    let e = result.expect_err("expected a refusal");
    (classify(&e).0, e.to_string())
}

/// A per-verb work-item key prefix.
fn key(tag: char, i: usize) -> String {
    format!("{tag}{i:02}")
}

/// A `Write` sink a `tracing` subscriber shares with the test.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Run `f` under a real `tracing` subscriber; its result and the log text.
fn captured<T>(f: impl FnOnce() -> T) -> (T, String) {
    let sink = Sink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let log = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    (out, log)
}

#[test]
fn capabilities_are_the_platform_six_in_command_order() {
    let names = Capability::ALL.map(Capability::as_str);
    assert_eq!(
        names,
        [
            "project.read",
            "proposal.create",
            "work_packet.create",
            "execution.update",
            "evidence.create",
            "health.update",
        ]
    );
    for c in Capability::ALL {
        assert_eq!(Capability::parse(c.as_str()), Some(c));
        assert!(Capabilities::all().contains(c) && !Capabilities::none().contains(c));
    }
    assert_eq!(Capability::parse("Project.Read"), None);
}

#[test]
fn an_unrecognised_capability_empties_the_whole_list_and_logs_no_value() {
    const UNKNOWN: &str = "totally.unknown-cap-7f3";
    let source = "projects.agent_capabilities";
    let (parsed, log) = captured(|| Capabilities::parse(source, &["project.read", UNKNOWN]));
    assert_eq!(parsed, Capabilities::none(), "project.read survived");
    assert_eq!(
        log.trim_end(),
        format!(
            " WARN project envelope: unrecognised capability; the list grants nothing \
             source=\"{source}\" position=1 count=2"
        )
    );
    assert!(!log.contains(UNKNOWN), "the value reached the log: {log}");

    let agent = Envelope::agent("reader", parsed);
    assert_eq!(
        refusal(agent.authorize(Verb::ProjectShow)),
        (
            "project_agent_scope",
            "This grant does not carry the project.read capability".to_string()
        )
    );
}

#[test]
fn an_empty_or_repeated_list_parses_without_a_warning() {
    let (empty, log) = captured(|| Capabilities::parse::<&str>("t", &[]));
    assert_eq!(empty, Capabilities::none());
    assert!(log.is_empty(), "{log}");

    let (repeated, log) =
        captured(|| Capabilities::parse("t", &["evidence.create", "evidence.create"]));
    assert!(repeated.contains(Capability::EvidenceCreate));
    assert!(!repeated.contains(Capability::ProjectRead));
    assert!(log.is_empty(), "{log}");
}

#[test]
fn every_human_only_verb_refuses_an_agent_holding_all_six_before_the_store() {
    use Verb as V;
    let human_only: Vec<Verb> = Verb::ALL
        .into_iter()
        .filter(|v| matches!(v.rule(), Rule::HumanOnly(_)))
        .collect();
    assert_eq!(
        human_only,
        [
            V::ProjectCreate,
            V::ApprovalInbox,
            V::WorkspaceActivity,
            V::WorkItemComplete,
            V::Archive,
            V::Restore,
            V::Pause,
            V::Resume,
            V::ProposalApprove,
            V::ProposalRequestChanges,
            V::ProposalReject,
            V::ProjectComplete,
            V::ProjectCancel,
            V::ProjectDelete,
        ]
    );
    let agent = Envelope::local_agent();
    let mut home = Home::new();
    for (i, verb) in human_only.into_iter().enumerate() {
        let expected = (
            "project_agent_scope",
            "This command requires a signed-in human".to_string(),
        );
        assert_eq!(refusal(home.probe(&agent, verb, &key('H', i))), expected);

        let lazy = tempfile::tempdir().unwrap();
        let paths = Paths::new(lazy.path());
        let cfg = Config::defaults();
        let mut ctx = Ctx::lazy(&paths, &cfg);
        let probe = Probe {
            verb,
            key: key('H', i),
        };
        assert_eq!(refusal(run(&mut ctx, &agent, probe)), expected);
        assert!(!paths.db_path().exists(), "{verb:?} opened the store");
    }
    assert_eq!(home.counts(), [0, 0, 0], "a refusal wrote a row");
}

#[test]
fn an_agent_reaches_exactly_the_verbs_its_capabilities_name() {
    let mut home = Home::new();
    let mut shared = 0;
    for (i, verb) in Verb::ALL.into_iter().enumerate() {
        if let Rule::Shared(..) = verb.rule() {
            home.probe(&Envelope::local_agent(), verb, &key('A', i))
                .unwrap();
            shared += 1;
        }
    }
    assert_eq!(shared, 13);
    assert_eq!(home.counts(), [13, 13, 13]);
    assert_eq!(home.event_actors(), ["project_agent:local-agent"]);

    let reader = Envelope::agent("reader", Capabilities::parse("t", &["project.read"]));
    for verb in Verb::ALL {
        let outcome = reader.authorize(verb);
        match verb.rule() {
            Rule::Shared(_, Capability::ProjectRead) => outcome.unwrap(),
            Rule::Shared(_, needed) => assert_eq!(
                refusal(outcome),
                (
                    "project_agent_scope",
                    format!(
                        "This grant does not carry the {} capability",
                        needed.as_str()
                    )
                ),
                "{verb:?}"
            ),
            Rule::HumanOnly(_) => assert_eq!(
                refusal(outcome),
                (
                    "project_agent_scope",
                    "This command requires a signed-in human".to_string()
                ),
                "{verb:?}"
            ),
        }
    }
}

#[test]
fn a_member_is_refused_every_lead_verb_and_a_lead_is_refused_delete() {
    use Verb as V;
    const RUN: &str = "Only the project lead or a workspace admin may run this command";
    const REVIEW: &str = "Only the project lead or a workspace admin may review a proposal";
    const HEALTH: &str = "Only the project lead or a workspace admin may report health";
    const DELETE: &str = "Only a workspace owner or admin can delete a project";
    let above_member = [
        (V::HealthUpdate, HEALTH),
        (V::Archive, RUN),
        (V::Restore, RUN),
        (V::Pause, RUN),
        (V::Resume, RUN),
        (V::ProposalApprove, REVIEW),
        (V::ProposalRequestChanges, REVIEW),
        (V::ProposalReject, REVIEW),
        (V::ProjectComplete, RUN),
        (V::ProjectCancel, RUN),
        (V::ProjectDelete, DELETE),
    ];
    let member = Envelope::user("mia", Tier::Member);
    let lead = Envelope::user("lee", Tier::Lead);
    let mut home = Home::new();
    for (i, verb) in Verb::ALL.into_iter().enumerate() {
        let before = home.counts();
        let as_member = home.probe(&member, verb, &key('M', i));
        match above_member.iter().find(|(v, _)| *v == verb) {
            Some((_, sentence)) => {
                assert_eq!(refusal(as_member), ("forbidden", (*sentence).to_string()));
                assert_eq!(home.counts(), before, "{verb:?} wrote as a member");
            }
            None => {
                as_member.unwrap();
            }
        }
        let as_lead = home.probe(&lead, verb, &key('L', i));
        if verb == V::ProjectDelete {
            assert_eq!(refusal(as_lead), ("forbidden", DELETE.to_string()));
        } else {
            as_lead.unwrap();
        }
    }
    // 16 member verbs as the member, 26 of 27 as the lead.
    assert_eq!(home.counts(), [42, 42, 42]);
}

#[test]
fn the_local_operator_admits_every_verb_as_itself() {
    let operator = Envelope::local_operator();
    let mut home = Home::new();
    for (i, verb) in Verb::ALL.into_iter().enumerate() {
        home.probe(&operator, verb, &key('O', i)).unwrap();
    }
    assert_eq!(home.counts(), [27, 27, 27]);
    assert_eq!(home.event_actors(), ["user:local-operator"]);
}

#[test]
fn a_human_actor_round_trips_through_the_row_the_event_and_the_reads() {
    let alice = Envelope::user("alice", Tier::Member);
    let mut home = Home::new();
    let created = home.exec(&alice, charter("ALICE")).unwrap().project;
    assert_eq!(
        (created.created_by.as_str(), created.lead_user_id.as_str()),
        ("alice", "alice")
    );
    let id = created.id.clone();
    let shown = home.exec(&alice, show::Request { id: id.clone() }).unwrap();
    assert_eq!(shown.project, created);
    let listed = home
        .exec(&Envelope::local_agent(), list::Request::default())
        .unwrap();
    assert_eq!(listed.projects, vec![created]);

    let row = project_read::project(&home.conn, &id).unwrap().unwrap();
    assert_eq!(
        [
            row.creator_principal_type.as_str(),
            row.creator_principal_id.as_str(),
            row.lead_principal_type.as_str(),
            row.lead_principal_id.as_str(),
        ],
        ["user", "alice", "user", "alice"]
    );
    assert_eq!(home.event_actors(), ["user:alice"]);
}
