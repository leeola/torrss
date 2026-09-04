//! The parsers and rulesets the running process reads titles with, kept
//! compiled.
//!
//! Both are data until they compile. Every page and every pass reads titles
//! through an [`Engine`], and a fresh one per read recompiles the same
//! regexes for every row of every listing.
//!
//! So the compiled engine lives here, beside the stores that produced it. A
//! write compiles the whole set first and only then touches a table, which
//! is what keeps the stored set to rules the process runs.

use std::sync::{Arc, RwLock, RwLockReadGuard};

use snafu::{ResultExt, Snafu};

use super::Ruleset;
use super::store::RulesetStore;
use crate::parser::Parser;
use crate::parser::store::ParserStore;
use crate::rules::{Engine, EngineError};

/// The compiled parsers and rulesets, rebuilt after every write.
pub(crate) struct Rulesets {
    store: RulesetStore,
    parsers: ParserStore,
    engine: RwLock<Arc<Engine>>,
}

/// Why the stored parsers and rulesets do not become a running engine.
#[derive(Debug, Snafu)]
#[snafu(module)]
pub(crate) enum LoadError {
    #[snafu(display("the stored rules could not be read: {source}"))]
    Store { source: sqlx::Error },

    #[snafu(display("the stored rules do not compile: {source}"))]
    Engine { source: EngineError },
}

/// Why a write to the rulesets did not happen.
#[derive(Debug, Snafu)]
pub(crate) enum SaveError {
    #[snafu(display("the ruleset could not be written: {source}"))]
    Store { source: sqlx::Error },

    /// The set the write produces does not compile.
    ///
    /// Reported before the table is touched, so the stored set stays one the
    /// process runs.
    #[snafu(display("the ruleset does not compile: {source}"))]
    Engine { source: EngineError },

    #[snafu(display("ruleset {id} is the template of another ruleset"))]
    InUse { id: String },
}

impl Rulesets {
    /// Reads every stored parser and ruleset and compiles them.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Engine`] when the stored set does not compile.
    /// A set written through [`Self::save`] or [`Self::save_parser`] always
    /// does, so this reports a table edited outside the application.
    pub(crate) async fn load(store: RulesetStore, parsers: ParserStore) -> Result<Self, LoadError> {
        let engine = Engine::new(
            parsers.list().await.context(load_error::StoreSnafu)?,
            store.list().await.context(load_error::StoreSnafu)?,
        )
        .context(load_error::EngineSnafu)?;

        Ok(Self {
            store,
            parsers,
            engine: RwLock::new(Arc::new(engine)),
        })
    }

    /// Returns the engine as it stands.
    ///
    /// A caller holds the snapshot for as long as it needs one. A request
    /// that reads the engine twice otherwise sees a save land between the
    /// two reads and renders one page against two different rule sets.
    pub(crate) fn engine(&self) -> Arc<Engine> {
        Arc::clone(&self.read())
    }

