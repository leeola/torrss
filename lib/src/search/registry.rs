//! The parsers and searches the running process reads titles with, kept
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

use super::Search;
use super::store::SearchStore;
use crate::parser::Parser;
use crate::parser::shipped;
use crate::parser::store::ParserStore;
use crate::rules::{Engine, EngineError};

/// The compiled parsers and searches, rebuilt after every write.
pub(crate) struct Searches {
    store: SearchStore,
    parsers: ParserStore,
    engine: RwLock<Arc<Engine>>,
}

/// Why the stored parsers and searches do not become a running engine.
#[derive(Debug, Snafu)]
#[snafu(module)]
pub(crate) enum LoadError {
    #[snafu(display("the stored rules could not be read: {source}"))]
    Store { source: sqlx::Error },

    #[snafu(display("the stored rules do not compile: {source}"))]
    Engine { source: EngineError },
}

/// Why a write to the searches did not happen.
#[derive(Debug, Snafu)]
pub(crate) enum SaveError {
    #[snafu(display("the search could not be written: {source}"))]
    Store { source: sqlx::Error },

    /// The set the write produces does not compile.
    ///
    /// Reported before the table is touched, so the stored set stays one the
    /// process runs.
    #[snafu(display("the search does not compile: {source}"))]
    Engine { source: EngineError },

    #[snafu(display("{id} is what another search reads with"))]
    InUse { id: String },

    /// The binary carries the parser, so no table row stands behind it.
    #[snafu(display("{id} is built in. Copy it to change it."))]
    BuiltIn { id: String },
}

impl Searches {
    /// Reads every stored parser and search and compiles them.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Engine`] when the stored set does not compile.
    /// A set written through [`Self::save`] or [`Self::save_parser`] always
    /// does, so this reports a table edited outside the application.
    pub(crate) async fn load(store: SearchStore, parsers: ParserStore) -> Result<Self, LoadError> {
        let engine = Engine::new(
            with_shipped(parsers.list().await.context(load_error::StoreSnafu)?),
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

    /// Writes `search`, replacing the stored one of the same id.
    ///
    /// # Errors
    ///
    /// Returns [`SaveError::Engine`] when the resulting set does not
    /// compile, before anything is written. A pattern the reader broke
    /// mid-edit therefore leaves the stored rules as they were.
    #[allow(
        dead_code,
        reason = "the search editor's Save posts to a route that writes through this"
    )]
    pub(crate) async fn save(&self, search: Search) -> Result<(), SaveError> {
        let engine = self.rebuilt_with(search.clone())?;

        self.store.upsert(&search).await.context(StoreSnafu)?;
        self.swap(engine);

        Ok(())
    }

    /// Removes the search `id`, and reports whether one was there.
    #[allow(
        dead_code,
        reason = "the search editor's Delete posts to a route that writes through this"
    )]
    pub(crate) async fn remove(&self, id: &str) -> Result<bool, SaveError> {
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
    ///
    /// Returns [`SaveError::BuiltIn`] for a parser the binary carries. The
    /// reader copies one to write their own version of it.
    pub(crate) async fn save_parser(&self, parser: Parser) -> Result<(), SaveError> {
        self.refuse_built_in(&parser.id)?;

        let engine = self.rebuilt_with_parser(parser.clone())?;

        self.parsers.upsert(&parser).await.context(StoreSnafu)?;
        self.swap(engine);

        Ok(())
    }

    /// Removes the parser `id`, and reports whether one was there.
    ///
    /// # Errors
    ///
    /// Returns [`SaveError::InUse`] while a search reads with it. A search
    /// whose parser is gone reads no title, so the reader removes those
    /// first.
    ///
    /// Returns [`SaveError::BuiltIn`] for a parser the binary carries, which
    /// no table row stands behind.
    pub(crate) async fn remove_parser(&self, id: &str) -> Result<bool, SaveError> {
        self.refuse_built_in(id)?;

        {
            let engine = self.read();

            let Some(parser) = engine.parser(id) else {
                return Ok(false);
            };

            if engine.searches_on(parser).next().is_some() {
                return InUseSnafu { id }.fail();
            }
        }

        if !self.parsers.remove(id).await.context(StoreSnafu)? {
            return Ok(false);
        }

        self.reload().await?;

        Ok(true)
    }

    /// Switches the search `id` on or off, and reports whether one was
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

    /// Refuses a write to a parser the binary carries, before any table is
    /// touched.
    fn refuse_built_in(&self, id: &str) -> Result<(), SaveError> {
        if self.read().parser(id).is_some_and(|parser| parser.built_in) {
            return BuiltInSnafu { id }.fail();
        }

        Ok(())
    }

    /// Compiles the running set with `search` replaced or appended.
    ///
    /// The whole set compiles rather than the one search alone, because the
    /// engine is built from a set and every parser beside it has to keep
    /// compiling too.
    fn rebuilt_with(&self, search: Search) -> Result<Arc<Engine>, SaveError> {
        let engine = self.read();

        let mut searches = engine
            .searches()
            .filter(|stored| stored.id != search.id)
            .cloned()
            .collect::<Vec<_>>();

        searches.push(search);

        Ok(Arc::new(
            Engine::new(engine.parsers().cloned().collect(), searches).context(EngineSnafu)?,
        ))
    }

    /// Compiles the running set with `parser` replaced or appended.
    ///
    /// The whole set compiles rather than the one parser alone, because a
    /// set is what the engine is built from and the searches beside it have
    /// to keep compiling too.
    fn rebuilt_with_parser(&self, parser: Parser) -> Result<Arc<Engine>, SaveError> {
        let engine = self.read();

        let mut parsers = engine
            .parsers()
            .filter(|stored| !stored.built_in && stored.id != parser.id)
            .cloned()
            .collect::<Vec<_>>();

        parsers.push(parser);

        Ok(Arc::new(
            Engine::new(with_shipped(parsers), engine.searches().cloned().collect())
                .context(EngineSnafu)?,
        ))
    }

    /// Recompiles from the tables, so the write and the running engine agree.
    ///
    /// Both stores answer before the lock is taken. A guard is not `Send`, so
    /// one held across an await makes the whole handler future not `Send`,
    /// which the router refuses.
    async fn reload(&self) -> Result<(), SaveError> {
        let engine = Engine::new(
            with_shipped(self.parsers.list().await.context(StoreSnafu)?),
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
            .expect("the search engine lock is never poisoned") = engine;
    }

    fn read(&self) -> RwLockReadGuard<'_, Arc<Engine>> {
        self.engine
            .read()
            .expect("the search engine lock is never poisoned")
    }
}

