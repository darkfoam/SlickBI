//! MVP-01: a 500k-row CSV reaches the data pane within three seconds.
//!
//! The test writes the file outside the measured section, then uses `Session::load_path`, which is
//! exactly what the window's drag-and-drop handler calls. A CSV has no sheet picker, so the drop
//! cannot put a modal in front of the view.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use slickbi_app::Session;
use slickbi_spec::{DataType, Role};

const ROWS: usize = 500_000;
const BUDGET: Duration = Duration::from_secs(3);

#[test]
fn a_500k_row_csv_reaches_the_data_pane_within_three_seconds() {
    let path = std::env::temp_dir().join(format!(
        "slickbi-mvp01-{}-{}.csv",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    write_csv(&path);

    // The only picker in the open path is the spreadsheet sheet picker; a CSV never reaches it.
    assert!(
        slickbi_connectors::SpreadsheetKind::from_path(&path).is_none(),
        "a CSV does not open a sheet modal"
    );

    let mut session = Session::new();
    let started = Instant::now();
    session.load_path(&path).expect("loads the CSV");
    let elapsed = started.elapsed();
    println!(
        "MVP-01: {ROWS} rows loaded into the model in {:.3}s",
        elapsed.as_secs_f64()
    );

    assert!(
        elapsed < BUDGET,
        "a {ROWS}-row CSV took {elapsed:?} to load, over the {BUDGET:?} MVP-01 budget"
    );

    let model = &session.workbook().models[0];
    assert_eq!(model.fields.len(), 12, "every field reaches the pane");
    let field = |name: &str| {
        model
            .fields
            .iter()
            .find(|field| field.name == name)
            .unwrap_or_else(|| panic!("missing field {name}"))
    };
    assert_eq!(field("id").data_type, DataType::Integer);
    assert_eq!(field("name").data_type, DataType::Text);
    assert_eq!(field("category").data_type, DataType::Text);
    assert_eq!(field("day").data_type, DataType::Date);
    assert_eq!(field("amount").data_type, DataType::Number);
    assert_eq!(field("quantity").data_type, DataType::Integer);
    assert_eq!(field("ratio").data_type, DataType::Number);
    assert_eq!(field("active").data_type, DataType::Boolean);
    assert_eq!(field("region").data_type, DataType::Text);
    assert_eq!(field("note").data_type, DataType::Text);
    assert_eq!(field("code").data_type, DataType::Text);
    assert_eq!(field("balance").data_type, DataType::Number);

    let (dimensions, measures) = session.field_groups();
    assert_eq!(
        dimensions.len() + measures.len(),
        12,
        "the data pane lists every field"
    );
    assert_eq!(
        dimensions
            .iter()
            .find(|field| field.name == "region")
            .map(|field| field.role),
        Some(Role::Dimension)
    );
    assert_eq!(
        measures
            .iter()
            .find(|field| field.name == "amount")
            .map(|field| field.role),
        Some(Role::Measure)
    );

    std::fs::remove_file(&path).ok();
}

fn write_csv(path: &Path) {
    let file = File::create(path).expect("creates the CSV");
    let mut out = BufWriter::new(file);
    writeln!(
        out,
        "id,name,category,day,amount,quantity,ratio,active,region,note,code,balance"
    )
    .expect("writes the header");
    let dates = [
        "2025-01-01",
        "2025-02-01",
        "2025-03-01",
        "2025-04-01",
        "2025-05-01",
        "2025-06-01",
        "2025-07-01",
        "2025-08-01",
        "2025-09-01",
        "2025-10-01",
        "2025-11-01",
        "2025-12-01",
    ];
    let categories = ["Alpha", "Beta", "Gamma"];
    let regions = ["North", "South", "West"];

    for row in 0..ROWS {
        let amount = 10.0 + (row % 10_000) as f64 * 0.37;
        let ratio = (row % 1_000) as f64 / 1_000.0;
        writeln!(
            out,
            "{row},Name {row},{},{},{amount:.2},{},{ratio:.4},{},{},Note {row},C-{row:06},{:.2}",
            categories[row % categories.len()],
            dates[row % dates.len()],
            row % 100,
            row % 2 == 0,
            regions[row % regions.len()],
            amount * 2.0
        )
        .expect("writes a row");
    }
    out.flush().expect("flushes the CSV");
}
