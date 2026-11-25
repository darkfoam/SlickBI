use iwork::Document;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("output path for the Numbers fixture");
    let mut doc =
        Document::new_spreadsheet("Transactions", "Transactions", 4, 8).expect("new spreadsheet");
    let mut table = doc.table_mut("Transactions").expect("table");

    let header = [
        "Account",
        "Description",
        "Day",
        "Type",
        "Amount",
        "Balance",
        "Derived",
        "Note",
    ];
    for (index, text) in header.iter().enumerate() {
        table.set((0, index), *text).expect("writes a header cell");
    }

    let rows = [
        (
            1001,
            "Coffee shop",
            "01/02/25",
            "Debit",
            -4.25,
            1245.75,
            4.25,
            "A-1",
        ),
        (
            1001, "Payroll", "01/03/25", "Credit", 2000.0, 3245.75, -2000.0, "B-2",
        ),
        (
            1001, "Rent", "01/04/25", "Debit", -1200.0, 2045.75, 1200.0, "C-3",
        ),
    ];
    for (row, (account, description, day, kind, amount, balance, derived, note)) in
        rows.into_iter().enumerate()
    {
        let row = row + 1;
        table.set((row, 0), account).expect("account");
        table.set((row, 1), description).expect("description");
        table.set((row, 2), day).expect("day");
        table.set((row, 3), kind).expect("type");
        table.currency((row, 4), amount, "USD").expect("amount");
        table.currency((row, 5), balance, "USD").expect("balance");
        table
            .formula((row, 6), &format!("=-E{}", row + 1), derived)
            .expect("derived");
        table.set((row, 7), note).expect("note");
    }

    doc.save(&path).expect("saves");
    println!("wrote {path}");
}