/// Puts `stored` ahead of the parsers the binary carries.
///
/// A stored parser reads first because the reader wrote it for the trackers
/// they follow, and a copy of a shipped parser is how they replace one. The
/// shipped order behind it carries the classification, most specific first.
fn with_shipped(stored: Vec<Parser>) -> Vec<Parser> {
    let mut parsers = stored;
    parsers.extend(shipped::parsers());

    parsers
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    use super::{SaveError, Searches, shipped};
    use crate::parser::store::ParserStore;
    use crate::parser::{Field, FieldKind, Parser};
    use crate::search::Search;
    use crate::search::store::SearchStore;

    fn search(id: &str, parser: &str) -> Search {
        Search {
            id: id.to_owned(),
            name: id.to_owned(),
            enabled: false,
            parser: parser.to_owned(),
            conditions: Vec::new(),
            tests: Vec::new(),
        }
    }

    async fn loaded(pool: &SqlitePool) -> Searches {
        Searches::load(
            SearchStore::new(pool.clone()),
            ParserStore::new(pool.clone()),
        )
        .await
        .expect("the stored set compiles")
    }

    /// A registry over `pool` with `shows` already saved as a parser.
    async fn with_parser(pool: &SqlitePool) -> Searches {
        let searches = loaded(pool).await;
        searches
            .save_parser(parser("shows", r"^(?<show>\w+)"))
            .await
            .expect("the parser the searches read with");

        searches
    }

    #[sqlx::test]
    async fn an_empty_database_loads_an_engine_with_no_searches(pool: SqlitePool) {
        assert_eq!(loaded(&pool).await.engine().searches().count(), 0);
    }

    #[sqlx::test]
    async fn a_saved_search_reaches_the_engine_and_the_table(pool: SqlitePool) {
        let searches = with_parser(&pool).await;
        searches
            .save(search("hollow", "shows"))
            .await
            .expect("save");

        assert_eq!(
            searches.engine().search("hollow"),
            Some(&search("hollow", "shows")),
            "the running engine sees the save"
        );
        assert_eq!(
            loaded(&pool).await.engine().searches().count(),
            1,
            "and so does a process that starts after it"
        );
    }

    #[sqlx::test]
    async fn a_search_on_an_absent_parser_is_never_written(pool: SqlitePool) {
        let searches = loaded(&pool).await;
        let outcome = searches.save(search("hollow", "absent")).await;

        assert!(
            matches!(outcome, Err(SaveError::Engine { .. })),
            "a parser no declaration carries is reported rather than stored"
        );
        assert_eq!(
            loaded(&pool).await.engine().searches().count(),
            0,
            "the table is untouched"
        );
    }

    #[sqlx::test]
    async fn a_search_on_a_shipped_parser_is_written(pool: SqlitePool) {
        loaded(&pool)
            .await
            .save(search("hollow", "series"))
            .await
            .expect("a shipped parser needs no row");

        assert_eq!(
            loaded(&pool).await.engine().search("hollow"),
            Some(&search("hollow", "series")),
            "a process that starts after it reads the row"
        );
    }

    #[sqlx::test]
    async fn removing_a_parser_a_search_reads_with_is_refused(pool: SqlitePool) {
        let searches = with_parser(&pool).await;
        searches
            .save(search("hollow", "shows"))
            .await
            .expect("the search on it");

        assert!(
            matches!(
                searches.remove_parser("shows").await,
                Err(SaveError::InUse { .. })
            ),
            "a search whose parser is gone reads no title"
        );
        assert!(
            searches.remove("hollow").await.expect("it goes"),
            "the search itself removes"
        );
        assert!(
            searches.remove_parser("shows").await.expect("now free"),
            "and the parser follows once nothing reads with it"
        );
    }

    #[sqlx::test]
    async fn set_enabled_shows_in_the_next_engine(pool: SqlitePool) {
        let searches = with_parser(&pool).await;
        searches
            .save(search("hollow", "shows"))
            .await
            .expect("save");

        assert!(
            !searches.engine().search("hollow").expect("stored").enabled,
            "a saved search starts switched off"
        );
        assert!(
            searches.set_enabled("hollow", true).await.expect("enable"),
            "a stored row"
        );
        assert!(
            searches.engine().search("hollow").expect("stored").enabled,
            "the flip reaches the running engine"
        );
        assert!(
            !searches.set_enabled("absent", true).await.expect("unknown"),
            "no row to flip"
        );
    }

    /// A parser over one show field, which is all the compile step reads.
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
            built_in: false,
        }
    }

    #[sqlx::test]
    async fn a_saved_parser_reaches_the_engine_and_the_table(pool: SqlitePool) {
        let searches = loaded(&pool).await;
        searches
            .save_parser(parser("shows", r"^(?<show>\w+)"))
            .await
            .expect("save");

        assert_eq!(
            searches.engine().parser("shows"),
            Some(&parser("shows", r"^(?<show>\w+)")),
            "the running engine sees the save"
        );
        assert_eq!(
            loaded(&pool)
                .await
                .engine()
                .parsers()
                .filter(|parser| !parser.built_in)
                .count(),
            1,
            "and so does a process that starts after it"
        );
    }

    #[sqlx::test]
    async fn a_parser_that_does_not_compile_is_never_written(pool: SqlitePool) {
        let searches = loaded(&pool).await;
        let outcome = searches.save_parser(parser("shows", "(")).await;

        assert!(
            matches!(outcome, Err(SaveError::Engine { .. })),
            "a broken pattern is reported rather than stored"
        );
        assert_eq!(
            loaded(&pool)
                .await
                .engine()
                .parsers()
                .filter(|parser| !parser.built_in)
                .count(),
            0,
            "the table is untouched"
        );
    }

    #[sqlx::test]
    async fn remove_parser_reports_whether_one_was_there(pool: SqlitePool) {
        let searches = loaded(&pool).await;
        searches
            .save_parser(parser("shows", r"^(?<show>\w+)"))
            .await
            .expect("save");

        assert!(searches.remove_parser("shows").await.expect("remove"));
        assert_eq!(
            searches
                .engine()
                .parsers()
                .filter(|parser| !parser.built_in)
                .count(),
            0
        );
        assert!(
            !searches.remove_parser("shows").await.expect("remove"),
            "an id no parser carries reports the same absence"
        );
    }

    #[sqlx::test]
    async fn a_fresh_database_loads_the_shipped_parsers(pool: SqlitePool) {
        assert_eq!(
            ids(&loaded(&pool).await),
            shipped::parsers()
                .into_iter()
                .map(|parser| parser.id)
                .collect::<Vec<_>>(),
            "a reader who has written nothing still reads a title"
        );
    }

    #[sqlx::test]
    async fn a_saved_parser_reads_before_the_shipped_set(pool: SqlitePool) {
        let searches = loaded(&pool).await;
        searches
            .save_parser(parser("series-copy", r"^(?<show>\w+)"))
            .await
            .expect("save");

        assert_eq!(
            ids(&searches).first().map(String::as_str),
            Some("series-copy"),
            "the reader wrote it for their own trackers, so it reads first"
        );
    }

    #[sqlx::test]
    async fn a_built_in_parser_is_never_saved_or_removed(pool: SqlitePool) {
        let searches = loaded(&pool).await;

        assert!(
            matches!(
                searches
                    .save_parser(parser("series", r"^(?<show>\w+)"))
                    .await,
                Err(SaveError::BuiltIn { .. })
            ),
            "the binary carries it, so no row stands behind the write"
        );
        assert!(
            matches!(
                searches.remove_parser("series").await,
                Err(SaveError::BuiltIn { .. })
            ),
            "and none stands behind the removal either"
        );
    }

    #[sqlx::test]
    async fn a_copy_of_a_built_in_parser_saves_under_its_own_id(pool: SqlitePool) {
        let searches = loaded(&pool).await;

        let copy = Parser {
            id: "series-copy".to_owned(),
            built_in: false,
            ..shipped::parsers()
                .into_iter()
                .find(|parser| parser.id == "series")
                .expect("the shipped set carries the series parser")
        };

        searches.save_parser(copy.clone()).await.expect("save");

        assert_eq!(
            searches.engine().parsers().next(),
            Some(&copy),
            "a copy is the reader's own parser, so it reads ahead of the set it came from"
        );
    }

    /// Every parser the engine carries, in declaration order.
    fn ids(searches: &Searches) -> Vec<String> {
        searches
            .engine()
            .parsers()
            .map(|parser| parser.id.clone())
            .collect()
    }
}
