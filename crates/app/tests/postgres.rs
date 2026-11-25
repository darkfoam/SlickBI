//! MVP-02: a read-only PostgreSQL table reaches the field list and first chart within two seconds.
//!
//! The test skips when `SLICKBI_TEST_PG` is absent, so `cargo xtask ci` stays green without a
//! server. Run it against the container described in `docs/MVP.md`:
//!
//! ```sh
//! SLICKBI_TEST_PG=127.0.0.1:55432/demo \
//! SLICKBI_TEST_PG_USER=reader SLICKBI_TEST_PG_PASS=readerpass \
//!   cargo test -p slickbi-app --test postgres -- --nocapture
//! ```
//!
//! `psql -U postgres -d demo -c '\d sales'` reports:
//!
//! ```text
//!  region      | text                        | not null
//!  amount      | numeric(12,2)               |
//!  units       | integer                     |
//!  ratio       | double precision            |
//!  active      | boolean                     |
//!  day         | date                        |
//!  recorded_at | timestamp without time zone |
//!  note        | character varying(200)      |
//! ```
//!
//! The assertions below are that field list, mapped to the application's types.

use std::time::{Duration, Instant};

use slickbi_app::{Session, Shelf};
use slickbi_connectors::DatabaseKind;
use slickbi_spec::{DataType, FieldId};

const BUDGET: Duration = Duration::from_secs(2);

#[test]
fn a_read_only_postgres_table_reaches_the_first_chart_within_two_seconds() {
    let Ok(location) = std::env::var("SLICKBI_TEST_PG") else {
        return;
    };
    let user = std::env::var("SLICKBI_TEST_PG_USER").ok();
    let password = std::env::var("SLICKBI_TEST_PG_PASS").ok();

    let mut session = Session::new();
    let started = Instant::now();
    session
        .connect_database(
            DatabaseKind::Postgres,
            &location,
            user.as_deref(),
            password.as_deref(),
            "sales",
        )
        .expect("connects to PostgreSQL with the read-only user");
    let connected = started.elapsed();

    // The field list, exactly as `\d sales` reports it, mapped to the application's types.
    let model = &session.workbook().models[0];
    assert_eq!(model.fields.len(), 8, "every column reaches the data pane");
    let field = |name: &str| {
        model
            .fields
            .iter()
            .find(|field| field.name == name)
            .unwrap_or_else(|| panic!("missing field {name}"))
    };
    assert_eq!(field("region").data_type, DataType::Text);
    assert_eq!(field("amount").data_type, DataType::Number);
    assert_eq!(field("units").data_type, DataType::Integer);
    assert_eq!(field("ratio").data_type, DataType::Number);
    assert_eq!(field("active").data_type, DataType::Boolean);
    assert_eq!(field("day").data_type, DataType::Date);
    assert_eq!(field("recorded_at").data_type, DataType::Timestamp);
    assert_eq!(field("note").data_type, DataType::Text);

    let (dimensions, measures) = session.field_groups();
    assert_eq!(
        dimensions.len() + measures.len(),
        8,
        "the pane lists every field"
    );
    assert_eq!(dimensions.len(), 5);
    assert_eq!(measures.len(), 3);

    session.assign(Shelf::X, &FieldId::new("region"));
    session.assign(Shelf::Y, &FieldId::new("amount"));
    let scene = session
        .scene()
        .unwrap_or_else(|| panic!("first chart: {:?}", session.error()));
    assert_eq!(scene.series.len(), 1);
    assert!(
        matches!(
            scene.series[0].kind,
            slickbi_scene::SeriesKind::Bars(_)
        ),
        "a region by amount chart is bars"
    );

    let first_chart = started.elapsed();
    println!(
        "MVP-02: connected and imported in {connected:.3?}; field list + first chart in {first_chart:.3?}"
    );
    assert!(
        first_chart < BUDGET,
        "first chart took {first_chart:?}, over the {BUDGET:?} MVP-02 budget"
    );
}