    /// Writes `ruleset`, replacing the stored one of the same id.
    ///
    /// A template is written disabled whatever the caller passed. It claims
    /// nothing and the editor offers it no switch, so a flag it cannot show
    /// is one the reader cannot clear.
    ///
    /// # Errors
    ///
    /// Returns [`SaveError::Engine`] when the resulting set does not
    /// compile, before anything is written. A pattern the reader broke
    /// mid-edit therefore leaves the stored rules as they were.
    #[allow(
        dead_code,
        reason = "the ruleset editor's Save posts to a route that writes through this"
    )]
    pub(crate) async fn save(&self, ruleset: Ruleset) -> Result<(), SaveError> {
        let ruleset = Ruleset {
            enabled: ruleset.enabled && !ruleset.template,
            ..ruleset
        };

        let engine = self.rebuilt_with(ruleset.clone())?;

        self.store.upsert(&ruleset).await.context(StoreSnafu)?;
        self.swap(engine);

        Ok(())
    }

    /// Removes the ruleset `id`, and reports whether one was there.
    ///
    /// # Errors
    ///
    /// Returns [`SaveError::InUse`] while another ruleset is based on this
    /// one. A ruleset whose template is gone resolves no field, so the
    /// reader removes those first.
    #[allow(
        dead_code,
        reason = "the ruleset editor's Delete posts to a route that writes through this"
    )]
    pub(crate) async fn remove(&self, id: &str) -> Result<bool, SaveError> {
        {
            let engine = self.read();

            let Some(ruleset) = engine.ruleset(id) else {
                return Ok(false);
            };

            if engine.derived(ruleset).next().is_some() {
                return InUseSnafu { id }.fail();
            }
        }

        if !self.store.remove(id).await.context(StoreSnafu)? {
            return Ok(false);
        }

        self.reload().await?;

        Ok(true)
    }

    /// Writes `parser`, replacing the stored one of the same id.
    ///
    /// # Errors
    ///
    /// Returns [`SaveError::Engine`] when the resulting set does not
    /// compile, before anything is written. A pattern the reader broke
    /// mid-edit therefore leaves the stored parsers as they were.
    pub(crate) async fn save_parser(&self, parser: Parser) -> Result<(), SaveError> {
        let engine = self.rebuilt_with_parser(parser.clone())?;

        self.parsers.upsert(&parser).await.context(StoreSnafu)?;
        self.swap(engine);

        Ok(())
    }

    /// Removes the parser `id`, and reports whether one was there.
    ///
    /// Nothing parses through a parser yet, so no ruleset holds one back.
    pub(crate) async fn remove_parser(&self, id: &str) -> Result<bool, SaveError> {
        if !self.parsers.remove(id).await.context(StoreSnafu)? {
            return Ok(false);
        }

        self.reload().await?;

        Ok(true)
    }

    /// Switches the ruleset `id` on or off, and reports whether one was
    /// there.
    pub(crate) async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, SaveError> {
        if !self
            .store
            .set_enabled(id, enabled)
            .await
            .context(StoreSnafu)?
        {
            return Ok(false);
        }

        self.reload().await?;

        Ok(true)
    }

    /// Compiles the running set with `ruleset` replaced or appended.
    ///
    /// The whole set compiles rather than the one ruleset alone, because a
    /// ruleset carries its template's fields by reference and an edit to a
    /// template changes what every ruleset on it parses.
    fn rebuilt_with(&self, ruleset: Ruleset) -> Result<Arc<Engine>, SaveError> {
        let engine = self.read();

        let mut rulesets = engine
            .rulesets()
            .filter(|stored| stored.id != ruleset.id)
            .cloned()
            .collect::<Vec<_>>();

        rulesets.push(ruleset);

        Ok(Arc::new(
            Engine::new(engine.parsers().cloned().collect(), rulesets).context(EngineSnafu)?,
        ))
    }

    /// Compiles the running set with `parser` replaced or appended.
    ///
    /// The whole set compiles rather than the one parser alone, because a
    /// set is what the engine is built from and the rulesets beside it have
    /// to keep compiling too.
    fn rebuilt_with_parser(&self, parser: Parser) -> Result<Arc<Engine>, SaveError> {
        let engine = self.read();

        let mut parsers = engine
            .parsers()
            .filter(|stored| stored.id != parser.id)
            .cloned()
            .collect::<Vec<_>>();

        parsers.push(parser);

        Ok(Arc::new(
            Engine::new(parsers, engine.rulesets().cloned().collect()).context(EngineSnafu)?,
        ))
    }

    /// Recompiles from the tables, so the write and the running engine agree.
    ///
    /// Both stores answer before the lock is taken. A guard is not `Send`, so
    /// one held across an await makes the whole handler future not `Send`,
    /// which the router refuses.
    async fn reload(&self) -> Result<(), SaveError> {
        let engine = Engine::new(
            self.parsers.list().await.context(StoreSnafu)?,
            self.store.list().await.context(StoreSnafu)?,
        )
        .context(EngineSnafu)?;

        self.swap(Arc::new(engine));

        Ok(())
    }

    fn swap(&self, engine: Arc<Engine>) {
        // Nothing panics while either guard is held, so the lock never
        // poisons.
        *self
            .engine
            .write()
            .expect("the ruleset engine lock is never poisoned") = engine;
    }

    fn read(&self) -> RwLockReadGuard<'_, Arc<Engine>> {
        self.engine
            .read()
            .expect("the ruleset engine lock is never poisoned")
    }
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    use super::{LoadError, Rulesets, SaveError};
    use crate::parser::store::ParserStore;
    use crate::parser::{Field, FieldKind, Parser};
    use crate::ruleset::Ruleset;
    use crate::ruleset::store::RulesetStore;

    /// The same shape as [`ruleset`], marked as a template so a ruleset is
    /// allowed to be based on it.
    fn template(id: &str, pattern: &str) -> Ruleset {
        Ruleset {
            template: true,
            ..ruleset(id, None, pattern)
        }
    }

    fn ruleset(id: &str, based_on: Option<&str>, pattern: &str) -> Ruleset {
        Ruleset {
            id: id.to_owned(),
            name: id.to_owned(),
            enabled: false,
            template: false,
            based_on: based_on.map(ToOwned::to_owned),
            fields: vec![Field {
                name: "show".to_owned(),
                kind: FieldKind::Text,
                pattern: Some(pattern.to_owned()),
                required: true,
                tight: true,
                identity: true,
            }],
            conditions: Vec::new(),
            tests: Vec::new(),
        }
    }

    async fn loaded(pool: &SqlitePool) -> Rulesets {
        Rulesets::load(
            RulesetStore::new(pool.clone()),
            ParserStore::new(pool.clone()),
        )
        .await
        .expect("the stored set compiles")
    }

    #[sqlx::test]
    async fn an_empty_database_loads_an_engine_with_no_rulesets(pool: SqlitePool) {
        assert_eq!(loaded(&pool).await.engine().rulesets().count(), 0);
    }

    #[sqlx::test]
    async fn a_saved_ruleset_reaches_the_engine_and_the_table(pool: SqlitePool) {
        let rulesets = loaded(&pool).await;
        rulesets
            .save(ruleset("series", None, r"^(?<show>\w+)"))
            .await
            .expect("save");

        assert_eq!(
            rulesets.engine().ruleset("series"),
            Some(&ruleset("series", None, r"^(?<show>\w+)")),
            "the running engine sees the save"
        );
        assert_eq!(
            loaded(&pool).await.engine().rulesets().count(),
            1,
            "and so does a process that starts after it"
        );
    }

    #[sqlx::test]
    async fn a_ruleset_that_does_not_compile_is_never_written(pool: SqlitePool) {
        let rulesets = loaded(&pool).await;
        let outcome = rulesets.save(ruleset("series", None, "(")).await;

        assert!(
            matches!(outcome, Err(SaveError::Engine { .. })),
            "a broken pattern is reported rather than stored"
        );
        assert_eq!(
            loaded(&pool).await.engine().rulesets().count(),
            0,
            "the table is untouched"
        );
    }

    #[sqlx::test]
    async fn removing_a_template_a_ruleset_is_based_on_is_refused(pool: SqlitePool) {
        let rulesets = loaded(&pool).await;
        rulesets
            .save(template("series", r"^(?<show>\w+)"))
            .await
            .expect("the template");
        rulesets
            .save(ruleset("archive", Some("series"), "^Ashfall"))
            .await
            .expect("the ruleset on it");

        assert!(
            matches!(
                rulesets.remove("series").await,
                Err(SaveError::InUse { .. })
            ),
            "a ruleset whose template is gone resolves no field"
        );
        assert!(
            rulesets.remove("archive").await.expect("it goes"),
            "the ruleset itself removes"
        );
        assert!(
            !rulesets.remove("archive").await.expect("nothing left"),
            "an id no ruleset carries reports the same absence"
        );
    }

    #[sqlx::test]
    async fn a_template_is_stored_disabled(pool: SqlitePool) {
        let rulesets = loaded(&pool).await;
        rulesets
            .save(Ruleset {
                enabled: true,
                ..template("series", r"^(?<show>\w+)")
            })
            .await
            .expect("save");

        assert_eq!(
            rulesets.engine().ruleset("series").map(|one| one.enabled),
            Some(false),
            "a template carries no switch"
        );
    }

    #[sqlx::test]
    async fn set_enabled_shows_in_the_next_engine(pool: SqlitePool) {
        let rulesets = loaded(&pool).await;
        rulesets
            .save(ruleset("series", None, r"^(?<show>\w+)"))
            .await
            .expect("save");

        assert!(
            !rulesets.engine().ruleset("series").expect("stored").enabled,
            "a saved ruleset starts switched off"
        );
        assert!(
            rulesets.set_enabled("series", true).await.expect("enable"),
            "a stored row"
        );
        assert!(
            rulesets.engine().ruleset("series").expect("stored").enabled,
            "the flip reaches the running engine"
        );
        assert!(
            !rulesets.set_enabled("absent", true).await.expect("unknown"),
            "no row to flip"
        );
    }

    /// The `based_on` foreign key refuses a template no row carries, but not
    /// one that names a ruleset which is no template.
    ///
    /// That is the shape a stored set takes that the key permits and the
    /// engine still refuses, so the load is where it surfaces.
    #[sqlx::test]
    async fn a_stored_ruleset_based_on_a_non_template_fails_the_load(pool: SqlitePool) {
        let store = RulesetStore::new(pool.clone());
        store
            .upsert(&ruleset("first", None, "^First"))
            .await
            .expect("the first");
        store
            .upsert(&ruleset("second", Some("first"), "^Second"))
            .await
            .expect("a ruleset based on a ruleset");

        assert!(
            matches!(
                Rulesets::load(store, ParserStore::new(pool.clone())).await,
                Err(LoadError::Engine { .. })
            ),
            "only a template serves as one"
        );
    }

    /// The same shape as [`ruleset`], as a parser.
    fn parser(id: &str, pattern: &str) -> Parser {
        Parser {
            id: id.to_owned(),
            name: id.to_owned(),
            fields: vec![Field {
                name: "show".to_owned(),
                kind: FieldKind::Text,
                pattern: Some(pattern.to_owned()),
                required: true,
                tight: true,
                identity: true,
            }],
            tests: Vec::new(),
        }
    }

    #[sqlx::test]
    async fn a_saved_parser_reaches_the_engine_and_the_table(pool: SqlitePool) {
        let rulesets = loaded(&pool).await;
        rulesets
            .save_parser(parser("series", r"^(?<show>\w+)"))
            .await
            .expect("save");

        assert_eq!(
            rulesets.engine().parser("series"),
            Some(&parser("series", r"^(?<show>\w+)")),
            "the running engine sees the save"
        );
        assert_eq!(
            loaded(&pool).await.engine().parsers().count(),
            1,
            "and so does a process that starts after it"
        );
    }

    #[sqlx::test]
    async fn a_parser_that_does_not_compile_is_never_written(pool: SqlitePool) {
        let rulesets = loaded(&pool).await;
        let outcome = rulesets.save_parser(parser("series", "(")).await;

        assert!(
            matches!(outcome, Err(SaveError::Engine { .. })),
            "a broken pattern is reported rather than stored"
        );
        assert_eq!(
            loaded(&pool).await.engine().parsers().count(),
            0,
            "the table is untouched"
        );
    }

    #[sqlx::test]
    async fn remove_parser_reports_whether_one_was_there(pool: SqlitePool) {
        let rulesets = loaded(&pool).await;
        rulesets
            .save_parser(parser("series", r"^(?<show>\w+)"))
            .await
            .expect("save");

        assert!(rulesets.remove_parser("series").await.expect("remove"));
        assert_eq!(rulesets.engine().parsers().count(), 0);
        assert!(
            !rulesets.remove_parser("series").await.expect("remove"),
            "an id no parser carries reports the same absence"
        );
    }
}
